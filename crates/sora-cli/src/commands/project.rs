//! プロジェクト運用コマンド(init / version snapshot / doctor)。

use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::json;
use sora_core::error::CoreError;

use crate::output::CmdResult;

/// プロジェクト雛形のディレクトリ構成(技術要件書 §14)。
const PROJECT_DIRS: [&str; 9] = [
    "devices", "manuals", "imports", "exports", "analysis", "tone", "versions", "logs", "memory",
];

#[derive(clap::Args)]
pub struct InitArgs {
    /// 作成先ディレクトリ(既定: カレントディレクトリ)
    #[arg(default_value = ".")]
    dir: PathBuf,
}

impl InitArgs {
    pub fn run(self) -> CmdResult {
        std::fs::create_dir_all(&self.dir).map_err(|e| CoreError::Io {
            path: self.dir.clone(),
            source: e,
        })?;
        for d in PROJECT_DIRS {
            let path = self.dir.join(d);
            std::fs::create_dir_all(&path).map_err(|e| CoreError::Io { path, source: e })?;
        }

        let mut created = Vec::new();

        // sora.config.json(存在すれば触らない: 非破壊)
        let config_path = self.dir.join("sora.config.json");
        if !config_path.exists() {
            let config = json!({
                "schema_version": "1.0",
                "control_level": 1,
                "devices": [],
                "preferences": { "genres": [], "default_ppq": 480 }
            });
            write_if_absent(
                &config_path,
                &(serde_json::to_string_pretty(&config)? + "\n"),
            )?;
            created.push(config_path.clone());
        }

        // project-context.json
        let context_path = self.dir.join("project-context.json");
        if !context_path.exists() {
            let context = json!({
                "schema_version": "1.0",
                "sections": [],
                "tracks": [],
                "user_notes": []
            });
            write_if_absent(
                &context_path,
                &(serde_json::to_string_pretty(&context)? + "\n"),
            )?;
            created.push(context_path.clone());
        }

        // decision-log.md
        let log_path = self.dir.join("decision-log.md");
        if !log_path.exists() {
            write_if_absent(
                &log_path,
                "# 制作判断ログ\n\n採択・却下とその理由を時系列で記録する。\n",
            )?;
            created.push(log_path.clone());
        }

        Ok(json!({
            "project_root": self.dir,
            "directories": PROJECT_DIRS,
            "created_files": created,
        }))
    }
}

fn write_if_absent(path: &Path, content: &str) -> anyhow::Result<()> {
    std::fs::write(path, content).map_err(|e| CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

#[derive(Subcommand)]
pub enum VersionCommand {
    /// exports/ の現在の内容を versions/<label>/ へスナップショットする
    Snapshot(SnapshotArgs),
}

impl VersionCommand {
    pub fn run(self) -> CmdResult {
        match self {
            VersionCommand::Snapshot(a) => a.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct SnapshotArgs {
    /// スナップショットのラベル(例: "v1")
    label: String,
    /// プロジェクトルート(既定: カレントディレクトリ)
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

impl SnapshotArgs {
    fn run(self) -> CmdResult {
        let exports = self.root.join("exports");
        let dest = self.root.join("versions").join(&self.label);
        if dest.exists() {
            anyhow::bail!(
                "snapshot `{}` already exists at {} (既存バージョンは不変です)",
                self.label,
                dest.display()
            );
        }
        std::fs::create_dir_all(&dest).map_err(|e| CoreError::Io {
            path: dest.clone(),
            source: e,
        })?;

        let mut copied = Vec::new();
        if exports.is_dir() {
            for entry in std::fs::read_dir(&exports).map_err(|e| CoreError::Io {
                path: exports.clone(),
                source: e,
            })? {
                let entry = entry.map_err(|e| CoreError::Io {
                    path: exports.clone(),
                    source: e,
                })?;
                let from = entry.path();
                if from.is_file() {
                    let to = dest.join(entry.file_name());
                    std::fs::copy(&from, &to).map_err(|e| CoreError::Io {
                        path: from.clone(),
                        source: e,
                    })?;
                    copied.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        }

        // CHANGELOG.md へ追記
        append_changelog(&self.root, &self.label, &copied)?;
        super::record_action(
            &self.root,
            "version.snapshot",
            json!({ "label": self.label, "files": copied.len() }),
        );

        Ok(json!({
            "snapshot": dest,
            "files": copied,
        }))
    }
}

fn append_changelog(root: &Path, label: &str, files: &[String]) -> anyhow::Result<()> {
    use std::io::Write;
    let path = root.join("CHANGELOG.md");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| CoreError::Io {
            path: path.clone(),
            source: e,
        })?;
    writeln!(file, "\n## {label}\n").ok();
    for f in files {
        writeln!(file, "- {f}").ok();
    }
    Ok(())
}

/// 環境診断(§5)。仮想 MIDI ポート(IAC Driver / loopMIDI)の検出を含む(§9)。
pub fn doctor() -> CmdResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_path = cwd.join("sora.config.json");
    let config_present = config_path.exists();

    let config = if config_present {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    } else {
        None
    };
    let control_level = config
        .as_ref()
        .and_then(|v| v.get("control_level").and_then(|l| l.as_u64()));
    let configured_port = config
        .as_ref()
        .and_then(|v| v.pointer("/midi/port_name").and_then(|p| p.as_str()))
        .map(String::from);

    let missing_dirs: Vec<&str> = PROJECT_DIRS
        .iter()
        .filter(|d| !cwd.join(d).is_dir())
        .copied()
        .collect();

    Ok(json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "project_root": cwd,
        "config_present": config_present,
        "control_level": control_level,
        "missing_directories": missing_dirs,
        "midi": doctor_midi(configured_port),
        "daw": doctor_daw(&cwd),
    }))
}

/// DAW 統合の診断(§11)。解決されるアダプタと Studio One セットアップ状態。
fn doctor_daw(root: &Path) -> serde_json::Value {
    use sora_core::model::SoraConfig;
    use sora_daw::studio_one::{StudioOnePaths, setup};

    let config: Option<SoraConfig> =
        sora_core::validate::load_validated(&root.join("sora.config.json")).ok();
    let resolved = sora_daw::adapter::resolve_adapter(root, config.as_ref())
        .name()
        .to_string();

    let settings = config
        .as_ref()
        .and_then(|c| c.daw.as_ref())
        .and_then(|d| d.studio_one.as_ref());
    let trigger_port = settings.and_then(|s| s.trigger_port.clone());
    let song_path = settings.and_then(|s| s.song_path.clone());
    let ports = sora_mcp::ops::list_output_ports().unwrap_or_default();
    let trigger_port_found = trigger_port
        .as_deref()
        .map(|name| ports.iter().any(|p| p.contains(name)));
    let song_path_exists = song_path.as_deref().map(|p| Path::new(p).is_file());

    let paths = StudioOnePaths::resolve(config.as_ref());
    let studio_one_present = paths.app_support.is_dir();
    let check = setup::check(&paths);

    let mut hints: Vec<String> = Vec::new();
    if studio_one_present && !check.ok {
        hints.push(
            "Sora Bridge / Sora Surface が未導入または不完全です。`sora daw setup studio-one` を実行してください"
                .to_string(),
        );
    }
    if trigger_port.is_none() {
        hints.push(
            "daw.studio_one.trigger_port が未設定です(Sora Surface 用の仮想 MIDI ポート。演奏用 midi.port_name とは別にする)"
                .to_string(),
        );
    } else if trigger_port_found == Some(false) {
        hints.push(
            "trigger_port がポート一覧に見つかりません。Audio MIDI 設定(IAC)/ loopMIDI でポートを作成してください"
                .to_string(),
        );
    }
    if song_path.is_none() {
        hints.push(
            "daw.studio_one.song_path が未設定です(read_project と書き込み前バックアップ §11.4 に必要)"
                .to_string(),
        );
    }

    json!({
        "resolved_adapter": resolved,
        "studio_one": {
            "present": studio_one_present,
            "app_support": paths.app_support,
            "setup": check,
            "trigger_port": trigger_port,
            "trigger_port_found": trigger_port_found,
            "song_path": song_path,
            "song_path_exists": song_path_exists,
        },
        "hints": hints,
    })
}

/// 仮想 MIDI 送信経路の診断(§9)。ポートの自動作成はせず、手順を提示する。
fn doctor_midi(configured_port: Option<String>) -> serde_json::Value {
    let ports = sora_mcp::ops::list_output_ports().unwrap_or_default();
    let configured_port_found = configured_port
        .as_deref()
        .map(|name| ports.iter().any(|p| p.contains(name)));

    // 仮想 MIDI の代表的なポート名で検出(macOS: IAC / Windows: loopMIDI)
    let virtual_port_present = ports
        .iter()
        .any(|p| p.contains("IAC") || p.to_lowercase().contains("loopmidi"));

    let hint = match (std::env::consts::OS, virtual_port_present) {
        ("macos", false) => Some(
            "仮想 MIDI ポートが見つかりません。Audio MIDI 設定 → ウィンドウ → MIDI スタジオ → IAC ドライバ を開き、「装置はオンライン」を有効化してください。作成後、sora.config.json の midi.port_name にポート名を設定します",
        ),
        ("windows", false) => Some(
            "仮想 MIDI ポートが見つかりません。loopMIDI をインストールしてポートを作成し、sora.config.json の midi.port_name に設定してください",
        ),
        _ => None,
    };

    json!({
        "output_ports": ports,
        "virtual_port_present": virtual_port_present,
        "configured_port": configured_port,
        "configured_port_found": configured_port_found,
        "hint": hint,
    })
}
