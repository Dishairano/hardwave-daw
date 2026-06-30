//! Standard MIDI File (SMF, `.mid`) export. Serializes a [`MidiClip`]'s
//! notes to a format-0 SMF so an idea can be carried into another DAW,
//! plug-in, or notation tool. Tempo is embedded as a meta event; the
//! clip's tick resolution maps straight onto the SMF division (PPQ).

use crate::{MidiClip, PPQ};

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

/// Render `clip` to format-0 Standard MIDI File bytes at `bpm`. The SMF
/// division is the project PPQ, so note ticks transfer 1:1.
pub fn write_smf(clip: &MidiClip, bpm: f64) -> Vec<u8> {
    // (absolute_tick, is_note_on, channel, note, velocity)
    let mut events: Vec<(u64, bool, u8, u8, u8)> = Vec::new();
    for n in &clip.notes {
        if n.muted {
            continue;
        }
        let vel = velocity_to_midi(n.velocity);
        events.push((n.start_tick, true, n.channel & 0x0F, n.pitch & 0x7F, vel));
        events.push((
            n.start_tick + n.duration_ticks.max(1),
            false,
            n.channel & 0x0F,
            n.pitch & 0x7F,
            0,
        ));
    }
    // Sort by tick; at the same tick, note-offs precede note-ons so a
    // repeated pitch retriggers cleanly.
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut track = Vec::new();
    // Tempo meta at delta 0: FF 51 03 + microseconds-per-quarter.
    let usec_per_quarter = (60_000_000.0 / bpm.max(1.0)).round() as u32;
    write_vlq(&mut track, 0);
    track.extend_from_slice(&[0xFF, 0x51, 0x03]);
    track.push(((usec_per_quarter >> 16) & 0xFF) as u8);
    track.push(((usec_per_quarter >> 8) & 0xFF) as u8);
    track.push((usec_per_quarter & 0xFF) as u8);

    let mut prev_tick = 0u64;
    for (tick, is_on, ch, note, vel) in events {
        let delta = (tick - prev_tick) as u32;
        prev_tick = tick;
        write_vlq(&mut track, delta);
        if is_on {
            track.push(0x90 | ch);
            track.push(note);
            track.push(vel);
        } else {
            track.push(0x80 | ch);
            track.push(note);
            track.push(0);
        }
    }
    // End of track.
    write_vlq(&mut track, 0);
    track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    let mut out = Vec::with_capacity(22 + track.len());
    // MThd: format 0, 1 track, division = PPQ.
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // format 0
    out.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    // MTrk.
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend_from_slice(&track);
    out
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
}
