//! JSON Schema レジストリ(技術要件書 §4.6)。
//!
//! Rust 型が単一ソース。`sora schema dump` がここから schemas/ を生成し、
//! `--check` で CI がドリフトを検出する。

use schemars::{JsonSchema, Schema, schema_for};

use crate::error::CoreError;
use crate::model::{DeviceProfile, PartPlan, ProjectContext, SoraConfig};

/// 登録済みスキーマの一覧(名前 → ファイル名は `<name>.schema.json`)。
pub const SCHEMA_NAMES: [&str; 4] = [
    "sora-config",
    "device-profile",
    "project-context",
    "part-plan",
];

/// 名前からスキーマを引く。
pub fn schema_by_name(name: &str) -> Result<Schema, CoreError> {
    match name {
        "sora-config" => Ok(schema_for!(SoraConfig)),
        "device-profile" => Ok(schema_for!(DeviceProfile)),
        "project-context" => Ok(schema_for!(ProjectContext)),
        "part-plan" => Ok(schema_for!(PartPlan)),
        other => Err(CoreError::UnknownSchema {
            name: other.to_string(),
            available: SCHEMA_NAMES.iter().map(|s| s.to_string()).collect(),
        }),
    }
}

/// 全スキーマを (名前, JSON 文字列) で返す。出力は決定論的(キー順固定)。
pub fn dump_all() -> Vec<(&'static str, String)> {
    SCHEMA_NAMES
        .iter()
        .map(|name| {
            // SCHEMA_NAMES 由来の name は必ず登録済みで、schemars Value は常にシリアライズ可能
            #[allow(clippy::expect_used)]
            {
                let schema = schema_by_name(name).expect("registered name");
                let json =
                    serde_json::to_string_pretty(schema.as_value()).expect("schema serializes");
                (*name, json + "\n")
            }
        })
        .collect()
}

/// 型パラメータからスキーマ名を静的に対応付けるためのトレイト。
pub trait NamedSchema: JsonSchema {
    const SCHEMA_NAME: &'static str;
}

impl NamedSchema for SoraConfig {
    const SCHEMA_NAME: &'static str = "sora-config";
}
impl NamedSchema for DeviceProfile {
    const SCHEMA_NAME: &'static str = "device-profile";
}
impl NamedSchema for ProjectContext {
    const SCHEMA_NAME: &'static str = "project-context";
}
impl NamedSchema for PartPlan {
    const SCHEMA_NAME: &'static str = "part-plan";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_dump_and_have_descriptions() {
        for (name, json) in dump_all() {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(
                value.get("description").is_some(),
                "schema `{name}` must have a top-level description (doc comment on the type)"
            );
        }
    }

    #[test]
    fn unknown_schema_lists_available() {
        let err = schema_by_name("nope").unwrap_err();
        assert_eq!(err.code(), "UNKNOWN_SCHEMA");
    }
}
