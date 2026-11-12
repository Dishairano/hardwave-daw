//! WAV files whose audio is Ogg Vorbis.
//!
//! FL Studio ships some of its own samples (909 Clap, 909 Snare among
//! them) as .wav files that hold Ogg Vorbis: the format tag is one of the
//! Ogg Vorbis ACM codec's (0x674f for its mode 1) and the data chunk is a
//! whole Ogg stream, headers and all. symphonia's WAV reader refuses those
//! tags, so the samples would not load; its Ogg reader and Vorbis decoder,
//! which the DAW already ships, read the stream itself.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::audio_file::AudioFileError;

/// The Ogg Vorbis ACM codec's format tags: modes 1, 2 and 3, and 1+, 2+
/// and 3+.
const VORBIS_TAGS: [u16; 6] = [0x674f, 0x6750, 0x6751, 0x676f, 0x6770, 0x6771];

/// What a WAV holding Ogg Vorbis holds.
#[derive(Debug)]
pub struct OggInWav {
    /// The whole Ogg stream, as an .ogg file would hold it.
    pub stream: Vec<u8>,
    /// How many frames the sample is (the fact chunk), when it says. The
    /// decoder gives the stream's last block whole, past the sample's end.
    pub frames: Option<u64>,
}

/// The Ogg stream inside a WAV file whose audio is Ogg Vorbis, or none
/// for any other file. A WAV with a Vorbis tag whose data is not a whole
/// Ogg stream (the codec's modes that keep the Vorbis headers apart) is
/// refused by name.
pub fn ogg_stream(path: &Path) -> Result<Option<OggInWav>, AudioFileError> {
    let Ok(mut file) = std::fs::File::open(path) else {
        // Not ours to report: the ordinary reader says it is missing.
        return Ok(None);
    };
    let len = file.metadata()?.len();
    let mut head = [0u8; 12];
    if file.read_exact(&mut head).is_err() || &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Ok(None);
    }
    let mut tag: Option<u16> = None;
    let mut data: Option<(u64, u64)> = None;
    let mut frames: Option<u64> = None;
    let mut at = 12u64;
    while at + 8 <= len {
        file.seek(SeekFrom::Start(at))?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as u64;
        let body = at + 8;
        match &chunk[0..4] {
            b"fmt " if size >= 2 => {
                let mut t = [0u8; 2];
                file.read_exact(&mut t)?;
                let t = u16::from_le_bytes(t);
                if !VORBIS_TAGS.contains(&t) {
                    // An ordinary WAV: nothing more to look at.
                    return Ok(None);
                }
                tag = Some(t);
            }
            b"data" => data = Some((body, size.min(len - body))),
            b"fact" if size >= 4 => {
                let mut n = [0u8; 4];
                file.read_exact(&mut n)?;
                frames = Some(u32::from_le_bytes(n) as u64).filter(|n| *n > 0);
            }
            _ => {}
        }
        // Chunks start on even offsets.
        at = body + size + (size & 1);
    }
    let (Some(tag), Some((start, size))) = (tag, data) else {
        return Ok(None);
    };
    file.seek(SeekFrom::Start(start))?;
    let mut stream = Vec::with_capacity(size as usize);
    file.take(size).read_to_end(&mut stream)?;
    if !stream.starts_with(b"OggS") {
        return Err(AudioFileError::UnsupportedFormat(format!(
            "this WAV holds Ogg Vorbis without its own Ogg stream (format 0x{tag:04x}), \
             which the DAW does not read yet"
        )));
    }
    Ok(Some(OggInWav { stream, frames }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A short 440 Hz tone at half scale, a quarter of a second at 44.1 kHz
    /// mono, encoded as Ogg Vorbis.
    pub(crate) const TONE: &[u8] = include_bytes!("../testdata/tone-440.ogg");

    /// A WAV holding `data` with format `tag`, laid out as FL's 909 Clap:
    /// a 26-byte fmt chunk, the data, then a fact chunk.
    pub(crate) fn wav_holding(tag: u16, data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&44_100u32.to_le_bytes());
        fmt.extend_from_slice(&14_000u32.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        fmt.extend_from_slice(&8u16.to_le_bytes());
        fmt.extend_from_slice(&[0x24, 0x07, 0x09, 0x20, 0x10, 0x07, 0x09, 0x20]);
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        for (id, chunk) in [
            (b"fmt ", fmt.as_slice()),
            (b"data", data),
            (b"fact", &11_025u32.to_le_bytes()[..]),
        ] {
            body.extend_from_slice(id);
            body.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            body.extend_from_slice(chunk);
            if chunk.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(body.len() as u32).to_le_bytes());
        wav.extend_from_slice(&body);
        wav
    }

    fn written(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("hw-oggwav-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_ogg_stream_is_found_in_the_data_chunk() {
        let path = written("clap.wav", &wav_holding(0x674f, TONE));
        let found = ogg_stream(&path).unwrap().expect("an Ogg stream");
        assert_eq!(found.stream, TONE);
        assert_eq!(found.frames, Some(11_025), "the fact chunk's length");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_ordinary_wav_or_another_file_is_not_looked_into() {
        let pcm = written("pcm.wav", &wav_holding(1, &[0u8; 64]));
        assert!(ogg_stream(&pcm).unwrap().is_none());
        let ogg = written("tone.ogg", TONE);
        assert!(ogg_stream(&ogg).unwrap().is_none());
        assert!(ogg_stream(&pcm.with_extension("missing"))
            .unwrap()
            .is_none());
        let _ = std::fs::remove_file(&pcm);
        let _ = std::fs::remove_file(&ogg);
    }

    #[test]
    fn vorbis_without_an_ogg_stream_is_refused_by_name() {
        let path = written("mode2.wav", &wav_holding(0x6750, &TONE[100..]));
        let err = ogg_stream(&path).unwrap_err().to_string();
        assert!(
            err.contains("Ogg Vorbis") && err.contains("0x6750"),
            "{err}"
        );
        let _ = std::fs::remove_file(&path);
    }
}
