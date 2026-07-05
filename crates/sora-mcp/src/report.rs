//! anyhow エラー → [`ErrorReport`] 正規化(技術要件書 §6.4)。
//!
//! CLI と MCP でエラー表現を同一に保つため、正規化はこの関数に一元化する。
//! sora-cli の `emit_error` も MCP ツールのエラーレスポンスもここを通る。

use sora_audio::AudioError;
use sora_core::error::{CoreError, ErrorReport, ExitCode};

/// anyhow チェーンを走査し、[`CoreError`] / [`AudioError`] を downcast して
/// 構造化 [`ErrorReport`] と終了コードへ正規化する。
pub fn normalize(err: &anyhow::Error) -> (ErrorReport, ExitCode) {
    // 根本原因(最も具体的なメッセージ)を除いた context 層を chain とする。
    let chain_without = |leaf: &str| -> Vec<String> {
        err.chain()
            .map(|c| c.to_string())
            .filter(|m| m != leaf)
            .collect()
    };

    if let Some(core) = err.downcast_ref::<CoreError>() {
        let msg = core.to_string();
        (
            ErrorReport::from_core(core, chain_without(&msg)),
            core.exit_code(),
        )
    } else if let Some(audio) = err.downcast_ref::<AudioError>() {
        let msg = audio.to_string();
        let report = ErrorReport {
            code: audio.code().to_string(),
            message: msg.clone(),
            details: serde_json::Value::Null,
            hint: audio.hint(),
            chain: chain_without(&msg),
        };
        // デコード/解析系は環境要因(ファイル不正・非対応形式)として扱う
        (report, ExitCode::Environment)
    } else {
        let chain: Vec<String> = err.chain().skip(1).map(|c| c.to_string()).collect();
        (
            ErrorReport::internal(err.to_string(), chain),
            ExitCode::Internal,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_core_error_with_context_chain() {
        let core = CoreError::UnknownArticulation {
            name: "palm-mute".into(),
            device: "h7s".into(),
            available: vec!["palm_mute".into()],
        };
        let err = anyhow::Error::new(core).context("compiling plan.json");
        let (report, exit) = normalize(&err);
        assert_eq!(report.code, "UNKNOWN_ARTICULATION");
        assert_eq!(exit, ExitCode::Validation);
        assert_eq!(report.chain, vec!["compiling plan.json".to_string()]);
    }

    #[test]
    fn normalizes_unknown_error_as_internal() {
        let err = anyhow::anyhow!("something unexpected");
        let (report, exit) = normalize(&err);
        assert_eq!(report.code, "INTERNAL");
        assert_eq!(exit, ExitCode::Internal);
    }
}
