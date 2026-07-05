//! `sora midi send` / `sora midi panic`(仮想 MIDI 送信、技術要件書 §9)。
//!
//! 送信本体は sora-mcp の `ops::send_file` と共有(RAII オールノートオフ含む)。
//! CLI 側は Ctrl-C の捕捉とポート解決のみを担う。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Context;
use serde_json::json;
use sora_mcp::ops;

use crate::output::CmdResult;

/// Ctrl-C を捕捉するフラグを設定する(多重登録は無視)。
fn install_ctrlc() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    let _ = ctrlc::set_handler(move || {
        f.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    flag
}

#[derive(clap::Args)]
pub struct SendArgs {
    /// 送信する SMF ファイル
    file: PathBuf,
    /// 出力ポート名(部分一致。省略時は sora.config.json の midi.port_name)
    #[arg(long)]
    port: Option<String>,
    /// 参照する sora.config.json(既定: ./sora.config.json)
    #[arg(long)]
    config: Option<PathBuf>,
}

impl SendArgs {
    pub fn run(self) -> CmdResult {
        let port_name = self.resolve_port()?;
        let interrupted = install_ctrlc();
        let stats = ops::send_file(&self.file, &port_name, Some(&interrupted))
            .with_context(|| format!("sending {}", self.file.display()))?;
        Ok(serde_json::to_value(stats)?)
    }

    fn resolve_port(&self) -> anyhow::Result<String> {
        if let Some(p) = &self.port {
            return Ok(p.clone());
        }
        let config_path = self
            .config
            .clone()
            .unwrap_or_else(|| PathBuf::from("sora.config.json"));
        let cfg: sora_core::model::SoraConfig = sora_core::validate::load_validated(&config_path)
            .with_context(|| {
            format!(
                "no --port given and could not read {} for midi.port_name",
                config_path.display()
            )
        })?;
        cfg.midi.map(|m| m.port_name).ok_or_else(|| {
            anyhow::anyhow!(
                "sora.config.json に midi.port_name がありません。--port で指定してください"
            )
        })
    }
}

#[derive(clap::Args)]
pub struct PanicArgs {
    /// 出力ポート名(部分一致)
    #[arg(long)]
    port: String,
}

impl PanicArgs {
    pub fn run(self) -> CmdResult {
        ops::panic_port(&self.port)?;
        Ok(json!({ "port": self.port, "panic": "all-notes-off sent to 16 channels" }))
    }
}
