//! CLI の出力とエラー正規化(技術要件書 §6.3)。
//!
//! 全コマンドは成功時に JSON を stdout へ、失敗時は [`ErrorReport`] を
//! stdout へ出し、[`ExitCode`] で終了する。CLI と MCP でエラー表現を統一する。

use std::process::ExitCode as ProcExitCode;

use serde::Serialize;
use sora_core::error::{CoreError, ErrorReport, ExitCode};

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
pub fn emit_error(err: &anyhow::Error) -> ProcExitCode {
    let (report, exit) = match err.downcast_ref::<CoreError>() {
        Some(core) => {
            // message は CoreError(最も具体的な根本原因)。
            // chain は それを包む context 層(外側 → 内側)で、根本原因は除く。
            let core_msg = core.to_string();
            let chain: Vec<String> = err
                .chain()
                .map(|c| c.to_string())
                .filter(|m| *m != core_msg)
                .collect();
            (ErrorReport::from_core(core, chain), core.exit_code())
        }
        None => {
            // 予期しないエラー: 最外殻を message、残りの原因を chain とする。
            let chain: Vec<String> = err.chain().skip(1).map(|c| c.to_string()).collect();
            (
                ErrorReport::internal(err.to_string(), chain),
                ExitCode::Internal,
            )
        }
    };

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
