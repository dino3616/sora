//! DawAdapter トレイトとアダプタ解決(技術要件書 §11.1, §11.2)。

use std::path::Path;

use sora_core::model::{AutomationPlan, SoraConfig};

use crate::error::DawError;
use crate::types::{
    DawCapabilities, DawProjectState, RenderReceipt, RenderRequest, TransportCmd, TransportState,
    WriteClipRequest, WriteReceipt,
};

/// DAW 統合の抽象操作(§11.1)。Agent と MCP ツールはこのトレイトのみを見る。
pub trait DawAdapter {
    /// アダプタ名(config の daw.name と対応)。
    fn name(&self) -> &'static str;

    /// 何ができるかを実行時に申告する。
    fn capabilities(&self) -> DawCapabilities;

    /// DAW プロジェクト状態を読み取る(§11.3)。
    fn read_project(&mut self) -> Result<DawProjectState, DawError>;

    /// トランスポートを制御する。
    fn transport(&mut self, cmd: TransportCmd) -> Result<TransportState, DawError>;

    /// MIDI クリップを配置する(§11.4 の安全規約に従う)。
    fn write_clip(&mut self, req: WriteClipRequest) -> Result<WriteReceipt, DawError>;

    /// Automation Plan を適用する(§4.5)。
    fn write_automation(&mut self, plan: &AutomationPlan) -> Result<WriteReceipt, DawError>;

    /// ステム/ミックスをレンダリングする。
    fn render(&mut self, req: RenderRequest) -> Result<RenderReceipt, DawError>;
}

/// 非対応操作の定型エラーを作る(各アダプタで使う)。
pub(crate) fn not_supported(adapter: &str, operation: &str, fallback: &str) -> DawError {
    DawError::NotSupported {
        operation: operation.to_string(),
        adapter: adapter.to_string(),
        fallback: fallback.to_string(),
    }
}

/// config からアダプタを解決する。
/// `daw.name` が Studio One を指す場合は Studio One アダプタ、
/// それ以外(未設定含む)は Generic(常設フォールバック)。
pub fn resolve_adapter(root: &Path, config: Option<&SoraConfig>) -> Box<dyn DawAdapter> {
    let daw_name = config
        .and_then(|c| c.daw.as_ref())
        .map(|d| d.name.to_lowercase())
        .unwrap_or_default();
    if daw_name.replace([' ', '_'], "-").contains("studio-one") {
        Box::new(crate::studio_one::StudioOneAdapter::new(root, config))
    } else {
        Box::new(crate::generic::GenericFileAdapter::new(root))
    }
}

/// 利用可能な全アダプタのケイパビリティを列挙する(`sora daw probe`)。
pub fn probe_all(root: &Path, config: Option<&SoraConfig>) -> Vec<DawCapabilities> {
    vec![
        crate::studio_one::StudioOneAdapter::new(root, config).capabilities(),
        crate::generic::GenericFileAdapter::new(root).capabilities(),
    ]
}
