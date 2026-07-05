//! sora.config.json — 環境プロファイル(技術要件書 §4.1)。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 環境プロファイル。ユーザーの制作環境(DAW・デバイス・好み)と
/// control level を宣言する。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoraConfig {
    /// スキーマバージョン(semver)
    pub schema_version: String,
    /// DAW 情報
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daw: Option<DawInfo>,
    /// 制御深度の上限(0-5)。引き上げはユーザーの明示的な依頼に基づく
    /// `sora config set control-level` のみで行う(技術要件書 §2.4)
    #[serde(default = "default_control_level")]
    pub control_level: u8,
    /// 利用可能なデバイス
    #[serde(default)]
    pub devices: Vec<DeviceRef>,
    /// ユーザーの好み
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferences: Option<Preferences>,
    /// MIDI 送信設定(control level 2+)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<MidiConfig>,
    /// パス慣習
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<PathsConfig>,
}

fn default_control_level() -> u8 {
    1
}

/// DAW 情報。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DawInfo {
    /// DAW 名(例: "Studio One")
    pub name: String,
    /// バージョン
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// OS(例: "macOS 15")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// Studio One 統合の設定(技術要件書 §11.2.1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub studio_one: Option<StudioOneSettings>,
}

/// Studio One 統合(Sora Bridge + Sora Surface)の設定。
/// 省略時は OS 既定パスを使う。
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StudioOneSettings {
    /// ユーザーコンテンツディレクトリ(既定: ~/Documents/Studio One)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_content: Option<String>,
    /// アプリ設定ディレクトリ
    /// (既定: macOS = ~/Library/Application Support/PreSonus/Studio One 5)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_support: Option<String>,
    /// 現在の .song のパス。read_project と書き込み前バックアップ(§11.4)に使う
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub song_path: Option<String>,
    /// Sora Surface が受信する仮想 MIDI ポート名(部分一致)。
    /// 演奏プレビュー用の midi.port_name とは別ポートにすること
    /// (同一ポートだと再生ノートがコマンドとして解釈される)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_port: Option<String>,
    /// Bridge 処理完了(inbox → outbox 移動)の待機タイムアウト(ミリ秒、既定 10000)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_timeout_ms: Option<u64>,
}

/// デバイス参照。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceRef {
    /// デバイス ID(Device Profile の id と一致)
    pub id: String,
    /// Profile ファイルのパス(プロジェクトルート相対)
    pub profile: String,
}

/// ユーザーの好み。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// 好みのジャンル(例: "modern metal", "j-rock")
    #[serde(default)]
    pub genres: Vec<String>,
    /// 既定 PPQ(省略時 480)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_ppq: Option<u32>,
    /// 既定ヒューマナイズ強度
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<HumanizeDefaults>,
}

/// 既定ヒューマナイズ強度(Plan 側で seed とともに明示される)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HumanizeDefaults {
    /// タイミング揺らぎ標準偏差(ミリ秒)
    pub timing_ms: f64,
    /// ベロシティ揺らぎ幅
    pub velocity: u8,
}

/// MIDI 送信設定。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MidiConfig {
    /// 仮想 MIDI ポート名(macOS: IAC Driver のポート名)
    pub port_name: String,
}

/// パス慣習。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathsConfig {
    /// 生成物の出力先(既定 "exports/")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exports: Option<String>,
    /// マニュアル置き場(既定 "manuals/")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manuals: Option<String>,
}
