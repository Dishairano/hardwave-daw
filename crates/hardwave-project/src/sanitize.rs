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
        Ok(())
    }
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
    fn a_song_too_big_to_be_real_is_refused() {
        let mut p = Project::default();
        for i in 0..=MAX_TRACKS {
            p.add_midi_track(format!("{i}"));
        }
        assert!(p.make_safe().is_err());
    }
}
