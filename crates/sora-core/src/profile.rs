//! Device Profile の L3 検証と解決(技術要件書 §4.2, §4.6)。
//!
//! 生の [`DeviceProfile`](crate::model::DeviceProfile) は JSON 表現に忠実で、
//! ノートは文字列/数値が混在する。本モジュールがそれを検証済みの
//! [`ResolvedProfile`] へ解決し、以降のコンパイル工程は不変条件を前提にできる。

use std::collections::BTreeMap;

use crate::error::{CoreError, ValidationIssue};
use crate::model::{Confidence, DeviceProfile, DeviceType, KeyswitchMode, Polyphony};
use crate::types::{MidiNote, OctaveConvention};

/// 既定のキースイッチ先行 tick 数(技術要件書 §7)。
pub const DEFAULT_KEYSWITCH_LEAD_TICKS: u32 = 20;

/// 検証済み・解決済みの Device Profile。
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub id: String,
    pub device_type: DeviceType,
    pub convention: OctaveConvention,
    /// 演奏可能音域(low, high)
    pub note_range: Option<(MidiNote, MidiNote)>,
    /// articulation ID → キースイッチ
    pub keyswitches: BTreeMap<String, ResolvedKeyswitch>,
    /// kit_piece ID → ノート
    pub drum_map: BTreeMap<String, ResolvedDrumPiece>,
    /// CC 番号 → safe_range(未定義なら None)。CC レーン検証に使う
    pub cc_map: BTreeMap<u8, Option<[u8; 2]>>,
    pub polyphony: Polyphony,
    /// 出力チャンネル(0 始まり)
    pub midi_channel: u8,
    pub keyswitch_lead_ticks: u32,
}

#[derive(Debug, Clone)]
pub struct ResolvedKeyswitch {
    pub note: MidiNote,
    pub mode: KeyswitchMode,
    pub confidence: Confidence,
}

#[derive(Debug, Clone)]
pub struct ResolvedDrumPiece {
    pub note: MidiNote,
    pub confidence: Confidence,
}

impl ResolvedProfile {
    /// Profile を検証しつつ解決する。エラーは全件列挙(§4.6)。
    pub fn resolve(profile: &DeviceProfile) -> Result<Self, CoreError> {
        let mut issues: Vec<ValidationIssue> = Vec::new();

        let convention: OctaveConvention = match profile.octave_convention.parse() {
            Ok(c) => c,
            Err(e) => {
                issues.push(issue("/octave_convention", &e));
                // 以降のノート解決のために暫定値で続行(エラーは既に記録済み)
                OctaveConvention::C3Is60
            }
        };

        let note_range = match &profile.note_range {
            Some(range) => {
                let low = MidiNote::resolve(&range.low, convention);
                let high = MidiNote::resolve(&range.high, convention);
                match (low, high) {
                    (Ok(low), Ok(high)) => {
                        if low > high {
                            issues.push(ValidationIssue {
                                pointer: "/note_range".to_string(),
                                code: "INVERTED_NOTE_RANGE".to_string(),
                                message: format!(
                                    "note_range low ({}) > high ({})",
                                    low.value(),
                                    high.value()
                                ),
                                hint: Some(
                                    "low と high が逆になっていないか確認してください".to_string(),
                                ),
                            });
                            None
                        } else {
                            Some((low, high))
                        }
                    }
                    (low, high) => {
                        if let Err(e) = low {
                            issues.push(issue("/note_range/low", &e));
                        }
                        if let Err(e) = high {
                            issues.push(issue("/note_range/high", &e));
                        }
                        None
                    }
                }
            }
            None => None,
        };

        let mut keyswitches = BTreeMap::new();
        for (i, ks) in profile.keyswitches.iter().enumerate() {
            let pointer = format!("/keyswitches/{i}");
            match MidiNote::resolve(&ks.note, convention) {
                Ok(note) => {
                    // キースイッチ × 演奏音域の衝突検査(§7)
                    if let Some((low, high)) = note_range
                        && note >= low
                        && note <= high
                    {
                        issues.push(ValidationIssue {
                                pointer: format!("{pointer}/note"),
                                code: "KEYSWITCH_COLLISION".to_string(),
                                message: format!(
                                    "keyswitch `{}` (note {}) collides with playable range {}..={}",
                                    ks.articulation,
                                    note.value(),
                                    low.value(),
                                    high.value()
                                ),
                                hint: Some(
                                    "キースイッチは演奏音域の外に置く必要があります。octave_convention の取り違えの可能性も確認してください".to_string(),
                                ),
                            });
                    }
                    if keyswitches
                        .insert(
                            ks.articulation.clone(),
                            ResolvedKeyswitch {
                                note,
                                mode: ks.mode,
                                confidence: ks.confidence,
                            },
                        )
                        .is_some()
                    {
                        issues.push(ValidationIssue {
                            pointer: format!("{pointer}/articulation"),
                            code: "DUPLICATE_ARTICULATION".to_string(),
                            message: format!(
                                "articulation `{}` is defined more than once",
                                ks.articulation
                            ),
                            hint: None,
                        });
                    }
                }
                Err(e) => issues.push(issue(&format!("{pointer}/note"), &e)),
            }
        }

        let mut drum_map = BTreeMap::new();
        if let Some(entries) = &profile.drum_map {
            for (i, piece) in entries.iter().enumerate() {
                let pointer = format!("/drum_map/{i}");
                match MidiNote::resolve(&piece.note, convention) {
                    Ok(note) => {
                        if drum_map
                            .insert(
                                piece.kit_piece.clone(),
                                ResolvedDrumPiece {
                                    note,
                                    confidence: piece.confidence,
                                },
                            )
                            .is_some()
                        {
                            issues.push(ValidationIssue {
                                pointer: format!("{pointer}/kit_piece"),
                                code: "DUPLICATE_KIT_PIECE".to_string(),
                                message: format!(
                                    "kit_piece `{}` is defined more than once",
                                    piece.kit_piece
                                ),
                                hint: None,
                            });
                        }
                    }
                    Err(e) => issues.push(issue(&format!("{pointer}/note"), &e)),
                }
            }
        }

        // instrument なのに発音手段がない、という起草ミスの検出
        if profile.device_type == DeviceType::Instrument
            && profile.note_range.is_none()
            && profile.drum_map.is_none()
        {
            issues.push(ValidationIssue {
                pointer: "/note_range".to_string(),
                code: "MISSING_PLAYABLE_DEFINITION".to_string(),
                message: "instrument profile must define note_range or drum_map".to_string(),
                hint: Some(
                    "音程系インストゥルメントは note_range、ドラム音源は drum_map を定義してください".to_string(),
                ),
            });
        }

        if let Some(channel) = profile.midi_channel
            && channel > 15
        {
            issues.push(ValidationIssue {
                pointer: "/midi_channel".to_string(),
                code: "INVALID_MIDI_CHANNEL".to_string(),
                message: format!("midi_channel {channel} out of range 0..=15"),
                hint: Some("チャンネルは 0 始まり(ch10 は 9)です".to_string()),
            });
        }

        let mut cc_map: BTreeMap<u8, Option<[u8; 2]>> = BTreeMap::new();
        for (i, cc) in profile.cc_map.iter().enumerate() {
            if cc.cc > 127 {
                issues.push(ValidationIssue {
                    pointer: format!("/cc_map/{i}/cc"),
                    code: "INVALID_CC_NUMBER".to_string(),
                    message: format!("cc number {} out of range 0..=127", cc.cc),
                    hint: None,
                });
            }
            if let Some([min, max]) = cc.safe_range
                && (min > max || max > 127)
            {
                issues.push(ValidationIssue {
                    pointer: format!("/cc_map/{i}/safe_range"),
                    code: "INVALID_SAFE_RANGE".to_string(),
                    message: format!("safe_range [{min}, {max}] is invalid"),
                    hint: None,
                });
            }
            cc_map.insert(cc.cc, cc.safe_range);
        }

        if !issues.is_empty() {
            return Err(CoreError::Validation { issues });
        }

        // ドラム音源の既定はチャンネル 10(=9)(§7)
        let midi_channel = profile
            .midi_channel
            .unwrap_or(if profile.drum_map.is_some() { 9 } else { 0 });

        Ok(ResolvedProfile {
            id: profile.id.clone(),
            device_type: profile.device_type,
            convention,
            note_range,
            keyswitches,
            drum_map,
            cc_map,
            polyphony: profile.polyphony,
            midi_channel,
            keyswitch_lead_ticks: profile
                .keyswitch_lead_ticks
                .unwrap_or(DEFAULT_KEYSWITCH_LEAD_TICKS),
        })
    }
}

fn issue(pointer: &str, err: &CoreError) -> ValidationIssue {
    ValidationIssue {
        pointer: pointer.to_string(),
        code: err.code().to_string(),
        message: err.to_string(),
        hint: err.hint(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::parse_validated;
    use serde_json::json;

    fn h7s_profile() -> DeviceProfile {
        parse_validated(&json!({
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
        .unwrap()
    }

    #[test]
    fn resolves_valid_profile() {
        let resolved = ResolvedProfile::resolve(&h7s_profile()).unwrap();
        // C3=60: B-1 = 11 + (-1+2)*12 = 23, E4 = 4 + (4+2)*12 = 76
        assert_eq!(resolved.note_range.unwrap().0.value(), 23);
        assert_eq!(resolved.note_range.unwrap().1.value(), 76);
        // C-2 = 0 + (-2+2)*12 = 0
        assert_eq!(resolved.keyswitches["palm_mute"].note.value(), 0);
        assert_eq!(resolved.midi_channel, 0);
        assert_eq!(resolved.keyswitch_lead_ticks, DEFAULT_KEYSWITCH_LEAD_TICKS);
    }

    #[test]
    fn keyswitch_collision_is_detected() {
        let mut profile = h7s_profile();
        // 演奏音域内にキースイッチを置く
        profile.keyswitches[0].note = crate::types::NoteSpec::Name("C1".to_string());
        let err = ResolvedProfile::resolve(&profile).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!()
        };
        assert!(issues.iter().any(|i| i.code == "KEYSWITCH_COLLISION"));
    }

    #[test]
    fn multiple_issues_collected() {
        let mut profile = h7s_profile();
        profile.octave_convention = "C5=60".to_string();
        profile.keyswitches[1].articulation = "palm_mute".to_string(); // 重複
        let err = ResolvedProfile::resolve(&profile).unwrap_err();
        let CoreError::Validation { issues } = err else {
            panic!()
        };
        assert!(issues.len() >= 2, "{issues:?}");
    }

    #[test]
    fn drum_profile_defaults_to_channel_10() {
        let profile: DeviceProfile = parse_validated(&json!({
            "schema_version": "1.0",
            "id": "modo-drum",
            "name": "MODO DRUM",
            "device_type": "instrument",
            "octave_convention": "C3=60",
            "drum_map": [
                { "kit_piece": "kick", "note": 36, "confidence": "manual" },
                { "kit_piece": "snare", "note": 38, "confidence": "manual" }
            ]
        }))
        .unwrap();
        let resolved = ResolvedProfile::resolve(&profile).unwrap();
        assert_eq!(resolved.midi_channel, 9);
        assert_eq!(resolved.drum_map["kick"].note.value(), 36);
    }
}
