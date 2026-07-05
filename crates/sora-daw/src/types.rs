//! DAW アダプタの入出力型(技術要件書 §11.1, §11.3, §11.4)。

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// アダプタが実行時に申告するケイパビリティ集合(§11.1)。
/// MCP ツールは非対応操作を「未対応 + 代替はファイル書き出し」と即答する。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DawCapabilities {
    /// アダプタ名(例: "studio-one", "generic")
    pub adapter: String,
    /// プロジェクト状態の読み取り(§11.3)
    pub read: bool,
    /// トランスポート制御
    pub transport: bool,
    /// MIDI クリップの配置
    pub write_clip: bool,
    /// オートメーションの適用
    pub write_automation: bool,
    /// ステム/ミックスのレンダリング
    pub render: bool,
    /// 選択中トラック/クリップの取得(「これ」の決定的参照)
    pub selection: bool,
    /// 制約・未検証事項の注記(Agent がユーザー説明に使う)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// トランスポート操作(§5 `sora daw transport`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TransportCmd {
    /// 再生開始
    Play,
    /// 停止
    Stop,
    /// 録音
    Record,
    /// 巻き戻し
    Rewind,
    /// 早送り
    Forward,
    /// 曲頭へ戻る
    ReturnToZero,
}

impl TransportCmd {
    /// 表示・ログ用の安定名。
    pub fn as_str(self) -> &'static str {
        match self {
            TransportCmd::Play => "play",
            TransportCmd::Stop => "stop",
            TransportCmd::Record => "record",
            TransportCmd::Rewind => "rewind",
            TransportCmd::Forward => "forward",
            TransportCmd::ReturnToZero => "return-to-zero",
        }
    }
}

/// トランスポート操作の結果。
/// 現状のアダプタは一方向(送信)のため、DAW 側の実状態は `verified: false`。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TransportState {
    /// 要求した操作
    pub requested: TransportCmd,
    /// トリガーの送達に成功したか(ポート接続と送信の成否)
    pub delivered: bool,
    /// DAW 側の状態を読み返して確認できたか
    pub verified: bool,
    /// 送達経路の説明(例: "Sora Surface note 0x18 via IAC")
    pub route: String,
}

/// DAW プロジェクト状態(§11.3)。project-context.json へ `confidence: "daw"` でマージされる。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DawProjectState {
    /// 取得元(例: .song のパス)
    pub source: PathBuf,
    /// テンポ(BPM)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// 拍子(例: "4/4")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_signature: Option<String>,
    /// サンプルレート(Hz)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    /// トラック一覧
    pub tracks: Vec<DawTrack>,
    /// マーカー一覧
    pub markers: Vec<DawMarker>,
    /// この状態の鮮度に関する注記(例: 「最後に保存された時点の状態」)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// DAW トラック。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DawTrack {
    /// DAW 内の安定 ID(Studio One: trackID GUID)
    pub id: String,
    /// トラック名
    pub name: String,
    /// トラック種別(例: "Music", "Audio")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 表示色(例: "FFFF943D")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// クリップ数
    pub clip_count: usize,
}

/// DAW マーカー。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DawMarker {
    /// マーカー名
    pub name: String,
    /// 位置(拍。4/4 なら 4 拍 = 1 小節)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_beats: Option<f64>,
}

/// MIDI クリップ配置の要求(§5 `sora daw write-clip`)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteClipRequest {
    /// 配置する SMF ファイル
    pub midi_file: PathBuf,
    /// 配置先トラックのヒント(アダプタが対応する場合のみ使用)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_hint: Option<String>,
}

/// 書き込みの完了状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WriteStatus {
    /// DAW 内で適用が確認できた
    Applied,
    /// DAW への要求をキューに置いた(トリガー待ち・ユーザー操作待ち)
    Queued,
    /// ファイルとして書き出した(DAW には未接触。手動インポート)
    Exported,
}

/// 書き込みレシート(§11.4)。「何を・どこへ・undo 手順」を必ず持つ。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WriteReceipt {
    /// 操作名(例: "write_clip")
    pub action: String,
    /// 実行アダプタ
    pub adapter: String,
    /// 完了状態
    pub status: WriteStatus,
    /// 書き込み先の説明(トラック・パス等)
    pub target: String,
    /// この操作で作成されたファイル
    pub files: Vec<PathBuf>,
    /// 書き込み前スナップショットの保存先(§11.4。DAW に触れない操作は None)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
    /// undo 手順(ユーザー向け・上から順に)
    pub undo: Vec<String>,
    /// 次のアクションや制約の注記
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// レンダリング要求(§5 `sora daw render`)。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderRequest {
    /// 対象トラック(省略時はミックス全体)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// 出力先パス
    pub out: PathBuf,
}

/// レンダリング結果。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenderReceipt {
    /// 出力ファイル
    pub out: PathBuf,
    /// 実行アダプタ
    pub adapter: String,
}
