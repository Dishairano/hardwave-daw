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

    // The resampler's output runs `delay` frames behind its input. Kept as
    // it came, every resampled sample played that much late (6.7 ms at
    // 48 kHz to 44.1 kHz) and lost as much of its end. So the first `delay`
    // frames are dropped, silence is fed after the last input until the
    // whole tail is out, and the result is cut to the exact length.
    let input_frames = channels[0].len();
    let ratio = dst_sr as f64 / src_sr as f64;
    let expected = (input_frames as f64 * ratio).round() as usize;
    let delay = resampler.output_delay();
    let mut out: Vec<Vec<f32>> = (0..num_channels)
        .map(|_| Vec::with_capacity(expected + delay + chunk_size * 2))
        .collect();
    let mut chunk: Vec<Vec<f32>> = vec![vec![0.0; chunk_size]; num_channels];
    let mut cursor = 0_usize;
    while out[0].len() < expected + delay {
        // The next block of input: the file while it lasts, then silence.
        let have = input_frames.saturating_sub(cursor).min(chunk_size);
        for (dst, src) in chunk.iter_mut().zip(channels) {
            if have > 0 {
                dst[..have].copy_from_slice(&src[cursor..cursor + have]);
            }
            dst[have..].fill(0.0);
        }
        let input_slices: Vec<&[f32]> = chunk.iter().map(|v| v.as_slice()).collect();
        let processed = resampler
            .process(&input_slices, None)
            .map_err(|e| AudioFileError::Decode(format!("resample: {e}")))?;
        for (ch_idx, block) in processed.into_iter().enumerate() {
            out[ch_idx].extend_from_slice(&block);
        }
        cursor += chunk_size;
    }
    for ch in &mut out {
        ch.drain(..delay.min(ch.len()));
        ch.truncate(expected);
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

    /// A file's rate, channels and length from its header, without
    /// decoding it. None when the header does not say (a stream with no
    /// frame count) or the file cannot be read.
    pub fn probe(path: &Path) -> Option<AudioFileInfo> {
        if crate::aiff_reader::looks_like_aiff(path)
            || matches!(crate::ogg_in_wav::ogg_stream(path), Ok(Some(_)))
        {
            return Self::read(path).ok().map(|(info, _)| info);
        }
        let file = std::fs::File::open(path).ok()?;
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
            .ok()?;
        let track = probed.format.default_track()?;
        let sample_rate = track.codec_params.sample_rate?;
        let total_frames = track.codec_params.n_frames?;
        let channels = track.codec_params.channels.map(|c| c.count() as u16)?;
        Some(AudioFileInfo {
            sample_rate,
            channels,
            total_frames,
            duration_secs: total_frames as f64 / sample_rate.max(1) as f64,
        })
    }

    pub fn read(path: &Path) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        // symphonia 0.5 lacks AIFF — dispatch to our own reader.
        let (info, channels) = if crate::aiff_reader::looks_like_aiff(path) {
            crate::aiff_reader::read_aiff(path)?
        } else if let Some(ogg) = crate::ogg_in_wav::ogg_stream(path)? {
            // A WAV holding Ogg Vorbis (FL's 909 Clap and Snare): the
            // stream inside it, read as the .ogg it is, to the sample's
            // own length.
            let mss = MediaSourceStream::new(
                Box::new(std::io::Cursor::new(ogg.stream)),
                Default::default(),
            );
            let mut hint = Hint::new();
            hint.with_extension("ogg");
            let (info, mut channels) = Self::decode(mss, &hint)?;
            if let Some(frames) = ogg.frames {
                for ch in &mut channels {
                    ch.truncate(frames as usize);
                }
            }
            (info, channels)
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
        Self::decode(mss, &hint)
    }

    /// Decode a whole stream into deinterleaved f32 channels.
    fn decode(
        mss: MediaSourceStream,
        hint: &Hint,
    ) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
        let probed = symphonia::default::get_probe()
            .format(
                hint,
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
                // 24-bit, what most sample packs' WAVs are, and the rarer
                // depths: these were skipped, so the file came in with no
                // audio at all and every clip of it played silence.
                AudioBufferRef::S24(buf) => extend_channels(&mut channel_buffers, &buf),
                AudioBufferRef::U24(buf) => extend_channels(&mut channel_buffers, &buf),
                AudioBufferRef::S8(buf) => extend_channels(&mut channel_buffers, &buf),
                AudioBufferRef::U8(buf) => extend_channels(&mut channel_buffers, &buf),
                AudioBufferRef::U16(buf) => extend_channels(&mut channel_buffers, &buf),
                AudioBufferRef::U32(buf) => extend_channels(&mut channel_buffers, &buf),
                AudioBufferRef::F64(buf) => extend_channels(&mut channel_buffers, &buf),
            }
        }

        Ok((info, channel_buffers))
    }
}

/// A decoded block, in whatever sample type the file holds, onto the end of
/// each channel as -1 to 1.
fn extend_channels<S>(channels: &mut [Vec<f32>], buf: &symphonia::core::audio::AudioBuffer<S>)
where
    S: symphonia::core::sample::Sample + symphonia::core::conv::IntoSample<f32>,
{
    for (ch_buf, ch_idx) in channels.iter_mut().zip(0..buf.spec().channels.count()) {
        ch_buf.extend(buf.chan(ch_idx).iter().map(|&s| s.into_sample()));
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

    /// FL's 909 Clap and Snare: a .wav whose audio is an Ogg Vorbis
    /// stream. They were refused as an unsupported format.
    #[test]
    fn a_wav_holding_ogg_vorbis_reads_as_its_audio() {
        let wav = crate::ogg_in_wav::tests::wav_holding(0x674f, crate::ogg_in_wav::tests::TONE);
        let (info, ch) = read(&wav).expect("the Vorbis inside is read");
        assert_eq!((info.sample_rate, info.channels), (44_100, 1));
        // A quarter of a second, as the fact chunk says, not the last
        // Vorbis block whole.
        assert_eq!(ch[0].len(), 11_025);
        let peak = ch[0].iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!((0.4..0.6).contains(&peak), "the tone at half scale: {peak}");
        // The header alone says the same.
        let path = std::env::temp_dir().join(format!("hw-oggwav-probe-{}.wav", std::process::id()));
        std::fs::write(&path, &wav).unwrap();
        let probed = AudioFileReader::probe(&path).expect("a header");
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            (probed.sample_rate, probed.total_frames),
            (44_100, ch[0].len() as u64)
        );
    }

    #[test]
    fn an_ordinary_file_still_reads() {
        let (info, ch) = read(&wav(48_000, 2, 480)).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, ch[0].len()),
            (48_000, 2, 480)
        );
    }

    /// A WAV at `bits` written by hound, two channels, a frame at half
    /// scale up and one at half scale down.
    fn wav_at(bits: u16) -> Vec<u8> {
        let path = std::env::temp_dir().join(format!("hw-depth-{}-{bits}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        let half = 1i32 << (bits - 2);
        for _ in 0..100 {
            for v in [half, -half] {
                if bits == 8 {
                    w.write_sample(v as i8).unwrap();
                } else {
                    w.write_sample(v).unwrap();
                }
            }
        }
        w.finalize().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        bytes
    }

    /// Most of a real FL project's samples were 24-bit WAVs: each read as a
    /// file with no audio in it, so its clips played silence.
    #[test]
    fn every_bit_depth_reads_as_audio() {
        for bits in [8u16, 16, 24, 32] {
            let (info, ch) = read(&wav_at(bits)).unwrap();
            assert_eq!((info.channels, ch.len()), (2, 2), "{bits}-bit");
            assert_eq!(ch[0].len(), 100, "{bits}-bit: every frame");
            assert!(
                (ch[0][0] - 0.5).abs() < 0.01,
                "{bits}-bit left: {}",
                ch[0][0]
            );
            assert!(
                (ch[1][0] + 0.5).abs() < 0.01,
                "{bits}-bit right: {}",
                ch[1][0]
            );
        }
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

#[cfg(test)]
mod resampling {
    use super::*;

    /// Where the loudest sample sits within `range`.
    fn peak_in(ch: &[f32], range: std::ops::Range<usize>) -> usize {
        range
            .max_by(|a, b| ch[*a].abs().total_cmp(&ch[*b].abs()))
            .unwrap()
    }

    #[test]
    fn a_resampled_sample_keeps_its_timing_and_its_end() {
        // A click near the start and one near the end. Before, both came
        // out 6.7 ms late at 48 to 44.1 kHz and the last 10 ms were cut.
        for (src, dst) in [
            (48_000u32, 44_100u32),
            (44_100, 48_000),
            (96_000, 48_000),
            (22_050, 48_000),
        ] {
            let n = src as usize * 2;
            let mut ch = vec![0.0f32; n];
            ch[1000] = 1.0;
            ch[n - 2000] = 1.0;
            let out = resample_channels(&[ch.clone(), ch], src, dst).unwrap();
            let ratio = dst as f64 / src as f64;
            let len = out[0].len();
            assert_eq!(
                len,
                (n as f64 * ratio).round() as usize,
                "{src}->{dst}: length"
            );
            assert_eq!(out[1].len(), len);
            let first = peak_in(&out[0], 0..len / 2) as f64;
            let last = peak_in(&out[0], len / 2..len) as f64;
            assert!(
                (first - 1000.0 * ratio).abs() <= 1.0,
                "{src}->{dst}: first click at {first}"
            );
            assert!(
                (last - (n - 2000) as f64 * ratio).abs() <= 1.0,
                "{src}->{dst}: last click at {last}"
            );
        }
    }

    #[test]
    fn a_short_sample_comes_through_whole() {
        // Shorter than one block of the resampler.
        let ch: Vec<f32> = (0..300).map(|i| (i as f32 * 0.05).sin()).collect();
        let out = resample_channels(&[ch], 48_000, 44_100).unwrap();
        assert_eq!(
            out[0].len(),
            (300.0f64 * 44_100.0 / 48_000.0).round() as usize
        );
        assert!(out[0].iter().any(|v| v.abs() > 0.5), "the sound is there");
    }
}

#[cfg(test)]
mod probing {
    use super::*;

    /// The header says the rate, channels and length without the file
    /// being decoded.
    #[test]
    fn a_probe_reads_the_header() {
        let path = std::env::temp_dir().join(format!("hw-probe-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..44_100 * 2 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
        let info = AudioFileReader::probe(&path).expect("a header");
        assert_eq!(
            (info.sample_rate, info.channels, info.total_frames),
            (44_100, 2, 44_100)
        );
        assert!((info.duration_secs - 1.0).abs() < 1e-9);
        assert!(AudioFileReader::probe(&path.with_extension("missing")).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
