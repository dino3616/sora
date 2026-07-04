//! Part Plan — MIDI 生成の中間表現(技術要件書 §4.4)。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::NoteSpec;

/// Part Plan(IR)。Agent が音楽的判断として起草し、コンパイラが
/// Device Profile を参照して検証・奏法解決・`.mid` 出力を行う。
/// レビュー・diff・バージョン管理の単位。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartPlan {
    /// スキーマバージョン(semver)
    pub schema_version: String,
    /// パート ID(kebab-case。出力ファイル名の基底になる。例: "guitar-riff-v1")
    pub part_id: String,
    /// 対象デバイス ID(Device Profile の id)
    pub device: String,
    /// テンポ(BPM)
    pub bpm: f64,
    /// 拍子(例: "4/4", "7/8")
    pub time_signature: String,
    /// PPQ(4分音符あたりの tick 数。省略時 480)
    #[serde(default = "default_ppq")]
    pub ppq: u32,
    /// セクション列(曲の時間順)
    pub sections: Vec<PlanSection>,
    /// ヒューマナイズ設定。省略時は適用しない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<Humanize>,
    /// CC オートメーションレーン。キースイッチで表現できない連続制御用
    /// (例: Heavier7Strings のパームミュートは CC16(P.M. mix)で制御する)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<CcLane>,
    /// フレーズ設計の音楽的説明(なぜこう作ったか)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_notes: Option<String>,
}

/// 単一 CC の時間変化(オートメーションレーン)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CcLane {
    /// CC 番号(0-127)。Device Profile の cc_map に定義されているものを推奨
    pub cc: u8,
    /// 用途の覚書(任意。例: "palm mute mix")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// 制御点列(時間順)
    pub points: Vec<CcPoint>,
}

/// CC 制御点。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CcPoint {
    /// 位置 "bar.beat.tick"
    pub at: String,
    /// CC 値(0-127)
    pub value: u8,
}

fn default_ppq() -> u32 {
    480
}

/// Plan 内の 1 セクション。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanSection {
    /// セクションラベル(project-context.json の sections と対応させる)
    pub label: String,
    /// 開始小節(1 始まり)
    pub start_bar: u32,
    /// フレーズ列
    pub phrases: Vec<Phrase>,
}

/// フレーズ(音楽的なまとまり。レビュー時の単位)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    /// フレーズの説明(任意)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// ノート列
    pub notes: Vec<PlanNote>,
}

/// Plan 内の 1 ノート。
/// pitch(音程系)と kit_piece(ドラム系)はどちらか一方のみ指定する。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanNote {
    /// 音程(ノート名または MIDI 番号。ドラムでは使わない)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<NoteSpec>,
    /// キットピース ID(ドラムのみ。Device Profile の drum_map から参照)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_piece: Option<String>,
    /// 開始位置 "bar.beat.tick"(bar/beat は 1 始まり。例: "1.1.000")
    pub start: String,
    /// 長さ "bars.beats.ticks"(オフセット表記。例: "0.0.240" = 8分音符@PPQ480)
    pub duration: String,
    /// ベロシティ(1-127)
    pub velocity: u8,
    /// 奏法 ID(Device Profile の keyswitches から参照。省略時は奏法なし)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub articulation: Option<String>,
}

/// ヒューマナイズ設定。seed 必須 — 同一 Plan + 同一 Profile から
/// バイト同一の .mid を再現するため(技術要件書 §4.4)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Humanize {
    /// タイミング揺らぎの標準偏差(ミリ秒)。±3σ でクリップ。
    /// 小節頭のキックとベースのダウンビートには適用されない
    pub timing_ms: f64,
    /// ベロシティ揺らぎの最大幅(±この値の一様分布)
    pub velocity: u8,
    /// 乱数シード(必須。再現性の保証)
    pub seed: u64,
}
