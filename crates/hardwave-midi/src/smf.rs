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

/// Render `clip` to format-0 Standard MIDI File bytes at `bpm`. The SMF
/// division is the project PPQ, so note ticks transfer 1:1.
pub fn write_smf(clip: &MidiClip, bpm: f64) -> Vec<u8> {
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

    let mut track = Vec::new();
    // Tempo meta at delta 0: FF 51 03 + microseconds-per-quarter.
    let usec_per_quarter = (60_000_000.0 / bpm.max(1.0)).round() as u32;
    write_vlq(&mut track, 0);
    track.extend_from_slice(&[0xFF, 0x51, 0x03]);
    track.push(((usec_per_quarter >> 16) & 0xFF) as u8);
    track.push(((usec_per_quarter >> 8) & 0xFF) as u8);
    track.push((usec_per_quarter & 0xFF) as u8);

    let mut prev_tick = 0u64;
    for (tick, _order, message) in events {
        let delta = (tick - prev_tick) as u32;
        prev_tick = tick;
        write_vlq(&mut track, delta);
        track.extend_from_slice(&message);
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
}
