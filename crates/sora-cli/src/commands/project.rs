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

/// 環境診断(§5)。
pub fn doctor() -> CmdResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_path = cwd.join("sora.config.json");
    let config_present = config_path.exists();

    let control_level = if config_present {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("control_level").and_then(|l| l.as_u64()))
    } else {
        None
    };

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
        "notes": [
            "control level 2+ の仮想 MIDI 送信は Milestone 4 で対応予定",
            "DAW 統合は Milestone 5 で対応予定"
        ]
    }))
}
