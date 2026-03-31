//! Audio file reading via symphonia (WAV, FLAC, MP3, OGG, AAC).

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioFileError {
    #[error("File not found: {0}")]
    NotFound(String),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct AudioFileInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: u64,
    pub duration_secs: f64,
}

/// Reads an entire audio file into memory as deinterleaved f32 channels.
pub struct AudioFileReader;

impl AudioFileReader {
    pub fn read(path: &Path) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        let file = std::fs::File::open(path)
            .map_err(|_| AudioFileError::NotFound(path.display().to_string()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| AudioFileError::UnsupportedFormat(e.to_string()))?;

        let mut format = probed.format;

        let track = format.default_track()
            .ok_or_else(|| AudioFileError::Decode("No audio track found".into()))?;

        let sample_rate = track.codec_params.sample_rate.unwrap_or(48000);
        let channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
        let total_frames = track.codec_params.n_frames.unwrap_or(0);
        let duration_secs = total_frames as f64 / sample_rate as f64;

        let info = AudioFileInfo { sample_rate, channels, total_frames, duration_secs };

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| AudioFileError::Decode(e.to_string()))?;

        let mut channel_buffers: Vec<Vec<f32>> = (0..channels).map(|_| Vec::new()).collect();

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(_) => break,
            };

            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(_) => continue,
            };

            match decoded {
                AudioBufferRef::F32(buf) => {
                    for ch in 0..channels as usize {
                        if ch < buf.spec().channels.count() {
                            channel_buffers[ch].extend_from_slice(buf.chan(ch));
                        }
                    }
                }
                AudioBufferRef::S16(buf) => {
                    for ch in 0..channels as usize {
                        if ch < buf.spec().channels.count() {
                            channel_buffers[ch].extend(
                                buf.chan(ch).iter().map(|&s| s as f32 / 32768.0)
                            );
                        }
                    }
                }
                AudioBufferRef::S32(buf) => {
                    for ch in 0..channels as usize {
                        if ch < buf.spec().channels.count() {
                            channel_buffers[ch].extend(
                                buf.chan(ch).iter().map(|&s| s as f32 / 2147483648.0)
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        Ok((info, channel_buffers))
    }
}
