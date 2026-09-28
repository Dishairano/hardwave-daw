//! Comping: build one good take out of several passes.
//!
//! Loop recording already keeps every pass. Until now the last pass
//! played and the earlier ones sat muted underneath, so "the second half
//! of take 1 with the first half of take 3" meant splitting clips by hand
//! and muting the right pieces without losing count.
//!
//! Choosing a range from a take does exactly that, in one step and one
//! undo: the overlapping passes are split at both edges, the chosen
//! take's piece plays, and the pieces of the other takes over that range
//! are muted. Nothing is destroyed, so a different choice over the same
//! range is another click, not a re-record.

use crate::AppState;
use hardwave_project::clip::{ClipContent, ClipPlacement};
use hardwave_project::track::Track;
use tauri::State;

fn clip_id_of(placement: &ClipPlacement) -> &str {
    match &placement.content {
        ClipContent::Audio(ac) => &ac.id,
        ClipContent::Midi(mc) => &mc.id,
    }
}

/// Which take a piece came from. Audio pieces carry the pool key of the
/// file they were recorded into, which survives every split; a MIDI piece
/// has no such thing, so its own id stands in.
fn take_of(placement: &ClipPlacement) -> String {
    match &placement.content {
        ClipContent::Audio(ac) => ac.source_path.clone(),
        ClipContent::Midi(mc) => mc.id.clone(),
    }
}

fn end_of(placement: &ClipPlacement) -> u64 {
    placement.position_ticks + placement.length_ticks
}

/// Split one clip at an absolute tick, in place on the track.
///
/// The same arithmetic `split_clip` uses, without the locking, so a comp
/// can make several cuts inside one snapshot.
fn split_at(track: &mut Track, index: usize, at_ticks: u64) {
    let original = track.clips[index].clone();
    let start = original.position_ticks;
    let end = end_of(&original);
    if at_ticks <= start || at_ticks >= end {
        return;
    }
    let first_ticks = at_ticks - start;
    track.clips[index].length_ticks = first_ticks;

    let mut right = original.clone();
    right.position_ticks = at_ticks;
    right.length_ticks = end - at_ticks;
    let new_id = uuid::Uuid::new_v4().to_string();
    match &mut right.content {
        ClipContent::Audio(ac) => {
            let total_src = ac.source_end.saturating_sub(ac.source_start) as u128;
            let offset =
                (total_src * first_ticks as u128 / original.length_ticks.max(1) as u128) as u64;
            ac.source_start = ac.source_start.saturating_add(offset);
            ac.id = new_id;
        }
        ClipContent::Midi(mc) => {
            mc.id = new_id;
        }
    }
    track.clips.push(right);
}

fn set_muted(placement: &mut ClipPlacement, muted: bool) {
    match &mut placement.content {
        ClipContent::Audio(ac) => ac.muted = muted,
        ClipContent::Midi(mc) => mc.clip.muted = muted,
    }
}

/// Play this take over this range, and only this take.
///
/// Returns how many pieces the range ended up covering, which is what the
/// app tells the user.
#[tauri::command]
pub fn comp_take_range(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    start_ticks: u64,
    end_ticks: u64,
) -> Result<usize, String> {
    if end_ticks <= start_ticks {
        return Err("choose a range with some length".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let touched = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        apply_comp(track, &clip_id, start_ticks, end_ticks)?
    };
    state.engine.lock().rebuild_graph();
    Ok(touched)
}

/// The comp itself, on a track and nothing else, so it can be tested
/// without an engine.
fn apply_comp(
    track: &mut Track,
    clip_id: &str,
    start_ticks: u64,
    end_ticks: u64,
) -> Result<usize, String> {
    if end_ticks <= start_ticks {
        return Err("choose a range with some length".into());
    }
    {
        let chosen = track
            .clips
            .iter()
            .find(|c| clip_id_of(c) == clip_id)
            .ok_or_else(|| format!("Clip not found: {clip_id}"))?;
        let chosen_take = take_of(chosen);

        // Cut every overlapping piece at both edges of the range. The list
        // is rebuilt after each pass because a split appends a new piece.
        for edge in [start_ticks, end_ticks] {
            while let Some(index) = track.clips.iter().position(|c| {
                c.position_ticks < edge
                    && end_of(c) > edge
                    && c.position_ticks < end_ticks
                    && end_of(c) > start_ticks
            }) {
                split_at(track, index, edge);
            }
        }

        // Inside the range: the chosen take plays, the rest are muted.
        // Outside it nothing is touched, so an earlier choice elsewhere on
        // the timeline stands.
        let mut touched = 0usize;
        for placement in track.clips.iter_mut() {
            if placement.position_ticks < start_ticks || end_of(placement) > end_ticks {
                continue;
            }
            let from_chosen = take_of(placement) == chosen_take;
            set_muted(placement, !from_chosen);
            touched += 1;
        }
        Ok(touched)
    }
}

/// Put the passes of a take on lanes of their own, so they can be seen
/// side by side instead of stacked on top of each other.
///
/// Lane 0 keeps the take that plays. The rest are numbered in the order
/// they were recorded, which is the order they sit in the clip list.
#[tauri::command]
pub fn spread_takes_to_lanes(state: State<AppState>, track_id: String) -> Result<u32, String> {
    state.engine.lock().snapshot_before_mutation();
    let lanes = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;

        // One lane per take, in the order the takes first appear.
        let mut order: Vec<String> = Vec::new();
        for placement in track.clips.iter() {
            let take = take_of(placement);
            if !order.contains(&take) {
                order.push(take);
            }
        }
        for placement in track.clips.iter_mut() {
            let take = take_of(placement);
            let lane = order.iter().position(|t| t == &take).unwrap_or(0) as u32;
            placement.lane = lane;
        }
        order.len() as u32
    };
    state.engine.lock().rebuild_graph();
    Ok(lanes)
}

#[cfg(test)]
mod comp_tests {
    use super::*;
    use hardwave_project::clip::AudioClip;

    fn take_clip(id: &str, source: &str, position: u64, length: u64) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: id.to_string(),
                name: source.to_string(),
                source_path: source.to_string(),
                source_hash: String::new(),
                source_start: 0,
                source_end: length,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                source_file: format!("/takes/{source}"),
            }),
            track_id: "t1".to_string(),
            position_ticks: position,
            length_ticks: length,
            lane: 0,
        }
    }

    /// Two passes of the same bar, stacked the way loop recording leaves
    /// them.
    fn track_with_two_takes() -> Track {
        let mut track = Track::new_audio("t1".to_string(), "Vocal".to_string());
        track.clips.push(take_clip("a", "take-1.wav", 0, 3840));
        track.clips.push(take_clip("b", "take-2.wav", 0, 3840));
        track
    }

    fn muted_of(track: &Track, source: &str, position: u64) -> Option<bool> {
        track.clips.iter().find_map(|c| match &c.content {
            ClipContent::Audio(ac) if ac.source_path == source && c.position_ticks == position => {
                Some(ac.muted)
            }
            _ => None,
        })
    }

    #[test]
    fn choosing_a_range_from_one_take_silences_the_others_over_that_range() {
        let mut track = track_with_two_takes();
        apply_comp(&mut track, "a", 960, 1920).expect("comp");
        assert_eq!(muted_of(&track, "take-1.wav", 960), Some(false));
        assert_eq!(muted_of(&track, "take-2.wav", 960), Some(true));
    }

    #[test]
    fn what_lies_outside_the_range_is_left_exactly_as_it_was() {
        let mut track = track_with_two_takes();
        apply_comp(&mut track, "a", 960, 1920).expect("comp");
        // The piece of take 2 before the range keeps the state it had.
        assert_eq!(muted_of(&track, "take-2.wav", 0), Some(false));
        assert_eq!(muted_of(&track, "take-2.wav", 1920), Some(false));
    }

    #[test]
    fn the_takes_are_cut_at_both_edges_of_the_range() {
        let mut track = track_with_two_takes();
        apply_comp(&mut track, "a", 960, 1920).expect("comp");
        // Two takes, each in three pieces.
        assert_eq!(track.clips.len(), 6);
        for position in [0, 960, 1920] {
            assert!(
                muted_of(&track, "take-1.wav", position).is_some(),
                "take 1 should have a piece starting at {position}"
            );
        }
    }

    #[test]
    fn a_second_choice_over_the_same_range_swaps_which_take_plays() {
        let mut track = track_with_two_takes();
        apply_comp(&mut track, "a", 960, 1920).expect("first comp");
        let second = track
            .clips
            .iter()
            .find_map(|c| match &c.content {
                ClipContent::Audio(ac)
                    if ac.source_path == "take-2.wav" && c.position_ticks == 960 =>
                {
                    Some(ac.id.clone())
                }
                _ => None,
            })
            .expect("take 2 has a piece over the range");
        apply_comp(&mut track, &second, 960, 1920).expect("second comp");
        assert_eq!(muted_of(&track, "take-2.wav", 960), Some(false));
        assert_eq!(muted_of(&track, "take-1.wav", 960), Some(true));
    }

    #[test]
    fn a_range_with_no_length_is_refused() {
        let mut track = track_with_two_takes();
        assert!(apply_comp(&mut track, "a", 960, 960).is_err());
    }
}
