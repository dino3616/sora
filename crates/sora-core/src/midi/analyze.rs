//! 音楽的解析(`sora midi analyze`、技術要件書 §5)。
//!
//! 数値化はここ(Tool 層)、解釈と優先度付けは Agent 層 — の分担に従い、
//! 本モジュールは推定値と確信度の材料のみを返す。

use std::path::Path;

use serde::Serialize;

use crate::error::CoreError;

use super::inspect::{MidiInspection, NoteDump, inspect_file};

/// analyze 結果(`--format json` でそのまま出力)。
#[derive(Debug, Serialize)]
pub struct MidiAnalysis {
    /// テンポ(SMF メタイベント由来なら "stated"、なければ null)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    pub bpm_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_signature: Option<String>,
    /// 調性中心の候補(相関降順、上位 3 件)
    pub key_candidates: Vec<KeyCandidate>,
    /// リズム統計
    pub rhythm: RhythmProfile,
    /// フレーズ境界候補(1 拍以上の無音ギャップ)
    pub phrase_gaps: Vec<PhraseGap>,
}

#[derive(Debug, Serialize)]
pub struct KeyCandidate {
    /// 主音(例: "E")
    pub tonic: String,
    /// "major" | "minor"
    pub mode: &'static str,
    /// Krumhansl-Schmuckler プロファイルとの相関(-1..1)
    pub correlation: f64,
}

#[derive(Debug, Serialize)]
pub struct RhythmProfile {
    /// 16 分グリッド位置(0-15)ごとのオンセット数(4/4 前提の拍位置分布)
    pub onset_grid_16th: [u32; 16],
    /// 出現頻度の高いノート間隔(IOI、tick)上位 5 件
    pub common_iois: Vec<IoiEntry>,
    /// 全体ノート密度(ノート数 / 拍)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_per_beat: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct IoiEntry {
    pub ticks: u64,
    pub count: u32,
}

#[derive(Debug, Serialize)]
pub struct PhraseGap {
    pub start_tick: u64,
    pub end_tick: u64,
}

/// Krumhansl-Kessler メジャープロファイル。
const KK_MAJOR: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
/// Krumhansl-Kessler マイナープロファイル。
const KK_MINOR: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

const PITCH_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// SMF ファイルを解析する。
pub fn analyze_file(path: &Path) -> Result<MidiAnalysis, CoreError> {
    let inspection = inspect_file(path, true)?;
    Ok(analyze_inspection(&inspection))
}

pub fn analyze_inspection(inspection: &MidiInspection) -> MidiAnalysis {
    let notes: Vec<&NoteDump> = inspection
        .tracks
        .iter()
        .filter_map(|t| t.notes.as_ref())
        .flatten()
        .collect();

    let bpm = inspection.tempos.first().map(|t| t.bpm);
    let time_signature = inspection
        .time_signatures
        .first()
        .map(|ts| format!("{}/{}", ts.numerator, ts.denominator));
    let ppq = inspection.ppq.unwrap_or(480) as u64;

    // 調性推定: 時価重み付きピッチクラス分布 × K-K プロファイル相関(24 通り)
    let mut histogram = [0.0f64; 12];
    for n in &notes {
        histogram[(n.pitch % 12) as usize] += n.duration as f64;
    }
    let mut key_candidates: Vec<KeyCandidate> = (0..12)
        .flat_map(|tonic| {
            [("major", &KK_MAJOR), ("minor", &KK_MINOR)].map(|(mode, profile)| {
                let mut rotated = [0.0f64; 12];
                for (i, r) in rotated.iter_mut().enumerate() {
                    *r = histogram[(i + tonic) % 12];
                }
                KeyCandidate {
                    tonic: PITCH_NAMES[tonic].to_string(),
                    mode,
                    correlation: (pearson(&rotated, profile) * 10_000.0).round() / 10_000.0,
                }
            })
        })
        .collect();
    key_candidates.sort_by(|a, b| {
        b.correlation
            .partial_cmp(&a.correlation)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    key_candidates.truncate(3);
    if notes.is_empty() {
        key_candidates.clear();
    }

    // リズム統計
    let sixteenth = ppq / 4;
    let mut onset_grid_16th = [0u32; 16];
    let mut starts: Vec<u64> = notes.iter().map(|n| n.tick).collect();
    starts.sort_unstable();
    starts.dedup();
    for s in &starts {
        let pos_in_bar = (s / sixteenth) % 16;
        onset_grid_16th[pos_in_bar as usize] += 1;
    }
    let mut ioi_counts: std::collections::BTreeMap<u64, u32> = Default::default();
    for pair in starts.windows(2) {
        *ioi_counts.entry(pair[1] - pair[0]).or_default() += 1;
    }
    let mut common_iois: Vec<IoiEntry> = ioi_counts
        .into_iter()
        .map(|(ticks, count)| IoiEntry { ticks, count })
        .collect();
    common_iois.sort_by(|a, b| (b.count, a.ticks).cmp(&(a.count, b.ticks)));
    common_iois.truncate(5);

    // フレーズ境界候補: 1 拍以上の無音ギャップ
    let mut intervals: Vec<(u64, u64)> = notes
        .iter()
        .map(|n| (n.tick, n.tick + n.duration))
        .collect();
    intervals.sort_unstable();
    let mut phrase_gaps = Vec::new();
    let mut coverage_end = 0u64;
    for (start, end) in intervals {
        if start > coverage_end && start - coverage_end >= ppq && coverage_end > 0 {
            phrase_gaps.push(PhraseGap {
                start_tick: coverage_end,
                end_tick: start,
            });
        }
        coverage_end = coverage_end.max(end);
    }

    let notes_per_beat = inspection
        .tracks
        .iter()
        .filter_map(|t| t.stats.as_ref().and_then(|s| s.notes_per_beat))
        .fold(None, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        });

    MidiAnalysis {
        bpm,
        bpm_source: if bpm.is_some() { "stated" } else { "unknown" },
        time_signature,
        key_candidates,
        rhythm: RhythmProfile {
            onset_grid_16th,
            common_iois,
            notes_per_beat,
        },
        phrase_gaps,
    }
}

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let mean_a = a.iter().sum::<f64>() / 12.0;
    let mean_b = b.iter().sum::<f64>() / 12.0;
    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for i in 0..12 {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }
    if var_a == 0.0 || var_b == 0.0 {
        return 0.0;
    }
    cov / (var_a * var_b).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::inspect::inspect_bytes;
    use crate::model::{DeviceProfile, PartPlan};
    use crate::validate::parse_validated;
    use serde_json::json;

    /// E マイナー系のフレーズをコンパイル → 解析して調性推定を確認する。
    #[test]
    fn detects_e_minor_from_compiled_phrase() {
        let profile: DeviceProfile = parse_validated(&json!({
            "schema_version": "1.0",
            "id": "test-guitar",
            "name": "Test Guitar",
            "device_type": "instrument",
            "octave_convention": "C3=60",
            "note_range": { "low": 0, "high": 127 }
        }))
        .unwrap();
        // E ナチュラルマイナースケールの音を時価多めに置く
        let notes: Vec<serde_json::Value> = [
            "E1", "F#1", "G1", "A1", "B1", "C2", "D2", "E2", "B1", "G1", "E1", "E1",
        ]
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "pitch": p,
                "start": format!("{}.{}.000", i / 4 + 1, i % 4 + 1),
                "duration": "0.0.480",
                "velocity": 100
            })
        })
        .collect();
        let plan: PartPlan = parse_validated(&json!({
            "schema_version": "1.0",
            "part_id": "em-test",
            "device": "test-guitar",
            "bpm": 120.0,
            "time_signature": "4/4",
            "sections": [{ "label": "verse", "start_bar": 1, "phrases": [{ "notes": notes }] }]
        }))
        .unwrap();
        let out = crate::midi::compile(&plan, &profile).unwrap();
        let inspection = inspect_bytes(&out.bytes, true).unwrap();
        let analysis = analyze_inspection(&inspection);

        assert_eq!(analysis.bpm, Some(120.0));
        assert_eq!(analysis.time_signature.as_deref(), Some("4/4"));
        let top = &analysis.key_candidates[0];
        assert_eq!(
            (top.tonic.as_str(), top.mode),
            ("E", "minor"),
            "{:?}",
            analysis.key_candidates
        );
    }
}
