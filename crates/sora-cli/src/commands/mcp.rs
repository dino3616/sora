//! `sora mcp serve` — MCP サーバー起動(技術要件書 §8)。
//!
//! stdout は MCP トランスポート専用なので、他コマンドと違い JSON 結果を
//! stdout に出さない(診断は stderr へ)。

use std::path::PathBuf;
use std::process::ExitCode as ProcExitCode;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum McpCommand {
    /// stdio で MCP サーバーを起動する(Agent クライアント用)
    Serve(ServeArgs),
}

#[derive(clap::Args)]
pub struct ServeArgs {
    /// プロジェクトルート(既定: カレントディレクトリ)
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

impl McpCommand {
    /// サーバーをクライアント切断まで実行する。stdout には何も出さない。
    pub fn run_blocking(self) -> ProcExitCode {
        let McpCommand::Serve(args) = self;
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("failed to start tokio runtime: {e}");
                return ProcExitCode::from(sora_core::error::ExitCode::Internal as u8);
            }
        };
        match runtime.block_on(sora_mcp::server::serve_stdio(args.root)) {
            Ok(()) => ProcExitCode::SUCCESS,
            Err(e) => {
                eprintln!("sora mcp serve failed: {e:#}");
                ProcExitCode::from(sora_core::error::ExitCode::Environment as u8)
            }
        }
    }
}
