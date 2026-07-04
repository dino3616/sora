//! `sora config` サブコマンド。
//!
//! control level の変更はユーザーの明示的な依頼に基づいてのみ行う(技術要件書 §2.4)。
//! Agent はユーザーが自然言語で依頼した場合に限りこのコマンドを実行してよく、
//! 自発的な引き上げは CLAUDE.md の行動規範で禁止される。MCP には公開しない。

use std::path::PathBuf;

use anyhow::Context;
use clap::Subcommand;
use serde_json::json;
use sora_core::error::{CoreError, ValidationIssue};
use sora_core::model::SoraConfig;
use sora_core::validate::load_validated;

use crate::output::CmdResult;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// 設定値を変更する
    #[command(subcommand)]
    Set(SetCommand),
}

impl ConfigCommand {
    pub fn run(self) -> CmdResult {
        match self {
            ConfigCommand::Set(cmd) => cmd.run(),
        }
    }
}

#[derive(Subcommand)]
pub enum SetCommand {
    /// control level(0-5)を変更する。現在値→新値と新たに有効になる操作を表示する
    ControlLevel(ControlLevelArgs),
}

impl SetCommand {
    fn run(self) -> CmdResult {
        match self {
            SetCommand::ControlLevel(a) => a.run(),
        }
    }
}

/// 各 control level で新たに有効になる操作(技術要件書 §2.4)。
const LEVEL_CAPABILITIES: [&str; 6] = [
    "提案のみ(ファイル生成なし)",
    "ファイル書き出し(MIDI コンパイル等)",
    "仮想 MIDI 送信(send_midi)",
    "DAW プロジェクト読み取り(read_daw_project)",
    "DAW 書き込み(write_clip / write_automation / transport)",
    "レンダリング・フルエージェント編集(render_stem)",
];

#[derive(clap::Args)]
pub struct ControlLevelArgs {
    /// 新しい control level(0-5)
    level: u8,
    /// 対象 config(既定: ./sora.config.json)
    #[arg(long)]
    config: Option<PathBuf>,
}

impl ControlLevelArgs {
    fn run(self) -> CmdResult {
        if self.level > 5 {
            return Err(CoreError::Validation {
                issues: vec![ValidationIssue {
                    pointer: "/control_level".to_string(),
                    code: "INVALID_CONTROL_LEVEL".to_string(),
                    message: format!("control_level {} out of range 0..=5", self.level),
                    hint: Some("control level は 0〜5 です".to_string()),
                }],
            }
            .into());
        }

        let config_path = self
            .config
            .unwrap_or_else(|| PathBuf::from("sora.config.json"));
        let mut cfg: SoraConfig = load_validated(&config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        let previous = cfg.control_level;
        cfg.control_level = self.level;

        // Sora 管理ファイル(config)への書き込み。ユーザー由来素材ではないため許可。
        let serialized = serde_json::to_string_pretty(&cfg)? + "\n";
        std::fs::write(&config_path, &serialized).map_err(|e| CoreError::Io {
            path: config_path.clone(),
            source: e,
        })?;

        // 新たに有効/無効になる操作
        let newly_enabled: Vec<&str> = if self.level > previous {
            LEVEL_CAPABILITIES[(previous as usize + 1)..=(self.level as usize)].to_vec()
        } else {
            Vec::new()
        };

        // actions.jsonl へ記録(§8)
        let base = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        super::record_action(
            base,
            "config.set.control-level",
            json!({ "previous": previous, "new": self.level }),
        );

        Ok(json!({
            "config": config_path,
            "previous_level": previous,
            "new_level": self.level,
            "newly_enabled": newly_enabled,
            "note": if self.level > previous {
                "control level を引き上げました。この操作はユーザーの明示的な依頼に基づくものである必要があります。"
            } else {
                "control level を変更しました。"
            }
        }))
    }
}
