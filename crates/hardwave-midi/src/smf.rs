//! Standard MIDI File (SMF, `.mid`) export. Serializes a [`MidiClip`]'s
//! notes and controller movements to a format-0 SMF so an idea can be carried into another DAW,
//! plug-in, or notation tool. Tempo is embedded as a meta event; the
//! clip's tick resolution maps straight onto the SMF division (PPQ).

use crate::{MidiClip, MidiControlKind, PPQ};

/// Variable-length quantity (SMF delta-time encoding): 7 bits per byte,
/// high bit set on all but the last.
fn write_vlq(out: &mut Vec<u8>, mut value: u32) {
    let mut buffer = value & 0x7F;
    while {
        value >>= 7;
        value > 0
    } {
        buffer <<= 8;
        buffer |= 0x80 | (value & 0x7F);
    }
    loop {
        out.push((buffer & 0xFF) as u8);
        if buffer & 0x80 != 0 {
            buffer >>= 8;
        } else {
            break;
        }
    }
}

fn velocity_to_midi(v: f32) -> u8 {
    // Map 0..1 → 1..127 (a note-on of velocity 0 is a note-off in MIDI,
    // so the floor is 1 for an audible note).
    ((v.clamp(0.0, 1.0) * 127.0).round() as u8).max(1)
}

/// One tempo or signature change for a song's conductor track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmfTempo {
    pub tick: u64,
    pub bpm: f64,
    pub time_sig: (u32, u32),
}

/// One track of a song export: its name and every note and controller
/// move it plays, at song ticks.
pub struct SmfTrack<'a> {
    pub name: &'a str,
    pub clip: &'a MidiClip,
}

/// Render `clip` to format-0 Standard MIDI File bytes at `bpm`. The SMF
/// division is the project PPQ, so note ticks transfer 1:1.
pub fn write_smf(clip: &MidiClip, bpm: f64) -> Vec<u8> {
    let mut track = Vec::new();
    write_tempo(&mut track, 0, bpm);
    let events = clip_events(clip);
    write_events(&mut track, events, 0);
    end_track(&mut track);
    let mut out = header(0, 1);
    push_chunk(&mut out, &track);
    out
}

/// A whole song as a format-1 Standard MIDI File: a conductor track with
/// every tempo and signature change, then one track per MIDI track, named.
/// A tempo ramp is written as the step at its start: SMF has no ramps.
pub fn write_song_smf(tempos: &[SmfTempo], tracks: &[SmfTrack<'_>]) -> Vec<u8> {
    let mut conductor = Vec::new();
    let mut prev = 0u64;
    let mut sorted = tempos.to_vec();
    sorted.sort_by_key(|t| t.tick);
    if sorted.first().is_none_or(|t| t.tick > 0) {
        sorted.insert(
            0,
            SmfTempo {
                tick: 0,
                bpm: 120.0,
                time_sig: (4, 4),
            },
        );
    }
    let mut last_sig = None;
    for t in &sorted {
        let delta = (t.tick - prev) as u32;
        prev = t.tick;
        write_tempo(&mut conductor, delta, t.bpm);
        if last_sig != Some(t.time_sig) {
            last_sig = Some(t.time_sig);
            write_time_sig(&mut conductor, 0, t.time_sig);
        }
    }
    end_track(&mut conductor);

    let mut out = header(1, 1 + tracks.len() as u16);
    push_chunk(&mut out, &conductor);
    for t in tracks {
        let mut chunk = Vec::new();
        // Track name meta: FF 03 len text.
        let name = t.name.as_bytes();
        write_vlq(&mut chunk, 0);
        chunk.extend_from_slice(&[0xFF, 0x03]);
        write_vlq(&mut chunk, name.len() as u32);
        chunk.extend_from_slice(name);
        write_events(&mut chunk, clip_events(t.clip), 0);
        end_track(&mut chunk);
        push_chunk(&mut out, &chunk);
    }
    out
}

fn header(format: u16, tracks: u16) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&tracks.to_be_bytes());
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    out
}

fn push_chunk(out: &mut Vec<u8>, track: &[u8]) {
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend_from_slice(track);
}

/// Tempo meta: FF 51 03 + microseconds per quarter.
fn write_tempo(track: &mut Vec<u8>, delta: u32, bpm: f64) {
    let usec_per_quarter = (60_000_000.0 / bpm.max(1.0)).round() as u32;
    write_vlq(track, delta);
    track.extend_from_slice(&[0xFF, 0x51, 0x03]);
    track.push(((usec_per_quarter >> 16) & 0xFF) as u8);
    track.push(((usec_per_quarter >> 8) & 0xFF) as u8);
    track.push((usec_per_quarter & 0xFF) as u8);
}

/// Time signature meta: FF 58 04 nn dd cc bb, the denominator as a power
/// of two, 24 clocks per click and 8 thirty-seconds per quarter.
fn write_time_sig(track: &mut Vec<u8>, delta: u32, (num, den): (u32, u32)) {
    write_vlq(track, delta);
    let power = den.max(1).trailing_zeros().min(7) as u8;
    track.extend_from_slice(&[0xFF, 0x58, 0x04, num.clamp(1, 255) as u8, power, 24, 8]);
}

fn end_track(track: &mut Vec<u8>) {
    write_vlq(track, 0);
    track.extend_from_slice(&[0xFF, 0x2F, 0x00]);
}

fn write_events(track: &mut Vec<u8>, events: Vec<(u64, u8, Vec<u8>)>, start: u64) {
    let mut prev_tick = start;
    for (tick, _order, message) in events {
        let delta = (tick - prev_tick) as u32;
        prev_tick = tick;
        write_vlq(track, delta);
        track.extend_from_slice(&message);
    }
}

/// A clip's notes and controller moves as (tick, order, message), sorted.
fn clip_events(clip: &MidiClip) -> Vec<(u64, u8, Vec<u8>)> {
    // (absolute_tick, order at that tick, raw MIDI message). Order keeps a
    // note-off before a controller move, and both before a note-on, so a
    // repeated pitch retriggers cleanly and a bend applies to the note it
    // was played on rather than the one before it.
    const OFF: u8 = 0;
    const CONTROL: u8 = 1;
    const ON: u8 = 2;
    let mut events: Vec<(u64, u8, Vec<u8>)> = Vec::new();
    for n in &clip.notes {
        if n.muted {
            continue;
        }
        let ch = n.channel & 0x0F;
        let pitch = n.pitch & 0x7F;
        events.push((
            n.start_tick,
            ON,
            vec![0x90 | ch, pitch, velocity_to_midi(n.velocity)],
        ));
        events.push((
            n.start_tick + n.duration_ticks.max(1),
            OFF,
            vec![0x80 | ch, pitch, 0],
        ));
    }
    for c in &clip.controls {
        let ch = c.channel & 0x0F;
        let message = match c.kind {
            MidiControlKind::Cc(cc) => vec![
                0xB0 | ch,
                cc & 0x7F,
                (c.value.clamp(0.0, 1.0) * 127.0).round() as u8,
            ],
            MidiControlKind::PitchBend => {
                // 14 bit, centre 8192, sent low byte first.
                let raw = ((c.value.clamp(-1.0, 1.0) * 8191.0).round() as i32 + 8192)
                    .clamp(0, 16383) as u16;
                vec![0xE0 | ch, (raw & 0x7F) as u8, ((raw >> 7) & 0x7F) as u8]
            }
            MidiControlKind::ChannelPressure => {
                vec![0xD0 | ch, (c.value.clamp(0.0, 1.0) * 127.0).round() as u8]
            }
        };
        events.push((c.tick, CONTROL, message));
    }
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MidiNote;

    fn clip_with(notes: Vec<MidiNote>) -> MidiClip {
        let mut c = MidiClip::new("c".into(), "c".into(), 1920);
        c.notes = notes;
        c
    }

    fn note(start: u64, dur: u64, pitch: u8, vel: f32) -> MidiNote {
        MidiNote {
            start_tick: start,
            duration_ticks: dur,
            pitch,
            velocity: vel,
            channel: 0,
            muted: false,
            ..Default::default()
        }
    }

    #[test]
    fn vlq_encodes_known_values() {
        let mut v = Vec::new();
        write_vlq(&mut v, 0);
        assert_eq!(v, vec![0x00]);
        v.clear();
        write_vlq(&mut v, 127);
        assert_eq!(v, vec![0x7F]);
        v.clear();
        write_vlq(&mut v, 128);
        assert_eq!(v, vec![0x81, 0x00]);
        v.clear();
        write_vlq(&mut v, 960);
        assert_eq!(v, vec![0x87, 0x40]);
    }

    #[test]
    fn smf_has_header_and_note_events() {
        let smf = write_smf(&clip_with(vec![note(0, 480, 60, 1.0)]), 120.0);
        assert_eq!(&smf[0..4], b"MThd");
        // format 0, 1 track, division 960.
        assert_eq!(&smf[8..10], &0u16.to_be_bytes());
        assert_eq!(&smf[10..12], &1u16.to_be_bytes());
        assert_eq!(&smf[12..14], &(PPQ as u16).to_be_bytes());
        // MTrk present.
        assert_eq!(&smf[14..18], b"MTrk");
        // Note-on (0x90) and note-off (0x80) bytes appear in the track.
        assert!(smf.contains(&0x90), "note-on status missing");
        assert!(smf.contains(&0x80), "note-off status missing");
        // Ends with end-of-track meta.
        assert_eq!(&smf[smf.len() - 3..], &[0xFF, 0x2F, 0x00]);
    }

    #[test]
    fn muted_notes_are_skipped() {
        let mut n = note(0, 480, 64, 1.0);
        n.muted = true;
        let smf = write_smf(&clip_with(vec![n]), 120.0);
        // No note-on present (only tempo + end-of-track).
        assert!(!smf.contains(&0x90), "muted note must not emit a note-on");
    }

    #[test]
    fn tempo_meta_encodes_bpm() {
        let smf = write_smf(&clip_with(vec![note(0, 100, 60, 0.8)]), 120.0);
        // 120 BPM = 500000 usec/quarter = 0x07A120.
        let idx = smf
            .windows(3)
            .position(|w| w == [0xFF, 0x51, 0x03])
            .unwrap();
        let us = ((smf[idx + 3] as u32) << 16) | ((smf[idx + 4] as u32) << 8) | smf[idx + 5] as u32;
        assert_eq!(us, 500_000);
    }

    #[test]
    fn controller_movements_are_written_as_midi_messages() {
        use crate::{MidiControlKind, MidiControlPoint};
        let mut clip = clip_with(vec![MidiNote {
            start_tick: 0,
            duration_ticks: 480,
            pitch: 60,
            velocity: 0.8,
            channel: 0,
            muted: false,
            ..Default::default()
        }]);
        clip.controls = vec![
            MidiControlPoint {
                tick: 0,
                channel: 0,
                kind: MidiControlKind::Cc(1),
                value: 1.0,
            },
            MidiControlPoint {
                tick: 240,
                channel: 0,
                kind: MidiControlKind::PitchBend,
                value: 1.0,
            },
        ];
        let bytes = write_smf(&clip, 120.0);
        // Mod wheel full up: B0 01 7F.
        assert!(
            bytes.windows(3).any(|w| w == [0xB0, 0x01, 0x7F]),
            "the exported file carries the mod wheel"
        );
        // Bend fully up: E0 7F 7F (16383 = max).
        assert!(
            bytes.windows(3).any(|w| w == [0xE0, 0x7F, 0x7F]),
            "the exported file carries the pitch bend"
        );
    }

    #[test]
    fn a_control_at_the_same_tick_comes_before_the_note() {
        use crate::{MidiControlKind, MidiControlPoint};
        let mut clip = clip_with(vec![MidiNote {
            start_tick: 480,
            duration_ticks: 480,
            pitch: 60,
            velocity: 0.8,
            channel: 0,
            muted: false,
            ..Default::default()
        }]);
        clip.controls = vec![MidiControlPoint {
            tick: 480,
            channel: 0,
            kind: MidiControlKind::Cc(11),
            value: 0.5,
        }];
        let bytes = write_smf(&clip, 120.0);
        let cc_at = bytes
            .windows(2)
            .position(|w| w == [0xB0, 0x0B])
            .expect("cc written");
        let note_at = bytes
            .windows(2)
            .position(|w| w == [0x90, 60])
            .expect("note written");
        assert!(cc_at < note_at, "expression is set before the note sounds");
    }

    /// The chunks of an SMF: (kind, body).
    fn chunks(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 8 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            out.push((&bytes[at..at + 4], &bytes[at + 8..at + 8 + len]));
            at += 8 + len;
        }
        out
    }

    #[test]
    fn a_song_is_a_conductor_track_and_one_named_track_each() {
        let lead = clip_with(vec![note(0, 480, 60, 0.8)]);
        let bass = clip_with(vec![note(960, 480, 36, 0.8)]);
        let bytes = write_song_smf(
            &[
                SmfTempo {
                    tick: 0,
                    bpm: 150.0,
                    time_sig: (4, 4),
                },
                SmfTempo {
                    tick: 7680,
                    bpm: 160.0,
                    time_sig: (7, 8),
                },
            ],
            &[
                SmfTrack {
                    name: "Lead",
                    clip: &lead,
                },
                SmfTrack {
                    name: "Bass",
                    clip: &bass,
                },
            ],
        );
        let c = chunks(&bytes);
        assert_eq!(c.len(), 4, "header, conductor and two tracks");
        assert_eq!(c[0].1[0..2], [0, 1], "format 1");
        assert_eq!(c[0].1[2..4], [0, 3], "three tracks");
        let conductor = c[1].1;
        let tempos = conductor
            .windows(3)
            .filter(|w| w == &[0xFF, 0x51, 0x03])
            .count();
        assert_eq!(tempos, 2, "both tempos");
        assert!(
            conductor.windows(5).any(|w| w == [0xFF, 0x58, 0x04, 7, 3]),
            "7/8 as 7 over two to the third"
        );
        assert!(c[2].1.windows(4).any(|w| w == b"Lead"));
        assert!(c[3].1.windows(4).any(|w| w == b"Bass"));
        assert!(
            c[3].1.windows(3).any(|w| w == [0x90, 36, 102]),
            "the bass note"
        );
    }

    #[test]
    fn the_single_clip_file_is_unchanged() {
        let clip = clip_with(vec![note(0, 480, 60, 1.0)]);
        let bytes = write_smf(&clip, 120.0);
        let c = chunks(&bytes);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].1[0..4], [0, 0, 0, 1], "format 0, one track");
    }
}
