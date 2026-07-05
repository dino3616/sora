//! CLI と MCP サーバーで共有する運用ヘルパ。
//!
//! ファイル書き込み(非破壊)・操作ログ・Profile 解決・仮想 MIDI 送信を
//! 一箇所に置き、CLI と MCP の挙動差を防ぐ(技術要件書 §6.4, §8, §9)。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use midir::{MidiOutput, MidiOutputConnection};
use serde::Serialize;
use sora_core::error::CoreError;
use sora_core::midi::{panic_messages, plan_playback};
use sora_core::model::{DeviceProfile, SoraConfig};
use sora_core::validate::load_validated;

/// ファイルへ書き込む(非破壊: 既存パスは上書きしない。技術要件書 §13)。
pub fn write_new_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    sora_core::fsutil::write_new_file(path, bytes)?;
    Ok(())
}

/// `<base>/logs/actions.jsonl` へ 1 行追記する(技術要件書 §8)。
/// ベストエフォート: 失敗しても本処理は成功扱い(ログのために操作を止めない)。
pub fn record_action(base: &Path, action: &str, details: serde_json::Value) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let log_dir = base.join("logs");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = serde_json::json!({ "ts": ts, "action": action, "details": details });
    if let Ok(line) = serde_json::to_string(&entry) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("actions.jsonl"))
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Device Profile を解決する。`profile` 明示があればそれを、なければ
/// `config`(既定: ./sora.config.json)の devices から解決する。
pub fn resolve_profile(
    device_id: &str,
    profile: Option<&Path>,
    config: Option<&Path>,
) -> anyhow::Result<DeviceProfile> {
    if let Some(path) = profile {
        return Ok(load_validated(path)?);
    }
    let config_path = config.unwrap_or_else(|| Path::new("sora.config.json"));
    let cfg: SoraConfig = load_validated(config_path).with_context(|| {
        format!(
            "no explicit profile given and could not read config {} to resolve device `{}`",
            config_path.display(),
            device_id
        )
    })?;
    let entry = cfg
        .devices
        .iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| CoreError::Validation {
            issues: vec![sora_core::error::ValidationIssue {
                pointer: "/device".to_string(),
                code: "DEVICE_NOT_IN_CONFIG".to_string(),
                message: format!(
                    "device `{device_id}` not found in {}",
                    config_path.display()
                ),
                hint: Some(
                    "sora.config.json の devices に追加するか profile パスを明示してください"
                        .to_string(),
                ),
            }],
        })?;
    // profile パスは config からの相対
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(load_validated(&base.join(&entry.profile))?)
}

/// 送信ハンドル。Drop 時に必ず全チャンネルへ All-Notes-Off を送る(RAII、§9)。
struct MidiGuard {
    conn: MidiOutputConnection,
}

impl MidiGuard {
    fn send(&mut self, data: &[u8]) {
        // 個別送信の失敗は握りつぶす(クリーンアップ継続を優先)
        let _ = self.conn.send(data);
    }

    fn all_notes_off(&mut self) {
        for msg in panic_messages() {
            let _ = self.conn.send(&msg);
        }
    }
}

impl Drop for MidiGuard {
    fn drop(&mut self) {
        self.all_notes_off();
    }
}

/// 指定ポート名(部分一致)へ接続する。
fn connect(port_name: &str) -> anyhow::Result<MidiGuard> {
    let output = MidiOutput::new("sora").context("initializing MIDI output")?;
    let ports = output.ports();
    let matched = ports.iter().find(|p| {
        output
            .port_name(p)
            .map(|n| n.contains(port_name))
            .unwrap_or(false)
    });
    let port = matched.ok_or_else(|| {
        let available: Vec<String> = ports
            .iter()
            .filter_map(|p| output.port_name(p).ok())
            .collect();
        CoreError::Io {
            path: PathBuf::from(port_name),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "MIDI ポート `{port_name}` が見つかりません。利用可能: [{}]。macOS は Audio MIDI 設定 → IAC Driver でポートを有効化してください",
                    available.join(", ")
                ),
            ),
        }
    })?;
    let conn = output
        .connect(port, "sora-out")
        .map_err(|e| CoreError::Io {
            path: PathBuf::from(port_name),
            source: std::io::Error::other(e.to_string()),
        })?;
    Ok(MidiGuard { conn })
}

/// 仮想 MIDI 送信の結果(§9)。
#[derive(Debug, Serialize)]
pub struct SendStats {
    /// 接続したポート名(要求値)
    pub port: String,
    /// 送信済みメッセージ数
    pub messages_sent: usize,
    /// 総メッセージ数
    pub total_messages: usize,
    /// 演奏全体の長さ(マイクロ秒)
    pub duration_us: u64,
    /// 中断されたか
    pub aborted: bool,
    /// 使用チャンネル(0 始まり)
    pub channels: Vec<u8>,
}

/// SMF を仮想 MIDI ポートへ実時間送信する(§9)。
/// `interrupted` が true になった時点で中断し、必ず All-Notes-Off を送る。
pub fn send_file(
    file: &Path,
    port_name: &str,
    interrupted: Option<&AtomicBool>,
) -> anyhow::Result<SendStats> {
    let playback =
        plan_playback(file).with_context(|| format!("planning playback of {}", file.display()))?;

    let mut guard = connect(port_name)?;

    let start = Instant::now();
    let mut sent = 0usize;
    let mut aborted = false;
    for msg in &playback.messages {
        if interrupted
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            aborted = true;
            break;
        }
        // 目標時刻まで待機(実時間スケジューリング)
        let target = Duration::from_micros(msg.at_us);
        let elapsed = start.elapsed();
        if target > elapsed {
            std::thread::sleep(target - elapsed);
        }
        guard.send(&msg.data);
        sent += 1;
    }
    // 明示的 all-notes-off(Drop でも走るが完了時点で即座に消音)
    guard.all_notes_off();
    drop(guard);

    Ok(SendStats {
        port: port_name.to_string(),
        messages_sent: sent,
        total_messages: playback.messages.len(),
        duration_us: playback.duration_us,
        aborted,
        channels: playback.channels,
    })
}

/// 全チャンネルへ All-Notes-Off を送る(手動リセット、§9)。
pub fn panic_port(port_name: &str) -> anyhow::Result<()> {
    let mut guard = connect(port_name)?;
    guard.all_notes_off();
    drop(guard);
    Ok(())
}

/// 利用可能な MIDI 出力ポート名を列挙する(doctor 用)。
pub fn list_output_ports() -> anyhow::Result<Vec<String>> {
    let output = MidiOutput::new("sora-doctor").context("initializing MIDI output")?;
    Ok(output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect())
}
