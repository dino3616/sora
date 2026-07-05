//! DAW プロジェクト状態 → project-context.json への反映提案(§11.3)。
//!
//! CLI(`sora daw read`)と MCP(`read_daw_project`)で共有する機械処理。
//! 手動記述(stated)と衝突する値は上書きせず conflict として報告し、
//! 反映の判断は Agent が行う(両論併記 + ユーザー確認)。

use serde_json::{Value, json};
use sora_core::model::ProjectContext;

use crate::types::DawProjectState;

/// 反映提案を作る。action は `fill`(空欄を埋める)/ `conflict`(両論併記が必要)/
/// `add`(context に無いトラック)。
pub fn merge_suggestions(context: Option<&ProjectContext>, state: &DawProjectState) -> Vec<Value> {
    let mut suggestions = Vec::new();
    let current_bpm = context.and_then(|c| c.bpm);
    match (current_bpm, state.bpm) {
        (None, Some(daw)) => suggestions.push(json!({
            "field": "bpm", "action": "fill", "daw_value": daw, "confidence": "daw"
        })),
        (Some(cur), Some(daw)) if (cur - daw).abs() > 0.01 => suggestions.push(json!({
            "field": "bpm", "action": "conflict", "current": cur, "daw_value": daw,
            "note": "手動記述と DAW 由来が衝突。上書きせず両論併記し、ユーザーに確認する"
        })),
        _ => {}
    }
    let current_ts = context.and_then(|c| c.time_signature.clone());
    match (current_ts, state.time_signature.clone()) {
        (None, Some(daw)) => suggestions.push(json!({
            "field": "time_signature", "action": "fill", "daw_value": daw, "confidence": "daw"
        })),
        (Some(cur), Some(daw)) if cur != daw => suggestions.push(json!({
            "field": "time_signature", "action": "conflict", "current": cur, "daw_value": daw,
            "note": "手動記述と DAW 由来が衝突。上書きせず両論併記し、ユーザーに確認する"
        })),
        _ => {}
    }
    // context の tracks に無い DAW トラックを列挙
    let known: Vec<String> = context
        .map(|c| {
            c.tracks
                .iter()
                .map(|t| t.id.to_lowercase().replace(['-', '_'], " "))
                .collect()
        })
        .unwrap_or_default();
    for track in &state.tracks {
        let name = track.name.to_lowercase().replace(['-', '_'], " ");
        if !name.is_empty() && !known.iter().any(|k| k == &name) {
            suggestions.push(json!({
                "field": "tracks", "action": "add", "daw_value": track,
            }));
        }
    }
    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DawTrack;
    use std::path::PathBuf;

    fn state(bpm: Option<f64>, tracks: &[&str]) -> DawProjectState {
        DawProjectState {
            source: PathBuf::from("test.song"),
            bpm,
            time_signature: Some("4/4".to_string()),
            sample_rate: None,
            tracks: tracks
                .iter()
                .map(|name| DawTrack {
                    id: format!("{{{name}}}"),
                    name: (*name).to_string(),
                    kind: None,
                    color: None,
                    clip_count: 0,
                })
                .collect(),
            markers: vec![],
            notes: vec![],
        }
    }

    fn context(bpm: Option<f64>) -> ProjectContext {
        ProjectContext {
            schema_version: "1.0".to_string(),
            bpm,
            time_signature: None,
            key: None,
            sections: vec![],
            tracks: vec![],
            chord_progression: None,
            user_notes: vec![],
            references: vec![],
        }
    }

    #[test]
    fn fills_empty_fields_and_adds_tracks() {
        let ctx = context(None);
        let suggestions = merge_suggestions(Some(&ctx), &state(Some(123.0), &["Bass"]));
        assert!(
            suggestions
                .iter()
                .any(|s| s["field"] == "bpm" && s["action"] == "fill")
        );
        assert!(
            suggestions
                .iter()
                .any(|s| s["field"] == "tracks" && s["action"] == "add")
        );
    }

    #[test]
    fn reports_conflict_without_overwriting() {
        let ctx = context(Some(120.0));
        let suggestions = merge_suggestions(Some(&ctx), &state(Some(123.0), &[]));
        let bpm = suggestions.iter().find(|s| s["field"] == "bpm").unwrap();
        assert_eq!(bpm["action"], "conflict");
        assert_eq!(bpm["current"], 120.0);
        assert_eq!(bpm["daw_value"], 123.0);
    }
}
