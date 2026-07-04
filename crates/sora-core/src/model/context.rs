//! Project Context(技術要件書 §4.3)。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Provenance;

/// 現在の曲に関する構造化された理解。
/// Sora は孤立して生成せず、まずこれを読む(Vision 原則 4)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectContext {
    /// スキーマバージョン(semver)
    pub schema_version: String,
    /// テンポ(BPM)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// 拍子(例: "4/4")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_signature: Option<String>,
    /// キー(調)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyInfo>,
    /// セクション構成(曲の時間順)
    #[serde(default)]
    pub sections: Vec<SectionInfo>,
    /// トラック台帳
    #[serde(default)]
    pub tracks: Vec<TrackInfo>,
    /// コード進行(自由形式。例: "E5 - G5 - A5")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chord_progression: Option<String>,
    /// ユーザーメモ(意図・確定事項・未定事項)
    #[serde(default)]
    pub user_notes: Vec<String>,
    /// リファレンス(参照楽曲・音像の説明)
    #[serde(default)]
    pub references: Vec<String>,
}

/// キー情報。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyInfo {
    /// 主音(例: "E", "F#")
    pub tonic: String,
    /// 旋法
    pub mode: KeyMode,
    /// 値の出所(stated: ユーザー申告 / estimated: 解析推定 / daw: DAW 由来)
    pub confidence: Provenance,
}

/// 旋法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyMode {
    Major,
    Minor,
}

/// セクション 1 エントリ。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SectionInfo {
    /// ラベル(例: "verse", "chorus", "breakdown")
    pub label: String,
    /// 開始小節(1 始まり)
    pub start_bar: u32,
    /// 終了小節(この小節を含む)
    pub end_bar: u32,
}

/// トラック 1 エントリ。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackInfo {
    /// トラック ID(kebab-case)
    pub id: String,
    /// 音楽的役割(例: "bass", "rhythm_guitar", "drums")
    pub role: String,
    /// 元素材のパス(ユーザー由来。書き換え禁止)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// アクティブ素材ポインタ — 「このリフ」等の指示語が指す現在の正。
    /// 新バージョン採用のたびに更新される。指示語の解決順: (1) ユーザー明示パス
    /// (2) DAW 選択状態 (3) この値。解決結果は応答で復唱すること
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_source: Option<String>,
    /// 使用デバイス ID(Device Profile の id)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// 解析レポートのパス
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<String>,
}
