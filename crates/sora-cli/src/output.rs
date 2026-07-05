//! CLI の出力とエラー正規化(技術要件書 §6.3)。
//!
//! 全コマンドは成功時に JSON を stdout へ、失敗時は [`ErrorReport`] を
//! stdout へ出し、[`ExitCode`] で終了する。CLI と MCP でエラー表現を統一する。

use std::process::ExitCode as ProcExitCode;

use serde::Serialize;
use sora_core::error::{ErrorReport, ExitCode};

/// コマンドの実行結果。`Ok` は成功データ、`Err` は anyhow チェーン。
pub type CmdResult = anyhow::Result<serde_json::Value>;

/// 成功データを JSON で stdout に出す。
pub fn emit_success(value: &serde_json::Value) {
    // pretty で人間可読にする(Agent も解析できる)
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
        );
    }
}

/// anyhow エラーを ErrorReport へ正規化して stdout に出し、終了コードを返す。
/// 正規化は sora-mcp と共有(§6.4: CLI と MCP でエラー表現を同一に保つ)。
pub fn emit_error(err: &anyhow::Error) -> ProcExitCode {
    let (report, exit) = sora_mcp::report::normalize(err);
    let wrapped = ErrorEnvelope { error: report };
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{}",
            serde_json::to_string_pretty(&wrapped).unwrap_or_else(|_| "{}".to_string())
        );
    }
    to_proc_exit(exit)
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorReport,
}

fn to_proc_exit(code: ExitCode) -> ProcExitCode {
    ProcExitCode::from(code as u8)
}
