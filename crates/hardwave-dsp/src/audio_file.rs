//! Audio file reading via symphonia (WAV, FLAC, MP3, OGG, AAC).

use rubato::{FftFixedIn, Resampler};
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
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
    /// Read a file, resampling to `target_sample_rate` if provided and different
    /// from the file's native rate. Returns the (possibly updated) info and
    /// deinterleaved f32 channels.
    pub fn read_resampled(
        path: &Path,
        target_sample_rate: Option<u32>,
    ) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        let (mut info, channels) = Self::read(path)?;
        let Some(target) = target_sample_rate else {
            return Ok((info, channels));
        };
        if target == info.sample_rate || channels.is_empty() {
            return Ok((info, channels));
        }

        // Offline resample with rubato's FFT fixed-in converter.
        let chunk_size = 1024_usize;
        let num_channels = channels.len();
        let mut resampler = FftFixedIn::<f32>::new(
            info.sample_rate as usize,
            target as usize,
            chunk_size,
            2,
            num_channels,
        )
        .map_err(|e| AudioFileError::Decode(format!("resampler init: {e}")))?;

        let input_frames = channels[0].len();
        let mut out: Vec<Vec<f32>> = (0..num_channels).map(|_| Vec::new()).collect();
        let mut cursor = 0_usize;

        while cursor + chunk_size <= input_frames {
            let input_slices: Vec<&[f32]> = channels
                .iter()
                .map(|ch| &ch[cursor..cursor + chunk_size])
                .collect();
            let processed = resampler
                .process(&input_slices, None)
                .map_err(|e| AudioFileError::Decode(format!("resample: {e}")))?;
            for (ch_idx, chunk) in processed.into_iter().enumerate() {
                out[ch_idx].extend_from_slice(&chunk);
            }
            cursor += chunk_size;
        }
        // Flush remaining frames (pad the final partial chunk with zeros).
        if cursor < input_frames {
            let remaining = input_frames - cursor;
            let mut padded: Vec<Vec<f32>> = channels
                .iter()
                .map(|ch| {
                    let mut v = ch[cursor..].to_vec();
                    v.resize(chunk_size, 0.0);
                    v
                })
                .collect();
            let input_slices: Vec<&[f32]> = padded.iter_mut().map(|v| v.as_slice()).collect();
            if let Ok(processed) = resampler.process(&input_slices, None) {
                // Keep only the proportionally relevant output frames.
                let ratio = target as f64 / info.sample_rate as f64;
                let keep = (remaining as f64 * ratio).round() as usize;
                for (ch_idx, chunk) in processed.into_iter().enumerate() {
                    let take = keep.min(chunk.len());
                    out[ch_idx].extend_from_slice(&chunk[..take]);
                }
            }
        }

        info.sample_rate = target;
        info.total_frames = out.first().map(|c| c.len() as u64).unwrap_or(0);
        info.duration_secs = info.total_frames as f64 / target as f64;
        Ok((info, out))
    }

    pub fn read(path: &Path) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        let file = std::fs::File::open(path)
            .map_err(|_| AudioFileError::NotFound(path.display().to_string()))?;
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
            .map_err(|e| AudioFileError::UnsupportedFormat(e.to_string()))?;

        let mut format = probed.format;

        let track = format
            .default_track()
            .ok_or_else(|| AudioFileError::Decode("No audio track found".into()))?;

        let sample_rate = track.codec_params.sample_rate.unwrap_or(48000);
        let channels = track
            .codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2);
        let total_frames = track.codec_params.n_frames.unwrap_or(0);
        let duration_secs = total_frames as f64 / sample_rate as f64;

        let info = AudioFileInfo {
            sample_rate,
            channels,
            total_frames,
            duration_secs,
        };

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| AudioFileError::Decode(e.to_string()))?;

        let mut channel_buffers: Vec<Vec<f32>> = (0..channels).map(|_| Vec::new()).collect();

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(_) => break,
            };

            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(_) => continue,
            };

            match decoded {
                AudioBufferRef::F32(buf) => {
                    for (ch_buf, ch_idx) in channel_buffers
                        .iter_mut()
                        .zip(0..buf.spec().channels.count())
                    {
                        ch_buf.extend_from_slice(buf.chan(ch_idx));
                    }
                }
                AudioBufferRef::S16(buf) => {
                    for (ch_buf, ch_idx) in channel_buffers
                        .iter_mut()
                        .zip(0..buf.spec().channels.count())
                    {
                        ch_buf.extend(buf.chan(ch_idx).iter().map(|&s| s as f32 / 32768.0));
                    }
                }
                AudioBufferRef::S32(buf) => {
                    for (ch_buf, ch_idx) in channel_buffers
                        .iter_mut()
                        .zip(0..buf.spec().channels.count())
                    {
                        ch_buf.extend(buf.chan(ch_idx).iter().map(|&s| s as f32 / 2147483648.0));
                    }
                }
                _ => {}
            }
        }

        Ok((info, channel_buffers))
    }
}
