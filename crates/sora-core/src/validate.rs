//! 検証 3 層(技術要件書 §4.6)。
//!
//! - L1: jsonschema による構造検証。全エラーを JSON Pointer 付きで一括列挙する。
//!   JSON の書き手が LLM である本プロダクトでは、この網羅的リストが
//!   Agent の自己修正ループの効率を直接決める。
//! - L2: serde(`deny_unknown_fields`)による型付き変換。typo を黙殺しない。
//! - L3: ドメイン検証(相互制約)。各モジュールの resolve/compile が担う。

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{CoreError, ValidationIssue};
use crate::schema::{NamedSchema, schema_by_name};

/// schema_version の major 互換を検証する(§4.6 運用要件)。
fn check_schema_version(value: &Value, issues: &mut Vec<ValidationIssue>) {
    const SUPPORTED_MAJOR: &str = "1";
    let Some(version) = value.get("schema_version").and_then(Value::as_str) else {
        // 欠落自体は L1 の required 検証が報告する
        return;
    };
    let major = version.split('.').next().unwrap_or("");
    if major != SUPPORTED_MAJOR {
        issues.push(ValidationIssue {
            pointer: "/schema_version".to_string(),
            code: "UNSUPPORTED_SCHEMA_VERSION".to_string(),
            message: format!(
                "schema_version `{version}` is not supported (expected major {SUPPORTED_MAJOR})"
            ),
            hint: Some(format!(
                "サポートされる schema_version は {SUPPORTED_MAJOR}.x です"
            )),
        });
    }
}

/// L1: JSON Schema 構造検証。エラーを全件収集する。
pub fn validate_structure(
    schema_name: &str,
    value: &Value,
) -> Result<Vec<ValidationIssue>, CoreError> {
    let schema = schema_by_name(schema_name)?;
    let schema_value = serde_json::to_value(&schema).map_err(|e| CoreError::JsonParse {
        path: format!("<schema:{schema_name}>").into(),
        source: e,
    })?;
    let validator =
        jsonschema::validator_for(&schema_value).map_err(|e| CoreError::Validation {
            issues: vec![ValidationIssue {
                pointer: String::new(),
                code: "SCHEMA_COMPILE_ERROR".to_string(),
                message: e.to_string(),
                hint: None,
            }],
        })?;

    let mut issues: Vec<ValidationIssue> = validator
        .iter_errors(value)
        .map(|err| ValidationIssue {
            pointer: err.instance_path.to_string(),
            code: "SCHEMA_VIOLATION".to_string(),
            message: err.to_string(),
            hint: None,
        })
        .collect();

    check_schema_version(value, &mut issues);
    Ok(issues)
}

/// L1 + L2: 構造検証してから型付きデシリアライズする。
/// L1 で全件列挙し、通過後に L2(パス情報付き)を行う。
pub fn parse_validated<T: NamedSchema + DeserializeOwned>(value: &Value) -> Result<T, CoreError> {
    let issues = validate_structure(T::SCHEMA_NAME, value)?;
    if !issues.is_empty() {
        return Err(CoreError::Validation { issues });
    }

    // L2: L1 を通過しても untagged enum の解釈等で失敗し得るため、パス付きで報告する
    let raw = serde_json::to_string(value).map_err(|e| CoreError::JsonParse {
        path: format!("<value:{}>", T::SCHEMA_NAME).into(),
        source: e,
    })?;
    let mut deserializer = serde_json::Deserializer::from_str(&raw);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|e| {
        let pointer = format!("/{}", e.path().to_string().replace('.', "/"));
        CoreError::Validation {
            issues: vec![ValidationIssue {
                pointer,
                code: "DESERIALIZE_ERROR".to_string(),
                message: e.inner().to_string(),
                hint: None,
            }],
        }
    })
}

/// ファイルから読み込み、L1 + L2 検証を通して返す。
pub fn load_validated<T: NamedSchema + DeserializeOwned>(
    path: &std::path::Path,
) -> Result<T, CoreError> {
    let raw = std::fs::read_to_string(path).map_err(|e| CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|e| CoreError::JsonParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    parse_validated(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PartPlan;
    use serde_json::json;

    fn minimal_plan() -> Value {
        json!({
            "schema_version": "1.0",
            "part_id": "test-riff",
            "device": "heavier7strings",
            "bpm": 142.0,
            "time_signature": "4/4",
            "sections": [{
                "label": "verse",
                "start_bar": 1,
                "phrases": [{
                    "notes": [{
                        "pitch": "E1",
                        "start": "1.1.000",
                        "duration": "0.0.240",
                        "velocity": 112,
                        "articulation": "palm_mute"
                    }]
                }]
            }]
        })
    }

    #[test]
    fn valid_plan_parses() {
        let plan: PartPlan = parse_validated(&minimal_plan()).unwrap();
        assert_eq!(plan.part_id, "test-riff");
        assert_eq!(plan.ppq, 480, "ppq defaults to 480");
    }

    #[test]
    fn issues_are_collected_not_first_error_only() {
        let mut value = minimal_plan();
        // 2 つの独立したエラーを仕込む: 必須フィールド欠落 + 型違い
        value.as_object_mut().unwrap().remove("device");
        value["bpm"] = json!("fast");
        let err = parse_validated::<PartPlan>(&value).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!("expected Validation error");
        };
        assert!(
            issues.len() >= 2,
            "must collect all issues, got: {issues:?}"
        );
    }

    #[test]
    fn unknown_field_is_rejected_with_pointer() {
        let mut value = minimal_plan();
        // typo: articulation → articulaton
        value["sections"][0]["phrases"][0]["notes"][0]
            .as_object_mut()
            .unwrap()
            .remove("articulation");
        value["sections"][0]["phrases"][0]["notes"][0]["articulaton"] = json!("palm_mute");
        let err = parse_validated::<PartPlan>(&value).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!("expected Validation error");
        };
        assert!(
            issues.iter().any(|i| i.pointer.contains("/notes/0")),
            "issue must point at the offending note: {issues:?}"
        );
    }

    #[test]
    fn unsupported_schema_version_is_reported() {
        let mut value = minimal_plan();
        value["schema_version"] = json!("2.0");
        let err = parse_validated::<PartPlan>(&value).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!("expected Validation error");
        };
        assert!(
            issues
                .iter()
                .any(|i| i.code == "UNSUPPORTED_SCHEMA_VERSION")
        );
    }
}
