//! Golden file テスト — コンパイラのバイト出力を固定し、リグレッションを検出する。
//!
//! 決定論性(技術要件書 §13)の回帰テスト。SMF バイト列を `tests/golden/*.mid` に
//! 保存し、再コンパイル結果と比較する。意図的に出力を変えた場合は環境変数
//! `SORA_BLESS=1` で golden を更新する。

// テストコードでは unwrap/expect を許可する
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use sora_core::midi::compile;
use sora_core::model::{DeviceProfile, PartPlan};
use sora_core::validate::parse_validated;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// バイト列を golden と比較する。`SORA_BLESS=1` なら更新する。
fn assert_golden(name: &str, bytes: &[u8]) {
    let path = golden_dir().join(name);
    if std::env::var("SORA_BLESS").is_ok() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        return;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|_| {
        panic!("golden `{name}` が存在しません。初回は SORA_BLESS=1 cargo test で生成してください")
    });
    assert_eq!(
        bytes,
        expected.as_slice(),
        "golden `{name}` と一致しません。意図的な変更なら SORA_BLESS=1 で更新してください"
    );
}

fn guitar_profile() -> DeviceProfile {
    parse_validated(&serde_json::json!({
        "schema_version": "1.0",
        "id": "heavier7strings",
        "name": "Heavier7Strings",
        "device_type": "instrument",
        "roles": ["rhythm_guitar"],
        "octave_convention": "C3=60",
        "note_range": { "low": "B0", "high": "E5" },
        "keyswitches": [
            { "articulation": "palm_mute", "note": "C#0", "mode": "momentary", "confidence": "verified" },
            { "articulation": "pinch_harmonic", "note": "D0", "mode": "latch", "confidence": "verified" }
        ]
    }))
    .unwrap()
}

#[test]
fn golden_palm_muted_riff() {
    let plan: PartPlan = parse_validated(&serde_json::json!({
        "schema_version": "1.0",
        "part_id": "golden-riff",
        "device": "heavier7strings",
        "bpm": 150.0,
        "time_signature": "4/4",
        "humanize": { "timing_ms": 6, "velocity": 8, "seed": 1 },
        "sections": [{
            "label": "verse",
            "start_bar": 1,
            "phrases": [{
                "notes": [
                    { "pitch": "E1", "start": "1.1.000", "duration": "0.0.120", "velocity": 118, "articulation": "palm_mute" },
                    { "pitch": "E1", "start": "1.1.120", "duration": "0.0.120", "velocity": 104, "articulation": "palm_mute" },
                    { "pitch": "G1", "start": "1.1.240", "duration": "0.0.120", "velocity": 112, "articulation": "palm_mute" },
                    { "pitch": "A1", "start": "1.2.000", "duration": "0.0.240", "velocity": 114, "articulation": "palm_mute" },
                    { "pitch": "E2", "start": "1.3.000", "duration": "0.1.000", "velocity": 120, "articulation": "pinch_harmonic" }
                ]
            }]
        }]
    }))
    .unwrap();

    let out = compile(&plan, &guitar_profile()).unwrap();
    assert_golden("palm_muted_riff.mid", &out.bytes);

    // 同一入力での再コンパイルがバイト同一であること(決定論)
    let again = compile(&plan, &guitar_profile()).unwrap();
    assert_eq!(out.bytes, again.bytes);
}

#[test]
fn golden_drum_groove() {
    let profile: DeviceProfile = parse_validated(&serde_json::json!({
        "schema_version": "1.0",
        "id": "modo-drum",
        "name": "MODO DRUM",
        "device_type": "instrument",
        "octave_convention": "C3=60",
        "drum_map": [
            { "kit_piece": "kick", "note": 36, "confidence": "verified" },
            { "kit_piece": "snare", "note": 38, "confidence": "verified" },
            { "kit_piece": "hihat_closed", "note": 42, "confidence": "verified" }
        ]
    }))
    .unwrap();
    let plan: PartPlan = parse_validated(&serde_json::json!({
        "schema_version": "1.0",
        "part_id": "golden-drums",
        "device": "modo-drum",
        "bpm": 150.0,
        "time_signature": "4/4",
        "humanize": { "timing_ms": 5, "velocity": 6, "seed": 3 },
        "sections": [{
            "label": "verse",
            "start_bar": 1,
            "phrases": [{
                "notes": [
                    { "kit_piece": "kick", "start": "1.1.000", "duration": "0.0.120", "velocity": 120 },
                    { "kit_piece": "hihat_closed", "start": "1.1.000", "duration": "0.0.120", "velocity": 90 },
                    { "kit_piece": "hihat_closed", "start": "1.1.240", "duration": "0.0.120", "velocity": 84 },
                    { "kit_piece": "snare", "start": "1.2.000", "duration": "0.0.120", "velocity": 112 },
                    { "kit_piece": "hihat_closed", "start": "1.2.000", "duration": "0.0.120", "velocity": 88 }
                ]
            }]
        }]
    }))
    .unwrap();

    let out = compile(&plan, &profile).unwrap();
    assert_golden("drum_groove.mid", &out.bytes);
}
