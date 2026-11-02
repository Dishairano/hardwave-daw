//! The song following its tempo map.
//!
//! Notes, patterns and automation sit in ticks, so a tempo change moves them
//! in time by itself. Audio has two ways to go, chosen per clip:
//!
//!   - it follows the tempo (`SampleTreatment::follow_bpm`): its file is laid
//!     over the beats, so the engine stretches it, pitch kept, as the tempo
//!     moves, and a loop stays on the beat. Its length in ticks stays.
//!   - it keeps its speed: it plays as recorded from the beat it starts on.
//!     A tempo change then changes how many ticks that audio covers, so its
//!     length in ticks is worked out again here. Without that, a faster song
//!     cut the end off a vocal and a slower one left a gap after it.

use crate::clip::{ClipContent, ClipPlacement};
use crate::tempo::TempoMap;
use crate::Project;

impl Project {
    /// Keep the song together after its tempo map changed from `before` to
    /// the map it has now. Every arrangement shares the one tempo map, so the
    /// clips of the ones not on the playlist are kept too.
    pub fn follow_tempo_change(&mut self, before: &TempoMap) {
        let after = self.tempo_map.clone();
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                keep_speed_through(clip, before, &after);
            }
        }
        for arrangement in &mut self.arrangements {
            for timeline in arrangement.timelines.values_mut() {
                for clip in &mut timeline.clips {
                    keep_speed_through(clip, before, &after);
                }
            }
        }
    }

    /// Make a clip follow the tempo or keep its speed. Returns whether it
    /// changed. See [`set_follows_tempo`].
    pub fn set_clip_follows_tempo(
        &mut self,
        track_id: &str,
        clip_id: &str,
        follows: bool,
    ) -> Result<bool, String> {
        let map = self.tempo_map.clone();
        let track = self
            .track_mut(track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let clip = track
            .clips
            .iter_mut()
            .find(|c| matches!(&c.content, ClipContent::Audio(a) if a.id == clip_id))
            .ok_or_else(|| format!("Audio clip not found: {clip_id}"))?;
        Ok(set_follows_tempo(clip, follows, &map))
    }
}

/// A clip that keeps its speed, made to cover the same audio from the same
/// beat under the tempo map `after` as it did under `before`. Its fades are
/// lengths of time as well, so they keep theirs. Clips held to the beat (MIDI,
/// following the tempo, warped) are left as they are.
pub fn keep_speed_through(placement: &mut ClipPlacement, before: &TempoMap, after: &TempoMap) {
    let ClipContent::Audio(audio) = &mut placement.content else {
        return;
    };
    if audio.follows_tempo() {
        return;
    }
    let start = placement.position_ticks;
    let end = start + placement.length_ticks;
    let start_before = before.secs_at_tick(start);
    let end_before = before.secs_at_tick(end);
    let start_after = after.secs_at_tick(start);
    let end_after = start_after + (end_before - start_before);
    // Round to the nearest tick: a stretch of the song the change did not
    // touch converts back to exactly the tick it had.
    let new_end = (after.tick_at_secs(end_after).round() as u64).max(start + 1);
    let new_length = new_end - start;

    if audio.fade_in_ticks > 0 {
        let secs = before.secs_at_tick(start + audio.fade_in_ticks) - start_before;
        let to = after.tick_at_secs(start_after + secs).round() as u64;
        audio.fade_in_ticks = to.saturating_sub(start).min(new_length);
    }
    if audio.fade_out_ticks > 0 {
        let from = end.saturating_sub(audio.fade_out_ticks);
        let secs = end_before - before.secs_at_tick(from);
        let at = after.tick_at_secs(end_after - secs).round() as u64;
        audio.fade_out_ticks = new_end.saturating_sub(at).min(new_length);
    }
    placement.length_ticks = new_length;
}

/// Switch a clip between following the tempo and keeping its speed, without
/// changing how it sounds where it sits now.
///
/// Following is set at the tempo the clip covers on average, so it plays at
/// the speed it did and only moves when the tempo does. Going back, the
/// stretch it has been playing at becomes its stretch. A warped clip follows
/// its markers whatever this says. Returns whether anything changed.
pub fn set_follows_tempo(placement: &mut ClipPlacement, follows: bool, map: &TempoMap) -> bool {
    let start = placement.position_ticks;
    let end = start + placement.length_ticks;
    let ClipContent::Audio(audio) = &mut placement.content else {
        return false;
    };
    let average = map.average_bpm_between(start, end);
    match (follows, audio.follow_bpm()) {
        (true, None) => {
            audio.treatment.follow_bpm = Some(average);
            true
        }
        (false, Some(bpm)) => {
            let stretch = if audio.stretch_ratio <= 0.01 {
                1.0
            } else {
                audio.stretch_ratio
            };
            audio.stretch_ratio = stretch * bpm / average;
            audio.treatment.follow_bpm = None;
            true
        }
        _ => false,
    }
}

/// How many ticks `secs` of audio covers from `at` when it keeps its speed:
/// what an imported or recorded clip that keeps its speed is long. Counted
/// through the tempo map, so a clip placed in a faster part of the song is
/// longer in ticks than the same clip at the start.
pub fn keep_speed_length(map: &TempoMap, at: u64, secs: f64) -> u64 {
    let end = map.tick_at_secs(map.secs_at_tick(at) + secs.max(0.0));
    ((end - at as f64).round() as u64).max(1)
}

/// How many ticks `secs` of a loop in `bpm` covers when it follows the
/// tempo: its own beats, whatever the song's tempo is.
pub fn follow_length(secs: f64, bpm: f64) -> u64 {
    let beats = secs.max(0.0) * bpm.max(1.0) / 60.0;
    ((beats * hardwave_midi::PPQ as f64).round() as u64).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{AudioClip, MidiClipRef};
    use crate::tempo::{TempoEntry, TempoRamp};
    use hardwave_midi::PPQ;

    const BAR: u64 = PPQ * 4;

    fn entry(tick: u64, bpm: f64, ramp: TempoRamp) -> TempoEntry {
        TempoEntry {
            tick,
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            ramp,
        }
    }

    fn steady(bpm: f64) -> TempoMap {
        TempoMap {
            entries: vec![entry(0, bpm, TempoRamp::Instant)],
        }
    }

    /// 150, a ramp to 160 over bars 9 to 17, and back to 150 for the drop.
    fn build_and_drop() -> TempoMap {
        TempoMap {
            entries: vec![
                entry(0, 150.0, TempoRamp::Instant),
                entry(BAR * 8, 150.0, TempoRamp::Linear),
                entry(BAR * 16 - 1, 160.0, TempoRamp::Instant),
                entry(BAR * 16, 150.0, TempoRamp::Instant),
            ],
        }
    }

    fn audio(at: u64, length: u64, follow_bpm: Option<f64>) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "a".into(),
                name: "a".into(),
                source_path: "s".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 0,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                warp_markers: Vec::new(),
                source_file: String::new(),
                treatment: crate::clip::SampleTreatment {
                    follow_bpm,
                    ..Default::default()
                },
            }),
            track_id: "t".into(),
            position_ticks: at,
            length_ticks: length,
            lane: 0,
            color: None,
        }
    }

    fn secs_covered(map: &TempoMap, clip: &ClipPlacement) -> f64 {
        map.secs_at_tick(clip.position_ticks + clip.length_ticks)
            - map.secs_at_tick(clip.position_ticks)
    }

    fn audio_of(clip: &ClipPlacement) -> &AudioClip {
        match &clip.content {
            ClipContent::Audio(a) => a,
            _ => unreachable!(),
        }
    }

    /// A vocal across the build keeps playing the 12.8 seconds it was
    /// recorded as. It used to keep its eight bars instead, which at the
    /// faster tempo is shorter, so its end was cut off.
    #[test]
    fn a_clip_that_keeps_its_speed_still_plays_all_of_its_audio() {
        let before = steady(150.0);
        let mut vocal = audio(BAR * 11, BAR * 8, None);
        let recorded = secs_covered(&before, &vocal);
        assert!((recorded - 12.8).abs() < 1e-9);

        let after = build_and_drop();
        keep_speed_through(&mut vocal, &before, &after);
        assert_eq!(vocal.position_ticks, BAR * 11, "it starts where it did");
        assert!(vocal.length_ticks > BAR * 8, "faster song, more ticks");
        let now = secs_covered(&after, &vocal);
        assert!((now - recorded).abs() < 0.001, "{now}s of the {recorded}s");
    }

    #[test]
    fn a_clip_that_follows_the_tempo_keeps_its_bars() {
        let mut lp = audio(0, BAR * 16, Some(150.0));
        keep_speed_through(&mut lp, &steady(150.0), &build_and_drop());
        assert_eq!(lp.length_ticks, BAR * 16);

        let mut notes = ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "m".into(),
                clip: hardwave_midi::MidiClip::new("m".into(), "m".into(), BAR),
            }),
            ..audio(0, BAR, None)
        };
        keep_speed_through(&mut notes, &steady(150.0), &build_and_drop());
        assert_eq!(notes.length_ticks, BAR, "notes are in beats already");
    }

    /// A tempo change somewhere else leaves a clip alone to the tick: being
    /// worked out again must not nudge clips the change never reached.
    #[test]
    fn a_clip_the_change_did_not_reach_does_not_move() {
        let before = steady(150.0);
        let after = build_and_drop();
        let mut intro = audio(BAR, BAR * 4, None);
        keep_speed_through(&mut intro, &before, &after);
        assert_eq!(intro.length_ticks, BAR * 4);
        // In the drop the tempo is back at 150, but the clip starts later in
        // time; its length in ticks is still the same.
        let mut drop = audio(BAR * 17, BAR * 2 + 123, None);
        keep_speed_through(&mut drop, &before, &after);
        assert_eq!(drop.length_ticks, BAR * 2 + 123);
    }

    #[test]
    fn its_fades_keep_their_length_in_time() {
        let before = steady(150.0);
        let after = steady(120.0);
        let mut clip = audio(BAR * 2, BAR * 4, None);
        if let ClipContent::Audio(a) = &mut clip.content {
            a.fade_in_ticks = PPQ;
            a.fade_out_ticks = PPQ * 2;
        }
        keep_speed_through(&mut clip, &before, &after);
        // Slower song: the same seconds are fewer ticks.
        let a = audio_of(&clip);
        assert_eq!(clip.length_ticks, BAR * 4 * 120 / 150);
        assert_eq!(a.fade_in_ticks, PPQ * 120 / 150);
        assert_eq!(a.fade_out_ticks, PPQ * 2 * 120 / 150);
    }

    /// Changing the tempo and changing it back gives the clip back as it was.
    #[test]
    fn there_and_back_is_where_it_started() {
        let a = steady(150.0);
        let b = build_and_drop();
        let mut clip = audio(BAR * 10 + 77, BAR * 9 + 5, None);
        for _ in 0..50 {
            keep_speed_through(&mut clip, &a, &b);
            keep_speed_through(&mut clip, &b, &a);
        }
        assert!(
            clip.length_ticks.abs_diff(BAR * 9 + 5) <= 1,
            "{}",
            clip.length_ticks
        );
    }

    #[test]
    fn every_arrangement_keeps_its_clips() {
        let mut project = Project {
            tempo_map: steady(150.0),
            ..Default::default()
        };
        let id = project.add_audio_track("Vox".into());
        let mut placed = audio(BAR * 11, BAR * 8, None);
        placed.track_id = id.clone();
        project.track_mut(&id).unwrap().clips.push(placed.clone());
        project.ensure_arrangements();
        project.arrangements.push(crate::arrangement::Arrangement {
            id: "other".into(),
            name: "Other".into(),
            timelines: [(
                id.clone(),
                crate::arrangement::TrackTimeline {
                    clips: vec![placed],
                    automation_clips: Vec::new(),
                },
            )]
            .into_iter()
            .collect(),
        });

        let before = project.tempo_map.clone();
        project.tempo_map = build_and_drop();
        project.follow_tempo_change(&before);
        let live = project.track(&id).unwrap().clips[0].length_ticks;
        let other = project.arrangements.last().unwrap().timelines[&id].clips[0].length_ticks;
        assert!(live > BAR * 8);
        assert_eq!(live, other, "the arrangement not on show was kept too");
    }

    /// Switching between the two ways changes nothing that can be heard
    /// where the clip sits: following starts at the tempo it covers, and
    /// keeping its speed keeps the stretch it was playing at.
    #[test]
    fn switching_the_way_a_clip_follows_does_not_change_its_sound() {
        let map = build_and_drop();
        let mut clip = audio(BAR * 8, BAR * 8, None);
        assert!(set_follows_tempo(&mut clip, true, &map));
        let bpm = audio_of(&clip).follow_bpm().unwrap();
        // Over the clip, it plays the file at its own speed: the beats the
        // clip covers take the seconds the file lasts at that tempo.
        let file_secs = (BAR * 8) as f64 / PPQ as f64 * 60.0 / bpm;
        assert!((file_secs - secs_covered(&map, &clip)).abs() < 1e-9);

        // A loop at 128 laid in at 150 plays at 150/128; kept at its speed,
        // that is a stretch of 128/150.
        let mut lp = audio(0, BAR * 4, Some(128.0));
        assert!(set_follows_tempo(&mut lp, false, &steady(150.0)));
        let a = audio_of(&lp);
        assert_eq!(a.follow_bpm(), None);
        assert!((a.stretch_ratio - 128.0 / 150.0).abs() < 1e-12);
        assert!(
            !set_follows_tempo(&mut lp, false, &steady(150.0)),
            "already"
        );
    }

    #[test]
    fn an_imported_clip_is_as_long_as_its_audio_where_it_lands() {
        let map = build_and_drop();
        // Two seconds at the start is five beats at 150.
        assert_eq!(keep_speed_length(&map, 0, 2.0), PPQ * 5);
        // The same two seconds in the build is more beats.
        assert!(keep_speed_length(&map, BAR * 12, 2.0) > PPQ * 5);
        // A loop in 128 is its own beats: 3.75 s at 128 is eight beats.
        assert_eq!(follow_length(3.75, 128.0), PPQ * 8);
    }

    #[test]
    fn a_loop_that_follows_plays_its_file_at_a_fixed_rate_per_tick() {
        let clip = audio(0, BAR, Some(150.0));
        let a = audio_of(&clip);
        // At 150 a beat is 0.4 s: 19 200 frames at 48 kHz, 20 a tick.
        assert!((a.file_frames_per_tick(48_000.0).unwrap() - 20.0).abs() < 1e-12);
        let kept = audio(0, BAR, None);
        assert_eq!(audio_of(&kept).file_frames_per_tick(48_000.0), None);
    }
}
