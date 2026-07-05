//! control level ゲート(技術要件書 §2.4, §8)。
//!
//! 各 MCP ツールは要求 level を持ち、`sora.config.json` の `control_level` が
//! 上限を宣言する。上限超過の呼び出しは実行前に拒否し、「必要な level と
//! 有効化方法」を返す。level の引き上げはユーザーの明示的な依頼に基づく
//! `sora config set control-level <n>`(CLI)のみで行い、MCP からは変更できない。

use std::fmt;
use std::path::Path;

use serde_json::json;
use sora_core::error::ErrorReport;
use sora_core::model::SoraConfig;
use sora_core::validate::load_validated;

/// ゲート拒否を anyhow 経路(CLI)で運ぶためのラッパ。
/// `report::normalize` が downcast して同一の ErrorReport 表現に戻す(§6.4)。
#[derive(Debug)]
pub struct GateRejection(pub ErrorReport);

impl fmt::Display for GateRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.message)
    }
}

impl std::error::Error for GateRejection {}

/// [`check`] の anyhow 版(CLI コマンド用)。
pub fn require(root: &Path, tool: &str, required: u8) -> anyhow::Result<u8> {
    check(root, tool, required).map_err(|report| anyhow::Error::new(GateRejection(report)))
}

/// config 不在・不正時の既定 level(SoraConfig の既定値と一致させる)。
const DEFAULT_LEVEL: u8 = 1;

/// `<root>/sora.config.json` から現在の control level を読む。
/// config が無い・読めない場合は既定の 1(安全側 = 低い方に倒す)。
pub fn current_level(root: &Path) -> u8 {
    let path = root.join("sora.config.json");
    if !path.exists() {
        return DEFAULT_LEVEL;
    }
    load_validated::<SoraConfig>(&path)
        .map(|c| c.control_level)
        .unwrap_or(DEFAULT_LEVEL)
}

/// 要求 level を検査する。満たせば現在値を、不足なら拒否レポートを返す。
// ErrorReport は details に JSON を持つため Err が大きいが、拒否は例外経路
// なのでコストは問題にならない(Box にすると呼び出し側の JSON 化が煩雑になる)
#[allow(clippy::result_large_err)]
pub fn check(root: &Path, tool: &str, required: u8) -> Result<u8, ErrorReport> {
    let current = current_level(root);
    if current >= required {
        return Ok(current);
    }
    Err(ErrorReport {
        code: "CONTROL_LEVEL_REQUIRED".to_string(),
        message: format!("tool `{tool}` requires control level {required} (current: {current})"),
        details: json!({
            "tool": tool,
            "required_level": required,
            "current_level": current,
        }),
        hint: Some(format!(
            "この操作には control level {required} が必要です。ユーザーに引き上げを依頼し、明示的な承認を得てから CLI で `sora config set control-level {required}` を実行してください(MCP からは変更できません)"
        )),
        chain: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &Path, level: u8) {
        let config = json!({
            "schema_version": "1.0",
            "control_level": level,
            "devices": []
        });
        #[allow(clippy::unwrap_used)] // テストの前提条件
        std::fs::write(
            dir.join("sora.config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn missing_config_defaults_to_level_1() {
        #[allow(clippy::unwrap_used)]
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(current_level(dir.path()), 1);
        assert!(check(dir.path(), "compose_part", 1).is_ok());
        assert!(check(dir.path(), "send_midi", 2).is_err());
    }

    #[test]
    fn rejects_below_required_level_with_hint() {
        #[allow(clippy::unwrap_used)]
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), 1);
        let err = match check(dir.path(), "send_midi", 2) {
            Err(e) => e,
            Ok(_) => panic!("expected rejection"),
        };
        assert_eq!(err.code, "CONTROL_LEVEL_REQUIRED");
        assert_eq!(err.details["required_level"], 2);
        assert!(
            err.hint
                .as_deref()
                .is_some_and(|h| h.contains("sora config set control-level 2"))
        );
    }

    #[test]
    fn allows_at_or_above_required_level() {
        #[allow(clippy::unwrap_used)]
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), 3);
        assert!(matches!(check(dir.path(), "send_midi", 2), Ok(3)));
    }
}
