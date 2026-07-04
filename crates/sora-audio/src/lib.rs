//! sora-audio — オーディオデコードと解析(技術要件書 §10)。
//!
//! symphonia でデコード(ffmpeg 非依存)、ebur128 でラウドネス(BS.1770-4)、
//! realfft で帯域バランス・クレストファクタ・ステレオ相関を測定する。
//! 数値化のみを担い、解釈と優先度付けは Agent 層が行う。

pub mod analyze;
pub mod decode;
pub mod error;
pub mod loudness;
pub mod spectrum;

pub use analyze::{AudioAnalysis, AudioComparison, analyze_file, compare_files};
pub use error::AudioError;
