//! 合成 WAV を使った解析の統合テスト。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::PathBuf;

use sora_audio::analyze_file;

/// 16-bit PCM WAV を書き出す(テスト用の最小ライタ)。
fn write_wav(path: &std::path::Path, samples: &[i16], sample_rate: u32, channels: u16) {
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;
    let data_len = (samples.len() * 2) as u32;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&channels.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    for s in samples {
        f.write_all(&s.to_le_bytes()).unwrap();
    }
    f.flush().unwrap();
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

#[test]
fn analyzes_1khz_sine_into_mid_band() {
    let sr = 44_100u32;
    let freq = 1000.0f64; // mid band (500-2000Hz)
    let secs = 2.0;
    let n = (sr as f64 * secs) as usize;
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            let t = i as f64 / sr as f64;
            ((t * freq * 2.0 * std::f64::consts::PI).sin() * 16000.0) as i16
        })
        .collect();
    let path = tmp("sora_audio_sine.wav");
    write_wav(&path, &samples, sr, 1);

    let analysis = analyze_file(&path).unwrap();
    std::fs::remove_file(&path).ok();

    assert_eq!(analysis.sample_rate, 44_100);
    assert_eq!(analysis.channels, 1);
    assert!((analysis.duration_seconds - 2.0).abs() < 0.05);
    assert!(analysis.loudness.integrated_lufs.is_finite());

    // エネルギーが mid バンドに集中しているはず
    let mid = analysis
        .spectrum
        .band_balance
        .iter()
        .find(|b| b.band == "mid")
        .unwrap();
    assert!(
        mid.ratio > 0.7,
        "1kHz sine should concentrate in mid band, got {}",
        mid.ratio
    );
    // モノラルなのでステレオ相関は null
    assert!(analysis.spectrum.stereo_correlation.is_none());
}

#[test]
fn stereo_correlation_is_one_for_identical_channels() {
    let sr = 44_100u32;
    let n = sr as usize; // 1 秒
    let mut interleaved: Vec<i16> = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let v = ((t * 440.0 * 2.0 * std::f64::consts::PI).sin() * 12000.0) as i16;
        interleaved.push(v); // L
        interleaved.push(v); // R(同一)
    }
    let path = tmp("sora_audio_stereo.wav");
    write_wav(&path, &interleaved, sr, 2);

    let analysis = analyze_file(&path).unwrap();
    std::fs::remove_file(&path).ok();

    assert_eq!(analysis.channels, 2);
    let corr = analysis.spectrum.stereo_correlation.unwrap();
    assert!(
        (corr - 1.0).abs() < 0.001,
        "identical channels -> correlation ~1, got {corr}"
    );
}
