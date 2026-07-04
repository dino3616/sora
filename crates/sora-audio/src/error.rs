//! オーディオ解析のエラー(技術要件書 §6 の lib 層規約に従う)。

use std::path::PathBuf;

/// sora-audio の型付きエラー。
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("I/O error on {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unsupported or corrupt audio in {path}: {message}")]
    Decode { path: PathBuf, message: String },

    #[error("no audio track found in {path}")]
    NoTrack { path: PathBuf },

    #[error("empty audio (no samples) in {path}")]
    Empty { path: PathBuf },

    #[error("analysis error: {message}")]
    Analysis { message: String },
}

impl AudioError {
    /// SCREAMING_SNAKE_CASE の安定 ID。
    pub fn code(&self) -> &'static str {
        match self {
            AudioError::Io { .. } => "AUDIO_IO_ERROR",
            AudioError::Decode { .. } => "AUDIO_DECODE_ERROR",
            AudioError::NoTrack { .. } => "AUDIO_NO_TRACK",
            AudioError::Empty { .. } => "AUDIO_EMPTY",
            AudioError::Analysis { .. } => "AUDIO_ANALYSIS_ERROR",
        }
    }

    /// 対応方法のヒント。
    pub fn hint(&self) -> Option<String> {
        match self {
            AudioError::Decode { .. } => Some(
                "対応形式は WAV/AIFF/FLAC/MP3 等。非対応の場合は WAV に変換してください"
                    .to_string(),
            ),
            _ => None,
        }
    }
}
