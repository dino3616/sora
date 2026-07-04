//! SMF → 実時間再生イベント列(技術要件書 §9)。
//!
//! テンポマップを解決して各 MIDI メッセージの絶対時刻(マイクロ秒)を算出する。
//! 実際のポート送信(midir)は CLI 側が担い、ここは純粋な変換なのでテスト可能。

use std::path::Path;

use midly::{MidiMessage, Smf, Timing, TrackEventKind};

use crate::error::CoreError;

/// 実時間送信用のタイム付きメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedMessage {
    /// 曲頭からの絶対時刻(マイクロ秒)
    pub at_us: u64,
    /// 送信する生 MIDI バイト列(ステータス + データ)
    pub data: Vec<u8>,
}

/// 再生スケジュール。
#[derive(Debug)]
pub struct Playback {
    pub messages: Vec<TimedMessage>,
    /// 全体長(マイクロ秒)
    pub duration_us: u64,
    /// 出現したチャンネル(0 始まり)
    pub channels: Vec<u8>,
}

/// SMF ファイルを再生スケジュールへ変換する。
pub fn plan_playback(path: &Path) -> Result<Playback, CoreError> {
    let bytes = std::fs::read(path).map_err(|e| CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let smf = Smf::parse(&bytes).map_err(|e| CoreError::MidiParse {
        path: path.to_path_buf(),
        source: e,
    })?;

    let ppq = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as u64,
        Timing::Timecode(..) => {
            return Err(CoreError::MidiParse {
                path: path.to_path_buf(),
                source: midly::Error::new(&midly::ErrorKind::Invalid("SMPTE timing unsupported")),
            });
        }
    };

    // 全トラックのイベントを (abs_tick, order, kind) で集約し、tick 昇順に処理
    struct Ev<'a> {
        tick: u64,
        kind: TrackEventKind<'a>,
    }
    let mut events: Vec<Ev> = Vec::new();
    for track in &smf.tracks {
        let mut abs = 0u64;
        for te in track {
            abs += te.delta.as_int() as u64;
            events.push(Ev {
                tick: abs,
                kind: te.kind,
            });
        }
    }
    // 安定ソート(同 tick は元の出現順を保つ)
    events.sort_by_key(|e| e.tick);

    let mut messages = Vec::new();
    let mut channels: Vec<u8> = Vec::new();
    let mut us_per_quarter = 500_000u64; // 既定 120BPM
    let mut last_tick = 0u64;
    let mut cur_us = 0u64;

    for ev in &events {
        // 直前イベントからの経過時間を加算(テンポは区間に対して適用)
        let delta_ticks = ev.tick - last_tick;
        cur_us += delta_ticks * us_per_quarter / ppq;
        last_tick = ev.tick;

        match ev.kind {
            TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) => {
                us_per_quarter = t.as_int() as u64;
            }
            TrackEventKind::Midi { channel, message } => {
                let ch = channel.as_int();
                if !channels.contains(&ch) {
                    channels.push(ch);
                }
                if let Some(data) = encode_message(ch, message) {
                    messages.push(TimedMessage {
                        at_us: cur_us,
                        data,
                    });
                }
            }
            _ => {}
        }
    }
    channels.sort_unstable();

    Ok(Playback {
        messages,
        duration_us: cur_us,
        channels,
    })
}

/// MIDI メッセージを生バイト列へエンコードする(チャンネルボイスのみ)。
fn encode_message(channel: u8, message: MidiMessage) -> Option<Vec<u8>> {
    let ch = channel & 0x0F;
    match message {
        MidiMessage::NoteOff { key, vel } => Some(vec![0x80 | ch, key.as_int(), vel.as_int()]),
        MidiMessage::NoteOn { key, vel } => Some(vec![0x90 | ch, key.as_int(), vel.as_int()]),
        MidiMessage::Aftertouch { key, vel } => Some(vec![0xA0 | ch, key.as_int(), vel.as_int()]),
        MidiMessage::Controller { controller, value } => {
            Some(vec![0xB0 | ch, controller.as_int(), value.as_int()])
        }
        MidiMessage::ProgramChange { program } => Some(vec![0xC0 | ch, program.as_int()]),
        MidiMessage::ChannelAftertouch { vel } => Some(vec![0xD0 | ch, vel.as_int()]),
        MidiMessage::PitchBend { bend } => {
            let v = bend.0.as_int();
            Some(vec![0xE0 | ch, (v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8])
        }
    }
}

/// パニック(全チャンネルの All Notes Off + Sustain Off)メッセージ列。
pub fn panic_messages() -> Vec<Vec<u8>> {
    let mut msgs = Vec::new();
    for ch in 0..16u8 {
        msgs.push(vec![0xB0 | ch, 123, 0]); // All Notes Off
        msgs.push(vec![0xB0 | ch, 120, 0]); // All Sound Off
        msgs.push(vec![0xB0 | ch, 64, 0]); // Sustain off
    }
    msgs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::compile;
    use crate::model::{DeviceProfile, PartPlan};
    use crate::validate::parse_validated;
    use serde_json::json;

    #[test]
    fn resolves_event_times_from_tempo() {
        let profile: DeviceProfile = parse_validated(&json!({
            "schema_version": "1.0", "id": "g", "name": "G", "device_type": "instrument",
            "octave_convention": "C3=60", "note_range": { "low": 0, "high": 127 }
        }))
        .unwrap();
        // 120BPM, 4/4: 1拍=0.5s。beat1 と beat2 のノート
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0", "part_id": "p", "device": "g",
            "bpm": 120.0, "time_signature": "4/4",
            "sections": [{ "label": "v", "start_bar": 1, "phrases": [{ "notes": [
                { "pitch": 60, "start": "1.1.000", "duration": "0.0.240", "velocity": 100 },
                { "pitch": 62, "start": "1.2.000", "duration": "0.0.240", "velocity": 100 }
            ]}]}]
        }))
        .unwrap();
        let out = compile(&plan, &profile).unwrap();
        let dir = std::env::temp_dir();
        let path = dir.join("sora_playback_test.mid");
        std::fs::write(&path, &out.bytes).unwrap();
        let pb = plan_playback(&path).unwrap();
        std::fs::remove_file(&path).ok();

        // 最初の note-on は t=0
        assert_eq!(pb.messages[0].at_us, 0);
        assert_eq!(pb.messages[0].data[0] & 0xF0, 0x90);
        // 2拍目の note-on は 0.5s = 500_000us 付近
        let second_on = pb
            .messages
            .iter()
            .find(|m| m.data[0] & 0xF0 == 0x90 && m.data[1] == 62)
            .unwrap();
        assert_eq!(second_on.at_us, 500_000);
        assert_eq!(pb.channels, vec![0]);
    }

    #[test]
    fn panic_covers_all_channels() {
        let msgs = panic_messages();
        assert_eq!(msgs.len(), 48); // 16ch × 3
        assert!(msgs.iter().any(|m| m[0] == 0xB0 && m[1] == 123));
    }
}
