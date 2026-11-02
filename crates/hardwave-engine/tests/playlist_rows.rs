//! Playlist rows as the engine hears them: a row's switch (FL's track
//! mute) and solo silence what sits on the row, wherever it plays through.

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
fn a_soloed_row_silences_every_other_row_but_not_clips_without_one() {
    let (engine, _) = three_clips();
    engine.project.lock().rows[1].soloed = true;
    let out = render(&engine, 6);
    assert!(peak(&out, 0, 1) < 1e-6, "row 0 is not soloed");
    assert!(peak(&out, 2, 3) > 0.3, "the soloed row plays");
    assert!(
        peak(&out, 4, 5) > 0.3,
        "a clip with no row is never silenced"
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
