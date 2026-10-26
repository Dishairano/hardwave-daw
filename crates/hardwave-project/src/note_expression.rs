//! A part's per-note pan and detune turned into automation: the pan as the
//! track's pan lane, the detune as pitch bend in the clip.
//!
//! Notes carry their own pan and fine tuning, which the built-in synth and
//! plug-ins with note expression follow. A plug-in without note expression,
//! or hardware on a MIDI out, only knows the track's pan and pitch bend,
//! and a part meant for one needs its expression as those. A lane holds one
//! value at a time, so this is for a line (one note at a time, or notes
//! sounding together that agree); a chord whose notes differ is refused.

use crate::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use crate::clip::ClipContent;
use crate::track::Track;
use hardwave_midi::{MidiControlKind, MidiControlPoint};

/// Pitch bend reaches this far, in cents: the usual two semitones, which a
/// plug-in uses unless it is set otherwise.
pub const BEND_RANGE_CENTS: f32 = 200.0;

/// What a conversion did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Converted {
    /// Points put on the track's pan lane.
    pub pan_points: usize,
    /// Pitch bend moves put in the clip.
    pub bends: usize,
}

/// Move the per-note pan and detune of the MIDI clip `clip_id` on `track`
/// into automation, leaving the notes centred and in tune. The pan lane
/// holds each note's pan from its start; around the clip it goes back to
/// what it was. The bend moves to each note's detune at its start and back
/// to zero where nothing sounds. Muted notes are left as they are.
pub fn expression_to_automation(track: &mut Track, clip_id: &str) -> Result<Converted, String> {
    let placement = track
        .clips
        .iter_mut()
        .find(|p| matches!(&p.content, ClipContent::Midi(m) if m.id == clip_id))
        .ok_or_else(|| format!("no MIDI clip {clip_id} on this track"))?;
    let (at, length) = (placement.position_ticks, placement.length_ticks);
    let ClipContent::Midi(midi) = &mut placement.content else {
        unreachable!("found as MIDI above")
    };
    let clip = &mut midi.clip;
    let mut order: Vec<usize> = (0..clip.notes.len())
        .filter(|&i| !clip.notes[i].muted)
        .collect();
    order.sort_by_key(|&i| clip.notes[i].start_tick);

    // Notes that sound together must agree, or one lane cannot hold them.
    for (k, &i) in order.iter().enumerate() {
        let a = &clip.notes[i];
        let a_end = a.start_tick + a.duration_ticks;
        for &j in &order[k + 1..] {
            let b = &clip.notes[j];
            if b.start_tick >= a_end {
                break;
            }
            if a.pan != b.pan || a.fine_cents != b.fine_cents {
                let bar = (at + b.start_tick) / (hardwave_midi::PPQ * 4) + 1;
                return Err(format!(
                    "notes sounding together at bar {bar} have a different pan or tuning, and one lane holds one value"
                ));
            }
        }
    }

    let mut done = Converted::default();
    let panned = order.iter().any(|&i| clip.notes[i].pan != 0.0);
    let detuned = order.iter().any(|&i| clip.notes[i].fine_cents != 0.0);

    if detuned {
        clip.controls
            .retain(|c| c.kind != MidiControlKind::PitchBend);
        for (k, &i) in order.iter().enumerate() {
            let n = &clip.notes[i];
            clip.controls.push(MidiControlPoint {
                tick: n.start_tick,
                channel: n.channel,
                kind: MidiControlKind::PitchBend,
                value: (n.fine_cents / BEND_RANGE_CENTS).clamp(-1.0, 1.0),
            });
            done.bends += 1;
            // Back to zero after it, unless the next note takes over there.
            let end = n.start_tick + n.duration_ticks;
            let next = order.get(k + 1).map(|&j| clip.notes[j].start_tick);
            if n.fine_cents != 0.0 && next.is_none_or(|s| s > end) {
                clip.controls.push(MidiControlPoint {
                    tick: end,
                    channel: n.channel,
                    kind: MidiControlKind::PitchBend,
                    value: 0.0,
                });
                done.bends += 1;
            }
        }
        clip.controls.sort_by_key(|c| c.tick);
        for &i in &order {
            clip.notes[i].fine_cents = 0.0;
        }
    }

    if panned {
        let pans: Vec<(u64, f32)> = order
            .iter()
            .map(|&i| (clip.notes[i].start_tick, clip.notes[i].pan))
            .collect();
        for &i in &order {
            clip.notes[i].pan = 0.0;
        }
        let own = ((track.pan + 1.0) / 2.0).clamp(0.0, 1.0);
        let lane_index = match track
            .automation_lanes
            .iter()
            .position(|l| matches!(l.target, AutomationTarget::TrackPan))
        {
            Some(i) => i,
            None => {
                track.automation_lanes.push(AutomationLane {
                    id: uuid::Uuid::new_v4().to_string(),
                    target: AutomationTarget::TrackPan,
                    points: Vec::new(),
                    visible: true,
                });
                track.automation_lanes.len() - 1
            }
        };
        let lane = &mut track.automation_lanes[lane_index];
        let end = at + length;
        // What the lane said around the clip, kept around it: the track's own
        // pan for a new lane.
        let (before, after) = if lane.points.is_empty() {
            (own, own)
        } else {
            (lane.value_at(at.saturating_sub(1)), lane.value_at(end))
        };
        lane.points.retain(|p| p.tick < at || p.tick > end);
        let point = |tick: u64, value: f64| AutomationPoint {
            tick,
            value,
            curve: CurveMode::Step,
            tension: 0.0,
        };
        if at > 0 {
            lane.points.push(point(at - 1, before));
        }
        for (tick, pan) in pans {
            lane.points
                .push(point(at + tick, ((pan as f64 + 1.0) / 2.0).clamp(0.0, 1.0)));
            done.pan_points += 1;
        }
        lane.points.push(point(end, after));
        lane.points.sort_by_key(|p| p.tick);
        lane.points.dedup_by_key(|p| p.tick);
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{ClipPlacement, MidiClipRef};
    use hardwave_midi::{MidiClip, MidiNote};

    fn track_with(notes: Vec<MidiNote>) -> Track {
        let mut track = Track::new_midi("t".into(), "Lead".into());
        let mut clip = MidiClip::new("c".into(), "c".into(), 3840);
        clip.notes = notes;
        track.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "c".into(),
                clip,
            }),
            track_id: track.id.clone(),
            position_ticks: 3840,
            length_ticks: 3840,
            lane: 0,
            color: None,
        });
        track
    }

    fn note(start: u64, pan: f32, fine: f32) -> MidiNote {
        MidiNote {
            start_tick: start,
            duration_ticks: 480,
            pitch: 60,
            velocity: 0.8,
            pan,
            fine_cents: fine,
            ..Default::default()
        }
    }

    fn notes(track: &Track) -> &Vec<MidiNote> {
        match &track.clips[0].content {
            ClipContent::Midi(m) => &m.clip.notes,
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_line_s_pan_becomes_the_track_s_pan_lane() {
        let mut track = track_with(vec![note(0, -1.0, 0.0), note(960, 1.0, 0.0)]);
        let done = expression_to_automation(&mut track, "c").expect("a line converts");
        assert_eq!(done.pan_points, 2);
        let lane = &track.automation_lanes[0];
        assert!(matches!(lane.target, AutomationTarget::TrackPan));
        // Each note's pan from its start, the track's own (centre) around it.
        assert_eq!(lane.value_at(3840), 0.0);
        assert_eq!(lane.value_at(3840 + 960), 1.0);
        assert_eq!(lane.value_at(100), 0.5);
        assert_eq!(lane.value_at(3840 * 2 + 10), 0.5);
        assert!(
            notes(&track).iter().all(|n| n.pan == 0.0),
            "the notes are centred"
        );
    }

    #[test]
    fn detune_becomes_pitch_bend_that_returns_to_zero() {
        let mut track = track_with(vec![note(0, 0.0, 50.0), note(1920, 0.0, 0.0)]);
        let done = expression_to_automation(&mut track, "c").expect("a line converts");
        let ClipContent::Midi(m) = &track.clips[0].content else {
            unreachable!()
        };
        let bends: Vec<(u64, f32)> = m.clip.controls.iter().map(|c| (c.tick, c.value)).collect();
        assert_eq!(bends, vec![(0, 0.25), (480, 0.0), (1920, 0.0)]);
        assert_eq!(done.bends, 3);
        assert!(notes(&track).iter().all(|n| n.fine_cents == 0.0));
    }

    #[test]
    fn a_chord_whose_notes_differ_is_refused_and_left_alone() {
        let mut track = track_with(vec![note(0, -0.5, 0.0), note(0, 0.5, 0.0)]);
        let err = expression_to_automation(&mut track, "c").expect_err("one lane, two pans");
        assert!(err.contains("bar 2"), "{err}");
        assert!(track.automation_lanes.is_empty());
        assert_eq!(notes(&track)[0].pan, -0.5);
    }
}
