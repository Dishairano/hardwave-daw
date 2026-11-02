//! Making a project from someone else safe to open.
//!
//! A project is a file, and a file can be written by anyone. Before a
//! song from disk, from Workspace or from a room partner reaches the
//! engine, every number in it that the engine would trust is put back
//! inside the range the DAW itself could have produced, and the things
//! the engine assumes are there are put there. A value outside that
//! range is not a song anyone made; it is the start of a crash, a huge
//! allocation, or a speaker-damaging output.
//!
//! Values are repaired, not refused, where a sensible value exists, so
//! an old or slightly damaged song still opens. Counts far beyond any
//! real song are refused: those are not damage, they are an attack on
//! memory.

use crate::clip::{ClipContent, ClipPlacement};
use crate::project::Project;
use crate::tempo::{TempoEntry, TempoMap};

/// The most a project may hold before it is refused outright.
pub const MAX_TRACKS: usize = 2_000;
pub const MAX_CLIPS: usize = 200_000;
pub const MAX_NOTES: usize = 2_000_000;
/// The most playlist rows a song or one of its arrangements may hold. FL
/// has 500; a song laid out as before FL's three lists has a row per
/// track, automation lane and automation clip, which a big song takes
/// into the thousands.
pub const MAX_ROWS: usize = 20_000;
/// The most channel rack groups.
pub const MAX_RACK_GROUPS: usize = 256;
/// The highest mixer number: one for every track a song may hold.
pub const MAX_MIXER_NUMBER: u16 = MAX_TRACKS as u16;

fn finite_or(value: f64, default: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        default
    }
}

fn clamp_finite(value: f64, low: f64, high: f64, default: f64) -> f64 {
    finite_or(value, default).clamp(low, high)
}

fn clamp_finite32(value: f32, low: f32, high: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        default
    }
}

fn safe_tempo(map: &mut TempoMap) {
    if map.entries.is_empty() {
        *map = TempoMap::default();
    }
    map.entries.sort_by_key(|e| e.tick);
    if map.entries[0].tick != 0 {
        let first = map.entries[0].clone();
        map.entries.insert(0, TempoEntry { tick: 0, ..first });
    }
    for e in &mut map.entries {
        e.bpm = clamp_finite(e.bpm, 10.0, 999.0, 140.0);
        e.time_sig_num = e.time_sig_num.clamp(1, 64);
        if !matches!(e.time_sig_den, 1 | 2 | 4 | 8 | 16 | 32) {
            e.time_sig_den = 4;
        }
    }
}

fn safe_clip(placement: &mut ClipPlacement, notes: &mut usize) {
    match &mut placement.content {
        ClipContent::Audio(a) => {
            a.gain_db = clamp_finite(a.gain_db, -200.0, 24.0, 0.0);
            a.pitch_semitones = clamp_finite(a.pitch_semitones, -48.0, 48.0, 0.0);
            // The engine allocates a stretched copy the length of the
            // clip times this; the UI never goes past a tenth or ten.
            a.stretch_ratio = clamp_finite(a.stretch_ratio, 0.1, 10.0, 1.0);
            if a.source_start > a.source_end {
                a.source_start = a.source_end;
            }
        }
        ClipContent::Midi(m) => {
            *notes += m.clip.notes.len();
            for n in &mut m.clip.notes {
                n.pitch = n.pitch.min(127);
                n.channel = n.channel.min(15);
                n.velocity = clamp_finite32(n.velocity, 0.0, 1.0, 0.8);
                n.pan = clamp_finite32(n.pan, -1.0, 1.0, 0.0);
                n.duration_ticks = n.duration_ticks.max(1);
            }
        }
    }
}

impl Project {
    /// Put every number the engine trusts back in range, or refuse a
    /// project too large to be anyone's song.
    pub fn make_safe(&mut self) -> Result<(), String> {
        safe_tempo(&mut self.tempo_map);
        if self.tracks.len() > MAX_TRACKS {
            return Err(format!(
                "this song has {} tracks, more than any song the DAW can open",
                self.tracks.len()
            ));
        }
        let mut clips = 0usize;
        let mut notes = 0usize;
        for track in &mut self.tracks {
            track.volume_db = clamp_finite(track.volume_db, -200.0, 24.0, 0.0);
            track.pan = clamp_finite(track.pan, -1.0, 1.0, 0.0);
            track.stereo_separation = clamp_finite(track.stereo_separation, 0.0, 2.0, 1.0);
            track.fine_tune_cents = clamp_finite32(track.fine_tune_cents, -1200.0, 1200.0, 0.0);
            track.filter_cutoff_hz =
                clamp_finite32(track.filter_cutoff_hz, 10.0, 30_000.0, 20_000.0);
            clips += track.clips.len();
            for placement in &mut track.clips {
                safe_clip(placement, &mut notes);
            }
        }
        for arrangement in &mut self.arrangements {
            for timeline in arrangement.timelines.values_mut() {
                clips += timeline.clips.len();
                for placement in &mut timeline.clips {
                    safe_clip(placement, &mut notes);
                }
            }
        }
        if clips > MAX_CLIPS {
            return Err(format!(
                "this song has {clips} clips, more than any song the DAW can open"
            ));
        }
        if notes > MAX_NOTES {
            return Err(format!(
                "this song has {notes} notes, more than any song the DAW can open"
            ));
        }
        safe_layout(self)
    }
}

impl Project {
    /// The mixer number a track added now takes: the one after the
    /// highest in use, as a track added to the mixer went to its end.
    pub fn next_mixer_number(&self) -> u16 {
        let highest = self.tracks.iter().filter_map(|t| t.mixer_number).max();
        highest.map_or(1, |n| n.saturating_add(1).clamp(1, MAX_MIXER_NUMBER))
    }
}

/// Where things show in FL's three lists, made consistent: rows and
/// groups within their limits, every mixer number used once, 0 only for
/// the master, and every track in at least one of the lists.
fn safe_layout(project: &mut Project) -> Result<(), String> {
    use crate::track::TrackKind;
    if project.rows.len() > MAX_ROWS || project.arrangements.iter().any(|a| a.rows.len() > MAX_ROWS)
    {
        return Err("this song has more playlist rows than any song the DAW can open".into());
    }
    if project.rack_groups.len() > MAX_RACK_GROUPS {
        return Err("this song has more channel rack groups than any song the DAW can open".into());
    }
    let row = |row: &mut Option<u32>| {
        if row.is_some_and(|r| r as usize >= MAX_ROWS) {
            *row = None;
        }
    };
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            row(&mut clip.row);
        }
        for lane in &mut track.automation_lanes {
            row(&mut lane.row);
        }
        for clip in &mut track.automation_clips {
            row(&mut clip.row);
            // Moving its own track's knob is what no target says.
            if clip.target_track.as_deref() == Some(track.id.as_str()) {
                clip.target_track = None;
            }
        }
    }
    for arrangement in &mut project.arrangements {
        for (id, timeline) in arrangement.timelines.iter_mut() {
            for clip in &mut timeline.clips {
                row(&mut clip.row);
            }
            for clip in &mut timeline.automation_clips {
                row(&mut clip.row);
                if clip.target_track.as_deref() == Some(id.as_str()) {
                    clip.target_track = None;
                }
            }
        }
    }

    // Mixer numbers: the first master is 0, nothing else is, automation
    // tracks have none, and a number taken twice goes to the first track
    // that has it.
    let mut taken = std::collections::HashSet::new();
    let mut master_seen = false;
    for track in &mut project.tracks {
        match track.kind {
            TrackKind::Master if !master_seen => {
                master_seen = true;
                track.mixer_number = Some(0);
            }
            TrackKind::Master | TrackKind::Automation => track.mixer_number = None,
            _ => {
                if track
                    .mixer_number
                    .is_some_and(|n| n == 0 || n > MAX_MIXER_NUMBER || !taken.insert(n))
                {
                    track.mixer_number = None;
                }
            }
        }
        // Buses, returns and the master are never channels.
        if matches!(
            track.kind,
            TrackKind::Master | TrackKind::Bus | TrackKind::Return
        ) {
            track.rack_group = None;
        }
    }

    // A track in neither list would be shown nowhere: it gets what a
    // track of its kind had before the three lists. A number is the next
    // after the highest, as a track added to the mixer went to its end.
    let mut next = taken.iter().max().map_or(1, |n| n + 1);
    let mut next_number = |taken: &mut std::collections::HashSet<u16>| {
        while next == 0 || next > MAX_MIXER_NUMBER || taken.contains(&next) {
            next = if next >= MAX_MIXER_NUMBER {
                1
            } else {
                next + 1
            };
        }
        taken.insert(next);
        next
    };
    for track in &mut project.tracks {
        let lost = !matches!(track.kind, TrackKind::Master)
            && track.mixer_number.is_none()
            && track.rack_group.is_none();
        if !lost {
            continue;
        }
        if !matches!(track.kind, TrackKind::Automation) {
            track.mixer_number = Some(next_number(&mut taken));
        }
        let listed = !track.id.starts_with("insert-")
            || matches!(track.kind, TrackKind::Midi)
            || !track.clips.is_empty();
        if matches!(
            track.kind,
            TrackKind::Audio | TrackKind::Midi | TrackKind::Automation
        ) && listed
        {
            track.rack_group = Some(0);
        }
    }

    // Every group a channel names exists.
    let groups = project.rack_groups.len();
    let mut unsorted = false;
    for track in &mut project.tracks {
        if let Some(g) = track.rack_group {
            if g as usize >= groups {
                track.rack_group = Some(0);
                unsorted = groups == 0;
            }
        }
    }
    if unsorted {
        project
            .rack_groups
            .push(crate::layout::DEFAULT_RACK_GROUP.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_without_a_tempo_gets_one() {
        let mut p = Project::default();
        p.tempo_map.entries.clear();
        p.make_safe().unwrap();
        assert_eq!(p.tempo_map.entries[0].tick, 0);
        assert!(p.tempo_map.entries[0].bpm > 0.0);
    }

    #[test]
    fn impossible_numbers_are_put_back_in_range() {
        let mut p = Project::default();
        p.tempo_map.entries[0].bpm = f64::NAN;
        p.tempo_map.entries[0].time_sig_den = 3;
        p.tempo_map.entries[0].time_sig_num = 0;
        let id = p.add_audio_track("x".into());
        let t = p.track_mut(&id).unwrap();
        t.volume_db = f64::INFINITY;
        t.pan = f64::NAN;
        p.make_safe().unwrap();
        let e = &p.tempo_map.entries[0];
        assert_eq!((e.bpm, e.time_sig_num, e.time_sig_den), (140.0, 1, 4));
        let t = p.track(&id).unwrap();
        assert!(t.volume_db.is_finite() && t.volume_db <= 24.0);
        assert_eq!(t.pan, 0.0);
    }

    #[test]
    fn a_tempo_map_that_starts_late_starts_at_zero() {
        let mut p = Project::default();
        p.tempo_map.entries[0].tick = 9600;
        p.make_safe().unwrap();
        assert_eq!(p.tempo_map.entries[0].tick, 0);
    }

    #[test]
    fn every_mixer_number_is_used_once_and_0_is_the_masters() {
        use crate::track::Track;
        let mut p = Project::default();
        p.tracks.truncate(3);
        p.tracks[0].mixer_number = Some(7);
        p.tracks[1].mixer_number = Some(2);
        p.tracks[2].mixer_number = Some(2);
        let mut zero = Track::new_audio("zero".into(), "Zero".into());
        zero.mixer_number = Some(0);
        zero.rack_group = Some(0);
        p.tracks.push(zero);
        let mut auto = Track::new_automation("auto".into(), "Auto".into());
        auto.mixer_number = Some(9);
        auto.rack_group = Some(0);
        p.tracks.push(auto);
        let mut bus = Track::new_bus("bus".into(), "Bus".into());
        bus.mixer_number = Some(5);
        bus.rack_group = Some(0);
        p.tracks.push(bus);
        p.make_safe().unwrap();
        let numbers: Vec<_> = p.tracks.iter().map(|t| t.mixer_number).collect();
        assert_eq!(
            numbers,
            [Some(0), Some(2), Some(6), None, None, Some(5)],
            "the second 2 goes after the highest; a channel's 0 is dropped"
        );
        assert_eq!(p.tracks[3].rack_group, Some(0), "it is still a channel");
        assert_eq!(p.tracks[5].rack_group, None, "a bus is never a channel");
    }

    #[test]
    fn a_cloned_track_takes_the_next_mixer_number() {
        let mut p = Project::default();
        let lead = p.add_midi_track("Lead".into());
        p.make_safe().unwrap();
        assert_eq!(p.track(&lead).unwrap().mixer_number, Some(501));
        let clone = p.duplicate_track(&lead, &Default::default()).unwrap();
        let t = p.track(&clone).unwrap();
        assert_eq!(t.mixer_number, Some(502), "not a second 501");
        assert_eq!(t.rack_group, Some(0));
    }

    #[test]
    fn a_track_in_neither_list_gets_what_its_kind_had_before() {
        let mut p = Project::default();
        let audio = p.add_audio_track("Vox".into());
        let midi = p.add_midi_track("Lead".into());
        let auto = p.add_automation_track("Auto".into());
        p.make_safe().unwrap();
        let t = |id: &str| p.track(id).unwrap().clone();
        assert_eq!(t(&audio).mixer_number, Some(501), "after the highest");
        assert_eq!(t(&audio).rack_group, Some(0));
        assert_eq!(t(&midi).mixer_number, Some(502));
        assert_eq!(t(&midi).rack_group, Some(0));
        assert_eq!(t(&auto).mixer_number, None, "the mixer has no automation");
        assert_eq!(t(&auto).rack_group, Some(0));
    }

    #[test]
    fn rows_and_groups_out_of_bounds_are_put_right() {
        use crate::automation::AutomationTarget;
        use crate::automation_clip::AutomationClip;
        let mut p = Project::default();
        p.rack_groups.clear();
        let id = p.add_midi_track("Lead".into());
        {
            let t = p.track_mut(&id).unwrap();
            t.rack_group = Some(40);
            let mut clip = AutomationClip::new("ac", AutomationTarget::TrackPan, 0, 960);
            clip.row = Some(MAX_ROWS as u32);
            clip.target_track = Some(id.clone());
            t.automation_clips.push(clip);
        }
        p.make_safe().unwrap();
        let t = p.track(&id).unwrap();
        assert_eq!(t.rack_group, Some(0));
        assert_eq!(p.rack_groups, ["Unsorted"]);
        assert_eq!(t.automation_clips[0].row, None);
        assert_eq!(t.automation_clips[0].target_track, None);

        p.rows = vec![crate::layout::PlaylistRow::default(); MAX_ROWS + 1];
        assert!(p.make_safe().is_err(), "more rows than any song has");
    }

    #[test]
    fn a_song_too_big_to_be_real_is_refused() {
        let mut p = Project::default();
        for i in 0..=MAX_TRACKS {
            p.add_midi_track(format!("{i}"));
        }
        assert!(p.make_safe().is_err());
    }
}
