//! スペクトル・ダイナミクス解析(realfft、技術要件書 §10)。

use realfft::RealFftPlanner;
use serde::Serialize;

use crate::decode::DecodedAudio;

/// 7 バンドの境界周波数(Hz)。sub / low / low-mid / mid / high-mid / high / air。
const BAND_EDGES: [f32; 8] = [20.0, 60.0, 120.0, 500.0, 2000.0, 4000.0, 8000.0, 20000.0];
const BAND_NAMES: [&str; 7] = ["sub", "low", "low_mid", "mid", "high_mid", "high", "air"];
const FFT_SIZE: usize = 4096;

/// スペクトル・ダイナミクス解析結果。
#[derive(Debug, Serialize)]
pub struct Spectrum {
    /// 帯域ごとのエネルギー比(合計 1.0)
    pub band_balance: Vec<BandEnergy>,
    /// クレストファクタ(ピーク/RMS、dB)。トランジェントの保持度合いの指標
    pub crest_factor_db: f64,
    /// ステレオ相関(-1..1)。低域モノ互換の目安。モノラルなら null
    pub stereo_correlation: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct BandEnergy {
    pub band: &'static str,
    /// 下限周波数(Hz)
    pub low_hz: f32,
    /// 上限周波数(Hz)
    pub high_hz: f32,
    /// エネルギー比(0..1)
    pub ratio: f64,
}

/// スペクトル解析を行う。
pub fn analyze(audio: &DecodedAudio) -> Spectrum {
    let mono = audio.mono();
    let band_balance = band_balance(&mono, audio.sample_rate);
    let crest_factor_db = crest_factor(&mono);
    let stereo_correlation = if audio.channels >= 2 {
        Some(correlation(&audio.channel(0), &audio.channel(1)))
    } else {
        None
    };
    Spectrum {
        band_balance,
        crest_factor_db,
        stereo_correlation,
    }
}

/// Welch 法的に FFT フレームを平均してバンドエネルギー比を求める。
fn band_balance(mono: &[f32], sample_rate: u32) -> Vec<BandEnergy> {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();

    // Hann 窓
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|n| {
            let x = std::f32::consts::PI * n as f32 / (FFT_SIZE as f32 - 1.0);
            x.sin().powi(2)
        })
        .collect();

    let mut band_energy = [0.0f64; 7];
    let bin_hz = sample_rate as f32 / FFT_SIZE as f32;
    let hop = FFT_SIZE / 2;
    let mut frames = 0u64;

    let mut start = 0;
    while start + FFT_SIZE <= mono.len() {
        for i in 0..FFT_SIZE {
            input[i] = mono[start + i] * window[i];
        }
        if fft.process(&mut input, &mut output).is_ok() {
            for (bin, c) in output.iter().enumerate() {
                let freq = bin as f32 * bin_hz;
                let power = (c.re * c.re + c.im * c.im) as f64;
                if let Some(b) = band_index(freq) {
                    band_energy[b] += power;
                }
            }
            frames += 1;
        }
        start += hop;
    }
    let _ = frames;

    let total: f64 = band_energy.iter().sum();
    (0..7)
        .map(|b| BandEnergy {
            band: BAND_NAMES[b],
            low_hz: BAND_EDGES[b],
            high_hz: BAND_EDGES[b + 1],
            ratio: if total > 0.0 {
                (band_energy[b] / total * 10_000.0).round() / 10_000.0
            } else {
                0.0
            },
        })
        .collect()
}

fn band_index(freq: f32) -> Option<usize> {
    if !(BAND_EDGES[0]..BAND_EDGES[7]).contains(&freq) {
        return None;
    }
    (0..7).find(|&b| freq >= BAND_EDGES[b] && freq < BAND_EDGES[b + 1])
}

/// クレストファクタ(20*log10(peak/rms))。
fn crest_factor(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let peak = samples.iter().fold(0.0f32, |m, &s| m.max(s.abs())) as f64;
    let rms = (samples
        .iter()
        .map(|&s| (s as f64) * (s as f64))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    if rms > 0.0 && peak > 0.0 {
        ((20.0 * (peak / rms).log10()) * 100.0).round() / 100.0
    } else {
        0.0
    }
}

/// ピアソン相関(2 チャンネル間)。
fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut sum_ab = 0.0f64;
    let mut sum_a2 = 0.0f64;
    let mut sum_b2 = 0.0f64;
    for i in 0..n {
        let x = a[i] as f64;
        let y = b[i] as f64;
        sum_ab += x * y;
        sum_a2 += x * x;
        sum_b2 += y * y;
    }
    if sum_a2 > 0.0 && sum_b2 > 0.0 {
        ((sum_ab / (sum_a2 * sum_b2).sqrt()) * 1000.0).round() / 1000.0
    } else {
        0.0
    }
}
