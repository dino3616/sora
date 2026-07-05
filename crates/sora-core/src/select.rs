//! Note Selector — 自然言語の範囲指定を構造化して解決する(技術要件書 §11.3)。
//!
//! selection ケイパビリティを持たない DAW アダプタ(Studio One 5 を含む)では、
//! 「ベーストラックの第 5〜8 小節の C2 以下のノート」「Verse セクションのノート」
//! のような指定を Agent がこのセレクタへ翻訳し、Tool 層が決定論的に解決する。
//! トラックの特定はファイル(指示語解決で確定した素材)が担う。
//!
//! 条件は AND 結合。合致 0 件は黙って無視せずエラーで返す
//! (Agent が成功と誤認しないため)。

use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::{CoreError, ValidationIssue};
use crate::model::{PartPlan, SectionInfo};
use crate::types::{MidiNote, NoteSpec, OctaveConvention};

/// 対象ノートの構造化セレクタ。指定した条件すべてに合致するノートが対象(AND)。
// serde(flatten) で編集リクエストに埋め込まれるため deny_unknown_fields は
// 付けない(flatten と併用すると外側のフィールドが未知扱いになる)
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
pub struct NoteSelector {
    /// 対象小節範囲 [start, end](1 始まり・両端含む)
    #[serde(default)]
    pub bars: Option<[u32; 2]>,
    /// セクションラベル(project-context.json の sections、または Plan 自身のセクション)
    #[serde(default)]
    pub section: Option<String>,
    /// 対象ノートの通し番号(時間順 0 始まり)
    #[serde(default)]
    pub note_indices: Option<Vec<usize>>,
    /// この音以上のノートに限定(ノート名または MIDI 番号。Profile の octave_convention 基準)
    #[serde(default)]
    pub pitch_min: Option<NoteSpec>,
    /// この音以下のノートに限定(例: "C2" で C2 以下)
    #[serde(default)]
    pub pitch_max: Option<NoteSpec>,
}

impl NoteSelector {
    /// 何も条件が指定されていないか。
    pub fn is_empty(&self) -> bool {
        self.bars.is_none()
            && self.section.is_none()
            && self.note_indices.is_none()
            && self.pitch_min.is_none()
            && self.pitch_max.is_none()
    }
}

fn issue(pointer: &str, code: &str, message: String, hint: Option<String>) -> CoreError {
    CoreError::Validation {
        issues: vec![ValidationIssue {
            pointer: pointer.to_string(),
            code: code.to_string(),
            message,
            hint,
        }],
    }
}

/// セクションラベルを小節範囲へ解決する。
/// 優先順: (1) project-context の sections(end_bar を持つ)
/// (2) Plan 自身のセクション(次セクション開始の直前まで、最後は無限大)。
fn resolve_section(
    label: &str,
    plan: &PartPlan,
    context_sections: &[SectionInfo],
) -> Result<[u32; 2], CoreError> {
    let wanted = label.to_lowercase();
    if let Some(s) = context_sections
        .iter()
        .find(|s| s.label.to_lowercase() == wanted)
    {
        return Ok([s.start_bar, s.end_bar]);
    }
    if let Some(i) = plan
        .sections
        .iter()
        .position(|s| s.label.to_lowercase() == wanted)
    {
        let start = plan.sections[i].start_bar;
        let end = plan
            .sections
            .get(i + 1)
            .map(|next| next.start_bar.saturating_sub(1))
            .unwrap_or(u32::MAX);
        return Ok([start, end]);
    }
    let mut available: Vec<String> = context_sections.iter().map(|s| s.label.clone()).collect();
    available.extend(plan.sections.iter().map(|s| s.label.clone()));
    available.sort();
    available.dedup();
    Err(issue(
        "/selector/section",
        "UNKNOWN_SECTION",
        format!("section `{label}` not found in project-context or plan"),
        Some(if available.is_empty() {
            "project-context.json の sections が空です。bars で小節範囲を直接指定してください"
                .to_string()
        } else {
            format!("利用可能なセクション: {}", available.join(", "))
        }),
    ))
}

/// セレクタに合致するノートの通し番号(時間順 0 始まり)を返す。
/// 条件が空・セクション未解決・合致 0 件はエラー。
pub fn select_notes(
    plan: &PartPlan,
    selector: &NoteSelector,
    context_sections: &[SectionInfo],
    convention: OctaveConvention,
) -> Result<Vec<usize>, CoreError> {
    if selector.is_empty() {
        return Err(issue(
            "/selector",
            "SELECTOR_EMPTY",
            "no selection criteria given".to_string(),
            Some(
                "bars / section / note_indices / pitch_min / pitch_max のいずれかを指定してください"
                    .to_string(),
            ),
        ));
    }

    // 小節範囲: bars と section の両方があれば積(AND)を取る
    let section_range = selector
        .section
        .as_deref()
        .map(|label| resolve_section(label, plan, context_sections))
        .transpose()?;
    let bar_range = match (selector.bars, section_range) {
        (Some([a1, a2]), Some([b1, b2])) => Some([a1.max(b1), a2.min(b2)]),
        (Some(r), None) | (None, Some(r)) => Some(r),
        (None, None) => None,
    };

    let pitch_min = selector
        .pitch_min
        .as_ref()
        .map(|s| MidiNote::resolve(s, convention))
        .transpose()?;
    let pitch_max = selector
        .pitch_max
        .as_ref()
        .map(|s| MidiNote::resolve(s, convention))
        .transpose()?;

    let mut matched = Vec::new();
    let mut index = 0usize;
    for section in &plan.sections {
        for phrase in &section.phrases {
            for note in &phrase.notes {
                let mut ok = true;
                if let Some(indices) = &selector.note_indices {
                    ok &= indices.contains(&index);
                }
                if let Some([start, end]) = bar_range {
                    ok &= note
                        .start
                        .split('.')
                        .next()
                        .and_then(|b| b.parse::<u32>().ok())
                        .is_some_and(|bar| bar >= start && bar <= end);
                }
                if pitch_min.is_some() || pitch_max.is_some() {
                    // kit_piece ノート(ドラム)は音高フィルタの対象外
                    match note
                        .pitch
                        .as_ref()
                        .map(|p| MidiNote::resolve(p, convention))
                        .transpose()?
                    {
                        Some(pitch) => {
                            if let Some(min) = pitch_min {
                                ok &= pitch >= min;
                            }
                            if let Some(max) = pitch_max {
                                ok &= pitch <= max;
                            }
                        }
                        None => ok = false,
                    }
                }
                if ok {
                    matched.push(index);
                }
                index += 1;
            }
        }
    }

    if matched.is_empty() {
        return Err(issue(
            "/selector",
            "SELECTOR_NO_MATCH",
            format!(
                "no notes matched (bars={:?}, section={:?}, pitch_min={:?}, pitch_max={:?}, note_indices={:?}; total notes {index})",
                selector.bars, selector.section, selector.pitch_min, selector.pitch_max, selector.note_indices
            ),
            Some("範囲・音高境界を確認してください。音名は Profile の octave_convention 基準で解決されます".to_string()),
        ));
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Phrase, PlanNote, PlanSection};

    fn note(pitch: &str, start: &str) -> PlanNote {
        PlanNote {
            pitch: Some(NoteSpec::Name(pitch.to_string())),
            kit_piece: None,
            start: start.to_string(),
            duration: "0.1.000".to_string(),
            velocity: 100,
            articulation: None,
        }
    }

    fn plan() -> PartPlan {
        PartPlan {
            schema_version: "1.0".to_string(),
            part_id: "test".to_string(),
            device: "dev".to_string(),
            bpm: 120.0,
            time_signature: "4/4".to_string(),
            ppq: 480,
            sections: vec![
                PlanSection {
                    label: "verse".to_string(),
                    start_bar: 1,
                    phrases: vec![Phrase {
                        label: None,
                        notes: vec![
                            note("E1", "1.1.000"),
                            note("E2", "2.1.000"),
                            note("B0", "3.1.000"),
                        ],
                    }],
                },
                PlanSection {
                    label: "chorus".to_string(),
                    start_bar: 5,
                    phrases: vec![Phrase {
                        label: None,
                        notes: vec![note("E1", "5.1.000"), note("G3", "6.1.000")],
                    }],
                },
            ],
            humanize: None,
            controls: vec![],
            program_changes: vec![],
            design_notes: None,
        }
    }

    fn conv() -> OctaveConvention {
        OctaveConvention::C3Is60
    }

    #[test]
    fn selects_by_bar_range() {
        let selector = NoteSelector {
            bars: Some([2, 3]),
            ..Default::default()
        };
        let matched = select_notes(&plan(), &selector, &[], conv()).unwrap();
        assert_eq!(matched, vec![1, 2]);
    }

    #[test]
    fn selects_by_plan_section_label() {
        let selector = NoteSelector {
            section: Some("Chorus".to_string()),
            ..Default::default()
        };
        let matched = select_notes(&plan(), &selector, &[], conv()).unwrap();
        assert_eq!(matched, vec![3, 4]);
    }

    #[test]
    fn context_sections_take_precedence() {
        // project-context では verse = 1..2 小節(Plan のセクション割りより優先)
        let context = vec![SectionInfo {
            label: "verse".to_string(),
            start_bar: 1,
            end_bar: 2,
        }];
        let selector = NoteSelector {
            section: Some("verse".to_string()),
            ..Default::default()
        };
        let matched = select_notes(&plan(), &selector, &context, conv()).unwrap();
        assert_eq!(matched, vec![0, 1]); // 3 小節目の B0 は範囲外
    }

    #[test]
    fn selects_by_pitch_bound() {
        // 「E1 以下のノート」(C3=60 基準: E1=40, B0=35)
        let selector = NoteSelector {
            pitch_max: Some(NoteSpec::Name("E1".to_string())),
            ..Default::default()
        };
        let matched = select_notes(&plan(), &selector, &[], conv()).unwrap();
        assert_eq!(matched, vec![0, 2, 3]);
    }

    #[test]
    fn combines_criteria_with_and() {
        // 「Verse セクションの E1 以下」
        let selector = NoteSelector {
            section: Some("verse".to_string()),
            pitch_max: Some(NoteSpec::Name("E1".to_string())),
            ..Default::default()
        };
        let matched = select_notes(&plan(), &selector, &[], conv()).unwrap();
        assert_eq!(matched, vec![0, 2]);
    }

    #[test]
    fn unknown_section_lists_available() {
        let selector = NoteSelector {
            section: Some("bridge".to_string()),
            ..Default::default()
        };
        let err = select_notes(&plan(), &selector, &[], conv()).unwrap_err();
        let CoreError::Validation { issues } = &err else {
            panic!("expected validation error");
        };
        assert_eq!(issues[0].code, "UNKNOWN_SECTION");
        assert!(issues[0].hint.as_ref().unwrap().contains("chorus"));
    }

    #[test]
    fn empty_selector_and_no_match_are_errors() {
        let err = select_notes(&plan(), &NoteSelector::default(), &[], conv()).unwrap_err();
        let CoreError::Validation { issues } = &err else {
            panic!("expected validation error");
        };
        assert_eq!(issues[0].code, "SELECTOR_EMPTY");

        let selector = NoteSelector {
            bars: Some([90, 99]),
            ..Default::default()
        };
        let err = select_notes(&plan(), &selector, &[], conv()).unwrap_err();
        let CoreError::Validation { issues } = &err else {
            panic!("expected validation error");
        };
        assert_eq!(issues[0].code, "SELECTOR_NO_MATCH");
    }
}
