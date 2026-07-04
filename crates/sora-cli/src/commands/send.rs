//! `sora midi send` / `sora midi panic`(仮想 MIDI 送信、技術要件書 §9)。
//!
//! ノート鳴りっぱなし防止を 3 重で担保する:
//! (a) [`MidiGuard`] の Drop による自動 All-Notes-Off(正常終了・エラー・panic 全経路)
//! (b) `sora midi panic` コマンド(手動リセット)
//! (c) 送信完了時の明示的 all-notes-off
//! さらに Ctrl-C を捕捉して Drop 経路へ合流させる。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use midir::{MidiOutput, MidiOutputConnection};
use serde_json::json;
use sora_core::error::CoreError;
use sora_core::midi::{panic_messages, plan_playback};

use crate::output::CmdResult;

/// 送信ハンドル。Drop 時に必ず全チャンネルへ All-Notes-Off を送る(RAII)。
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
        let available: Vec<String> =
            ports.iter().filter_map(|p| output.port_name(p).ok()).collect();
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

/// Ctrl-C を捕捉するフラグを設定する(多重登録は無視)。
fn install_ctrlc() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    let _ = ctrlc::set_handler(move || {
        f.store(true, Ordering::SeqCst);
    });
    flag
}

#[derive(clap::Args)]
pub struct SendArgs {
    /// 送信する SMF ファイル
    file: PathBuf,
    /// 出力ポート名(部分一致。省略時は sora.config.json の midi.port_name)
    #[arg(long)]
    port: Option<String>,
    /// 参照する sora.config.json(既定: ./sora.config.json)
    #[arg(long)]
    config: Option<PathBuf>,
}

impl SendArgs {
    pub fn run(self) -> CmdResult {
        let port_name = self.resolve_port()?;
        let playback = plan_playback(&self.file)
            .with_context(|| format!("planning playback of {}", self.file.display()))?;

        let mut guard = connect(&port_name)?;
        let interrupted = install_ctrlc();

        let start = Instant::now();
        let mut sent = 0usize;
        let mut aborted = false;
        for msg in &playback.messages {
            if interrupted.load(Ordering::SeqCst) {
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

        Ok(json!({
            "port": port_name,
            "messages_sent": sent,
            "total_messages": playback.messages.len(),
            "duration_us": playback.duration_us,
            "aborted": aborted,
            "channels": playback.channels,
        }))
    }

    fn resolve_port(&self) -> anyhow::Result<String> {
        if let Some(p) = &self.port {
            return Ok(p.clone());
        }
        let config_path = self
            .config
            .clone()
            .unwrap_or_else(|| PathBuf::from("sora.config.json"));
        let cfg: sora_core::model::SoraConfig = sora_core::validate::load_validated(&config_path)
            .with_context(|| {
            format!(
                "no --port given and could not read {} for midi.port_name",
                config_path.display()
            )
        })?;
        cfg.midi.map(|m| m.port_name).ok_or_else(|| {
            anyhow::anyhow!(
                "sora.config.json に midi.port_name がありません。--port で指定してください"
            )
        })
    }
}

#[derive(clap::Args)]
pub struct PanicArgs {
    /// 出力ポート名(部分一致)
    #[arg(long)]
    port: String,
}

impl PanicArgs {
    pub fn run(self) -> CmdResult {
        let mut guard = connect(&self.port)?;
        guard.all_notes_off();
        drop(guard);
        Ok(json!({ "port": self.port, "panic": "all-notes-off sent to 16 channels" }))
    }
}
