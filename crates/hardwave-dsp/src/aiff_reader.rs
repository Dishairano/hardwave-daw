//! Minimal AIFF / AIFF-C (uncompressed) reader.
//!
//! symphonia 0.5 does not bundle AIFF support — only WAV (RIFF), FLAC, MP3,
//! OGG, AAC, ALAC, ADPCM, CAF, ISO/MP4, MKV. AIFF lives in symphonia 0.6
//! which we haven't migrated to. This module fills the gap with a focused
//! reader sufficient for the common case: uncompressed AIFF with 8/16/24/32-bit
//! PCM samples and mono or stereo channels.
//!
//! Format spec — Apple AIFF 1.3 ("Audio Interchange File Format"):
//!
//! ```text
//! FORM chunk:
//!   "FORM"             (4 bytes)
//!   chunk_size         (4 bytes, BE u32 — size of everything after this field)
//!   "AIFF" or "AIFC"   (4 bytes — AIFC is the compressed variant)
//!
//! Subchunks (any order):
//!   COMM (Common):
//!     id "COMM"               (4 bytes)
//!     size                    (4 bytes, BE u32 — 18 for AIFF, 22+ for AIFC)
//!     num_channels            (2 bytes, BE i16)
//!     num_sample_frames       (4 bytes, BE u32)
//!     sample_size_bits        (2 bytes, BE i16 — 8 / 16 / 24 / 32)
//!     sample_rate             (10 bytes, IEEE 754 80-bit extended precision)
//!     [AIFC only: compression_type (4 bytes), compression_name (Pascal str)]
//!
//!   SSND (Sound Data):
//!     id "SSND"               (4 bytes)
//!     size                    (4 bytes, BE u32)
//!     offset                  (4 bytes, BE u32 — typically 0)
//!     block_size              (4 bytes, BE u32 — typically 0)
//!     audio_data              (rest — big-endian interleaved PCM)
//! ```

use std::io::Read;
use std::path::Path;

use crate::audio_file::{AudioFileError, AudioFileInfo};

/// Parsed AIFF metadata extracted from the COMM chunk.
struct CommChunk {
    num_channels: u16,
    num_sample_frames: u32,
    sample_size_bits: u16,
    sample_rate: u32,
}

/// Detect AIFF by file extension (case-insensitive).
pub fn looks_like_aiff(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            lower == "aiff" || lower == "aif" || lower == "aifc"
        })
        .unwrap_or(false)
}

/// Read an AIFF file and return deinterleaved f32 channels.
///
/// Only uncompressed PCM (8/16/24/32-bit) is supported. AIFC files that
/// declare a compression type other than `NONE` / `sowt` (little-endian
/// PCM) are rejected with `UnsupportedFormat`.
pub fn read_aiff(path: &Path) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| AudioFileError::NotFound(path.display().to_string()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    decode_aiff(&bytes)
}

/// Decode AIFF bytes directly (also useful for unit tests).
pub fn decode_aiff(bytes: &[u8]) -> Result<(AudioFileInfo, Vec<Vec<f32>>), AudioFileError> {
    if bytes.len() < 12 {
        return Err(AudioFileError::Decode("AIFF: file too short".into()));
    }
    if &bytes[0..4] != b"FORM" {
        return Err(AudioFileError::UnsupportedFormat(
            "AIFF: missing FORM signature".into(),
        ));
    }
    let form_type = &bytes[8..12];
    let is_aifc = match form_type {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => {
            return Err(AudioFileError::UnsupportedFormat(format!(
                "AIFF: unknown form type {:?}",
                form_type
            )))
        }
    };

    // Walk subchunks
    let mut cursor = 12usize;
    let mut comm: Option<CommChunk> = None;
    let mut comm_little_endian = false; // AIFC "sowt"
    let mut ssnd_start: Option<usize> = None;
    let mut ssnd_size: usize = 0;
    let mut ssnd_offset_value: u32 = 0;

    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = read_u32_be(&bytes[cursor + 4..cursor + 8])? as usize;
        let data_start = cursor + 8;
        let data_end = data_start.saturating_add(size);
        if data_end > bytes.len() {
            return Err(AudioFileError::Decode(format!(
                "AIFF: chunk {:?} extends past EOF (start={}, size={}, file={})",
                std::str::from_utf8(id).unwrap_or("?"),
                data_start,
                size,
                bytes.len()
            )));
        }

        match id {
            b"COMM" => {
                if size < 18 {
                    return Err(AudioFileError::Decode("AIFF: COMM chunk too small".into()));
                }
                let num_channels = read_u16_be(&bytes[data_start..data_start + 2])?;
                let num_sample_frames = read_u32_be(&bytes[data_start + 2..data_start + 6])?;
                let sample_size_bits = read_u16_be(&bytes[data_start + 6..data_start + 8])?;
                let sample_rate = read_extended_precision(&bytes[data_start + 8..data_start + 18])?;
                if is_aifc && size >= 22 {
                    let compression_type = &bytes[data_start + 18..data_start + 22];
                    // "NONE" = uncompressed big-endian PCM (standard)
                    // "sowt" = uncompressed little-endian PCM (Apple variant)
                    // "twos" = synonym for big-endian
                    // "fl32" / "FL32" = 32-bit float
                    // Anything else means compressed → reject.
                    let ct = compression_type;
                    if ct == b"NONE" || ct == b"twos" {
                        // big-endian PCM
                    } else if ct == b"sowt" {
                        comm_little_endian = true;
                    } else if ct == b"fl32" || ct == b"FL32" {
                        // 32-bit float (always big-endian per AIFC spec)
                    } else {
                        return Err(AudioFileError::UnsupportedFormat(format!(
                            "AIFC compression {:?} not supported",
                            std::str::from_utf8(ct).unwrap_or("?")
                        )));
                    }
                }
                comm = Some(CommChunk {
                    num_channels,
                    num_sample_frames,
                    sample_size_bits,
                    sample_rate,
                });
            }
            b"SSND" => {
                if size < 8 {
                    return Err(AudioFileError::Decode("AIFF: SSND chunk too small".into()));
                }
                ssnd_offset_value = read_u32_be(&bytes[data_start..data_start + 4])?;
                // block_size at data_start+4..+8 — unused for our PCM read.
                ssnd_start = Some(data_start + 8);
                ssnd_size = size - 8;
            }
            _ => {
                // Skip unrecognized chunks (FVER, ANNO, NAME, AUTH, COPY, MARK …).
            }
        }

        // Chunks pad to even byte boundary.
        cursor = data_end + (data_end & 1);
    }

    let comm = comm.ok_or_else(|| AudioFileError::Decode("AIFF: no COMM chunk".into()))?;
    let ssnd_start =
        ssnd_start.ok_or_else(|| AudioFileError::Decode("AIFF: no SSND chunk".into()))?;

    let channels = comm.num_channels as usize;
    if channels == 0 {
        return Err(AudioFileError::Decode("AIFF: zero channels".into()));
    }
    let bytes_per_sample = match comm.sample_size_bits {
        8 => 1,
        16 => 2,
        24 => 3,
        32 => 4,
        n => {
            return Err(AudioFileError::UnsupportedFormat(format!(
                "AIFF sample size {} bits not supported",
                n
            )))
        }
    };
    let frame_size = bytes_per_sample * channels;
    let audio_start = ssnd_start + ssnd_offset_value as usize;
    if audio_start > ssnd_start + ssnd_size {
        return Err(AudioFileError::Decode(
            "AIFF: SSND offset extends past chunk".into(),
        ));
    }
    let audio_bytes = &bytes[audio_start..ssnd_start + ssnd_size];
    let frames_in_chunk = audio_bytes.len() / frame_size.max(1);
    let frames = frames_in_chunk.min(comm.num_sample_frames as usize);

    let mut channel_bufs: Vec<Vec<f32>> =
        (0..channels).map(|_| Vec::with_capacity(frames)).collect();

    for f in 0..frames {
        let frame_off = f * frame_size;
        for (c, buf) in channel_bufs.iter_mut().enumerate() {
            let sample_off = frame_off + c * bytes_per_sample;
            let raw = &audio_bytes[sample_off..sample_off + bytes_per_sample];
            let value = match comm.sample_size_bits {
                8 => {
                    // 8-bit AIFF is signed (per spec).
                    raw[0] as i8 as f32 / 128.0
                }
                16 => {
                    let v = if comm_little_endian {
                        i16::from_le_bytes([raw[0], raw[1]])
                    } else {
                        i16::from_be_bytes([raw[0], raw[1]])
                    };
                    v as f32 / 32768.0
                }
                24 => {
                    // Sign-extend 24-bit to i32. AIFF big-endian default.
                    let bytes3 = if comm_little_endian {
                        [raw[2], raw[1], raw[0]]
                    } else {
                        [raw[0], raw[1], raw[2]]
                    };
                    let v = ((bytes3[0] as i32) << 24
                        | (bytes3[1] as i32) << 16
                        | (bytes3[2] as i32) << 8)
                        >> 8;
                    v as f32 / 8_388_608.0
                }
                32 => {
                    let v = if comm_little_endian {
                        i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
                    } else {
                        i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])
                    };
                    v as f32 / 2_147_483_648.0
                }
                _ => 0.0,
            };
            buf.push(value);
        }
    }

    let info = AudioFileInfo {
        sample_rate: comm.sample_rate,
        channels: comm.num_channels,
        total_frames: comm.num_sample_frames as u64,
        duration_secs: comm.num_sample_frames as f64 / comm.sample_rate.max(1) as f64,
    };
    Ok((info, channel_bufs))
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn read_u16_be(b: &[u8]) -> Result<u16, AudioFileError> {
    if b.len() < 2 {
        return Err(AudioFileError::Decode("AIFF: short u16".into()));
    }
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn read_u32_be(b: &[u8]) -> Result<u32, AudioFileError> {
    if b.len() < 4 {
        return Err(AudioFileError::Decode("AIFF: short u32".into()));
    }
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parse an Apple-style IEEE 754 80-bit extended precision sample rate.
///
/// AIFF stores sample rate as the legacy x86 long-double layout:
///   1 sign bit · 15 exponent bits · 1 explicit integer bit · 63 mantissa bits
///
/// Common audio rates (8000-192000 Hz) are far from the precision limits,
/// so an integer-only path is sufficient.
fn read_extended_precision(b: &[u8]) -> Result<u32, AudioFileError> {
    if b.len() < 10 {
        return Err(AudioFileError::Decode(
            "AIFF: short extended sample rate".into(),
        ));
    }
    let exponent_be = ((b[0] as u32) << 8) | b[1] as u32;
    let sign = (exponent_be & 0x8000) != 0;
    let exponent = (exponent_be & 0x7FFF) as i32;
    let mantissa: u64 = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exponent == 0 && mantissa == 0 {
        return Ok(0);
    }
    // Normalised value: (-1)^sign × 2^(exp-16383) × mantissa / 2^63
    let unbiased_exp = exponent - 16383 - 63;
    let value = if unbiased_exp >= 0 {
        mantissa.checked_shl(unbiased_exp as u32).unwrap_or(0)
    } else {
        let shift = (-unbiased_exp) as u32;
        if shift >= 64 {
            0
        } else {
            mantissa >> shift
        }
    };
    if sign && value > 0 {
        // Negative sample rates are nonsensical; treat as 0.
        return Ok(0);
    }
    Ok(value.min(u32::MAX as u64) as u32)
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid AIFF byte buffer in memory.
    /// `frames` of `channels` interleaved samples, `bits` per sample, big-endian.
    fn build_aiff(sample_rate: u32, channels: u16, bits: u16, samples: &[i32]) -> Vec<u8> {
        let bytes_per_sample = (bits / 8) as usize;
        let frame_count = samples.len() / channels as usize;
        let ssnd_data_size = bytes_per_sample * samples.len() + 8; // +8 for offset+block_size
        let comm_size = 18usize;
        let form_size = 4 + (8 + comm_size) + (8 + ssnd_data_size);

        let mut b = Vec::with_capacity(form_size + 8);
        b.extend_from_slice(b"FORM");
        b.extend_from_slice(&(form_size as u32).to_be_bytes());
        b.extend_from_slice(b"AIFF");

        // COMM
        b.extend_from_slice(b"COMM");
        b.extend_from_slice(&(comm_size as u32).to_be_bytes());
        b.extend_from_slice(&(channels as i16).to_be_bytes());
        b.extend_from_slice(&(frame_count as u32).to_be_bytes());
        b.extend_from_slice(&(bits as i16).to_be_bytes());
        b.extend_from_slice(&u32_to_extended_precision(sample_rate));

        // SSND
        b.extend_from_slice(b"SSND");
        b.extend_from_slice(&(ssnd_data_size as u32).to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes()); // offset
        b.extend_from_slice(&0u32.to_be_bytes()); // block_size
        for s in samples {
            match bits {
                8 => b.push(*s as i8 as u8),
                16 => b.extend_from_slice(&(*s as i16).to_be_bytes()),
                24 => {
                    let v = *s;
                    b.push(((v >> 16) & 0xFF) as u8);
                    b.push(((v >> 8) & 0xFF) as u8);
                    b.push((v & 0xFF) as u8);
                }
                32 => b.extend_from_slice(&s.to_be_bytes()),
                _ => panic!("unsupported test bits"),
            }
        }
        b
    }

    fn u32_to_extended_precision(value: u32) -> [u8; 10] {
        if value == 0 {
            return [0u8; 10];
        }
        let mantissa_bits_needed = 64 - (value as u64).leading_zeros();
        let exponent = mantissa_bits_needed + 16383 - 1;
        let mantissa = (value as u64) << (64 - mantissa_bits_needed);
        let mut out = [0u8; 10];
        out[0] = (exponent >> 8) as u8;
        out[1] = exponent as u8;
        out[2..].copy_from_slice(&mantissa.to_be_bytes());
        out
    }

    #[test]
    fn detects_aiff_extension() {
        assert!(looks_like_aiff(Path::new("foo.aiff")));
        assert!(looks_like_aiff(Path::new("foo.aif")));
        assert!(looks_like_aiff(Path::new("foo.aifc")));
        assert!(looks_like_aiff(Path::new("FOO.AIFF")));
        assert!(!looks_like_aiff(Path::new("foo.wav")));
        assert!(!looks_like_aiff(Path::new("noext")));
    }

    #[test]
    fn decode_16bit_mono_aiff() {
        // 4-sample 16-bit mono at 44100 Hz, values 0, max, min, mid
        let samples = [0i32, 32767, -32768, 16384];
        let aiff = build_aiff(44100, 1, 16, &samples);
        let (info, channels) = decode_aiff(&aiff).expect("decode");
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 1);
        assert_eq!(info.total_frames, 4);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].len(), 4);
        assert!((channels[0][0] - 0.0).abs() < 1e-6);
        assert!((channels[0][1] - (32767.0 / 32768.0)).abs() < 1e-6);
        assert!((channels[0][2] - (-1.0)).abs() < 1e-6);
        assert!((channels[0][3] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn decode_16bit_stereo_aiff() {
        // 3 frames stereo (6 samples): L0,R0, L1,R1, L2,R2
        let samples = [1000i32, -1000, 2000, -2000, 3000, -3000];
        let aiff = build_aiff(48000, 2, 16, &samples);
        let (info, channels) = decode_aiff(&aiff).expect("decode");
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.total_frames, 3);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].len(), 3);
        assert_eq!(channels[1].len(), 3);
        // Left channel should be positive; right should be negative.
        assert!(channels[0][0] > 0.0);
        assert!(channels[1][0] < 0.0);
    }

    #[test]
    fn decode_24bit_mono_aiff() {
        // 24-bit values near full scale
        let samples = [0i32, 8_388_607, -8_388_608];
        let aiff = build_aiff(44100, 1, 24, &samples);
        let (info, channels) = decode_aiff(&aiff).expect("decode");
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(channels[0].len(), 3);
        assert!((channels[0][0] - 0.0).abs() < 1e-6);
        assert!((channels[0][1] - (8_388_607.0 / 8_388_608.0)).abs() < 1e-3);
        assert!((channels[0][2] - (-1.0)).abs() < 1e-3);
    }

    #[test]
    fn rejects_missing_form() {
        let bad = b"FAKE\0\0\0\0AIFF".to_vec();
        let err = decode_aiff(&bad).unwrap_err();
        match err {
            AudioFileError::UnsupportedFormat(_) => {}
            other => panic!("expected UnsupportedFormat, got {:?}", other),
        }
    }

    #[test]
    fn rejects_short_file() {
        let err = decode_aiff(&[0u8; 4]).unwrap_err();
        match err {
            AudioFileError::Decode(_) => {}
            other => panic!("expected Decode, got {:?}", other),
        }
    }

    #[test]
    fn parse_sample_rate_44100() {
        // 44100 Hz IEEE 754 80-bit extended:
        // exponent = 16383 + 15 = 16398 = 0x400E
        // mantissa = 44100 << (64-16) = 44100 << 48 = 0xAC44_0000_0000_0000
        let bytes = [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0];
        let rate = read_extended_precision(&bytes).unwrap();
        assert_eq!(rate, 44100);
    }

    #[test]
    fn parse_sample_rate_48000() {
        // 48000 Hz: exponent = 16383 + 15 = 16398, mantissa = 48000 << 48
        let bytes = [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0];
        let rate = read_extended_precision(&bytes).unwrap();
        assert_eq!(rate, 48000);
    }
}
