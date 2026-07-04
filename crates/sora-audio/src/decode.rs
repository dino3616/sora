//! symphonia による pure-Rust デコード(技術要件書 §10、ffmpeg 非依存)。

use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::AudioError;

/// デコード済みオーディオ(インターリーブ f32、-1.0..1.0 目安)。
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: usize,
    /// インターリーブサンプル(frame 0 ch0, frame 0 ch1, frame 1 ch0, ...)
    pub interleaved: Vec<f32>,
}

impl DecodedAudio {
    pub fn frame_count(&self) -> usize {
        self.interleaved
            .len()
            .checked_div(self.channels)
            .unwrap_or(0)
    }

    /// 指定チャンネルのサンプルを取り出す(0 始まり)。
    pub fn channel(&self, ch: usize) -> Vec<f32> {
        if ch >= self.channels {
            return Vec::new();
        }
        self.interleaved
            .iter()
            .skip(ch)
            .step_by(self.channels)
            .copied()
            .collect()
    }

    /// モノラルへダウンミックス(全チャンネル平均)。
    pub fn mono(&self) -> Vec<f32> {
        if self.channels <= 1 {
            return self.interleaved.clone();
        }
        self.interleaved
            .chunks(self.channels)
            .map(|frame| frame.iter().sum::<f32>() / self.channels as f32)
            .collect()
    }
}

/// ファイルをデコードする。
pub fn decode_file(path: &Path) -> Result<DecodedAudio, AudioError> {
    let file = std::fs::File::open(path).map_err(|e| AudioError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::NoTrack {
            path: path.to_path_buf(),
        })?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &Default::default())
        .map_err(|e| AudioError::Decode {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(0);
    let mut interleaved: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // ストリーム終端(symphonia は IoError(UnexpectedEof) を返す)
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => {
                return Err(AudioError::Decode {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                });
            }
        };
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let spec = *decoded.spec();
                    sample_rate = spec.rate;
                    channels = spec.channels.count();
                    sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                }
                if let Some(buf) = sample_buf.as_mut() {
                    buf.copy_interleaved_ref(decoded);
                    interleaved.extend_from_slice(buf.samples());
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => {
                return Err(AudioError::Decode {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                });
            }
        }
    }

    if interleaved.is_empty() || channels == 0 || sample_rate == 0 {
        return Err(AudioError::Empty {
            path: path.to_path_buf(),
        });
    }

    Ok(DecodedAudio {
        sample_rate,
        channels,
        interleaved,
    })
}
