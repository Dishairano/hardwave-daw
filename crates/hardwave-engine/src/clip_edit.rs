//! Edits to a clip that have to agree with how the engine plays it.
//!
//! Moving a clip's left edge used to be a move plus a resize, so the content
//! slid along with the edge: dragging the start of a vocal right by a beat
//! moved the whole vocal a beat later instead of cutting its first beat off.
//! Trimming the head has to move the start inside the source by exactly the
//! amount the engine would have played in that time, which depends on the
//! clip's stretch, pitch, warp markers and the tempo map, so it lives here
//! next to the engine rather than in the command.

use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, WarpMarker};
use hardwave_project::TempoMap;

use crate::track_node::{build_warp_segments, warp_source_pos};

/// Source frames a clip moves through per timeline sample, counted in the
/// units of its `source_start`.
///
/// Mirrors the engine's own choice: a stretched or pitched clip is played
/// from a baked copy that already carries the clip's pitch, so only the
/// track's varispeed moves it faster; anything else is a plain resample.
fn source_frames_per_sample(clip: &AudioClip, track_pitch_factor: f64) -> f64 {
    let stretch = if clip.stretch_ratio <= 0.01 {
        1.0
    } else {
        clip.stretch_ratio
    };
    let pitch = if crate::engine::bake_params(clip).is_some() {
        track_pitch_factor
    } else {
        2.0_f64.powf(clip.pitch_semitones / 12.0) * track_pitch_factor
    };
    pitch / stretch
}

/// Move a clip's left edge to `new_position_ticks` and keep its right edge
/// where it is, without moving what plays where on the timeline.
///
/// Dragging the edge right cuts the start off; dragging it left brings back
/// what an earlier trim cut, as far as the source reaches. Returns the
/// position the edge ended up at, which is earlier than asked for only when
/// the source has nothing more to give.
pub fn trim_clip_start(
    clip: &mut ClipPlacement,
    new_position_ticks: u64,
    tempo_map: &TempoMap,
    sample_rate: f64,
    track_pitch_factor: f64,
) -> Result<u64, String> {
    let old_pos = clip.position_ticks;
    let end = old_pos + clip.length_ticks;
    if new_position_ticks >= end {
        return Err(format!(
            "the start {new_position_ticks} would be at or after the end {end}"
        ));
    }
    let mut new_pos = new_position_ticks;
    match &mut clip.content {
        ClipContent::Audio(audio) => {
            new_pos = trim_audio(
                audio,
                old_pos,
                new_pos,
                tempo_map,
                sample_rate,
                track_pitch_factor,
            );
        }
        ClipContent::Midi(midi) => {
            let clip = &mut midi.clip;
            if new_pos > old_pos {
                let cut = new_pos - old_pos;
                clip.notes.retain(|n| n.start_tick + n.duration_ticks > cut);
                for n in &mut clip.notes {
                    if n.start_tick < cut {
                        // A note held across the new edge keeps sounding
                        // from it, the rest of its length.
                        n.duration_ticks = n.start_tick + n.duration_ticks - cut;
                        n.start_tick = 0;
                    } else {
                        n.start_tick -= cut;
                    }
                }
                // The value each controller had at the new edge carries on
                // from it; the moves before it are gone.
                let mut kept: Vec<hardwave_midi::MidiControlPoint> = Vec::new();
                for c in clip.controls.iter().filter(|c| c.tick < cut) {
                    match kept
                        .iter_mut()
                        .find(|k| k.channel == c.channel && k.kind == c.kind)
                    {
                        Some(k) if k.tick <= c.tick => *k = *c,
                        Some(_) => {}
                        None => kept.push(*c),
                    }
                }
                for k in &mut kept {
                    k.tick = 0;
                }
                clip.controls.retain(|c| c.tick >= cut);
                for c in &mut clip.controls {
                    c.tick -= cut;
                }
                kept.append(&mut clip.controls);
                clip.controls = kept;
                clip.length_ticks = clip.length_ticks.saturating_sub(cut).max(1);
            } else {
                let grow = old_pos - new_pos;
                for n in &mut clip.notes {
                    n.start_tick += grow;
                }
                for c in &mut clip.controls {
                    c.tick += grow;
                }
                clip.length_ticks += grow;
            }
        }
    }
    clip.position_ticks = new_pos;
    clip.length_ticks = end - new_pos;
    Ok(new_pos)
}

fn trim_audio(
    audio: &mut AudioClip,
    old_pos: u64,
    new_pos: u64,
    tempo_map: &TempoMap,
    sample_rate: f64,
    track_pitch_factor: f64,
) -> u64 {
    // A reversed clip plays its window backwards from where it ends, so
    // what plays at any moment is counted from the right edge, which does
    // not move.
    if audio.reversed {
        return new_pos;
    }
    let step = source_frames_per_sample(audio, track_pitch_factor);
    let old_s = tempo_map.tick_to_samples(old_pos, sample_rate);
    let new_s = tempo_map.tick_to_samples(new_pos, sample_rate);

    if !audio.warp_markers.is_empty() {
        return trim_warped(
            audio,
            old_pos,
            new_pos,
            old_s,
            new_s,
            tempo_map,
            sample_rate,
            step,
        );
    }

    if new_pos >= old_pos {
        let advance = ((new_s - old_s) as f64 * step).round() as u64;
        // Past the end of the window the clip already plays silence; the
        // start never needs to go further than that.
        let limit = audio.source_end.max(audio.source_start);
        audio.source_start = (audio.source_start + advance).min(limit);
        new_pos
    } else {
        let wanted = ((old_s - new_s) as f64 * step).round() as u64;
        if wanted <= audio.source_start {
            audio.source_start -= wanted;
            new_pos
        } else {
            // The source starts here: the edge stops where it does.
            let reach = (audio.source_start as f64 / step.max(f64::EPSILON)) as u64;
            audio.source_start = 0;
            tempo_map.samples_to_tick(old_s.saturating_sub(reach), sample_rate)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn trim_warped(
    audio: &mut AudioClip,
    old_pos: u64,
    new_pos: u64,
    old_s: u64,
    new_s: u64,
    tempo_map: &TempoMap,
    sample_rate: f64,
    step: f64,
) -> u64 {
    // The same anchors the engine builds: marker ticks are clip-relative,
    // measured through the tempo map from the clip's start.
    let anchors: Vec<(u64, f64)> = audio
        .warp_markers
        .iter()
        .map(|m| {
            let abs = tempo_map.tick_to_samples(old_pos + m.clip_tick, sample_rate);
            (abs.saturating_sub(old_s), m.source_sample as f64)
        })
        .collect();
    let segs = build_warp_segments(&anchors, step, audio.source_start as f64);
    if new_pos >= old_pos {
        let cut = new_pos - old_pos;
        let at_edge = warp_source_pos(&segs, new_s - old_s).max(0.0).round() as u64;
        audio.warp_markers.retain(|m| m.clip_tick > cut);
        for m in &mut audio.warp_markers {
            m.clip_tick -= cut;
        }
        // Pin the new edge to what played there, so the stretch between it
        // and the next marker stays exactly as it was.
        audio.warp_markers.insert(
            0,
            WarpMarker {
                clip_tick: 0,
                source_sample: at_edge,
            },
        );
        audio.source_start = at_edge;
        new_pos
    } else {
        let first_step = segs.first().map(|s| s.step).unwrap_or(step);
        let wanted = ((old_s - new_s) as f64 * first_step).round() as u64;
        let (start, edge) = if wanted <= audio.source_start {
            (audio.source_start - wanted, new_pos)
        } else {
            let reach = (audio.source_start as f64 / first_step.max(f64::EPSILON)) as u64;
            (
                0,
                tempo_map.samples_to_tick(old_s.saturating_sub(reach), sample_rate),
            )
        };
        let grow = old_pos - edge;
        for m in &mut audio.warp_markers {
            m.clip_tick += grow;
        }
        audio.source_start = start;
        edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_midi::{MidiClip, MidiControlKind, MidiControlPoint, MidiNote};
    use hardwave_project::clip::{FadeCurve, MidiClipRef};

    const RATE: f64 = 48_000.0;
    const BEAT: u64 = 960;

    /// One beat at the default tempo, in samples.
    fn beat_samples() -> u64 {
        TempoMap::default().tick_to_samples(BEAT, RATE)
    }

    /// A beat is not a whole number of samples at most tempos, so where the
    /// edge lands can round a frame either way.
    fn assert_near(got: u64, want: u64, what: &str) {
        assert!(got.abs_diff(want) <= 1, "{what}: got {got}, want {want}");
    }

    fn audio(source_start: u64, source_end: u64) -> AudioClip {
        AudioClip {
            id: "a".into(),
            name: "a".into(),
            source_path: "s".into(),
            source_hash: String::new(),
            source_start,
            source_end,
            gain_db: 0.0,
            fade_in_ticks: 0,
            fade_out_ticks: 0,
            muted: false,
            reversed: false,
            pitch_semitones: 0.0,
            stretch_ratio: 1.0,
            warp_markers: Vec::new(),
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            source_file: String::new(),
            treatment: Default::default(),
        }
    }

    fn placed(content: ClipContent, position: u64, length: u64) -> ClipPlacement {
        ClipPlacement {
            content,
            track_id: "t".into(),
            position_ticks: position,
            length_ticks: length,
            lane: 0,
        }
    }

    fn source_start(c: &ClipPlacement) -> u64 {
        match &c.content {
            ClipContent::Audio(a) => a.source_start,
            ClipContent::Midi(_) => panic!("audio clip expected"),
        }
    }

    fn trim(c: &mut ClipPlacement, to: u64) -> u64 {
        trim_clip_start(c, to, &TempoMap::default(), RATE, 1.0).expect("trims")
    }

    #[test]
    fn dragging_the_left_edge_right_cuts_the_start_off_and_keeps_the_rest_in_place() {
        let b = beat_samples();
        let mut c = placed(ClipContent::Audio(audio(0, 8 * b)), 4 * BEAT, 8 * BEAT);
        assert_eq!(trim(&mut c, 5 * BEAT), 5 * BEAT);
        assert_eq!(c.position_ticks, 5 * BEAT);
        assert_eq!(c.length_ticks, 7 * BEAT, "the right edge stays put");
        assert_eq!(
            source_start(&c),
            b,
            "the second beat of the sample now plays where it always did"
        );
    }

    #[test]
    fn dragging_it_back_left_brings_the_cut_part_back() {
        let b = beat_samples();
        let mut c = placed(ClipContent::Audio(audio(2 * b, 8 * b)), 4 * BEAT, 6 * BEAT);
        assert_eq!(trim(&mut c, 3 * BEAT), 3 * BEAT);
        assert_near(source_start(&c), b, "one beat of the cut part is back");
        assert_eq!(c.length_ticks, 7 * BEAT);
    }

    #[test]
    fn the_edge_stops_where_the_sample_starts() {
        let b = beat_samples();
        let mut c = placed(ClipContent::Audio(audio(b, 8 * b)), 4 * BEAT, 7 * BEAT);
        // Asked for three beats earlier, but only one beat of sample is left.
        assert_eq!(trim(&mut c, BEAT), 3 * BEAT);
        assert_eq!(source_start(&c), 0);
        assert_eq!(c.position_ticks, 3 * BEAT);
        assert_eq!(c.length_ticks, 8 * BEAT);
    }

    #[test]
    fn a_stretched_clip_moves_through_its_source_at_its_own_speed() {
        let b = beat_samples();
        // Played twice as long: one beat on the timeline is half a beat of source.
        let mut a = audio(0, 8 * b);
        a.stretch_ratio = 2.0;
        let mut c = placed(ClipContent::Audio(a), 0, 16 * BEAT);
        trim(&mut c, 2 * BEAT);
        assert_near(
            source_start(&c),
            b,
            "two beats of timeline are one of source",
        );
    }

    #[test]
    fn a_track_pitched_up_an_octave_moves_through_its_source_twice_as_fast() {
        let b = beat_samples();
        let mut c = placed(ClipContent::Audio(audio(0, 8 * b)), 0, 4 * BEAT);
        trim_clip_start(&mut c, BEAT, &TempoMap::default(), RATE, 2.0).unwrap();
        assert_eq!(source_start(&c), 2 * b);
    }

    #[test]
    fn a_reversed_clip_keeps_its_window_because_it_plays_from_the_right_edge() {
        let b = beat_samples();
        let mut a = audio(0, 4 * b);
        a.reversed = true;
        let mut c = placed(ClipContent::Audio(a), 0, 4 * BEAT);
        trim(&mut c, BEAT);
        assert_eq!(source_start(&c), 0);
        assert_eq!(c.length_ticks, 3 * BEAT);
    }

    #[test]
    fn a_warped_clip_keeps_its_stretch_after_the_new_edge() {
        let b = beat_samples();
        let mut a = audio(0, 8 * b);
        // Beat two of the source lands on beat four of the clip: the first
        // four beats play two beats of source, at half speed.
        a.warp_markers = vec![WarpMarker {
            clip_tick: 4 * BEAT,
            source_sample: 2 * b,
        }];
        let mut c = placed(ClipContent::Audio(a), 0, 8 * BEAT);
        trim(&mut c, 2 * BEAT);
        let ClipContent::Audio(a) = &c.content else {
            unreachable!()
        };
        assert_eq!(
            a.source_start, b,
            "half way to the marker: one beat of source"
        );
        assert_eq!(
            a.warp_markers,
            vec![
                WarpMarker {
                    clip_tick: 0,
                    source_sample: b
                },
                WarpMarker {
                    clip_tick: 2 * BEAT,
                    source_sample: 2 * b
                },
            ],
            "the marker keeps its place on the timeline"
        );
    }

    #[test]
    fn a_midi_clip_loses_the_notes_before_the_edge_and_keeps_the_rest_where_they_sound() {
        let mut m = MidiClip::new("m".into(), "m".into(), 8 * BEAT);
        let note = |start: u64, len: u64, pitch: u8| MidiNote {
            start_tick: start,
            duration_ticks: len,
            pitch,
            ..Default::default()
        };
        m.notes = vec![
            note(0, BEAT, 60),
            note(BEAT, 2 * BEAT, 62),
            note(4 * BEAT, BEAT, 64),
        ];
        m.controls = vec![
            MidiControlPoint {
                tick: 0,
                channel: 0,
                kind: MidiControlKind::Cc(1),
                value: 0.2,
            },
            MidiControlPoint {
                tick: BEAT,
                channel: 0,
                kind: MidiControlKind::Cc(1),
                value: 0.7,
            },
            MidiControlPoint {
                tick: 5 * BEAT,
                channel: 0,
                kind: MidiControlKind::Cc(1),
                value: 0.1,
            },
        ];
        let mut c = placed(
            ClipContent::Midi(MidiClipRef {
                id: "m".into(),
                clip: m,
            }),
            4 * BEAT,
            8 * BEAT,
        );
        trim(&mut c, 6 * BEAT);
        let ClipContent::Midi(r) = &c.content else {
            unreachable!()
        };
        let notes: Vec<(u64, u64, u8)> = r
            .clip
            .notes
            .iter()
            .map(|n| (n.start_tick, n.duration_ticks, n.pitch))
            .collect();
        // Cut two beats: the first note is gone, the held one sounds its
        // last beat from the edge, the third moves two beats earlier in the
        // clip so it still sounds at beat eight of the song.
        assert_eq!(notes, vec![(0, BEAT, 62), (2 * BEAT, BEAT, 64)]);
        let controls: Vec<(u64, f32)> = r.clip.controls.iter().map(|c| (c.tick, c.value)).collect();
        assert_eq!(
            controls,
            vec![(0, 0.7), (3 * BEAT, 0.1)],
            "the mod wheel stays where it was"
        );
        assert_eq!((c.position_ticks, c.length_ticks), (6 * BEAT, 6 * BEAT));
    }

    #[test]
    fn a_midi_clip_dragged_left_makes_room_before_its_notes() {
        let mut m = MidiClip::new("m".into(), "m".into(), 4 * BEAT);
        m.notes = vec![MidiNote {
            start_tick: 0,
            duration_ticks: BEAT,
            ..Default::default()
        }];
        let mut c = placed(
            ClipContent::Midi(MidiClipRef {
                id: "m".into(),
                clip: m,
            }),
            4 * BEAT,
            4 * BEAT,
        );
        trim(&mut c, 2 * BEAT);
        let ClipContent::Midi(r) = &c.content else {
            unreachable!()
        };
        assert_eq!(
            r.clip.notes[0].start_tick,
            2 * BEAT,
            "the note still sounds at beat four"
        );
        assert_eq!((c.position_ticks, c.length_ticks), (2 * BEAT, 6 * BEAT));
    }

    #[test]
    fn the_edge_cannot_pass_the_end() {
        let mut c = placed(ClipContent::Audio(audio(0, 100)), 0, BEAT);
        assert!(trim_clip_start(&mut c, BEAT, &TempoMap::default(), RATE, 1.0).is_err());
    }
}
