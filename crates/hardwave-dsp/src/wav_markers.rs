//! The markers a WAV file carries besides its audio: cue points, the
//! regions and slice labels other editors write next to them, and the
//! sampler loops of its `smpl` chunk. FL's sample window loads these as
//! regions and slice markers; a sampler's "use loop points" plays the
//! file's own loop.
//!
//! Only the chunks that describe markers are read, from a file of any
//! size: the audio itself is skipped by its length, never loaded.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// What one marker in the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// A single point: a cue, a slice, a beat.
    Point,
    /// A stretch of the file with a length (a cue with an `ltxt` length).
    Region,
    /// A loop from the `smpl` chunk.
    Loop,
}

/// One marker, in frames from the start of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct WavMarker {
    pub kind: MarkerKind,
    pub start: u64,
    /// Where a region or loop ends (exclusive); the start for a point.
    pub end: u64,
    pub label: String,
}

/// A WAV file's markers and the rate its frames count in.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WavMarkers {
    pub sample_rate: u32,
    pub markers: Vec<WavMarker>,
}

/// The largest marker chunk read: no real cue list or label table comes
/// near it, and a file that claims more is not trusted with memory.
const MAX_CHUNK: u32 = 4 << 20;

/// The markers of a WAV file, in the order of their start. A file that is
/// not a WAV, or has no markers, gives none; only an unreadable file is an
/// error.
pub fn read_wav_markers(path: &Path) -> std::io::Result<WavMarkers> {
    let mut f = std::fs::File::open(path)?;
    Ok(markers_from(&mut f))
}

/// As `read_wav_markers`, from anything readable and seekable.
pub fn markers_from<R: Read + Seek>(r: &mut R) -> WavMarkers {
    let mut head = [0u8; 12];
    if r.read_exact(&mut head).is_err() || &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return WavMarkers::default();
    }
    let mut sample_rate = 0u32;
    // cue id -> frame
    let mut cues: Vec<(u32, u64)> = Vec::new();
    let mut labels: Vec<(u32, String)> = Vec::new();
    // cue id -> length in frames
    let mut lengths: Vec<(u32, u64)> = Vec::new();
    let mut loops: Vec<(u64, u64)> = Vec::new();

    loop {
        let mut ch = [0u8; 8];
        if r.read_exact(&mut ch).is_err() {
            break;
        }
        let id = [ch[0], ch[1], ch[2], ch[3]];
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        let padded = size as u64 + (size as u64 & 1);
        let wanted = matches!(&id, b"cue " | b"LIST" | b"smpl" | b"fmt ");
        if !wanted || size > MAX_CHUNK {
            if r.seek(SeekFrom::Current(padded as i64)).is_err() {
                break;
            }
            continue;
        }
        let mut body = vec![0u8; size as usize];
        if r.read_exact(&mut body).is_err() {
            break;
        }
        if size & 1 == 1 {
            let _ = r.seek(SeekFrom::Current(1));
        }
        match &id {
            b"cue " => read_cues(&body, &mut cues),
            b"LIST" => read_list(&body, &mut labels, &mut lengths),
            b"smpl" => read_smpl(&body, &mut loops),
            b"fmt " => sample_rate = u32_at(&body, 4).unwrap_or(0),
            _ => {}
        }
    }

    let label_of = |id: u32| {
        labels
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, l)| l.clone())
            .unwrap_or_default()
    };
    let mut out: Vec<WavMarker> = cues
        .iter()
        .map(
            |&(id, start)| match lengths.iter().find(|(i, _)| *i == id) {
                Some(&(_, len)) if len > 0 => WavMarker {
                    kind: MarkerKind::Region,
                    start,
                    end: start + len,
                    label: label_of(id),
                },
                _ => WavMarker {
                    kind: MarkerKind::Point,
                    start,
                    end: start,
                    label: label_of(id),
                },
            },
        )
        .collect();
    out.extend(loops.iter().map(|&(start, end)| WavMarker {
        kind: MarkerKind::Loop,
        start,
        end,
        label: String::new(),
    }));
    out.sort_by_key(|m| (m.start, m.end));
    WavMarkers {
        sample_rate,
        markers: out,
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `cue `: a count, then 24 bytes a point: id, position, data chunk id,
/// chunk start, block start, sample offset (the frame).
fn read_cues(b: &[u8], out: &mut Vec<(u32, u64)>) {
    let n = u32_at(b, 0).unwrap_or(0) as usize;
    for k in 0..n.min((b.len().saturating_sub(4)) / 24) {
        let at = 4 + k * 24;
        if let (Some(id), Some(offset)) = (u32_at(b, at), u32_at(b, at + 20)) {
            out.push((id, offset as u64));
        }
    }
}

/// `LIST` of type `adtl`: `labl` (a cue's name) and `ltxt` (a cue's
/// length, which makes it a region) sub-chunks.
fn read_list(b: &[u8], labels: &mut Vec<(u32, String)>, lengths: &mut Vec<(u32, u64)>) {
    if b.len() < 4 || &b[0..4] != b"adtl" {
        return;
    }
    let mut at = 4;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = u32_at(b, at + 4).unwrap_or(0) as usize;
        let body_start = at + 8;
        let body_end = (body_start + size).min(b.len());
        let body = &b[body_start..body_end];
        match id {
            b"labl" if body.len() >= 4 => {
                let cue = u32_at(body, 0).unwrap_or(0);
                let text = String::from_utf8_lossy(&body[4..])
                    .trim_end_matches('\0')
                    .trim()
                    .to_string();
                labels.push((cue, text));
            }
            b"ltxt" if body.len() >= 8 => {
                let cue = u32_at(body, 0).unwrap_or(0);
                let len = u32_at(body, 4).unwrap_or(0) as u64;
                lengths.push((cue, len));
            }
            _ => {}
        }
        at = body_start + size + (size & 1);
    }
}

/// `smpl`: 36 bytes of header (the loop count at 28), then 24 bytes a
/// loop: id, type, start, end (inclusive), fraction, play count.
fn read_smpl(b: &[u8], out: &mut Vec<(u64, u64)>) {
    let n = u32_at(b, 28).unwrap_or(0) as usize;
    for k in 0..n.min((b.len().saturating_sub(36)) / 24) {
        let at = 36 + k * 24;
        if let (Some(start), Some(end)) = (u32_at(b, at + 8), u32_at(b, at + 12)) {
            if end >= start {
                out.push((start as u64, end as u64 + 1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_le_bytes());
        c.extend_from_slice(body);
        if body.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    fn wav(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut w = b"RIFF".to_vec();
        w.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend(body);
        w
    }

    fn cue(points: &[(u32, u32)]) -> Vec<u8> {
        let mut b = (points.len() as u32).to_le_bytes().to_vec();
        for &(id, frame) in points {
            b.extend_from_slice(&id.to_le_bytes());
            b.extend_from_slice(&frame.to_le_bytes()); // position
            b.extend_from_slice(b"data");
            b.extend_from_slice(&0u32.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            b.extend_from_slice(&frame.to_le_bytes()); // sample offset
        }
        chunk(b"cue ", &b)
    }

    fn adtl(labels: &[(u32, &str)], lengths: &[(u32, u32)]) -> Vec<u8> {
        let mut b = b"adtl".to_vec();
        for &(id, text) in labels {
            let mut body = id.to_le_bytes().to_vec();
            body.extend_from_slice(text.as_bytes());
            body.push(0);
            b.extend(chunk(b"labl", &body));
        }
        for &(id, len) in lengths {
            let mut body = id.to_le_bytes().to_vec();
            body.extend_from_slice(&len.to_le_bytes());
            body.extend_from_slice(b"rgn ");
            body.extend_from_slice(&[0u8; 8]);
            b.extend(chunk(b"ltxt", &body));
        }
        chunk(b"LIST", &b)
    }

    fn smpl(loops: &[(u32, u32)]) -> Vec<u8> {
        let mut b = vec![0u8; 36];
        b[28..32].copy_from_slice(&(loops.len() as u32).to_le_bytes());
        for &(start, end) in loops {
            b.extend_from_slice(&0u32.to_le_bytes()); // id
            b.extend_from_slice(&0u32.to_le_bytes()); // type: forward
            b.extend_from_slice(&start.to_le_bytes());
            b.extend_from_slice(&end.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
        }
        chunk(b"smpl", &b)
    }

    #[test]
    fn cues_regions_labels_and_loops_come_out_in_order() {
        let data = chunk(b"data", &vec![0u8; 4000]);
        let file = wav(&[
            chunk(b"fmt ", &[0u8; 16]),
            data,
            cue(&[(1, 1000), (2, 200), (3, 3000)]),
            adtl(&[(1, "Drop"), (2, "Intro")], &[(2, 300)]),
            smpl(&[(500, 899)]),
        ]);
        let mut fmt = [0u8; 16];
        fmt[4..8].copy_from_slice(&44_100u32.to_le_bytes());
        let file = {
            let mut f = file;
            // Put the rate in the fmt chunk written above.
            let at = 12 + 8;
            f[at..at + 16].copy_from_slice(&fmt);
            f
        };
        let read = markers_from(&mut Cursor::new(file));
        assert_eq!(read.sample_rate, 44_100);
        let m = read.markers;
        assert_eq!(
            m,
            vec![
                WavMarker {
                    kind: MarkerKind::Region,
                    start: 200,
                    end: 500,
                    label: "Intro".into()
                },
                WavMarker {
                    kind: MarkerKind::Loop,
                    start: 500,
                    end: 900,
                    label: String::new()
                },
                WavMarker {
                    kind: MarkerKind::Point,
                    start: 1000,
                    end: 1000,
                    label: "Drop".into()
                },
                WavMarker {
                    kind: MarkerKind::Point,
                    start: 3000,
                    end: 3000,
                    label: String::new()
                },
            ]
        );
    }

    #[test]
    fn a_file_without_markers_or_not_a_wav_gives_none() {
        let plain = wav(&[chunk(b"fmt ", &[0u8; 16]), chunk(b"data", &[0u8; 10])]);
        assert!(markers_from(&mut Cursor::new(plain)).markers.is_empty());
        assert!(markers_from(&mut Cursor::new(b"fLaC....".to_vec()))
            .markers
            .is_empty());
        // A cue chunk that lies about its count stops at what it holds.
        let mut lying = (100u32).to_le_bytes().to_vec();
        lying.extend_from_slice(&[0u8; 24]);
        let file = wav(&[chunk(b"cue ", &lying)]);
        assert_eq!(markers_from(&mut Cursor::new(file)).markers.len(), 1);
        // A chunk that claims more than the file has ends the reading.
        let mut cut = wav(&[chunk(b"fmt ", &[0u8; 16])]);
        cut.extend_from_slice(b"cue ");
        cut.extend_from_slice(&1000u32.to_le_bytes());
        cut.extend_from_slice(&[0u8; 10]);
        assert!(markers_from(&mut Cursor::new(cut)).markers.is_empty());
    }
}
