//! Playlist rows as the engine hears them: a row's switch (FL's track
//! mute) and solo silence what sits on the row, wherever it plays through;
//! an automation clip moves the knob of the track it targets.

mod common;

use common::*;
use hardwave_engine::DawEngine;
use hardwave_project::automation::{AutomationPoint, AutomationTarget, CurveMode};
use hardwave_project::automation_clip::AutomationClip;
use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
use hardwave_project::PlaylistRow;

const RATE: u32 = 48_000;
/// One beat at the default 140 BPM, in samples.
const BEAT: usize = (RATE as usize * 60) / 140;

fn render(engine: &DawEngine, beats: usize) -> Vec<f32> {
    let mut out = Vec::new();
    engine
        .render_offline(RATE, (BEAT * beats) as u64, |block| {
            out.extend_from_slice(block);
            true
        })
        .unwrap();
    out
}

/// The loudest sample between two beats (interleaved stereo), leaving the
/// edges out where fades and declicks sit.
fn peak(out: &[f32], from_beat: usize, to_beat: usize) -> f32 {
    let margin = BEAT / 8;
    out[(from_beat * BEAT + margin) * 2..(to_beat * BEAT - margin) * 2]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

fn on_row(mut clip: ClipPlacement, row: Option<u32>) -> ClipPlacement {
    clip.row = row;
    clip
}

fn tone_clip(track: &str, frames: u64, beat: u64, row: Option<u32>) -> ClipPlacement {
    on_row(
        ClipPlacement {
            content: ClipContent::Audio(make_audio_clip("tone", frames)),
            track_id: track.into(),
            position_ticks: beat * hardwave_midi::PPQ,
            length_ticks: hardwave_midi::PPQ,
            lane: 0,
            color: None,
            row: None,
        },
        row,
    )
}

/// One audio track with a clip on rows 0, 1 and on no row, beats 0, 2, 4.
fn three_clips() -> (DawEngine, String) {
    let engine = DawEngine::new();
    let tone = make_sine_buffer(RATE, 2.0, 440.0, 0.5);
    let frames = tone.num_frames as u64;
    engine.audio_pool.insert("tone".into(), tone);
    let id = {
        let mut p = engine.project.lock();
        let id = p.add_audio_track("Drums".into());
        let t = p.track_mut(&id).unwrap();
        t.clips.push(tone_clip(&id, frames, 0, Some(0)));
        t.clips.push(tone_clip(&id, frames, 2, Some(1)));
        t.clips.push(tone_clip(&id, frames, 4, None));
        p.rows = vec![PlaylistRow::default(); 3];
        id
    };
    (engine, id)
}

/// A clip of `source` from `beat`, `beats` long, on `row`.
fn clip_of(
    track: &str,
    source: &str,
    frames: u64,
    beat: u64,
    beats: u64,
    row: Option<u32>,
) -> ClipPlacement {
    let mut c = tone_clip(track, frames, beat, row);
    let mut audio = make_audio_clip(source, frames);
    audio.id = format!("{source}-{beat}");
    c.content = ClipContent::Audio(audio);
    c.length_ticks = beats * hardwave_midi::PPQ;
    c
}

fn set_muted(engine: &DawEngine, id: &str, muted: bool) {
    let mut p = engine.project.lock();
    for t in p.tracks.iter_mut() {
        for c in t.clips.iter_mut() {
            if let ClipContent::Audio(a) = &mut c.content {
                if a.id == id {
                    a.muted = muted;
                }
            }
        }
    }
}

/// The worst difference between two renders, leaving out a few
/// milliseconds either side of each of `cuts` (in beats), where a clip's
/// own declick fades sit.
fn worst_apart_from(a: &[f32], b: &[f32], cuts: &[usize]) -> f32 {
    let margin = RATE as usize / 200;
    a.iter()
        .zip(b)
        .enumerate()
        .filter(|(i, _)| {
            let frame = i / 2;
            cuts.iter()
                .all(|&c| frame + margin < c * BEAT || frame > c * BEAT + margin)
        })
        .fold(0.0f32, |m, (_, (x, y))| m.max((x - y).abs()))
}

/// A beat from beat 0 to 6 and a clap from beat 2 to 3 on one track.
fn beat_and_clap(beat_row: Option<u32>, clap_row: Option<u32>) -> DawEngine {
    let engine = DawEngine::new();
    let beat = make_sine_buffer(RATE, 3.0, 110.0, 0.25);
    let beat_frames = beat.num_frames as u64;
    engine.audio_pool.insert("beat".into(), beat);
    let clap = make_sine_buffer(RATE, 1.0, 880.0, 0.25);
    let clap_frames = clap.num_frames as u64;
    engine.audio_pool.insert("clap".into(), clap);
    {
        let mut p = engine.project.lock();
        let id = p.add_audio_track("Drums".into());
        let t = p.track_mut(&id).unwrap();
        t.clips
            .push(clip_of(&id, "beat", beat_frames, 0, 6, beat_row));
        t.clips
            .push(clip_of(&id, "clap", clap_frames, 2, 1, clap_row));
        p.rows = vec![PlaylistRow::default(); 2];
    }
    engine
}

/// A clip that starts over a longer one on the same track plays on top of
/// it, and both play at full level, on one row, on two rows or on none:
/// the song is the sum of the two clips heard alone. Every overlap used to
/// become a crossfade as long as the overlap, so a clap dropped on bar 3 of
/// a beat faded the beat away from there to its end, 20 dB down at the end.
#[test]
fn overlapping_clips_on_one_track_both_play_at_full_level() {
    for (beat_row, clap_row) in [(Some(0), Some(0)), (Some(0), Some(1)), (None, None)] {
        let engine = beat_and_clap(beat_row, clap_row);
        let both = render(&engine, 6);
        set_muted(&engine, "clap-2", true);
        let beat_alone = render(&engine, 6);
        set_muted(&engine, "clap-2", false);
        set_muted(&engine, "beat-0", true);
        let clap_alone = render(&engine, 6);
        let sum: Vec<f32> = beat_alone
            .iter()
            .zip(&clap_alone)
            .map(|(a, b)| a + b)
            .collect();
        let worst = worst_apart_from(&both, &sum, &[]);
        let (with_clap, beat_only) = (peak(&both, 2, 3), peak(&beat_alone, 2, 3));
        assert!(
            with_clap > beat_only + 0.05,
            "rows {beat_row:?}/{clap_row:?}: the clap plays on top of the beat: {with_clap} against {beat_only}"
        );
        assert!(
            worst < 1e-5,
            "rows {beat_row:?}/{clap_row:?}: both clips play as they do alone: off by {worst}"
        );
    }
}

/// A clip split in two plays as it did before the cut, at full level on
/// both sides: the halves sit end to end, with nothing between them but
/// the left half's own declick at the cut.
#[test]
fn a_split_clip_plays_as_it_did_whole() {
    let engine = DawEngine::new();
    let tone = make_sine_buffer(RATE, 3.0, 220.0, 0.25);
    let frames = tone.num_frames as u64;
    engine.audio_pool.insert("tone".into(), tone);
    {
        let mut p = engine.project.lock();
        let id = p.add_audio_track("Vox".into());
        p.track_mut(&id)
            .unwrap()
            .clips
            .push(clip_of(&id, "tone", frames, 0, 6, None));
    }
    let whole = render(&engine, 6);
    {
        let mut p = engine.project.lock();
        let map = p.tempo_map.clone();
        let t = p.tracks.iter_mut().find(|t| !t.clips.is_empty()).unwrap();
        let right = hardwave_engine::clip_edit::split_clip(
            &mut t.clips[0],
            3 * hardwave_midi::PPQ,
            "right",
            &map,
            RATE as f64,
            1.0,
        )
        .unwrap();
        t.clips.push(right);
    }
    let split = render(&engine, 6);
    assert!(peak(&split, 2, 3) > 0.15 && peak(&split, 3, 4) > 0.15);
    let worst = worst_apart_from(&whole, &split, &[3]);
    assert!(
        worst < 1e-5,
        "the split clip plays as it did: off by {worst}"
    );
}

/// A comp, two takes over the same bars on their own lanes: the chosen
/// take's piece plays in the range, the other take's pieces outside it,
/// each at full level, cut end to end as the comp command cuts them.
#[test]
fn a_comp_plays_each_take_where_it_was_chosen_at_full_level() {
    let engine = DawEngine::new();
    let a = make_sine_buffer(RATE, 3.0, 110.0, 0.25);
    let b = make_sine_buffer(RATE, 3.0, 330.0, 0.25);
    let frames = a.num_frames as u64;
    engine.audio_pool.insert("take1".into(), a);
    engine.audio_pool.insert("take2".into(), b);
    {
        let mut p = engine.project.lock();
        let id = p.add_audio_track("Vox".into());
        let t = p.track_mut(&id).unwrap();
        t.clips.push(clip_of(&id, "take1", frames, 0, 6, None));
        let mut second = clip_of(&id, "take2", frames, 0, 6, None);
        second.lane = 1;
        t.clips.push(second);
    }
    // Each take alone, for comparison.
    set_muted(&engine, "take2-0", true);
    let take1 = render(&engine, 6);
    set_muted(&engine, "take2-0", false);
    set_muted(&engine, "take1-0", true);
    let take2 = render(&engine, 6);
    set_muted(&engine, "take1-0", false);

    // Take 2 over beats 2 to 4: both takes cut at 2 and 4, take 1's middle
    // and take 2's outer pieces muted.
    {
        let mut p = engine.project.lock();
        let map = p.tempo_map.clone();
        let t = p.tracks.iter_mut().find(|t| !t.clips.is_empty()).unwrap();
        for (index, take) in [(0usize, "take1"), (1, "take2")] {
            let mut mid = hardwave_engine::clip_edit::split_clip(
                &mut t.clips[index],
                2 * hardwave_midi::PPQ,
                &format!("{take}-2"),
                &map,
                RATE as f64,
                1.0,
            )
            .unwrap();
            let end = hardwave_engine::clip_edit::split_clip(
                &mut mid,
                4 * hardwave_midi::PPQ,
                &format!("{take}-4"),
                &map,
                RATE as f64,
                1.0,
            )
            .unwrap();
            t.clips.push(mid);
            t.clips.push(end);
        }
    }
    for id in ["take1-2", "take2-0", "take2-4"] {
        set_muted(&engine, id, true);
    }
    let comp = render(&engine, 6);
    let expected: Vec<f32> = take1
        .iter()
        .zip(&take2)
        .enumerate()
        .map(|(i, (x, y))| {
            let frame = i / 2;
            if (2 * BEAT..4 * BEAT).contains(&frame) {
                *y
            } else {
                *x
            }
        })
        .collect();
    assert!(peak(&comp, 2, 4) > 0.15 && peak(&comp, 0, 2) > 0.15 && peak(&comp, 4, 6) > 0.15);
    let worst = worst_apart_from(&comp, &expected, &[2, 4]);
    assert!(
        worst < 1e-5,
        "the comp plays each take as it is: off by {worst}"
    );
}

#[test]
fn a_muted_row_silences_its_clips_and_no_other() {
    let (engine, _) = three_clips();
    let all = render(&engine, 6);
    assert!(peak(&all, 0, 1) > 0.3 && peak(&all, 2, 3) > 0.3 && peak(&all, 4, 5) > 0.3);
    engine.project.lock().rows[1].muted = true;
    let out = render(&engine, 6);
    assert!(peak(&out, 0, 1) > 0.3, "row 0 still plays");
    assert!(peak(&out, 2, 3) < 1e-6, "the muted row's clip is silent");
    assert!(
        peak(&out, 4, 5) > 0.3,
        "a clip with no row is never silenced"
    );
}

#[test]
fn a_soloed_row_silences_every_other_row_including_clips_drawn_there() {
    let (engine, _) = three_clips();
    {
        let mut p = engine.project.lock();
        // A clip with no stored row is drawn on the row most of its channel's
        // clips use (the lowest of equals), here row 0, and obeys that row.
        let shown = hardwave_project::layout::shown_rows(&p.tracks);
        let drums = shown.last().expect("the drums track");
        assert_eq!(drums.clips[2], 0, "the clip with no row is drawn on row 0");
        p.rows[1].soloed = true;
    }
    let out = render(&engine, 6);
    assert!(peak(&out, 0, 1) < 1e-6, "row 0 is not soloed");
    assert!(peak(&out, 2, 3) > 0.3, "the soloed row plays");
    assert!(
        peak(&out, 4, 5) < 1e-6,
        "a clip with no row is silenced with the row it is drawn on"
    );
}

#[test]
fn notes_on_a_muted_row_are_not_played() {
    let engine = DawEngine::new();
    {
        let mut p = engine.project.lock();
        let id = p.add_midi_track("Lead".into());
        let t = p.track_mut(&id).unwrap();
        for (beat, row) in [(0u64, 0u32), (2, 1)] {
            let mut clip = hardwave_midi::MidiClip::new(format!("c{beat}"), "c".into(), 960);
            clip.notes.push(hardwave_midi::MidiNote {
                start_tick: 0,
                duration_ticks: 900,
                pitch: 69,
                velocity: 1.0,
                ..Default::default()
            });
            t.clips.push(ClipPlacement {
                content: ClipContent::Midi(MidiClipRef {
                    id: format!("c{beat}"),
                    clip,
                }),
                track_id: id.clone(),
                position_ticks: beat * hardwave_midi::PPQ,
                length_ticks: 960,
                lane: 0,
                color: None,
                row: Some(row),
            });
        }
        p.rows = vec![PlaylistRow::default(); 2];
        p.rows[0].muted = true;
    }
    let out = render(&engine, 4);
    assert!(peak(&out, 0, 1) < 1e-6, "the note on the muted row");
    assert!(peak(&out, 2, 3) > 0.01, "the note on the row that is on");
}

/// Volume automation that pulls the track to silence from beat 2.
fn duck(id: &str) -> AutomationClip {
    let mut clip = AutomationClip::new(
        format!("duck-{id}"),
        AutomationTarget::TrackVolume,
        2 * hardwave_midi::PPQ,
        4 * hardwave_midi::PPQ,
    );
    clip.lane.points = vec![
        AutomationPoint {
            tick: 0,
            value: 0.0,
            curve: CurveMode::Step,
            tension: 0.0,
        },
        AutomationPoint {
            tick: 4 * hardwave_midi::PPQ,
            value: 0.0,
            curve: CurveMode::Step,
            tension: 0.0,
        },
    ];
    clip
}

/// A tone on beats 0 to 6 on one track.
fn held_tone() -> (DawEngine, String) {
    let engine = DawEngine::new();
    let tone = make_sine_buffer(RATE, 4.0, 440.0, 0.5);
    let frames = tone.num_frames as u64;
    engine.audio_pool.insert("tone".into(), tone);
    let id = {
        let mut p = engine.project.lock();
        let id = p.add_audio_track("Pad".into());
        let mut clip = tone_clip(&id, frames, 0, None);
        clip.length_ticks = 6 * hardwave_midi::PPQ;
        p.track_mut(&id).unwrap().clips.push(clip);
        p.rows = vec![PlaylistRow::default(); 2];
        id
    };
    (engine, id)
}

#[test]
fn an_automation_clip_on_a_muted_row_does_not_move_its_knob() {
    let (engine, id) = held_tone();
    {
        let mut p = engine.project.lock();
        let mut clip = duck(&id);
        clip.row = Some(1);
        p.track_mut(&id).unwrap().automation_clips.push(clip);
    }
    let ducked = render(&engine, 6);
    assert!(peak(&ducked, 0, 2) > 0.3);
    assert!(peak(&ducked, 3, 5) < 0.01, "the clip pulls the volume down");
    engine.project.lock().rows[1].muted = true;
    let out = render(&engine, 6);
    assert!(peak(&out, 3, 5) > 0.3, "its row is off: the volume stays");
}

/// FL's automation clip is a channel of its own: placed on an automation
/// track, it moves the knob of the track it names.
#[test]
fn an_automation_clip_moves_the_track_it_targets() {
    let (engine, id) = held_tone();
    let auto = {
        let mut p = engine.project.lock();
        let auto = p.add_automation_track("Pad volume".into());
        p.track_mut(&auto).unwrap().automation_clips.push(duck(&id));
        auto
    };
    let untargeted = render(&engine, 6);
    assert!(
        peak(&untargeted, 3, 5) > 0.3,
        "with no target it moves its own (automation) track: nothing"
    );
    engine
        .project
        .lock()
        .track_mut(&auto)
        .unwrap()
        .automation_clips[0]
        .target_track = Some(id.clone());
    let out = render(&engine, 6);
    assert!(peak(&out, 0, 2) > 0.3, "before the clip");
    assert!(peak(&out, 3, 5) < 0.01, "the clip turns the pad down");
    // Its row switched off: the knob stays where it is.
    {
        let mut p = engine.project.lock();
        p.track_mut(&auto).unwrap().automation_clips[0].row = Some(0);
        p.rows[0].muted = true;
    }
    let out = render(&engine, 6);
    assert!(peak(&out, 3, 5) > 0.3);
}
