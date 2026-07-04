//! Part Plan → SMF コンパイラ(技術要件書 §2.2, §7)。
//!
//! 決定論性の保証: 同一の Plan + Profile(+ seed)からバイト同一の `.mid` を
//! 生成する。ヒューマナイズは ChaCha8 + 整数/2の冪除算のみで実装し、
//! プラットフォーム依存の超越関数を使わない。

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

use crate::error::{CoreError, ValidationIssue};
use crate::model::{Confidence, DeviceProfile, KeyswitchMode, PartPlan, Polyphony};
use crate::profile::ResolvedProfile;
use crate::types::{MidiNote, Velocity, parse_position};

use super::timing::{TimeGrid, micros_per_quarter, ms_to_ticks};

/// コンパイル結果。
#[derive(Debug)]
pub struct CompileOutput {
    /// SMF バイト列(Format 1)
    pub bytes: Vec<u8>,
    /// レビュー用レポート(警告含む)
    pub report: CompileReport,
}

/// コンパイルレポート(`--format json` でそのまま出力される)。
#[derive(Debug, Serialize)]
pub struct CompileReport {
    pub part_id: String,
    pub device: String,
    pub bpm: f64,
    pub time_signature: String,
    pub ppq: u32,
    pub note_count: usize,
    pub keyswitch_count: usize,
    /// 曲末尾の絶対 tick
    pub end_tick: u64,
    pub warnings: Vec<CompileWarning>,
}

/// コンパイル警告(エラーではないがレビューすべき事項)。
#[derive(Debug, Serialize)]
pub struct CompileWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

/// 解決済みノート(コンパイル中間表現)。
struct ResolvedNote {
    pointer: String,
    start: u64,
    duration: u64,
    note: MidiNote,
    velocity: Velocity,
    articulation: Option<String>,
    /// 小節頭のグルーヴ基準点(キック等)— タイミングヒューマナイズを免除
    timing_anchor: bool,
}

/// キースイッチイベント(解決済み)。
struct KeyswitchEvent {
    start: u64,
    duration: u64,
    note: MidiNote,
}

/// CC イベント(解決済み)。
struct CcEvent {
    at: u64,
    cc: u8,
    value: u8,
}

/// Part Plan を SMF へコンパイルする。
///
/// L3 検証(音域・奏法・時間位置の相互制約)はここで全件列挙され、
/// 1 件でもエラーがあれば `.mid` は生成されない。
pub fn compile(plan: &PartPlan, profile: &DeviceProfile) -> Result<CompileOutput, CoreError> {
    let resolved = ResolvedProfile::resolve(profile)?;
    let grid = TimeGrid::new(&plan.time_signature, plan.ppq)?;

    if plan.device != resolved.id {
        return Err(CoreError::Validation {
            issues: vec![ValidationIssue {
                pointer: "/device".to_string(),
                code: "DEVICE_MISMATCH".to_string(),
                message: format!(
                    "plan targets device `{}` but profile is `{}`",
                    plan.device, resolved.id
                ),
                hint: Some("plan.device と Profile の id を一致させてください".to_string()),
            }],
        });
    }

    let mut issues: Vec<ValidationIssue> = Vec::new();
    let mut warnings: Vec<CompileWarning> = Vec::new();
    let mut notes: Vec<ResolvedNote> = Vec::new();

    let is_bass_role = profile.roles.iter().any(|r| r.contains("bass"));

    let mut prev_section_start_bar = 0u32;
    for (si, section) in plan.sections.iter().enumerate() {
        let s_ptr = format!("/sections/{si}");
        if section.start_bar <= prev_section_start_bar {
            issues.push(ValidationIssue {
                pointer: format!("{s_ptr}/start_bar"),
                code: "SECTION_ORDER".to_string(),
                message: format!(
                    "section `{}` start_bar {} is not after previous section",
                    section.label, section.start_bar
                ),
                hint: Some("sections は曲の時間順(start_bar 昇順)に並べてください".to_string()),
            });
        }
        prev_section_start_bar = section.start_bar;

        for (pi, phrase) in section.phrases.iter().enumerate() {
            for (ni, plan_note) in phrase.notes.iter().enumerate() {
                let ptr = format!("{s_ptr}/phrases/{pi}/notes/{ni}");

                // 時間解決
                let start = match parse_position(&plan_note.start, grid.beats_per_bar) {
                    Ok(p) => {
                        if p.bar < section.start_bar {
                            warnings.push(CompileWarning {
                                code: "NOTE_BEFORE_SECTION".to_string(),
                                message: format!(
                                    "note at {} starts before section `{}` (start_bar {})",
                                    plan_note.start, section.label, section.start_bar
                                ),
                                pointer: Some(ptr.clone()),
                            });
                        }
                        Some(p.to_absolute_ticks(grid.ticks_per_beat(), grid.beats_per_bar))
                    }
                    Err(e) => {
                        push_issue(&mut issues, &ptr, e.code(), &e);
                        None
                    }
                };
                let duration = match plan_note.duration.parse::<crate::types::BarBeatTick>() {
                    Ok(d) => {
                        let ticks = d.to_duration_ticks(grid.ticks_per_beat(), grid.beats_per_bar);
                        if ticks == 0 {
                            push_issue(
                                &mut issues,
                                &ptr,
                                "ZERO_DURATION",
                                &CoreError::InvalidTimePosition {
                                    value: plan_note.duration.clone(),
                                    hint: "duration は 1 tick 以上が必要です".to_string(),
                                },
                            );
                            None
                        } else {
                            Some(ticks)
                        }
                    }
                    Err(e) => {
                        push_issue(&mut issues, &ptr, e.code(), &e);
                        None
                    }
                };

                // ピッチ解決(pitch XOR kit_piece)
                let mut timing_anchor = false;
                let midi_note = match (&plan_note.pitch, &plan_note.kit_piece) {
                    (Some(_), Some(_)) | (None, None) => {
                        issues.push(ValidationIssue {
                            pointer: ptr.clone(),
                            code: "PITCH_OR_KIT_PIECE".to_string(),
                            message: "exactly one of `pitch` or `kit_piece` must be set".to_string(),
                            hint: Some(
                                "音程系は pitch、ドラムは kit_piece をどちらか一方だけ指定してください".to_string(),
                            ),
                        });
                        None
                    }
                    (Some(spec), None) => match MidiNote::resolve(spec, resolved.convention) {
                        Ok(note) => {
                            if let Some((low, high)) = resolved.note_range {
                                if note < low || note > high {
                                    let e = CoreError::NoteOutOfRange {
                                        note: note.value(),
                                        low: low.value(),
                                        high: high.value(),
                                        device: resolved.id.clone(),
                                        transpose_hint: transpose_hint(note, low, high),
                                    };
                                    push_issue(&mut issues, &ptr, e.code(), &e);
                                    None
                                } else {
                                    Some(note)
                                }
                            } else {
                                Some(note)
                            }
                        }
                        Err(e) => {
                            push_issue(&mut issues, &ptr, e.code(), &e);
                            None
                        }
                    },
                    (None, Some(piece)) => {
                        if resolved.drum_map.is_empty() {
                            let e = CoreError::NoDrumMap {
                                device: resolved.id.clone(),
                            };
                            push_issue(&mut issues, &ptr, e.code(), &e);
                            None
                        } else if let Some(dp) = resolved.drum_map.get(piece) {
                            timing_anchor = piece == "kick";
                            if dp.confidence == Confidence::Unverified {
                                warnings.push(CompileWarning {
                                    code: "UNVERIFIED_MAPPING".to_string(),
                                    message: format!(
                                        "kit_piece `{piece}` mapping is unverified — 実機確認を推奨"
                                    ),
                                    pointer: Some(ptr.clone()),
                                });
                            }
                            Some(dp.note)
                        } else {
                            let e = CoreError::UnknownKitPiece {
                                name: piece.clone(),
                                device: resolved.id.clone(),
                                available: resolved.drum_map.keys().cloned().collect(),
                            };
                            push_issue(&mut issues, &ptr, e.code(), &e);
                            None
                        }
                    }
                };

                // ベロシティ
                let velocity = match Velocity::new(plan_note.velocity) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        push_issue(&mut issues, &ptr, e.code(), &e);
                        None
                    }
                };

                // 奏法
                let articulation = match &plan_note.articulation {
                    Some(name) => {
                        if let Some(ks) = resolved.keyswitches.get(name) {
                            if ks.confidence == Confidence::Unverified {
                                warnings.push(CompileWarning {
                                    code: "UNVERIFIED_ARTICULATION".to_string(),
                                    message: format!(
                                        "articulation `{name}` is unverified — 実機確認を推奨(profile verify-midi)"
                                    ),
                                    pointer: Some(ptr.clone()),
                                });
                            }
                            Some(name.clone())
                        } else {
                            let e = CoreError::UnknownArticulation {
                                name: name.clone(),
                                device: resolved.id.clone(),
                                available: resolved.keyswitches.keys().cloned().collect(),
                            };
                            push_issue(&mut issues, &ptr, e.code(), &e);
                            None
                        }
                    }
                    None => None,
                };

                if let (Some(start), Some(duration), Some(note), Some(velocity)) =
                    (start, duration, midi_note, velocity)
                {
                    // ベース役の小節頭ダウンビートもグルーヴ基準点(§7)
                    let on_downbeat = start % grid.ticks_per_bar() == 0;
                    notes.push(ResolvedNote {
                        pointer: ptr,
                        start,
                        duration,
                        note,
                        velocity,
                        articulation,
                        timing_anchor: on_downbeat && (timing_anchor || is_bass_role),
                    });
                }
            }
        }
    }

    // CC オートメーションレーン(パームミュート量など、キースイッチで表現できない連続制御)
    let mut cc_events: Vec<CcEvent> = Vec::new();
    for (li, lane) in plan.controls.iter().enumerate() {
        let lane_ptr = format!("/controls/{li}");
        if lane.cc > 127 {
            push_issue(
                &mut issues,
                &format!("{lane_ptr}/cc"),
                "INVALID_CC_NUMBER",
                &CoreError::InvalidVelocity { value: lane.cc },
            );
            continue;
        }
        // Profile の cc_map にない CC は警告(remap 可能なので許容)
        let safe_range = match resolved.cc_map.get(&lane.cc) {
            Some(sr) => *sr,
            None => {
                warnings.push(CompileWarning {
                    code: "CC_NOT_IN_PROFILE".to_string(),
                    message: format!(
                        "CC {} is not in device `{}` cc_map — remap されている可能性があります",
                        lane.cc, resolved.id
                    ),
                    pointer: Some(lane_ptr.clone()),
                });
                None
            }
        };
        for (pi, point) in lane.points.iter().enumerate() {
            let p_ptr = format!("{lane_ptr}/points/{pi}");
            let at = match parse_position(&point.at, grid.beats_per_bar) {
                Ok(p) => Some(p.to_absolute_ticks(grid.ticks_per_beat(), grid.beats_per_bar)),
                Err(e) => {
                    push_issue(&mut issues, &format!("{p_ptr}/at"), e.code(), &e);
                    None
                }
            };
            if point.value > 127 {
                push_issue(
                    &mut issues,
                    &format!("{p_ptr}/value"),
                    "INVALID_CC_VALUE",
                    &CoreError::InvalidVelocity { value: point.value },
                );
            } else if let Some([min, max]) = safe_range
                && (point.value < min || point.value > max)
            {
                issues.push(ValidationIssue {
                    pointer: format!("{p_ptr}/value"),
                    code: "CC_OUT_OF_SAFE_RANGE".to_string(),
                    message: format!(
                        "CC {} value {} outside safe_range {}..={}",
                        lane.cc, point.value, min, max
                    ),
                    hint: Some(
                        "Device Profile の safe_range 内に収めるか、明示的に範囲を広げてください"
                            .to_string(),
                    ),
                });
            }
            if let Some(at) = at {
                cc_events.push(CcEvent {
                    at,
                    cc: lane.cc,
                    value: point.value,
                });
            }
        }
    }

    if !issues.is_empty() {
        return Err(CoreError::Validation { issues });
    }

    // 決定論的な処理順: 開始位置 → ピッチ → Plan 内の出現順(pointer)
    notes.sort_by(|a, b| (a.start, a.note, &a.pointer).cmp(&(b.start, b.note, &b.pointer)));

    // ヒューマナイズ(seed 固定・整数演算のみ → プラットフォーム非依存)
    if let Some(humanize) = &plan.humanize {
        let mut rng = ChaCha8Rng::seed_from_u64(humanize.seed);
        let max_ticks = ms_to_ticks(humanize.timing_ms * 3.0, plan.bpm, plan.ppq);
        for n in notes.iter_mut() {
            // Irwin–Hall(一様乱数 12 個の和)で正規分布を近似。
            // 超越関数を避け、2^32 除算のみで決定論を保つ。
            let mut sum = 0.0f64;
            for _ in 0..12 {
                sum += rng.random::<u32>() as f64 / 4_294_967_296.0;
            }
            let std_normal = sum - 6.0; // ≈ N(0,1)
            let delta_ms = std_normal * humanize.timing_ms;
            let delta_ticks =
                ms_to_ticks(delta_ms, plan.bpm, plan.ppq).clamp(-max_ticks, max_ticks);
            let vel_delta =
                rng.random_range(-(humanize.velocity as i16)..=(humanize.velocity as i16));

            if !n.timing_anchor {
                n.start = n.start.saturating_add_signed(delta_ticks);
            }
            n.velocity = n.velocity.offset_clamped(vel_delta);
        }
        // ヒューマナイズで順序が入れ替わり得るため再ソート
        notes.sort_by(|a, b| (a.start, a.note, &a.pointer).cmp(&(b.start, b.note, &b.pointer)));
    }

    // モノフォニック: 同一ピッチの重なりをトリム(§7)
    if resolved.polyphony == Polyphony::Mono {
        for i in 0..notes.len() {
            let (head, tail) = notes.split_at_mut(i + 1);
            let cur = &mut head[i];
            if let Some(next) = tail.iter().find(|n| n.note == cur.note)
                && next.start < cur.start + cur.duration
            {
                warnings.push(CompileWarning {
                    code: "NOTE_OVERLAP_TRIMMED".to_string(),
                    message: format!(
                        "overlapping note {} at tick {} trimmed to {} ticks (mono device)",
                        cur.note.value(),
                        cur.start,
                        next.start - cur.start
                    ),
                    pointer: Some(cur.pointer.clone()),
                });
                cur.duration = next.start - cur.start;
            }
        }
    }

    // キースイッチイベント生成(§7)
    let mut keyswitches: Vec<KeyswitchEvent> = Vec::new();
    let mut latch_state: Option<String> = None;
    let lead = resolved.keyswitch_lead_ticks as u64;
    for n in &notes {
        match &n.articulation {
            Some(name) => {
                let ks = &resolved.keyswitches[name];
                let ks_start = n.start.saturating_sub(lead);
                match ks.mode {
                    KeyswitchMode::Momentary => keyswitches.push(KeyswitchEvent {
                        start: ks_start,
                        duration: n.start + n.duration - ks_start,
                        note: ks.note,
                    }),
                    KeyswitchMode::Latch => {
                        if latch_state.as_deref() != Some(name) {
                            keyswitches.push(KeyswitchEvent {
                                start: ks_start,
                                duration: 10,
                                note: ks.note,
                            });
                            latch_state = Some(name.clone());
                        }
                    }
                }
            }
            None => {
                if let Some(active) = latch_state.take() {
                    warnings.push(CompileWarning {
                        code: "LATCH_STILL_ACTIVE".to_string(),
                        message: format!(
                            "latch articulation `{active}` remains active at tick {} — 解除用の奏法(サステイン等)を明示してください",
                            n.start
                        ),
                        pointer: Some(n.pointer.clone()),
                    });
                }
            }
        }
    }

    // イベント列へ変換して SMF を構築
    let bytes = build_smf(
        plan,
        &grid,
        resolved.midi_channel,
        &notes,
        &keyswitches,
        &cc_events,
    );
    let end_tick = notes
        .iter()
        .map(|n| n.start + n.duration)
        .max()
        .unwrap_or(0);

    Ok(CompileOutput {
        bytes,
        report: CompileReport {
            part_id: plan.part_id.clone(),
            device: plan.device.clone(),
            bpm: plan.bpm,
            time_signature: plan.time_signature.clone(),
            ppq: plan.ppq,
            note_count: notes.len(),
            keyswitch_count: keyswitches.len(),
            end_tick,
            warnings,
        },
    })
}

fn push_issue(issues: &mut Vec<ValidationIssue>, pointer: &str, code: &str, err: &CoreError) {
    issues.push(ValidationIssue {
        pointer: pointer.to_string(),
        code: code.to_string(),
        message: err.to_string(),
        hint: err.hint(),
    });
}

/// 音域外ノートに対する移調ヒント(±3 オクターブ以内で収まる最小シフト)。
fn transpose_hint(note: MidiNote, low: MidiNote, high: MidiNote) -> Option<i8> {
    for octaves in 1..=3i32 {
        for sign in [1, -1] {
            let shift = sign * octaves * 12;
            let shifted = note.value() as i32 + shift;
            if shifted >= low.value() as i32 && shifted <= high.value() as i32 {
                return Some(shift as i8);
            }
        }
    }
    None
}

/// イベント種別の同 tick 内での順序(note off → CC → keyswitch on → note on)。
/// CC はノート発音前に確定させたいので keyswitch/note on より前。
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum EventClass {
    NoteOff = 0,
    ControlChange = 1,
    KeyswitchOn = 2,
    NoteOn = 3,
}

fn build_smf(
    plan: &PartPlan,
    grid: &TimeGrid,
    channel: u8,
    notes: &[ResolvedNote],
    keyswitches: &[KeyswitchEvent],
    cc_events: &[CcEvent],
) -> Vec<u8> {
    // (abs_tick, class, note) → 決定論的順序
    let mut events: Vec<(u64, EventClass, u8, MidiMessage)> = Vec::new();
    for cc in cc_events {
        events.push((
            cc.at,
            EventClass::ControlChange,
            cc.cc,
            MidiMessage::Controller {
                controller: u7::new(cc.cc),
                value: u7::new(cc.value),
            },
        ));
    }
    for n in notes {
        events.push((
            n.start,
            EventClass::NoteOn,
            n.note.value(),
            MidiMessage::NoteOn {
                key: u7::new(n.note.value()),
                vel: u7::new(n.velocity.value()),
            },
        ));
        events.push((
            n.start + n.duration,
            EventClass::NoteOff,
            n.note.value(),
            MidiMessage::NoteOff {
                key: u7::new(n.note.value()),
                vel: u7::new(64),
            },
        ));
    }
    for ks in keyswitches {
        events.push((
            ks.start,
            EventClass::KeyswitchOn,
            ks.note.value(),
            MidiMessage::NoteOn {
                key: u7::new(ks.note.value()),
                vel: u7::new(100),
            },
        ));
        events.push((
            ks.start + ks.duration,
            EventClass::NoteOff,
            ks.note.value(),
            MidiMessage::NoteOff {
                key: u7::new(ks.note.value()),
                vel: u7::new(64),
            },
        ));
    }
    events.sort_by(|a, b| (a.0, &a.1, a.2).cmp(&(b.0, &b.1, b.2)));

    // Track 0: テンポ・拍子
    let mut track0: Vec<TrackEvent> = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(b"Sora")),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(micros_per_quarter(plan.bpm)))),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(
                grid.beats_per_bar as u8,
                grid.denominator_log2(),
                24,
                8,
            )),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        },
    ];

    // Track 1: 演奏データ
    let mut track1: Vec<TrackEvent> = vec![TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(plan.part_id.as_bytes())),
    }];
    let mut last_tick = 0u64;
    for (tick, _, _, message) in &events {
        let delta = (tick - last_tick) as u32;
        last_tick = *tick;
        track1.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: u4::new(channel),
                message: *message,
            },
        });
    }
    track1.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(
            Format::Parallel,
            Timing::Metrical(u15::new(plan.ppq as u16)),
        ),
        tracks: vec![std::mem::take(&mut track0), track1],
    };
    let mut bytes = Vec::new();
    #[allow(clippy::expect_used)] // Vec への書き込みは I/O エラーを起こさない
    smf.write(&mut bytes)
        .expect("in-memory SMF write cannot fail");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::parse_validated;
    use serde_json::json;

    fn profile() -> DeviceProfile {
        parse_validated(&json!({
            "schema_version": "1.0",
            "id": "heavier7strings",
            "name": "Heavier7Strings",
            "device_type": "instrument",
            "roles": ["rhythm_guitar"],
            "octave_convention": "C3=60",
            "note_range": { "low": "B-1", "high": "E4" },
            "keyswitches": [
                { "articulation": "palm_mute", "note": "C-2", "mode": "momentary", "confidence": "verified" },
                { "articulation": "pinch_harmonic", "note": "D#-2", "mode": "latch", "confidence": "unverified" }
            ]
        }))
        .unwrap()
    }

    fn plan(notes: serde_json::Value) -> PartPlan {
        parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "test-riff",
            "device": "heavier7strings",
            "bpm": 142.0,
            "time_signature": "4/4",
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": notes }] }]
        }))
        .unwrap()
    }

    #[test]
    fn compiles_simple_riff_with_keyswitch() {
        let plan = plan(json!([
            { "pitch": "E0", "start": "1.1.000", "duration": "0.0.240", "velocity": 112, "articulation": "palm_mute" },
            { "pitch": "E0", "start": "1.2.000", "duration": "0.0.240", "velocity": 105 }
        ]));
        let out = compile(&plan, &profile()).unwrap();
        assert_eq!(out.report.note_count, 2);
        assert_eq!(out.report.keyswitch_count, 1);
        assert!(out.bytes.starts_with(b"MThd"));
        // 再コンパイルでバイト同一(決定論)
        let out2 = compile(&plan, &profile()).unwrap();
        assert_eq!(out.bytes, out2.bytes);
    }

    #[test]
    fn out_of_range_note_gets_transpose_hint() {
        let plan = plan(json!([
            { "pitch": "C-2", "start": "1.1.000", "duration": "0.1.000", "velocity": 100 }
        ]));
        let err = compile(&plan, &profile()).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!()
        };
        assert_eq!(issues[0].code, "NOTE_OUT_OF_RANGE");
        assert!(issues[0].hint.as_ref().unwrap().contains("+24"));
    }

    #[test]
    fn unknown_articulation_lists_available() {
        let plan = plan(json!([
            { "pitch": "E0", "start": "1.1.000", "duration": "0.1.000", "velocity": 100, "articulation": "palm-mute" }
        ]));
        let err = compile(&plan, &profile()).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!()
        };
        assert_eq!(issues[0].code, "UNKNOWN_ARTICULATION");
        assert!(issues[0].hint.as_ref().unwrap().contains("palm_mute"));
    }

    #[test]
    fn all_issues_reported_at_once() {
        let plan = plan(json!([
            { "pitch": "C-2", "start": "1.1.000", "duration": "0.1.000", "velocity": 100 },
            { "pitch": "E0", "start": "9.9.000", "duration": "0.1.000", "velocity": 100 },
            { "pitch": "E0", "start": "2.1.000", "duration": "0.0.000", "velocity": 200 }
        ]));
        let err = compile(&plan, &profile()).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!()
        };
        assert!(
            issues.len() >= 4,
            "expected all issues collected: {issues:?}"
        );
    }

    #[test]
    fn humanize_is_deterministic_and_seed_sensitive() {
        let base = json!([
            { "pitch": "E0", "start": "1.2.000", "duration": "0.0.240", "velocity": 100 },
            { "pitch": "G0", "start": "1.3.000", "duration": "0.0.240", "velocity": 100 }
        ]);
        let mut p1 = plan(base.clone());
        p1.humanize = Some(crate::model::Humanize {
            timing_ms: 8.0,
            velocity: 10,
            seed: 42,
        });
        let mut p2 = plan(base.clone());
        p2.humanize = Some(crate::model::Humanize {
            timing_ms: 8.0,
            velocity: 10,
            seed: 42,
        });
        let mut p3 = plan(base);
        p3.humanize = Some(crate::model::Humanize {
            timing_ms: 8.0,
            velocity: 10,
            seed: 43,
        });

        let prof = profile();
        let b1 = compile(&p1, &prof).unwrap().bytes;
        let b2 = compile(&p2, &prof).unwrap().bytes;
        let b3 = compile(&p3, &prof).unwrap().bytes;
        assert_eq!(b1, b2, "same seed → byte-identical");
        assert_ne!(b1, b3, "different seed → different bytes");
    }

    fn cc_profile() -> DeviceProfile {
        parse_validated(&json!({
            "schema_version": "1.0",
            "id": "heavier7strings",
            "name": "Heavier7Strings",
            "device_type": "instrument",
            "roles": ["rhythm_guitar"],
            "octave_convention": "C3=60",
            "note_range": { "low": "A0", "high": "E5" },
            "cc_map": [
                { "cc": 16, "function": "palm mute mix", "safe_range": [0, 127], "confidence": "manual" }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn cc_lane_emits_controller_events() {
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "pm-riff",
            "device": "heavier7strings",
            "bpm": 150.0,
            "time_signature": "4/4",
            "controls": [
                { "cc": 16, "function": "palm mute mix", "points": [
                    { "at": "1.1.000", "value": 100 },
                    { "at": "2.1.000", "value": 20 }
                ]}
            ],
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": [
                { "pitch": "E1", "start": "1.1.000", "duration": "0.0.240", "velocity": 110 }
            ]}]}]
        }))
        .unwrap();
        let out = compile(&plan, &cc_profile()).unwrap();
        let smf = Smf::parse(&out.bytes).unwrap();
        let cc_count = smf.tracks[1]
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    TrackEventKind::Midi {
                        message: MidiMessage::Controller { .. },
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            cc_count, 2,
            "two CC16 points should emit two controller events"
        );
    }

    #[test]
    fn cc_out_of_safe_range_errors() {
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "bad-cc",
            "device": "heavier7strings",
            "bpm": 150.0,
            "time_signature": "4/4",
            "controls": [
                { "cc": 16, "points": [ { "at": "1.1.000", "value": 200 } ] }
            ],
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": [
                { "pitch": "E1", "start": "1.1.000", "duration": "0.0.240", "velocity": 110 }
            ]}]}]
        }))
        .unwrap();
        // value 200 は L2(0-127)で弾かれるか、コンパイル時に弾かれる
        let err = compile(&plan, &cc_profile()).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!()
        };
        assert!(
            issues
                .iter()
                .any(|i| i.pointer.contains("/controls/0/points/0/value"))
        );
    }

    #[test]
    fn cc_not_in_profile_warns() {
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "unknown-cc",
            "device": "heavier7strings",
            "bpm": 150.0,
            "time_signature": "4/4",
            "controls": [
                { "cc": 99, "points": [ { "at": "1.1.000", "value": 64 } ] }
            ],
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": [
                { "pitch": "E1", "start": "1.1.000", "duration": "0.0.240", "velocity": 110 }
            ]}]}]
        }))
        .unwrap();
        let out = compile(&plan, &cc_profile()).unwrap();
        assert!(
            out.report
                .warnings
                .iter()
                .any(|w| w.code == "CC_NOT_IN_PROFILE")
        );
    }

    #[test]
    fn unverified_articulation_warns() {
        let plan = plan(json!([
            { "pitch": "E0", "start": "1.1.000", "duration": "0.0.240", "velocity": 100, "articulation": "pinch_harmonic" }
        ]));
        let out = compile(&plan, &profile()).unwrap();
        assert!(
            out.report
                .warnings
                .iter()
                .any(|w| w.code == "UNVERIFIED_ARTICULATION")
        );
    }

    #[test]
    fn mono_profile_trims_same_pitch_overlap() {
        let mut prof = profile();
        prof.polyphony = Polyphony::Mono;
        let plan = plan(json!([
            { "pitch": "E0", "start": "1.1.000", "duration": "2.0.000", "velocity": 100 },
            { "pitch": "E0", "start": "1.3.000", "duration": "0.1.000", "velocity": 100 }
        ]));
        let out = compile(&plan, &prof).unwrap();
        assert!(
            out.report
                .warnings
                .iter()
                .any(|w| w.code == "NOTE_OVERLAP_TRIMMED")
        );
    }

    #[test]
    fn kick_downbeat_is_timing_anchor() {
        let drum_profile: DeviceProfile = parse_validated(&json!({
            "schema_version": "1.0",
            "id": "modo-drum",
            "name": "MODO DRUM",
            "device_type": "instrument",
            "octave_convention": "C3=60",
            "drum_map": [
                { "kit_piece": "kick", "note": 36, "confidence": "verified" },
                { "kit_piece": "snare", "note": 38, "confidence": "verified" }
            ]
        }))
        .unwrap();
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "drums-v1",
            "device": "modo-drum",
            "bpm": 142.0,
            "time_signature": "4/4",
            "humanize": { "timing_ms": 8.0, "velocity": 10, "seed": 7 },
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": [
                { "kit_piece": "kick", "start": "1.1.000", "duration": "0.0.120", "velocity": 120 },
                { "kit_piece": "snare", "start": "1.2.000", "duration": "0.0.120", "velocity": 110 }
            ]}]}]
        }))
        .unwrap();
        let out = compile(&plan, &drum_profile).unwrap();
        // 小節頭キックは tick 0 のまま(アンカー)。SMF 先頭の演奏イベントの delta が 0 であることを確認
        let smf = Smf::parse(&out.bytes).unwrap();
        let first_note_on = smf.tracks[1]
            .iter()
            .scan(0u64, |acc, ev| {
                *acc += ev.delta.as_int() as u64;
                Some((*acc, ev.kind))
            })
            .find(|(_, kind)| {
                matches!(
                    kind,
                    TrackEventKind::Midi {
                        message: MidiMessage::NoteOn { .. },
                        ..
                    }
                )
            });
        assert_eq!(
            first_note_on.unwrap().0,
            0,
            "downbeat kick must stay on the grid"
        );
    }
}
