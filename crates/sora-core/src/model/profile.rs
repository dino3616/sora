//! Device Profile(技術要件書 §4.2)。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Confidence;
use crate::types::NoteSpec;

/// VST インストゥルメント/エフェクトの構造化記述(Device Profile)。
/// 「ハードコードより Device Profile」原則の実体。
/// instrument / effect を単一スキーマで表現し、該当しないフィールドは省略する。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfile {
    /// スキーマバージョン(semver。major 不一致の入力は拒否される)
    pub schema_version: String,
    /// デバイス ID(kebab-case。sora.config.json の devices[].id と一致させる)
    pub id: String,
    /// 表示名
    pub name: String,
    /// ベンダー名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// デバイス種別
    pub device_type: DeviceType,
    /// 音楽的役割(例: "rhythm_guitar", "bass", "drums", "mastering")
    #[serde(default)]
    pub roles: Vec<String>,
    /// オクターブ表記基準。"C3=60" または "C4=60"。
    /// ベンダー間で不統一のため、note_range・keyswitches・drum_map のいずれかを
    /// 定義する場合は宣言必須(曖昧だとキースイッチが全部ずれる)。
    /// マスタリング/EQ 等、音程を扱わない effect デバイスでは省略できる。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octave_convention: Option<String>,
    /// 演奏可能音域(instrument のみ)。キースイッチはこの外側に置く。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_range: Option<NoteRange>,
    /// キースイッチ奏法マップ(instrument のみ)
    #[serde(default)]
    pub keyswitches: Vec<Keyswitch>,
    /// MIDI CC マップ
    #[serde(default)]
    pub cc_map: Vec<CcMapEntry>,
    /// ドラムノートマップ(ドラム音源のみ)。kit_piece ID → ノート。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drum_map: Option<Vec<DrumMapEntry>>,
    /// エフェクトパラメータ(effect のみ)
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// 発音の同時性。mono はノートオーバーラップを自動トリムする
    #[serde(default)]
    pub polyphony: Polyphony,
    /// 出力 MIDI チャンネル(0 始まり)。既定: drum_map があれば 9(=ch10)、なければ 0
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_channel: Option<u8>,
    /// キースイッチを対象ノートの何 tick 前に置くか(既定 20)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyswitch_lead_ticks: Option<u32>,
    /// このデバイスの演奏慣習(Agent が Plan 起草時に参照する自由記述)
    #[serde(default)]
    pub playing_conventions: Vec<String>,
    /// 既知の制約(自由記述)
    #[serde(default)]
    pub constraints: Vec<String>,
    /// 参照した資料(マニュアル PDF のパス等)
    #[serde(default)]
    pub manual_refs: Vec<String>,
    /// プリセットカテゴリ(トーン提案・トーンマッチングの知識ベース)
    #[serde(default)]
    pub preset_categories: Vec<PresetCategory>,
}

/// デバイス種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    /// MIDI で発音する音源
    Instrument,
    /// オーディオ処理(アンプシム・マスタリング等)
    Effect,
}

/// 発音の同時性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Polyphony {
    /// 和音可(既定)
    #[default]
    Poly,
    /// 単音。同一ピッチの重なりは自動トリムされる
    Mono,
}

/// 演奏可能音域。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteRange {
    /// 最低音(ノート名または MIDI 番号)
    pub low: NoteSpec,
    /// 最高音(ノート名または MIDI 番号)
    pub high: NoteSpec,
}

/// キースイッチ 1 エントリ。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Keyswitch {
    /// 奏法 ID(snake_case。Part Plan の notes[].articulation から参照される)
    pub articulation: String,
    /// キースイッチノート(ノート名または MIDI 番号。octave_convention 基準)
    pub note: NoteSpec,
    /// 動作モード
    pub mode: KeyswitchMode,
    /// この情報の確信度
    pub confidence: Confidence,
    /// 情報源(例: "manual p.23")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// キースイッチの動作モード(技術要件書 §7)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyswitchMode {
    /// 押している間だけ有効。対象ノートと同じ長さで出力される
    Momentary,
    /// 一度押すと次の切替まで有効。短ノート(10 tick)で出力される
    Latch,
}

/// MIDI CC マップ 1 エントリ。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CcMapEntry {
    /// CC 番号(0-127)
    pub cc: u8,
    /// 機能の説明(例: "vibrato depth")
    pub function: String,
    /// 安全な値域 [min, max](これを超える値は明示フラグなしにエラー)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_range: Option<[u8; 2]>,
    /// この情報の確信度
    pub confidence: Confidence,
    /// 情報源
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// ドラムノートマップ 1 エントリ。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DrumMapEntry {
    /// キットピース ID(snake_case。例: "kick", "snare", "hihat_closed"。
    /// Part Plan の notes[].kit_piece から参照される)
    pub kit_piece: String,
    /// 対応ノート(octave_convention 基準)
    pub note: NoteSpec,
    /// この情報の確信度
    pub confidence: Confidence,
    /// 情報源
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// エフェクトパラメータ(effect デバイス用)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// パラメータ名(例: "gate.threshold")
    pub name: String,
    /// 単位(例: "dB", "%", "Hz")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 安全な値域 [min, max]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_range: Option<[f64; 2]>,
    /// DAW 側オートメーションのパラメータパス(Phase 4)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_target: Option<String>,
    /// この情報の確信度
    pub confidence: Confidence,
    /// 情報源
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// プリセットカテゴリ(名前ベースの知識。UC16 トーンマッチングの一次情報)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresetCategory {
    /// カテゴリ名(例: "High Gain", "Modern Metal")
    pub name: String,
    /// 説明(マニュアル・実聴に基づく特徴)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 代表プリセット名
    #[serde(default)]
    pub examples: Vec<String>,
    /// この情報の確信度
    pub confidence: Confidence,
}
