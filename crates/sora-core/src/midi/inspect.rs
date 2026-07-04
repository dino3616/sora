//! SMF 読み取り・統計(`sora midi inspect`、技術要件書 §5)。

use std::path::Path;

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use serde::Serialize;

use crate::error::CoreError;

/// inspect 結果(`--format json` でそのまま出力)。
#[derive(Debug, Serialize)]
pub struct MidiInspection {
    pub format: String,
    pub ppq: Option<u16>,
    pub track_count: usize,
    /// テンポイベント列(BPM 換算)
    pub tempos: Vec<TempoEvent>,
    pub time_signatures: Vec<TimeSignatureEvent>,
    pub tracks: Vec<TrackInspection>,
}

#[derive(Debug, Serialize)]
pub struct TempoEvent {
    pub tick: u64,
    pub bpm: f64,
}

#[derive(Debug, Serialize)]
pub struct TimeSignatureEvent {
    pub tick: u64,
    pub numerator: u8,
    pub denominator: u16,
}

#[derive(Debug, Serialize)]
pub struct TrackInspection {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub note_count: usize,
    pub cc_count: usize,
    /// 使用チャンネル(0 始まり)
    pub channels: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<NoteStats>,
    /// 全ノート(include_notes 指定時のみ)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<NoteDump>>,
}

#[derive(Debug, Serialize)]
pub struct NoteStats {
    pub pitch_min: u8,
    pub pitch_max: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    pub velocity_mean: f64,
    /// 最初のノートオン tick
    pub first_tick: u64,
    /// 最後のノートオフ tick
    pub last_tick: u64,
    /// ノート密度(1 拍 = PPQ tick あたりのノート数)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_per_beat: Option<f64>,
    /// ピッチクラス出現分布(C=0 起点、時価重み付き、合計 1.0)
    pub pitch_class_histogram: [f64; 12],
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteDump {
    pub tick: u64,
    pub duration: u64,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

/// SMF ファイルを読み取る。
pub fn inspect_file(path: &Path, include_notes: bool) -> Result<MidiInspection, CoreError> {
    let bytes = std::fs::read(path).map_err(|e| CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    inspect_bytes(&bytes, include_notes).map_err(|e| match e {
        InspectError::Parse(source) => CoreError::MidiParse {
            path: path.to_path_buf(),
            source,
        },
    })
}

#[derive(Debug)]
pub enum InspectError {
    Parse(midly::Error),
}

/// SMF バイト列を読み取る。
pub fn inspect_bytes(bytes: &[u8], include_notes: bool) -> Result<MidiInspection, InspectError> {
    let smf = Smf::parse(bytes).map_err(InspectError::Parse)?;

    let ppq = match smf.header.timing {
        Timing::Metrical(t) => Some(t.as_int()),
        Timing::Timecode(..) => None,
    };

    let mut tempos = Vec::new();
    let mut time_signatures = Vec::new();
    let mut tracks = Vec::new();

    for (index, track) in smf.tracks.iter().enumerate() {
        let mut abs = 0u64;
        let mut name = None;
        let mut cc_count = 0usize;
        let mut channels: Vec<u8> = Vec::new();
        let mut notes: Vec<NoteDump> = Vec::new();
        // (pitch, channel) → (start_tick, velocity) の未解決ノートオン
        let mut open: Vec<(u8, u8, u64, u8)> = Vec::new();

        for event in track {
            abs += event.delta.as_int() as u64;
            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                    name = Some(String::from_utf8_lossy(n).to_string());
                }
                TrackEventKind::Meta(MetaMessage::Tempo(micros)) => {
                    tempos.push(TempoEvent {
                        tick: abs,
                        bpm: (60_000_000.0 / micros.as_int() as f64 * 1000.0).round() / 1000.0,
                    });
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_log2, _, _)) => {
                    time_signatures.push(TimeSignatureEvent {
                        tick: abs,
                        numerator: num,
                        denominator: 1u16 << den_log2,
                    });
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    if !channels.contains(&ch) {
                        channels.push(ch);
                    }
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            open.push((key.as_int(), ch, abs, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(pos) = open
                                .iter()
                                .position(|(k, c, _, _)| *k == key.as_int() && *c == ch)
                            {
                                let (pitch, channel, start, velocity) = open.remove(pos);
                                notes.push(NoteDump {
                                    tick: start,
                                    duration: abs.saturating_sub(start).max(1),
                                    pitch,
                                    velocity,
                                    channel,
                                });
                            }
                        }
                        MidiMessage::Controller { .. } => cc_count += 1,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // 鳴りっぱなし(NoteOff 欠落)はトラック終端で閉じる
        for (pitch, channel, start, velocity) in open {
            notes.push(NoteDump {
                tick: start,
                duration: abs.saturating_sub(start).max(1),
                pitch,
                velocity,
                channel,
            });
        }
        notes.sort_by_key(|n| (n.tick, n.pitch));

        let stats = build_stats(&notes, ppq);
        tracks.push(TrackInspection {
            index,
            name,
            note_count: notes.len(),
            cc_count,
            channels,
            stats,
            notes: include_notes.then_some(notes),
        });
    }

    Ok(MidiInspection {
        format: format!("{:?}", smf.header.format),
        ppq,
        track_count: smf.tracks.len(),
        tempos,
        time_signatures,
        tracks,
    })
}

fn build_stats(notes: &[NoteDump], ppq: Option<u16>) -> Option<NoteStats> {
    if notes.is_empty() {
        return None;
    }
    let first_tick = notes.iter().map(|n| n.tick).min().unwrap_or(0);
    let last_tick = notes.iter().map(|n| n.tick + n.duration).max().unwrap_or(0);
    let mut histogram = [0.0f64; 12];
    for n in notes {
        histogram[(n.pitch % 12) as usize] += n.duration as f64;
    }
    let total: f64 = histogram.iter().sum();
    if total > 0.0 {
        for h in histogram.iter_mut() {
            *h = (*h / total * 10_000.0).round() / 10_000.0;
        }
    }
    let span_beats = ppq.map(|p| (last_tick - first_tick) as f64 / p as f64);
    Some(NoteStats {
        pitch_min: notes.iter().map(|n| n.pitch).min().unwrap_or(0),
        pitch_max: notes.iter().map(|n| n.pitch).max().unwrap_or(0),
        velocity_min: notes.iter().map(|n| n.velocity).min().unwrap_or(0),
        velocity_max: notes.iter().map(|n| n.velocity).max().unwrap_or(0),
        velocity_mean: {
            let sum: u64 = notes.iter().map(|n| n.velocity as u64).sum();
            ((sum as f64 / notes.len() as f64) * 100.0).round() / 100.0
        },
        first_tick,
        last_tick,
        notes_per_beat: span_beats
            .filter(|b| *b > 0.0)
            .map(|b| (notes.len() as f64 / b * 100.0).round() / 100.0),
        pitch_class_histogram: histogram,
    })
}
