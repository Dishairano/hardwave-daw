//! Audio file reading via symphonia (WAV, FLAC, MP3, OGG, AAC, ALAC, CAF, MP4).
//! AIFF / AIFC dispatches to `crate::aiff_reader` — symphonia 0.5 ships no AIFF
//! demuxer and we haven't migrated to 0.6.

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

/// The sample rates a file may claim. Real audio is 8 kHz to 384 kHz; a
/// file that says otherwise is damaged or made to hurt (a resampler set
/// up for four billion hertz asks for gigabytes before reading a sample).
pub const MIN_SAMPLE_RATE: u32 = 8_000;
pub const MAX_SAMPLE_RATE: u32 = 384_000;
/// The most channels a file may have.
pub const MAX_CHANNELS: usize = 32;
/// The most samples (frames times channels) one file may decode to:
/// two gigabytes of audio, past any sample anyone imports.
pub const MAX_DECODED_SAMPLES: usize = 512 * 1024 * 1024;

fn check_rate(rate: u32) -> Result<(), AudioFileError> {
    if (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&rate) {
        Ok(())
    } else {
        Err(AudioFileError::UnsupportedFormat(format!(
            "a sample rate of {rate} Hz is not one audio uses"
        )))
    }
}

/// What a decoder produced, made safe to hand to the engine: channel
/// count and rate in range, and every channel the same length, which
/// everything downstream indexes by.
fn settle(
    info: AudioFileInfo,
    mut channels: Vec<Vec<f32>>,
) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
    check_rate(info.sample_rate)?;
    if channels.is_empty() || channels.len() > MAX_CHANNELS {
        return Err(AudioFileError::UnsupportedFormat(format!(
            "{} channels is not something the DAW plays",
            channels.len()
        )));
    }
    let frames = channels.iter().map(Vec::len).min().unwrap_or(0);
    for ch in &mut channels {
        ch.truncate(frames);
    }
    let info = AudioFileInfo {
        channels: channels.len() as u16,
        total_frames: frames as u64,
        duration_secs: frames as f64 / info.sample_rate as f64,
        ..info
    };
    Ok((info, channels))
}

/// Offline resample a set of deinterleaved f32 channels from `src_sr` to
/// `dst_sr` using rubato's FFT fixed-in converter. Used both by
/// `AudioFileReader::read_resampled` at load time and by the engine when the
/// audio device's sample rate changes after files were already loaded into
/// the audio pool — in that case every cached buffer is re-resampled to keep
/// playback pitch correct.
pub fn resample_channels(
    channels: &[Vec<f32>],
    src_sr: u32,
    dst_sr: u32,
) -> Result<Vec<Vec<f32>>, AudioFileError> {
    if channels.is_empty() {
        return Ok(Vec::new());
    }
    check_rate(src_sr)?;
    check_rate(dst_sr)?;
    if src_sr == dst_sr {
        return Ok(channels.to_vec());
    }

    let chunk_size = 1024_usize;
    let num_channels = channels.len();
    let mut resampler = FftFixedIn::<f32>::new(
        src_sr as usize,
        dst_sr as usize,
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
    if cursor < input_frames {
        let remaining = input_frames - cursor;
        let padded: Vec<Vec<f32>> = channels
            .iter()
            .map(|ch| {
                let mut v = ch[cursor..].to_vec();
                v.resize(chunk_size, 0.0);
                v
            })
            .collect();
        let input_slices: Vec<&[f32]> = padded.iter().map(|v| v.as_slice()).collect();
        if let Ok(processed) = resampler.process(&input_slices, None) {
            let ratio = dst_sr as f64 / src_sr as f64;
            let keep = (remaining as f64 * ratio).round() as usize;
            for (ch_idx, chunk) in processed.into_iter().enumerate() {
                let take = keep.min(chunk.len());
                out[ch_idx].extend_from_slice(&chunk[..take]);
            }
        }
    }
    Ok(out)
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
        let out = resample_channels(&channels, info.sample_rate, target)?;
        info.sample_rate = target;
        info.total_frames = out.first().map(|c| c.len() as u64).unwrap_or(0);
        info.duration_secs = info.total_frames as f64 / target as f64;
        Ok((info, out))
    }

    pub fn read(path: &Path) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        // symphonia 0.5 lacks AIFF — dispatch to our own reader.
        let (info, channels) = if crate::aiff_reader::looks_like_aiff(path) {
            crate::aiff_reader::read_aiff(path)?
        } else {
            Self::read_with_symphonia(path)?
        };
        settle(info, channels)
    }

    fn read_with_symphonia(path: &Path) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
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
        check_rate(sample_rate)?;
        let channels = track
            .codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2);
        if channels == 0 || channels as usize > MAX_CHANNELS {
            return Err(AudioFileError::UnsupportedFormat(format!(
                "{channels} channels is not something the DAW plays"
            )));
        }
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

        let mut decoded_samples = 0usize;
        loop {
            if decoded_samples > MAX_DECODED_SAMPLES {
                return Err(AudioFileError::UnsupportedFormat(
                    "that file decodes to more audio than any sample holds".into(),
                ));
            }
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
            decoded_samples = decoded_samples.saturating_add(
                decoded
                    .frames()
                    .saturating_mul(decoded.spec().channels.count()),
            );

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

#[cfg(test)]
mod hostile_files {
    use super::*;

    /// A WAV file whose header says what it likes.
    fn wav(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        let data_len = frames * channels as u32 * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate.wrapping_mul(channels as u32 * 2)).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        b.resize(b.len() + data_len as usize, 0);
        b
    }

    fn read(bytes: &[u8]) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        let path = std::env::temp_dir().join(format!(
            "hw-hostile-{}-{}.wav",
            std::process::id(),
            bytes.len()
        ));
        std::fs::write(&path, bytes).unwrap();
        let out = AudioFileReader::read(&path);
        let _ = std::fs::remove_file(&path);
        out
    }

    #[test]
    fn an_ordinary_file_still_reads() {
        let (info, ch) = read(&wav(48_000, 2, 480)).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, ch[0].len()),
            (48_000, 2, 480)
        );
    }

    #[test]
    fn a_rate_no_audio_uses_is_refused_before_any_work() {
        let started = std::time::Instant::now();
        assert!(read(&wav(4_294_967_291, 2, 16)).is_err());
        assert!(read(&wav(1, 1, 16)).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(resample_channels(&[vec![0.0; 16]], 4_294_967_291, 48_000).is_err());
    }
}
