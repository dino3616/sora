//! 検証用 MIDI 生成(`sora profile verify-midi`、技術要件書 §16 リスク 1)。
//!
//! Profile の全奏法・全キットピースを 1 音ずつ鳴らす `.mid` を生成し、
//! ユーザーが実機で「キースイッチが意図通り発音するか」を確認できるようにする。
//! 確認結果で Profile の confidence を verified へ昇格する。

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use serde::Serialize;

use crate::error::CoreError;
use crate::model::{DeviceProfile, KeyswitchMode};
use crate::profile::ResolvedProfile;

const PPQ: u16 = 480;
const BPM: f64 = 90.0;
/// 各検証項目の間隔(2 小節 @ 4/4)
const SLOT_TICKS: u64 = 480 * 4 * 2;
/// 検証ノートの長さ(2 分音符)
const NOTE_TICKS: u64 = 960;

/// 検証項目(レポートで DAW 上の位置と対応付ける)。
#[derive(Debug, Serialize)]
pub struct VerifyItem {
    /// 小節番号(1 始まり、4/4)
    pub bar: u32,
    /// 検証対象("articulation:palm_mute" / "kit_piece:kick" / "range:low" 等)
    pub subject: String,
    /// 鳴らすノート番号
    pub note: u8,
    /// 期待される結果の説明
    pub expectation: String,
}

/// 検証用 MIDI の生成結果。
#[derive(Debug)]
pub struct VerifyMidi {
    pub bytes: Vec<u8>,
    pub items: Vec<VerifyItem>,
}

/// Profile から検証用 MIDI を生成する。
///
/// 構成(各項目 2 小節間隔、テンポ 90):
/// 1. 音域の下端・上端(instrument のみ)
/// 2. 各奏法: キースイッチ + 中央付近のノート
/// 3. 各キットピース: 単発
pub fn generate_verify_midi(profile: &DeviceProfile) -> Result<VerifyMidi, CoreError> {
    let resolved = ResolvedProfile::resolve(profile)?;
    let mut items: Vec<VerifyItem> = Vec::new();
    // (tick, class(0=off,1=ks_on,2=on), note, message)
    let mut events: Vec<(u64, u8, u8, MidiMessage)> = Vec::new();
    let mut texts: Vec<(u64, String)> = Vec::new();
    let mut slot = 0u64;

    let push_note = |events: &mut Vec<(u64, u8, u8, MidiMessage)>,
                     tick: u64,
                     note: u8,
                     vel: u8,
                     dur: u64,
                     class_on: u8| {
        events.push((
            tick,
            class_on,
            note,
            MidiMessage::NoteOn {
                key: u7::new(note),
                vel: u7::new(vel),
            },
        ));
        events.push((
            tick + dur,
            0,
            note,
            MidiMessage::NoteOff {
                key: u7::new(note),
                vel: u7::new(64),
            },
        ));
    };

    // 1. 音域チェック
    if let Some((low, high)) = resolved.note_range {
        for (subject, note) in [("range:low", low), ("range:high", high)] {
            let tick = slot * SLOT_TICKS;
            let bar = (slot * 2 + 1) as u32;
            texts.push((tick, format!("{subject} note={}", note.value())));
            push_note(&mut events, tick, note.value(), 100, NOTE_TICKS, 2);
            items.push(VerifyItem {
                bar,
                subject: subject.to_string(),
                note: note.value(),
                expectation:
                    "楽音として発音される(無音や異音なら note_range か octave_convention が誤り)"
                        .to_string(),
            });
            slot += 1;
        }
    }

    // 2. 奏法チェック(BTreeMap なので順序は articulation 名の辞書順で決定論的)
    let center = resolved
        .note_range
        .map(|(low, high)| {
            MidiNoteCenter {
                low: low.value(),
                high: high.value(),
            }
            .center()
        })
        .unwrap_or(60);
    for (name, ks) in &resolved.keyswitches {
        let tick = slot * SLOT_TICKS;
        let bar = (slot * 2 + 1) as u32;
        texts.push((tick, format!("articulation:{name} ks={}", ks.note.value())));
        let ks_dur = match ks.mode {
            KeyswitchMode::Momentary => NOTE_TICKS + 40,
            KeyswitchMode::Latch => 10,
        };
        // キースイッチはノートの 20 tick 前
        push_note(&mut events, tick, ks.note.value(), 100, ks_dur, 1);
        push_note(&mut events, tick + 20, center, 100, NOTE_TICKS, 2);
        items.push(VerifyItem {
            bar,
            subject: format!("articulation:{name}"),
            note: center,
            expectation: format!(
                "奏法 `{name}` で発音される(通常音のままならキースイッチ {} が誤り)",
                ks.note.value()
            ),
        });
        slot += 1;
    }

    // 3. キットピースチェック
    for (name, piece) in &resolved.drum_map {
        let tick = slot * SLOT_TICKS;
        let bar = (slot * 2 + 1) as u32;
        texts.push((
            tick,
            format!("kit_piece:{name} note={}", piece.note.value()),
        ));
        push_note(&mut events, tick, piece.note.value(), 110, 240, 2);
        items.push(VerifyItem {
            bar,
            subject: format!("kit_piece:{name}"),
            note: piece.note.value(),
            expectation: format!("`{name}` が発音される(別のピースが鳴るならマッピングが誤り)"),
        });
        slot += 1;
    }

    events.sort_by_key(|a| (a.0, a.1, a.2));

    // SMF 構築
    let track0 = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(b"Sora Verify")),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new((60_000_000.0 / BPM) as u32))),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        },
    ];

    let mut track1: Vec<TrackEvent> = vec![TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(profile.id.as_bytes())),
    }];
    // テキストマーカーとノートイベントをマージ
    let mut merged: Vec<(u64, u8, TrackEventKind)> = Vec::new();
    for (tick, text) in &texts {
        merged.push((
            *tick,
            0,
            TrackEventKind::Meta(MetaMessage::Text(text.as_bytes())),
        ));
    }
    for (tick, class, _, message) in &events {
        merged.push((
            *tick,
            class + 1,
            TrackEventKind::Midi {
                channel: u4::new(resolved.midi_channel),
                message: *message,
            },
        ));
    }
    merged.sort_by_key(|a| (a.0, a.1));

    let mut last = 0u64;
    for (tick, _, kind) in merged {
        track1.push(TrackEvent {
            delta: u28::new((tick - last) as u32),
            kind,
        });
        last = tick;
    }
    track1.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ))),
        tracks: vec![track0, track1],
    };
    let mut bytes = Vec::new();
    #[allow(clippy::expect_used)] // Vec への書き込みは I/O エラーを起こさない
    smf.write(&mut bytes)
        .expect("in-memory SMF write cannot fail");

    Ok(VerifyMidi { bytes, items })
}

struct MidiNoteCenter {
    low: u8,
    high: u8,
}

impl MidiNoteCenter {
    fn center(&self) -> u8 {
        self.low + (self.high - self.low) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::parse_validated;
    use serde_json::json;

    #[test]
    fn generates_items_for_range_and_articulations() {
        let profile: DeviceProfile = parse_validated(&json!({
            "schema_version": "1.0",
            "id": "heavier7strings",
            "name": "Heavier7Strings",
            "device_type": "instrument",
            "octave_convention": "C3=60",
            "note_range": { "low": "B-1", "high": "E4" },
            "keyswitches": [
                { "articulation": "palm_mute", "note": "C-2", "mode": "momentary", "confidence": "manual" },
                { "articulation": "pinch_harmonic", "note": "D#-2", "mode": "latch", "confidence": "unverified" }
            ]
        }))
        .unwrap();
        let verify = generate_verify_midi(&profile).unwrap();
        // range low/high + 2 奏法 = 4 項目
        assert_eq!(verify.items.len(), 4);
        assert!(verify.bytes.starts_with(b"MThd"));
        // 決定論
        let again = generate_verify_midi(&profile).unwrap();
        assert_eq!(verify.bytes, again.bytes);
        // 項目は 2 小節間隔
        assert_eq!(verify.items[0].bar, 1);
        assert_eq!(verify.items[1].bar, 3);
    }
}
