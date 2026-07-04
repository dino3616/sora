//! Data 層のスキーマ定義モデル(技術要件書 §4)。
//!
//! Rust 型が単一ソース。doc コメントは schemars 経由で JSON Schema の
//! description になり、Agent が Plan/Profile を起草する際の仕様として機能する。
//! すべて `deny_unknown_fields`(L2: フィールド名 typo を黙殺しない)。

mod automation;
mod config;
mod context;
mod plan;
mod profile;

pub use automation::*;
pub use config::*;
pub use context::*;
pub use plan::*;
pub use profile::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// フィールド単位の確信度(技術要件書 §4.2)。
/// unverified の奏法を使うとコンパイルレポートに警告が出る。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// ユーザーが実機で動作確認済み
    Verified,
    /// マニュアル記載のみ(実機未確認)
    Manual,
    /// 推測(マニュアル記述が曖昧、または情報源なし)
    Unverified,
}

/// Project Context における値の出所。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// ユーザーが明示的に申告した値(推定値と矛盾したら常にこちらを優先)
    Stated,
    /// MIDI/オーディオ解析からの推定値
    Estimated,
    /// DAW プロジェクトから読み取った値(Phase 4)
    Daw,
}
