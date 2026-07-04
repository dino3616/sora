//! 拍子・テンポの解決。

use crate::error::CoreError;

/// 解決済みの拍子とグリッド情報。
#[derive(Debug, Clone, Copy)]
pub struct TimeGrid {
    /// 拍子分子(1 小節の拍数)
    pub beats_per_bar: u32,
    /// 拍子分母(2 の冪)
    pub denominator: u32,
    /// PPQ(4分音符あたりの tick 数)
    pub ppq: u32,
}

impl TimeGrid {
    pub fn new(time_signature: &str, ppq: u32) -> Result<Self, CoreError> {
        let err = || CoreError::InvalidTimePosition {
            value: time_signature.to_string(),
            hint: "拍子は \"4/4\" や \"7/8\" の形式で、分母は 2 の冪(1,2,4,8,16,32)".to_string(),
        };
        let (num, den) = time_signature.split_once('/').ok_or_else(err)?;
        let beats_per_bar: u32 = num.parse().map_err(|_| err())?;
        let denominator: u32 = den.parse().map_err(|_| err())?;
        if beats_per_bar == 0 || !denominator.is_power_of_two() || denominator > 32 {
            return Err(err());
        }
        Ok(TimeGrid {
            beats_per_bar,
            denominator,
            ppq,
        })
    }

    /// 拍子分母基準の 1 拍あたり tick 数。
    pub fn ticks_per_beat(&self) -> u32 {
        self.ppq * 4 / self.denominator
    }

    /// 1 小節あたり tick 数。
    pub fn ticks_per_bar(&self) -> u64 {
        self.ticks_per_beat() as u64 * self.beats_per_bar as u64
    }

    /// SMF TimeSignature メタイベント用の分母 log2。
    pub fn denominator_log2(&self) -> u8 {
        self.denominator.trailing_zeros() as u8
    }
}

/// BPM → SMF Tempo メタイベント値(4分音符あたりマイクロ秒)。
pub fn micros_per_quarter(bpm: f64) -> u32 {
    (60_000_000.0 / bpm).round() as u32
}

/// ミリ秒 → tick 変換係数を掛けて丸める(ヒューマナイズ用)。
pub fn ms_to_ticks(ms: f64, bpm: f64, ppq: u32) -> i64 {
    (ms * ppq as f64 * bpm / 60_000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_four_grid() {
        let g = TimeGrid::new("4/4", 480).unwrap();
        assert_eq!(g.ticks_per_beat(), 480);
        assert_eq!(g.ticks_per_bar(), 1920);
        assert_eq!(g.denominator_log2(), 2);
    }

    #[test]
    fn seven_eight_grid() {
        let g = TimeGrid::new("7/8", 480).unwrap();
        assert_eq!(g.ticks_per_beat(), 240);
        assert_eq!(g.ticks_per_bar(), 1680);
        assert_eq!(g.denominator_log2(), 3);
    }

    #[test]
    fn invalid_signatures_rejected() {
        assert!(TimeGrid::new("4-4", 480).is_err());
        assert!(TimeGrid::new("4/3", 480).is_err());
        assert!(TimeGrid::new("0/4", 480).is_err());
    }

    #[test]
    fn tempo_conversion() {
        assert_eq!(micros_per_quarter(120.0), 500_000);
        assert_eq!(micros_per_quarter(142.0), 422_535);
        // 142BPM, PPQ480: 10ms ≒ 11.36 ticks → 11
        assert_eq!(ms_to_ticks(10.0, 142.0, 480), 11);
        assert_eq!(ms_to_ticks(-10.0, 142.0, 480), -11);
    }
}
