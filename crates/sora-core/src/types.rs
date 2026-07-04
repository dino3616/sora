//! ドメイン newtype 群(技術要件書 §4.6 L3)。
//!
//! "parse, don't validate" — 検証済みであることを型で運ぶ。
//! 生の JSON 値(文字列ノート名・数値)からの変換時にのみ検証が走り、
//! 以降のコンパイル工程は不変条件を前提にできる。

use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;

/// オクターブ表記基準。ベンダーごとに C3=60 と C4=60 が混在するため、
/// Device Profile ごとの宣言が必須(技術要件書 §4.2)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OctaveConvention {
    /// C3 = MIDI 60(Yamaha 系・多くのソフト音源)
    C3Is60,
    /// C4 = MIDI 60(国際式・Roland 系)
    C4Is60,
}

impl OctaveConvention {
    /// ノート名のオクターブ数値に加えるオフセット。
    /// `midi = pitch_class + (octave + offset) * 12`
    fn octave_offset(self) -> i32 {
        match self {
            // C3=60: (3 + 2) * 12 = 60
            OctaveConvention::C3Is60 => 2,
            // C4=60: (4 + 1) * 12 = 60
            OctaveConvention::C4Is60 => 1,
        }
    }
}

impl FromStr for OctaveConvention {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "C3=60" => Ok(OctaveConvention::C3Is60),
            "C4=60" => Ok(OctaveConvention::C4Is60),
            other => Err(CoreError::InvalidOctaveConvention {
                value: other.to_string(),
                allowed: vec!["C3=60".into(), "C4=60".into()],
            }),
        }
    }
}

impl fmt::Display for OctaveConvention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OctaveConvention::C3Is60 => write!(f, "C3=60"),
            OctaveConvention::C4Is60 => write!(f, "C4=60"),
        }
    }
}

/// 検証済み MIDI ノート番号(0..=127)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MidiNote(u8);

impl MidiNote {
    pub fn new(value: u8) -> Result<Self, CoreError> {
        if value > 127 {
            return Err(CoreError::NoteNumberOutOfMidiRange {
                value: value as i32,
            });
        }
        Ok(MidiNote(value))
    }

    /// ノート名(例: `"E1"`, `"D#0"`, `"Bb2"`)を規約に従って解決する。
    pub fn parse_name(name: &str, convention: OctaveConvention) -> Result<Self, CoreError> {
        let err = || CoreError::InvalidNoteName {
            value: name.to_string(),
            hint: "例: \"E1\", \"D#0\", \"Bb2\"。数値(0-127)も使用可".to_string(),
        };

        let mut chars = name.chars();
        let letter = chars.next().ok_or_else(err)?.to_ascii_uppercase();
        let base: i32 = match letter {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return Err(err()),
        };

        let rest: String = chars.collect();
        let (accidental, octave_str) = if let Some(stripped) = rest.strip_prefix('#') {
            (1, stripped)
        } else if let Some(stripped) = rest.strip_prefix('b') {
            (-1, stripped)
        } else {
            (0, rest.as_str())
        };

        let octave: i32 = octave_str.parse().map_err(|_| err())?;
        let midi = base + accidental + (octave + convention.octave_offset()) * 12;
        if !(0..=127).contains(&midi) {
            return Err(CoreError::NoteNumberOutOfMidiRange { value: midi });
        }
        Ok(MidiNote(midi as u8))
    }

    /// JSON 上の表現(文字列ノート名または数値)から解決する。
    pub fn resolve(spec: &NoteSpec, convention: OctaveConvention) -> Result<Self, CoreError> {
        match spec {
            NoteSpec::Number(n) => {
                if *n > 127 {
                    return Err(CoreError::NoteNumberOutOfMidiRange { value: *n as i32 });
                }
                Ok(MidiNote(*n as u8))
            }
            NoteSpec::Name(name) => Self::parse_name(name, convention),
        }
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

/// JSON 上のノート表現。文字列ノート名と数値のどちらも受け付け、
/// `MidiNote::resolve` で規約に従い正規化する(技術要件書 §4.2)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum NoteSpec {
    /// MIDI ノート番号(0-127)
    Number(u16),
    /// ノート名(例: "E1", "D#0"。オクターブ基準は Profile の octave_convention に従う)
    Name(String),
}

/// 検証済みベロシティ(1..=127)。0 はノートオフ扱いになるため不許可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Velocity(u8);

impl Velocity {
    pub fn new(value: u8) -> Result<Self, CoreError> {
        if value == 0 || value > 127 {
            return Err(CoreError::InvalidVelocity { value });
        }
        Ok(Velocity(value))
    }

    pub fn value(self) -> u8 {
        self.0
    }

    /// ヒューマナイズ適用後の値を 1..=127 にクランプして返す。
    pub fn offset_clamped(self, delta: i16) -> Velocity {
        let v = (self.0 as i16 + delta).clamp(1, 127);
        Velocity(v as u8)
    }
}

/// `bar.beat.tick` 形式の時間位置(bar/beat は 1 始まり、tick は 0 始まり)。
///
/// Part Plan の時間表現(技術要件書 §4.4)。人間可読・レビュー可能にするため
/// 絶対 tick ではなくこの形式を正とし、コンパイラが tick へ解決する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BarBeatTick {
    pub bar: u32,
    pub beat: u32,
    pub tick: u32,
}

impl BarBeatTick {
    /// 絶対 tick へ解決する。
    /// `ticks_per_beat` は拍子分母基準の 1 拍(例: 4/4@PPQ480 → 480、7/8@PPQ480 → 240)。
    pub fn to_absolute_ticks(self, ticks_per_beat: u32, beats_per_bar: u32) -> u64 {
        let bar0 = (self.bar - 1) as u64;
        let beat0 = (self.beat - 1) as u64;
        (bar0 * beats_per_bar as u64 + beat0) * ticks_per_beat as u64 + self.tick as u64
    }

    /// 長さ表現(`0.0.240` 等)を tick 数へ解決する。
    /// 長さの場合 bar/beat も 0 始まりのオフセットとして解釈する。
    pub fn to_duration_ticks(self, ticks_per_beat: u32, beats_per_bar: u32) -> u64 {
        (self.bar as u64 * beats_per_bar as u64 + self.beat as u64) * ticks_per_beat as u64
            + self.tick as u64
    }
}

impl FromStr for BarBeatTick {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || CoreError::InvalidTimePosition {
            value: s.to_string(),
            hint:
                "形式は \"bar.beat.tick\"(位置は 1.1.000 起点、長さは 0.0.240 のようなオフセット)"
                    .to_string(),
        };
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(err());
        }
        let bar: u32 = parts[0].parse().map_err(|_| err())?;
        let beat: u32 = parts[1].parse().map_err(|_| err())?;
        let tick: u32 = parts[2].parse().map_err(|_| err())?;
        Ok(BarBeatTick { bar, beat, tick })
    }
}

impl fmt::Display for BarBeatTick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{:03}", self.bar, self.beat, self.tick)
    }
}

/// 位置としての妥当性(bar/beat が 1 始まり)を検証して返す。
pub fn parse_position(s: &str, beats_per_bar: u32) -> Result<BarBeatTick, CoreError> {
    let bbt: BarBeatTick = s.parse()?;
    if bbt.bar == 0 || bbt.beat == 0 || bbt.beat > beats_per_bar {
        return Err(CoreError::InvalidTimePosition {
            value: s.to_string(),
            hint: format!(
                "位置は bar>=1, 1<=beat<={beats_per_bar} が必要(拍子に依存)。長さと混同していないか確認"
            ),
        });
    }
    Ok(bbt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_name_resolves_per_convention() {
        // C3=60 規約: E1 = 4 + (1+2)*12 = 40
        let n = MidiNote::parse_name("E1", OctaveConvention::C3Is60).unwrap();
        assert_eq!(n.value(), 40);
        // C4=60 規約: E1 = 4 + (1+1)*12 = 28
        let n = MidiNote::parse_name("E1", OctaveConvention::C4Is60).unwrap();
        assert_eq!(n.value(), 28);
        // 基準音そのもの
        assert_eq!(
            MidiNote::parse_name("C3", OctaveConvention::C3Is60)
                .unwrap()
                .value(),
            60
        );
        assert_eq!(
            MidiNote::parse_name("C4", OctaveConvention::C4Is60)
                .unwrap()
                .value(),
            60
        );
    }

    #[test]
    fn note_name_accidentals_and_negative_octaves() {
        assert_eq!(
            MidiNote::parse_name("D#0", OctaveConvention::C3Is60)
                .unwrap()
                .value(),
            27
        );
        assert_eq!(
            MidiNote::parse_name("Bb2", OctaveConvention::C3Is60)
                .unwrap()
                .value(),
            58
        );
        assert_eq!(
            MidiNote::parse_name("C-2", OctaveConvention::C3Is60)
                .unwrap()
                .value(),
            0
        );
    }

    #[test]
    fn invalid_note_names_are_rejected() {
        assert!(MidiNote::parse_name("H2", OctaveConvention::C3Is60).is_err());
        assert!(MidiNote::parse_name("C", OctaveConvention::C3Is60).is_err());
        assert!(MidiNote::parse_name("C99", OctaveConvention::C3Is60).is_err());
    }

    #[test]
    fn velocity_bounds() {
        assert!(Velocity::new(0).is_err());
        assert!(Velocity::new(128).is_err());
        assert_eq!(Velocity::new(112).unwrap().value(), 112);
        assert_eq!(Velocity::new(120).unwrap().offset_clamped(20).value(), 127);
        assert_eq!(Velocity::new(5).unwrap().offset_clamped(-20).value(), 1);
    }

    #[test]
    fn bar_beat_tick_roundtrip() {
        let p: BarBeatTick = "9.3.240".parse().unwrap();
        assert_eq!((p.bar, p.beat, p.tick), (9, 3, 240));
        assert_eq!(p.to_string(), "9.3.240");
        // 4/4, PPQ480: bar9 beat3 = (8*4 + 2) * 480 + 240
        assert_eq!(p.to_absolute_ticks(480, 4), (8 * 4 + 2) * 480 + 240);
        // 長さ 0.0.240 = 240 ticks
        let d: BarBeatTick = "0.0.240".parse().unwrap();
        assert_eq!(d.to_duration_ticks(480, 4), 240);
    }

    #[test]
    fn position_validation_rejects_zero_and_beat_overflow() {
        assert!(parse_position("0.1.000", 4).is_err());
        assert!(parse_position("1.5.000", 4).is_err());
        assert!(parse_position("1.4.000", 4).is_ok());
    }
}
