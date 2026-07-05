//! DAW 統合の型付きエラー(技術要件書 §11.1、§6.1 の lib 層規約)。
//!
//! variant は「Agent が取るべき次のアクション」単位で分ける:
//! - [`DawError::NotSupported`] → フォールバック(Generic のファイル書き出し等)へ切り替える
//! - [`DawError::NotConnected`] / [`DawError::Timeout`] → 環境を直す(セットアップ・ポート・DAW 起動)
//! - [`DawError::BackupUnavailable`] → undo 不能な書き込みはしない(§11.4)。バックアップ元を用意する

use std::path::PathBuf;

/// sora-daw の型付きエラー。
#[derive(Debug, thiserror::Error)]
pub enum DawError {
    #[error("adapter `{adapter}` does not support `{operation}`")]
    NotSupported {
        operation: String,
        adapter: String,
        /// Agent が切り替えるべき代替経路
        fallback: String,
    },

    #[error("DAW is not reachable via adapter `{adapter}`")]
    NotConnected { adapter: String, hint: String },

    #[error("`{operation}` timed out after {waited_ms} ms")]
    Timeout {
        operation: String,
        waited_ms: u64,
        hint: String,
    },

    #[error("DAW rejected `{operation}`: {reason}")]
    Rejected { operation: String, reason: String },

    #[error("cannot take pre-write backup: {reason}")]
    BackupUnavailable { reason: String },

    #[error("failed to parse song file {path}")]
    SongParse { path: PathBuf, reason: String },

    #[error("I/O error on {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 終了コード種別(sora-core::error::ExitCode と同じ規約)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DawErrorClass {
    /// Agent が自己修正可能(フォールバック切替・入力修正)
    Recoverable,
    /// 環境の問題(セットアップ・接続・ファイル不在)
    Environment,
}

impl DawError {
    /// SCREAMING_SNAKE_CASE の安定 ID。Agent はこれで分岐する。
    pub fn code(&self) -> &'static str {
        match self {
            DawError::NotSupported { .. } => "DAW_NOT_SUPPORTED",
            DawError::NotConnected { .. } => "DAW_NOT_CONNECTED",
            DawError::Timeout { .. } => "DAW_TIMEOUT",
            DawError::Rejected { .. } => "DAW_REJECTED",
            DawError::BackupUnavailable { .. } => "DAW_BACKUP_UNAVAILABLE",
            DawError::SongParse { .. } => "SONG_PARSE_ERROR",
            DawError::Io { .. } => "IO_ERROR",
        }
    }

    /// エラー分類(終了コードへの対応付けに使う)。
    pub fn class(&self) -> DawErrorClass {
        match self {
            DawError::NotSupported { .. } | DawError::Rejected { .. } => DawErrorClass::Recoverable,
            DawError::NotConnected { .. }
            | DawError::Timeout { .. }
            | DawError::BackupUnavailable { .. }
            | DawError::SongParse { .. }
            | DawError::Io { .. } => DawErrorClass::Environment,
        }
    }

    /// Agent 向けの修正ヒント。
    pub fn hint(&self) -> Option<String> {
        match self {
            DawError::NotSupported { fallback, .. } => Some(fallback.clone()),
            DawError::NotConnected { hint, .. } | DawError::Timeout { hint, .. } => {
                Some(hint.clone())
            }
            DawError::BackupUnavailable { .. } => Some(
                "undo 不能な書き込みは行いません(§11.4)。sora.config.json の daw.studio_one.song_path に保存済み .song のパスを設定するか、DAW でプロジェクトを保存してから再実行してください".to_string(),
            ),
            DawError::SongParse { .. } => Some(
                ".song が Studio One 5 形式か確認してください(開いている DAW で保存し直すと直る場合があります)".to_string(),
            ),
            _ => None,
        }
    }

    /// 構造化ペイロード(variant のフィールド)を JSON で返す。
    pub fn details(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            DawError::NotSupported {
                operation,
                adapter,
                fallback,
            } => json!({ "operation": operation, "adapter": adapter, "fallback": fallback }),
            DawError::NotConnected { adapter, .. } => json!({ "adapter": adapter }),
            DawError::Timeout {
                operation,
                waited_ms,
                ..
            } => json!({ "operation": operation, "waited_ms": waited_ms }),
            DawError::Rejected { operation, reason } => {
                json!({ "operation": operation, "reason": reason })
            }
            DawError::BackupUnavailable { reason } => json!({ "reason": reason }),
            DawError::SongParse { path, reason } => json!({ "path": path, "reason": reason }),
            DawError::Io { path, source } => {
                json!({ "path": path, "cause": source.to_string() })
            }
        }
    }
}
