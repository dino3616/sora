//! `sora automation apply` — Automation Plan の DAW 適用(技術要件書 §4.5, §5)。
//!
//! 要求 control level 4。アダプタが automation 非対応の場合は
//! DAW_NOT_SUPPORTED(フォールバック案内付き)を返す。

use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;
use sora_core::model::{AutomationPlan, SoraConfig};
use sora_core::validate::load_validated;
use sora_daw::adapter::{self, DawAdapter};
use sora_mcp::gate;

use crate::commands::record_action;
use crate::output::CmdResult;

#[derive(Subcommand)]
pub enum AutomationCommand {
    /// Automation Plan(.automation.json)を DAW へ適用する(level 4)
    Apply(ApplyArgs),
}

impl AutomationCommand {
    pub fn run(self) -> CmdResult {
        match self {
            AutomationCommand::Apply(a) => a.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct ApplyArgs {
    /// Automation Plan のパス(schemas/automation-plan.schema.json に従う)
    plan: PathBuf,
    /// プロジェクトルート(既定: カレントディレクトリ)
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// アダプタの明示指定("generic" で手動適用手順書の生成に切り替え)
    #[arg(long)]
    adapter: Option<String>,
}

impl ApplyArgs {
    fn run(self) -> CmdResult {
        gate::require(&self.root, "write_automation", 4)?;
        let plan: AutomationPlan = load_validated(&self.plan)?;
        let config: Option<SoraConfig> = load_validated(&self.root.join("sora.config.json")).ok();
        let mut adapter: Box<dyn DawAdapter> = match self.adapter.as_deref() {
            Some("generic") => Box::new(sora_daw::generic::GenericFileAdapter::new(&self.root)),
            Some("studio-one") => Box::new(sora_daw::studio_one::StudioOneAdapter::new(
                &self.root,
                config.as_ref(),
            )),
            _ => adapter::resolve_adapter(&self.root, config.as_ref()),
        };
        let receipt = adapter.write_automation(&plan)?;
        // §11.4: undo 情報を必ず actions.jsonl に残す
        record_action(
            &self.root,
            "automation.apply",
            json!({
                "plan": self.plan,
                "receipt": serde_json::to_value(&receipt)?,
            }),
        );
        Ok(serde_json::to_value(receipt)?)
    }
}
