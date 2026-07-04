//! 解析の統合と A/B 比較(技術要件書 §10、UC9/UC11/UC16)。
//!
//! 数値化はここ(Tool 層)、解釈と優先度付けは Agent 層 — の分担に従う。

use std::path::Path;

use serde::Serialize;

use crate::decode::decode_file;
use crate::error::AudioError;
use crate::loudness::{Loudness, measure};
use crate::spectrum::{Spectrum, analyze as analyze_spectrum};

/// 単一ファイルの解析結果。
#[derive(Debug, Serialize)]
pub struct AudioAnalysis {
    pub sample_rate: u32,
    pub channels: usize,
    pub duration_seconds: f64,
    pub loudness: Loudness,
    pub spectrum: Spectrum,
}

/// ファイルを解析する(`sora audio analyze`)。
pub fn analyze_file(path: &Path) -> Result<AudioAnalysis, AudioError> {
    let audio = decode_file(path)?;
    let duration_seconds =
        (audio.frame_count() as f64 / audio.sample_rate as f64 * 1000.0).round() / 1000.0;
    let loudness = measure(&audio)?;
    let spectrum = analyze_spectrum(&audio);
    Ok(AudioAnalysis {
        sample_rate: audio.sample_rate,
        channels: audio.channels,
        duration_seconds,
        loudness,
        spectrum,
    })
}

/// A/B 差分(`sora audio compare`)。b - a の差を返す。
#[derive(Debug, Serialize)]
pub struct AudioComparison {
    pub a: AudioAnalysis,
    pub b: AudioAnalysis,
    pub delta: ComparisonDelta,
}

#[derive(Debug, Serialize)]
pub struct ComparisonDelta {
    pub integrated_lufs: f64,
    pub loudness_range_lu: f64,
    pub true_peak_dbtp: f64,
    pub crest_factor_db: f64,
    /// 帯域ごとのエネルギー比の差(band 名 → b - a)
    pub band_balance: Vec<BandDelta>,
}

#[derive(Debug, Serialize)]
pub struct BandDelta {
    pub band: &'static str,
    pub delta_ratio: f64,
}

/// 2 ファイルを比較する。
pub fn compare_files(a_path: &Path, b_path: &Path) -> Result<AudioComparison, AudioError> {
    let a = analyze_file(a_path)?;
    let b = analyze_file(b_path)?;

    let band_balance = a
        .spectrum
        .band_balance
        .iter()
        .zip(b.spectrum.band_balance.iter())
        .map(|(ba, bb)| BandDelta {
            band: ba.band,
            delta_ratio: ((bb.ratio - ba.ratio) * 10_000.0).round() / 10_000.0,
        })
        .collect();

    let delta = ComparisonDelta {
        integrated_lufs: round2(b.loudness.integrated_lufs - a.loudness.integrated_lufs),
        loudness_range_lu: round2(b.loudness.loudness_range_lu - a.loudness.loudness_range_lu),
        true_peak_dbtp: round2(b.loudness.true_peak_dbtp - a.loudness.true_peak_dbtp),
        crest_factor_db: round2(b.spectrum.crest_factor_db - a.spectrum.crest_factor_db),
        band_balance,
    };

    Ok(AudioComparison { a, b, delta })
}

fn round2(v: f64) -> f64 {
    if v.is_finite() {
        (v * 100.0).round() / 100.0
    } else {
        v
    }
}
