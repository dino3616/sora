//! Automation Plan(技術要件書 §4.5)。
//!
//! Part Plan と同型の「Agent が書き、Tool が適用する」IR。Phase 2〜3 では手動適用用
//! ドキュメントの生成元、Phase 4 では DAW アダプタの入力になる。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// オートメーションプラン。単一パラメータの時間変化を記述する。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationPlan {
    /// スキーマバージョン(semver)
    pub schema_version: String,
    /// オートメーション対象
    pub target: AutomationTarget,
    /// 値の単位(例: "dB", "%", "Hz")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 制御点列(時間順)
    pub points: Vec<AutomationPoint>,
    /// この変化の音楽的理由
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// オートメーション対象。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationTarget {
    /// 対象トラック ID
    pub track: String,
    /// デバイス ID(Device Profile の id)
    pub device: String,
    /// パラメータ名(Device Profile の parameters に存在するものに限る)
    pub parameter: String,
}

/// 制御点。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationPoint {
    /// 位置 "bar.beat.tick"
    pub at: String,
    /// 値(unit に従う)
    pub value: f64,
    /// 補間カーブ
    #[serde(default)]
    pub curve: AutomationCurve,
}

/// 補間カーブ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutomationCurve {
    /// 線形
    #[default]
    Linear,
    /// 滑らかな S 字
    Smooth,
    /// 階段状(次の点まで値を保持)
    Hold,
}
