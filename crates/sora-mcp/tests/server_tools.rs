//! MCP ツールの結合テスト(技術要件書 §8)。
//!
//! - control level ゲート: 不足時は実行前拒否 + 有効化案内
//! - compose_part: Plan 検証 → コンパイル → 新規ファイル保存(非破壊)
//! - エラー表現: MCP のツールエラーが CLI と同じ ErrorReport 形式(§6.4)
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use sora_mcp::server::{ComposePartParams, ReadMidiMode, ReadMidiParams, SendMidiParams, SoraMcp};

/// テスト用プロジェクト(config + 最小 instrument profile)を作る。
fn setup_project(dir: &Path, control_level: u8) {
    std::fs::create_dir_all(dir.join("devices")).unwrap();
    let config = json!({
        "schema_version": "1.0",
        "control_level": control_level,
        "devices": [
            { "id": "test-guitar", "profile": "devices/test-guitar.profile.json" }
        ]
    });
    std::fs::write(
        dir.join("sora.config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
    let profile = json!({
        "schema_version": "1.0",
        "id": "test-guitar",
        "name": "Test Guitar",
        "device_type": "instrument",
        "octave_convention": "C3=60",
        "note_range": { "low": "B0", "high": "E5" },
        "keyswitches": [
            { "articulation": "palm_mute", "note": "C#0", "mode": "momentary", "confidence": "verified" }
        ]
    });
    std::fs::write(
        dir.join("devices/test-guitar.profile.json"),
        serde_json::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
}

fn minimal_plan(part_id: &str, articulation: &str) -> Value {
    json!({
        "schema_version": "1.0",
        "part_id": part_id,
        "device": "test-guitar",
        "bpm": 120.0,
        "time_signature": "4/4",
        "sections": [{
            "label": "verse",
            "start_bar": 1,
            "phrases": [{
                "notes": [
                    { "pitch": "E1", "start": "1.1.000", "duration": "0.1.000",
                      "velocity": 100, "articulation": articulation }
                ]
            }]
        }]
    })
}

fn structured(result: &CallToolResult) -> &Value {
    result.structured_content.as_ref().unwrap()
}

#[tokio::test]
async fn compose_part_writes_new_files_and_logs_action() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let result = server
        .compose_part(Parameters(ComposePartParams {
            plan: minimal_plan("riff-v1", "palm_mute"),
            profile_path: None,
            out: None,
        }))
        .await;

    assert_eq!(result.is_error, Some(false), "{:?}", result.content);
    let value = structured(&result);
    assert!(dir.path().join("exports/riff-v1.mid").is_file());
    assert!(dir.path().join("exports/riff-v1.plan.json").is_file());
    assert!(value["report"].is_object());

    // 同名で再実行 → 非破壊なので拒否される
    let again = server
        .compose_part(Parameters(ComposePartParams {
            plan: minimal_plan("riff-v1", "palm_mute"),
            profile_path: None,
            out: None,
        }))
        .await;
    assert_eq!(again.is_error, Some(true));

    // 操作ログが残る(§8 横断要件)
    let log = std::fs::read_to_string(dir.path().join("logs/actions.jsonl")).unwrap();
    assert!(log.lines().count() >= 2);
    assert!(log.contains("compose_part"));
}

#[tokio::test]
async fn compose_part_error_matches_cli_error_report_shape() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    let server = SoraMcp::new(dir.path().to_path_buf());

    // 存在しない奏法 → CLI と同じ全件列挙の VALIDATION_FAILED になる(§4.6, §6.4)
    let result = server
        .compose_part(Parameters(ComposePartParams {
            plan: minimal_plan("riff-v2", "slap"),
            profile_path: None,
            out: None,
        }))
        .await;

    assert_eq!(result.is_error, Some(true));
    let error = &structured(&result)["error"];
    assert_eq!(error["code"], "VALIDATION_FAILED");
    let issue = &error["details"]["issues"][0];
    assert_eq!(issue["code"], "UNKNOWN_ARTICULATION");
    assert!(issue["pointer"].as_str().unwrap().contains("/notes/0"));
    assert!(issue["hint"].as_str().unwrap().contains("palm_mute"));

    // CLI(共有正規化)が同じ CoreError から作る表現と一致することを確認する
    let issues: Vec<sora_core::error::ValidationIssue> =
        serde_json::from_value(error["details"]["issues"].clone()).unwrap();
    let core = sora_core::error::CoreError::Validation { issues };
    let (cli_report, _) = sora_mcp::report::normalize(&anyhow::Error::new(core));
    let cli_json = serde_json::to_value(&cli_report).unwrap();
    assert_eq!(error["code"], cli_json["code"]);
    assert_eq!(error["details"], cli_json["details"]);
    assert_eq!(error["message"], cli_json["message"]);
}

#[tokio::test]
async fn send_midi_is_rejected_below_level_2() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let result = server
        .send_midi(Parameters(SendMidiParams {
            file: "exports/anything.mid".into(),
            port: Some("Sora Test".into()),
        }))
        .await
        .unwrap();

    assert_eq!(result.is_error, Some(true));
    let error = &structured(&result)["error"];
    assert_eq!(error["code"], "CONTROL_LEVEL_REQUIRED");
    assert_eq!(error["details"]["required_level"], 2);
    assert!(
        error["hint"]
            .as_str()
            .unwrap()
            .contains("sora config set control-level 2")
    );

    // 拒否も操作ログに残る
    let log = std::fs::read_to_string(dir.path().join("logs/actions.jsonl")).unwrap();
    assert!(log.contains("CONTROL_LEVEL_REQUIRED"));
}

#[tokio::test]
async fn read_midi_inspects_compiled_output() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let composed = server
        .compose_part(Parameters(ComposePartParams {
            plan: minimal_plan("riff-v1", "palm_mute"),
            profile_path: None,
            out: None,
        }))
        .await;
    assert_eq!(composed.is_error, Some(false));

    let result = server
        .read_midi(Parameters(ReadMidiParams {
            file: "exports/riff-v1.mid".into(),
            mode: ReadMidiMode::Inspect,
            include_notes: false,
        }))
        .await;
    assert_eq!(result.is_error, Some(false), "{:?}", result.content);
}

#[tokio::test]
async fn analyze_project_aggregates_config_and_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let result = server.analyze_project().await;
    assert_eq!(result.is_error, Some(false));
    let value = structured(&result);
    assert_eq!(value["config"]["control_level"], 1);
    assert_eq!(
        value["devices"],
        json!(["devices/test-guitar.profile.json"])
    );
}
