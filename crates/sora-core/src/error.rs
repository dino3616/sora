//! エラーハンドリング(技術要件書 §6)。
//!
//! - variant は「Agent が取るべき次のアクション」が異なる単位で分ける
//! - 修正ヒントは文言でなくフィールド(データ)で持つ
//! - bin 層は [`ErrorReport`] へ正規化して JSON 出力する

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 検証 3 層(L1/L2/L3)の個別エラー。全件列挙して返す(技術要件書 §4.6)。
/// Deserialize も導出する(Agent/テストが ErrorReport を機械読みして扱うため)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// エラー位置の JSON Pointer(例: `/sections/0/phrases/1/notes/3/pitch`)
    pub pointer: String,
    /// 安定 ID(例: `TYPE_MISMATCH`, `UNKNOWN_FIELD`, `INVALID_NOTE_NAME`)
    pub code: String,
    /// 人間可読なメッセージ
    pub message: String,
    /// 修正のヒント(Agent の自己修正用)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// sora-core の型付きエラー。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("validation failed with {} issue(s)", .issues.len())]
    Validation { issues: Vec<ValidationIssue> },

    #[error("unknown articulation `{name}` for device `{device}`")]
    UnknownArticulation {
        name: String,
        device: String,
        available: Vec<String>,
    },

    #[error("note {note} out of range {low}..={high} for device `{device}`")]
    NoteOutOfRange {
        note: u8,
        low: u8,
        high: u8,
        device: String,
        /// この半音数だけ移調すれば全ノートが収まる場合に提示
        transpose_hint: Option<i8>,
    },

    #[error(
        "keyswitch note {note} collides with playable range {low}..={high} of device `{device}`"
    )]
    KeyswitchCollision {
        note: u8,
        low: u8,
        high: u8,
        device: String,
    },

    #[error("unknown kit piece `{name}` for device `{device}`")]
    UnknownKitPiece {
        name: String,
        device: String,
        available: Vec<String>,
    },

    #[error("device `{device}` has no drum map but plan uses kit pieces")]
    NoDrumMap { device: String },

    #[error("invalid note name `{value}`")]
    InvalidNoteName { value: String, hint: String },

    #[error("note number {value} outside MIDI range 0..=127")]
    NoteNumberOutOfMidiRange { value: i32 },

    #[error("invalid velocity {value} (must be 1..=127)")]
    InvalidVelocity { value: u8 },

    #[error("invalid time position `{value}`")]
    InvalidTimePosition { value: String, hint: String },

    #[error("invalid octave convention `{value}`")]
    InvalidOctaveConvention { value: String, allowed: Vec<String> },

    #[error("schema `{name}` is not registered")]
    UnknownSchema {
        name: String,
        available: Vec<String>,
    },

    #[error("MIDI parse error in {path}")]
    MidiParse {
        path: PathBuf,
        #[source]
        source: midly::Error,
    },

    #[error("refusing to overwrite existing file {path}")]
    FileExists { path: PathBuf },

    #[error("I/O error on {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON parse error in {path}")]
    JsonParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// 終了コード規約(技術要件書 §6.3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// 成功(警告があっても)
    Success = 0,
    /// 検証・ドメインエラー(Agent が自己修正可能)
    Validation = 1,
    /// 使用法エラー(clap)
    Usage = 2,
    /// 環境エラー(ポート未検出・ファイル不在・DAW 未接続)
    Environment = 3,
    /// 内部エラー(バグ)
    Internal = 4,
}

impl CoreError {
    /// SCREAMING_SNAKE_CASE の安定 ID。Agent はこれで分岐する。
    /// 後方互換を保つこと(変更はエラー設計の破壊的変更)。
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::Validation { .. } => "VALIDATION_FAILED",
            CoreError::UnknownArticulation { .. } => "UNKNOWN_ARTICULATION",
            CoreError::NoteOutOfRange { .. } => "NOTE_OUT_OF_RANGE",
            CoreError::KeyswitchCollision { .. } => "KEYSWITCH_COLLISION",
            CoreError::UnknownKitPiece { .. } => "UNKNOWN_KIT_PIECE",
            CoreError::NoDrumMap { .. } => "NO_DRUM_MAP",
            CoreError::InvalidNoteName { .. } => "INVALID_NOTE_NAME",
            CoreError::NoteNumberOutOfMidiRange { .. } => "NOTE_NUMBER_OUT_OF_MIDI_RANGE",
            CoreError::InvalidVelocity { .. } => "INVALID_VELOCITY",
            CoreError::InvalidTimePosition { .. } => "INVALID_TIME_POSITION",
            CoreError::InvalidOctaveConvention { .. } => "INVALID_OCTAVE_CONVENTION",
            CoreError::UnknownSchema { .. } => "UNKNOWN_SCHEMA",
            CoreError::MidiParse { .. } => "MIDI_PARSE_ERROR",
            CoreError::FileExists { .. } => "FILE_EXISTS",
            CoreError::Io { .. } => "IO_ERROR",
            CoreError::JsonParse { .. } => "JSON_PARSE_ERROR",
        }
    }

    /// このエラーに対応する終了コード。
    pub fn exit_code(&self) -> ExitCode {
        match self {
            CoreError::Io { .. } => ExitCode::Environment,
            _ => ExitCode::Validation,
        }
    }

    /// Agent 向けの修正ヒント。
    pub fn hint(&self) -> Option<String> {
        match self {
            CoreError::UnknownArticulation { available, .. } => Some(format!(
                "articulation は Device Profile の keyswitches に定義された ID を使ってください。利用可能: {}",
                available.join(", ")
            )),
            CoreError::NoteOutOfRange { transpose_hint, low, high, .. } => Some(match transpose_hint {
                Some(t) => format!("{t:+} 半音の移調で音域 {low}..={high} に収まります"),
                None => format!("Device Profile の note_range は {low}..={high} です"),
            }),
            CoreError::KeyswitchCollision { .. } => Some(
                "キースイッチノートが演奏音域と重なっています。Profile のキースイッチ定義か octave_convention を確認してください".to_string(),
            ),
            CoreError::UnknownKitPiece { available, .. } => Some(format!(
                "kit_piece は Device Profile の drum_map に定義された ID を使ってください。利用可能: {}",
                available.join(", ")
            )),
            CoreError::NoDrumMap { .. } => Some(
                "ドラム以外のデバイスでは kit_piece ではなく pitch を使ってください".to_string(),
            ),
            CoreError::InvalidNoteName { hint, .. } | CoreError::InvalidTimePosition { hint, .. } => {
                Some(hint.clone())
            }
            CoreError::InvalidOctaveConvention { allowed, .. } => {
                Some(format!("octave_convention に指定可能な値: {}", allowed.join(", ")))
            }
            CoreError::UnknownSchema { available, .. } => {
                Some(format!("利用可能なスキーマ: {}", available.join(", ")))
            }
            CoreError::FileExists { .. } => Some(
                "Sora は非破壊: 別名で保存するか version snapshot を使ってください".to_string(),
            ),
            _ => None,
        }
    }

    /// 構造化ペイロード(variant のフィールド)を JSON で返す。
    pub fn details(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            CoreError::Validation { issues } => json!({ "issues": issues }),
            CoreError::UnknownArticulation {
                name,
                device,
                available,
            } => {
                json!({ "name": name, "device": device, "available": available })
            }
            CoreError::NoteOutOfRange {
                note,
                low,
                high,
                device,
                transpose_hint,
            } => {
                json!({ "note": note, "low": low, "high": high, "device": device, "transpose_hint": transpose_hint })
            }
            CoreError::KeyswitchCollision {
                note,
                low,
                high,
                device,
            } => {
                json!({ "note": note, "low": low, "high": high, "device": device })
            }
            CoreError::UnknownKitPiece {
                name,
                device,
                available,
            } => {
                json!({ "name": name, "device": device, "available": available })
            }
            CoreError::NoDrumMap { device } => json!({ "device": device }),
            CoreError::InvalidNoteName { value, .. } => json!({ "value": value }),
            CoreError::NoteNumberOutOfMidiRange { value } => json!({ "value": value }),
            CoreError::InvalidVelocity { value } => json!({ "value": value }),
            CoreError::InvalidTimePosition { value, .. } => json!({ "value": value }),
            CoreError::InvalidOctaveConvention { value, allowed } => {
                json!({ "value": value, "allowed": allowed })
            }
            CoreError::UnknownSchema { name, available } => {
                json!({ "name": name, "available": available })
            }
            CoreError::MidiParse { path, source } => {
                json!({ "path": path, "cause": source.to_string() })
            }
            CoreError::FileExists { path } => json!({ "path": path }),
            CoreError::Io { path, source } => {
                json!({ "path": path, "cause": source.to_string() })
            }
            CoreError::JsonParse { path, source } => json!({
                "path": path,
                "cause": source.to_string(),
                "line": source.line(),
                "column": source.column(),
            }),
        }
    }
}

/// bin 層が stdout に出す構造化エラー(技術要件書 §6.3)。
/// CLI と MCP で同一表現を保証する。
#[derive(Debug, Clone, Serialize)]
pub struct ErrorReport {
    pub code: String,
    pub message: String,
    pub details: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// anyhow の context スタック(外側 → 内側)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<String>,
}

impl ErrorReport {
    pub fn from_core(err: &CoreError, chain: Vec<String>) -> Self {
        ErrorReport {
            code: err.code().to_string(),
            message: err.to_string(),
            details: err.details(),
            hint: err.hint(),
            chain,
        }
    }

    /// 予期しないエラー(バグ)用。
    pub fn internal(message: String, chain: Vec<String>) -> Self {
        ErrorReport {
            code: "INTERNAL".to_string(),
            message,
            details: serde_json::Value::Null,
            hint: Some("再試行せず、ログとともに報告してください".to_string()),
            chain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        let err = CoreError::UnknownArticulation {
            name: "palm-mute".into(),
            device: "heavier7strings".into(),
            available: vec!["palm_mute".into(), "pinch_harmonic".into()],
        };
        assert_eq!(err.code(), "UNKNOWN_ARTICULATION");
        assert_eq!(err.exit_code(), ExitCode::Validation);
        assert!(err.hint().unwrap().contains("palm_mute"));
    }

    #[test]
    fn error_report_serializes_with_details() {
        let err = CoreError::NoteOutOfRange {
            note: 20,
            low: 23,
            high: 88,
            device: "heavier7strings".into(),
            transpose_hint: Some(12),
        };
        let report = ErrorReport::from_core(&err, vec!["while compiling plan".into()]);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["code"], "NOTE_OUT_OF_RANGE");
        assert_eq!(json["details"]["transpose_hint"], 12);
        assert!(json["hint"].as_str().unwrap().contains("+12"));
    }
}
