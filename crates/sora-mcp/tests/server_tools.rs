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

// ---------------------------------------------------------------------------
// DAW 統合ツール(level 3-5、技術要件書 §8, §11)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_clip_is_rejected_below_level_4() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 3);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let result = server
        .write_clip(Parameters(sora_mcp::server::WriteClipParams {
            file: "exports/riff.mid".to_string(),
            track: None,
            adapter: Some("generic".to_string()),
        }))
        .await;
    assert_eq!(result.is_error, Some(true));
    let value = structured(&result);
    assert_eq!(value["error"]["code"], "CONTROL_LEVEL_REQUIRED");
    assert_eq!(value["error"]["details"]["required_level"], 4);
}

#[tokio::test]
async fn write_clip_generic_exports_with_receipt_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 4);
    let server = SoraMcp::new(dir.path().to_path_buf());

    // まず compose で .mid を作る
    let compose = server
        .compose_part(Parameters(ComposePartParams {
            plan: minimal_plan("clip-src", "palm_mute"),
            profile_path: None,
            out: None,
        }))
        .await;
    assert_ne!(compose.is_error, Some(true));

    let result = server
        .write_clip(Parameters(sora_mcp::server::WriteClipParams {
            file: "exports/clip-src.mid".to_string(),
            track: Some("bass".to_string()),
            adapter: Some("generic".to_string()),
        }))
        .await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let receipt = structured(&result);
    assert_eq!(receipt["status"], "exported");
    assert!(!receipt["undo"].as_array().unwrap().is_empty());
    let exported = receipt["files"][0].as_str().unwrap();
    assert!(exported.contains("daw-import"));
    assert!(Path::new(exported).is_file());

    // §11.4: レシートが actions.jsonl に記録されている
    let log = std::fs::read_to_string(dir.path().join("logs/actions.jsonl")).unwrap();
    assert!(log.contains("write_clip.receipt"));
}

#[tokio::test]
async fn write_automation_generic_generates_instructions() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 4);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let plan = json!({
        "schema_version": "1.0",
        "target": { "track": "bass", "device": "test-guitar", "parameter": "drive" },
        "unit": "%",
        "points": [
            { "at": "1.1.000", "value": 20.0 },
            { "at": "5.1.000", "value": 65.0, "curve": "smooth" }
        ],
        "rationale": "サビへ向けて歪みを増やす"
    });
    let result = server
        .write_automation(Parameters(sora_mcp::server::WriteAutomationParams {
            plan,
            adapter: Some("generic".to_string()),
        }))
        .await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let receipt = structured(&result);
    assert_eq!(receipt["status"], "exported");
    let doc_path = receipt["files"][0].as_str().unwrap();
    let doc = std::fs::read_to_string(doc_path).unwrap();
    assert!(doc.contains("drive"));
    assert!(doc.contains("5.1.000"));
}

#[tokio::test]
async fn render_stem_reports_not_supported_with_fallback() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 5);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let result = server
        .render_stem(Parameters(sora_mcp::server::RenderStemParams {
            out: "exports/mix.wav".to_string(),
            track: None,
        }))
        .await;
    assert_eq!(result.is_error, Some(true));
    let value = structured(&result);
    assert_eq!(value["error"]["code"], "DAW_NOT_SUPPORTED");
    assert!(
        value["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("analyze_audio")
    );
}

#[tokio::test]
async fn read_daw_project_without_song_path_guides_setup() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 3);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let result = server
        .read_daw_project(Parameters(sora_mcp::server::ReadDawProjectParams {
            song: None,
        }))
        .await;
    assert_eq!(result.is_error, Some(true));
    let value = structured(&result);
    assert_eq!(value["error"]["code"], "DAW_NOT_CONNECTED");
    assert!(
        value["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("song_path")
    );
}

// ---------------------------------------------------------------------------
// Note Selector(§11.3: selection 非対応アダプタの自然言語範囲指定フォールバック)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apply_articulations_selects_by_section_and_pitch() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    // project-context に verse = 1..2 小節を定義
    std::fs::write(
        dir.path().join("project-context.json"),
        serde_json::to_string_pretty(&json!({
            "schema_version": "1.0",
            "sections": [
                { "label": "verse", "start_bar": 1, "end_bar": 2 },
                { "label": "chorus", "start_bar": 3, "end_bar": 4 }
            ],
            "tracks": [],
            "user_notes": []
        }))
        .unwrap(),
    )
    .unwrap();
    let server = SoraMcp::new(dir.path().to_path_buf());

    // 2 小節目(verse 内)と 3 小節目(chorus 内)にノートを持つ Plan
    let plan = json!({
        "schema_version": "1.0",
        "part_id": "selector-src",
        "device": "test-guitar",
        "bpm": 120.0,
        "time_signature": "4/4",
        "sections": [{
            "label": "all",
            "start_bar": 1,
            "phrases": [{
                "notes": [
                    { "pitch": "E1", "start": "1.1.000", "duration": "0.1.000", "velocity": 100 },
                    { "pitch": "G3", "start": "2.1.000", "duration": "0.1.000", "velocity": 100 },
                    { "pitch": "E1", "start": "3.1.000", "duration": "0.1.000", "velocity": 100 }
                ]
            }]
        }]
    });
    let compose = server
        .compose_part(Parameters(ComposePartParams {
            plan,
            profile_path: None,
            out: None,
        }))
        .await;
    assert_ne!(compose.is_error, Some(true), "{compose:?}");

    // 「verse セクションの E1 以下」→ 1 ノートのみ(2 小節目の G3 は音高で除外、
    // 3 小節目の E1 はセクション範囲外)。edits は wire 形式(flatten)で渡す
    let params: sora_mcp::server::ApplyArticulationsParams = serde_json::from_value(json!({
        "file": "exports/selector-src.mid",
        "device": "test-guitar",
        "part_id": "selector-out",
        "edits": [
            { "articulation": "palm_mute", "section": "verse", "pitch_max": "E1" }
        ]
    }))
    .unwrap();
    let result = server.apply_articulations(Parameters(params)).await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let value = structured(&result);
    assert_eq!(value["edited_notes"], 1);

    // 生成 Plan で対象ノートのみに articulation が付いている
    let plan_out: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("exports/selector-out.plan.json")).unwrap(),
    )
    .unwrap();
    let notes = plan_out["sections"][0]["phrases"][0]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(notes[0]["articulation"], "palm_mute");
    assert!(notes[1].get("articulation").is_none() || notes[1]["articulation"].is_null());
}

#[tokio::test]
async fn apply_articulations_reports_unknown_section() {
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), 1);
    let server = SoraMcp::new(dir.path().to_path_buf());

    let compose = server
        .compose_part(Parameters(ComposePartParams {
            plan: minimal_plan("sec-src", "palm_mute"),
            profile_path: None,
            out: None,
        }))
        .await;
    assert_ne!(compose.is_error, Some(true));

    let params: sora_mcp::server::ApplyArticulationsParams = serde_json::from_value(json!({
        "file": "exports/sec-src.mid",
        "device": "test-guitar",
        "edits": [ { "articulation": "palm_mute", "section": "bridge" } ]
    }))
    .unwrap();
    let result = server.apply_articulations(Parameters(params)).await;
    assert_eq!(result.is_error, Some(true));
    let value = structured(&result);
    assert_eq!(value["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(
        value["error"]["details"]["issues"][0]["code"],
        "UNKNOWN_SECTION"
    );
}
