//! SMF → Part Plan 逆コンパイル(`sora midi decompile`、技術要件書 §2.2, UC5)。
//!
//! compile の逆操作。既存 MIDI を人間可読な Part Plan に戻し、Agent が奏法注釈を
//! 加えて再コンパイルできるようにする。キースイッチノートは articulation へ逆解決し、
//! ドラムはノート番号を kit_piece へ逆解決する。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::error::CoreError;
use crate::midi::inspect::{NoteDump, inspect_file};
use crate::midi::timing::TimeGrid;
use crate::model::{DeviceProfile, KeyswitchMode};
use crate::profile::ResolvedProfile;

/// 逆コンパイル結果(Part Plan JSON + メタ情報)。
#[derive(Debug, Serialize)]
pub struct DecompileOutput {
    /// 生成された Part Plan(そのまま plan.json として書き出せる)
    pub plan: serde_json::Value,
    /// 逆解決の要約(レビュー用)
    pub summary: DecompileSummary,
}

#[derive(Debug, Serialize)]
pub struct DecompileSummary {
    pub note_count: usize,
    pub keyswitch_notes_consumed: usize,
    pub articulations_detected: Vec<String>,
    pub warnings: Vec<String>,
}

/// SMF を Part Plan へ逆コンパイルする。
pub fn decompile_file(
    path: &Path,
    profile: &DeviceProfile,
    part_id: &str,
) -> Result<DecompileOutput, CoreError> {
    let inspection = inspect_file(path, true)?;
    let resolved = ResolvedProfile::resolve(profile)?;

    let bpm = inspection.tempos.first().map(|t| t.bpm).unwrap_or(120.0);
    let time_signature = inspection
        .time_signatures
        .first()
        .map(|ts| format!("{}/{}", ts.numerator, ts.denominator))
        .unwrap_or_else(|| "4/4".to_string());
    let ppq = inspection.ppq.unwrap_or(480) as u32;
    let grid = TimeGrid::new(&time_signature, ppq)?;

    // profile のチャンネルに一致するトラックの全ノートを収集
    let mut notes: Vec<NoteDump> = inspection
        .tracks
        .iter()
        .filter_map(|t| t.notes.as_ref())
        .flatten()
        .filter(|n| n.channel == resolved.midi_channel)
        .cloned()
        .collect();
    notes.sort_by_key(|n| (n.tick, n.pitch));

    // キースイッチノート番号 → articulation の逆引き
    let ks_by_note: BTreeMap<u8, (&str, KeyswitchMode)> = resolved
        .keyswitches
        .iter()
        .map(|(name, ks)| (ks.note.value(), (name.as_str(), ks.mode)))
        .collect();
    // drum note → kit_piece
    let piece_by_note: BTreeMap<u8, &str> = resolved
        .drum_map
        .iter()
        .map(|(name, p)| (p.note.value(), name.as_str()))
        .collect();

    // キースイッチノートと演奏ノートを分離
    let mut keyswitch_events: Vec<(u64, &str, KeyswitchMode)> = Vec::new();
    let mut playing: Vec<&NoteDump> = Vec::new();
    for n in &notes {
        if let Some((name, mode)) = ks_by_note.get(&n.pitch) {
            keyswitch_events.push((n.tick, name, *mode));
        } else {
            playing.push(n);
        }
    }
    keyswitch_events.sort_by_key(|(tick, _, _)| *tick);

    let lead = resolved.keyswitch_lead_ticks as u64;
    let mut warnings: Vec<String> = Vec::new();
    let mut articulations_detected: BTreeMap<String, ()> = BTreeMap::new();

    // 各演奏ノートへ articulation を割り当てる
    let mut plan_notes: Vec<serde_json::Value> = Vec::new();
    for n in &playing {
        let articulation = resolve_articulation(n.tick, &keyswitch_events, lead);
        if let Some(a) = &articulation {
            articulations_detected.insert(a.clone(), ());
        }

        let start = ticks_to_bbt(n.tick, &grid);
        let duration = ticks_to_duration(n.duration, &grid);

        let mut obj = serde_json::Map::new();
        if piece_by_note.contains_key(&n.pitch) {
            obj.insert(
                "kit_piece".to_string(),
                serde_json::json!(piece_by_note[&n.pitch]),
            );
        } else {
            obj.insert("pitch".to_string(), serde_json::json!(n.pitch));
        }
        obj.insert("start".to_string(), serde_json::json!(start));
        obj.insert("duration".to_string(), serde_json::json!(duration));
        obj.insert("velocity".to_string(), serde_json::json!(n.velocity));
        if let Some(a) = articulation {
            obj.insert("articulation".to_string(), serde_json::json!(a));
        }
        plan_notes.push(serde_json::Value::Object(obj));
    }

    // 音域外(profile に無い音)の検出
    if let Some((low, high)) = resolved.note_range {
        for n in &playing {
            if !piece_by_note.contains_key(&n.pitch)
                && (n.pitch < low.value() || n.pitch > high.value())
            {
                warnings.push(format!(
                    "note {} is outside profile range {}..={}",
                    n.pitch,
                    low.value(),
                    high.value()
                ));
                break;
            }
        }
    }

    let plan = serde_json::json!({
        "schema_version": "1.0",
        "part_id": part_id,
        "device": resolved.id,
        "bpm": bpm,
        "time_signature": time_signature,
        "ppq": ppq,
        "sections": [{
            "label": "section-1",
            "start_bar": 1,
            "phrases": [{ "notes": plan_notes }]
        }]
    });

    Ok(DecompileOutput {
        summary: DecompileSummary {
            note_count: playing.len(),
            keyswitch_notes_consumed: keyswitch_events.len(),
            articulations_detected: articulations_detected.into_keys().collect(),
            warnings,
        },
        plan,
    })
}

/// あるノート開始 tick に有効な articulation を求める。
/// - momentary: lead 窓内(直前 lead tick〜同時)に始まるキースイッチ
/// - latch: それ以前で最後に始まったキースイッチ(次の latch まで有効)
fn resolve_articulation(
    note_tick: u64,
    keyswitches: &[(u64, &str, KeyswitchMode)],
    lead: u64,
) -> Option<String> {
    // momentary を優先(lead 窓内)
    let window_start = note_tick.saturating_sub(lead + 2);
    if let Some((_, name, _)) = keyswitches
        .iter()
        .filter(|(t, _, mode)| {
            *mode == KeyswitchMode::Momentary && *t >= window_start && *t <= note_tick + 2
        })
        .max_by_key(|(t, _, _)| *t)
    {
        return Some((*name).to_string());
    }
    // latch: note_tick 以前で最後に始まったもの
    keyswitches
        .iter()
        .filter(|(t, _, mode)| *mode == KeyswitchMode::Latch && *t <= note_tick + 2)
        .max_by_key(|(t, _, _)| *t)
        .map(|(_, name, _)| (*name).to_string())
}

/// 絶対 tick → "bar.beat.tick"。
fn ticks_to_bbt(tick: u64, grid: &TimeGrid) -> String {
    let tpb = grid.ticks_per_beat() as u64;
    let ticks_per_bar = grid.ticks_per_bar();
    let bar = tick / ticks_per_bar;
    let within_bar = tick % ticks_per_bar;
    let beat = within_bar / tpb;
    let rem = within_bar % tpb;
    format!("{}.{}.{:03}", bar + 1, beat + 1, rem)
}

/// tick 数 → 長さ "bars.beats.ticks"(オフセット表記)。
fn ticks_to_duration(ticks: u64, grid: &TimeGrid) -> String {
    let tpb = grid.ticks_per_beat() as u64;
    let ticks_per_bar = grid.ticks_per_bar();
    let bars = ticks / ticks_per_bar;
    let within = ticks % ticks_per_bar;
    let beats = within / tpb;
    let rem = within % tpb;
    format!("{bars}.{beats}.{rem:03}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::compile;
    use crate::model::PartPlan;
    use crate::validate::parse_validated;
    use serde_json::json;

    fn guitar_profile() -> DeviceProfile {
        parse_validated(&json!({
            "schema_version": "1.0",
            "id": "heavier7strings",
            "name": "Heavier7Strings",
            "device_type": "instrument",
            "octave_convention": "C3=60",
            "note_range": { "low": "B0", "high": "E5" },
            "keyswitches": [
                { "articulation": "palm_mute", "note": "C#0", "mode": "momentary", "confidence": "verified" }
            ]
        }))
        .unwrap()
    }

    /// compile → decompile のラウンドトリップで articulation が復元されること。
    #[test]
    fn roundtrip_recovers_articulation() {
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "rt",
            "device": "heavier7strings",
            "bpm": 120.0,
            "time_signature": "4/4",
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": [
                { "pitch": "E1", "start": "1.1.000", "duration": "0.0.240", "velocity": 110, "articulation": "palm_mute" },
                { "pitch": "G1", "start": "1.2.000", "duration": "0.0.240", "velocity": 100 }
            ]}]}]
        }))
        .unwrap();
        let profile = guitar_profile();
        let compiled = compile(&plan, &profile).unwrap();

        // 一時ファイルへ書いて decompile
        let dir = std::env::temp_dir();
        let path = dir.join("sora_decompile_rt.mid");
        std::fs::write(&path, &compiled.bytes).unwrap();
        let out = decompile_file(&path, &profile, "rt-back").unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(out.summary.note_count, 2);
        assert_eq!(out.summary.keyswitch_notes_consumed, 1);
        assert_eq!(out.summary.articulations_detected, vec!["palm_mute"]);

        let notes = &out.plan["sections"][0]["phrases"][0]["notes"];
        assert_eq!(notes[0]["pitch"], 40); // E1 @ C3=60
        assert_eq!(notes[0]["articulation"], "palm_mute");
        assert_eq!(notes[0]["start"], "1.1.000");
        assert!(notes[1].get("articulation").is_none());
    }

    /// 逆コンパイルした Plan が再コンパイルできること(UC5 の往復)。
    #[test]
    fn decompiled_plan_recompiles() {
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "rt2",
            "device": "heavier7strings",
            "bpm": 140.0,
            "time_signature": "4/4",
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": [
                { "pitch": "E1", "start": "1.1.000", "duration": "0.0.240", "velocity": 110, "articulation": "palm_mute" }
            ]}]}]
        }))
        .unwrap();
        let profile = guitar_profile();
        let compiled = compile(&plan, &profile).unwrap();
        let dir = std::env::temp_dir();
        let path = dir.join("sora_decompile_rt2.mid");
        std::fs::write(&path, &compiled.bytes).unwrap();
        let out = decompile_file(&path, &profile, "rt2-back").unwrap();
        std::fs::remove_file(&path).ok();

        // 逆コンパイル結果を PartPlan として再検証・再コンパイル
        let reparsed: PartPlan = parse_validated(&out.plan).unwrap();
        let recompiled = compile(&reparsed, &profile).unwrap();
        assert_eq!(recompiled.report.note_count, 1);
        assert_eq!(recompiled.report.keyswitch_count, 1);
    }
}
