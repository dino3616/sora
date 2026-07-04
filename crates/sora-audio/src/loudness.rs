//! ラウドネス測定(ebur128 / ITU-R BS.1770-4、技術要件書 §10)。

use ebur128::{EbuR128, Mode};
use serde::Serialize;

use crate::decode::DecodedAudio;
use crate::error::AudioError;

/// ラウドネス測定値。
#[derive(Debug, Serialize)]
pub struct Loudness {
    /// Integrated Loudness(LUFS)
    pub integrated_lufs: f64,
    /// Loudness Range(LU)
    pub loudness_range_lu: f64,
    /// True Peak(dBTP、全チャンネル最大)
    pub true_peak_dbtp: f64,
}

/// デコード済みオーディオのラウドネスを測定する。
pub fn measure(audio: &DecodedAudio) -> Result<Loudness, AudioError> {
    let mut meter = EbuR128::new(
        audio.channels as u32,
        audio.sample_rate,
        Mode::I | Mode::LRA | Mode::TRUE_PEAK,
    )
    .map_err(|e| AudioError::Analysis {
        message: format!("ebur128 init: {e:?}"),
    })?;

    // ebur128 はインターリーブ f32 を受け取る
    meter
        .add_frames_f32(&audio.interleaved)
        .map_err(|e| AudioError::Analysis {
            message: format!("ebur128 add_frames: {e:?}"),
        })?;

    let integrated = meter.loudness_global().unwrap_or(f64::NEG_INFINITY);
    let lra = meter.loudness_range().unwrap_or(0.0);
    let mut true_peak = 0.0f64;
    for ch in 0..audio.channels {
        if let Ok(tp) = meter.true_peak(ch as u32) {
            true_peak = true_peak.max(tp);
        }
    }
    // 線形 true peak → dBTP
    let true_peak_dbtp = if true_peak > 0.0 {
        20.0 * true_peak.log10()
    } else {
        f64::NEG_INFINITY
    };

    Ok(Loudness {
        integrated_lufs: round2(integrated),
        loudness_range_lu: round2(lra),
        true_peak_dbtp: round2(true_peak_dbtp),
    })
}

fn round2(v: f64) -> f64 {
    if v.is_finite() {
        (v * 100.0).round() / 100.0
    } else {
        v
    }
}
