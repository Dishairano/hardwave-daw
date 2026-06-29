//! Multiple arrangements — FL Studio-style switchable playlists within a
//! single project. Channels, mixer, inserts, instruments and plugin state
//! are shared; only the per-track TIMELINE content (clip placements +
//! automation clips) is per-arrangement.
//!
//! Low-surgery design: the live `Track` keeps its `clips` /
//! `automation_clips`. An `Arrangement` is a stored snapshot keyed by
//! track id. Switching captures the live timeline into the currently
//! active arrangement, then applies the target arrangement's snapshot
//! onto the live tracks. Empty tracks (the 500 idle inserts) are not
//! stored, so the snapshot stays small.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::automation_clip::AutomationClip;
use crate::clip::ClipPlacement;
use crate::Project;

/// One track's timeline content within an arrangement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackTimeline {
    #[serde(default)]
    pub clips: Vec<ClipPlacement>,
    #[serde(default)]
    pub automation_clips: Vec<AutomationClip>,
}

/// A named arrangement: a snapshot of every (non-empty) track's timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arrangement {
    pub id: String,
    pub name: String,
    /// `track_id` → that track's timeline content in this arrangement.
    /// Tracks absent from the map have no clips in this arrangement.
    #[serde(default)]
    pub timelines: HashMap<String, TrackTimeline>,
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Snapshot of the live tracks' timeline content, keyed by track id.
/// Skips empty tracks so the idle inserts don't bloat the map.
fn snapshot_live(project: &Project) -> HashMap<String, TrackTimeline> {
    let mut map = HashMap::new();
    for t in &project.tracks {
        if t.clips.is_empty() && t.automation_clips.is_empty() {
            continue;
        }
        map.insert(
            t.id.clone(),
            TrackTimeline {
                clips: t.clips.clone(),
                automation_clips: t.automation_clips.clone(),
            },
        );
    }
    map
}

impl Project {
    /// Lazily create the first arrangement from the current timeline if
    /// none exist (so legacy projects gain a default "Arrangement 1").
    pub fn ensure_arrangements(&mut self) {
        if self.arrangements.is_empty() {
            let id = new_id();
            let timelines = snapshot_live(self);
            self.arrangements.push(Arrangement {
                id: id.clone(),
                name: "Arrangement 1".into(),
                timelines,
            });
            self.active_arrangement = id;
        } else if self.active_arrangement.is_empty()
            || !self
                .arrangements
                .iter()
                .any(|a| a.id == self.active_arrangement)
        {
            // Active pointer dangling (older save / deleted) → first one.
            self.active_arrangement = self.arrangements[0].id.clone();
        }
    }

    /// Capture the live timeline into the active arrangement. Call before
    /// switching away or saving so the active arrangement stays current.
    pub fn capture_active_arrangement(&mut self) {
        self.ensure_arrangements();
        let snapshot = snapshot_live(self);
        let active = self.active_arrangement.clone();
        if let Some(arr) = self.arrangements.iter_mut().find(|a| a.id == active) {
            arr.timelines = snapshot;
        }
    }

    /// Load an arrangement's timeline onto the live tracks. Tracks not in
    /// the snapshot are cleared. Sets it active. No-op if `id` is unknown.
    pub fn apply_arrangement(&mut self, id: &str) -> Result<(), String> {
        self.ensure_arrangements();
        let timelines = self
            .arrangements
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("Arrangement not found: {id}"))?
            .timelines
            .clone();
        for t in &mut self.tracks {
            match timelines.get(&t.id) {
                Some(tl) => {
                    t.clips = tl.clips.clone();
                    t.automation_clips = tl.automation_clips.clone();
                }
                None => {
                    t.clips.clear();
                    t.automation_clips.clear();
                }
            }
        }
        self.active_arrangement = id.to_string();
        Ok(())
    }

    /// Create a new arrangement and switch to it. `copy_current` clones
    /// the live timeline into the new arrangement (FL's "duplicate");
    /// otherwise it starts empty. Returns the new arrangement id.
    pub fn create_arrangement(&mut self, name: &str, copy_current: bool) -> String {
        self.capture_active_arrangement();
        let id = new_id();
        let timelines = if copy_current {
            snapshot_live(self)
        } else {
            HashMap::new()
        };
        self.arrangements.push(Arrangement {
            id: id.clone(),
            name: name.to_string(),
            timelines,
        });
        // Switch onto it (clears the live timeline when starting empty).
        let _ = self.apply_arrangement(&id);
        id
    }

    /// Delete an arrangement. Refuses to remove the last one. If the
    /// active arrangement is deleted, switches to the first remaining.
    pub fn delete_arrangement(&mut self, id: &str) -> Result<(), String> {
        self.ensure_arrangements();
        if self.arrangements.len() <= 1 {
            return Err("Cannot delete the only arrangement".into());
        }
        if !self.arrangements.iter().any(|a| a.id == id) {
            return Err(format!("Arrangement not found: {id}"));
        }
        let was_active = self.active_arrangement == id;
        self.arrangements.retain(|a| a.id != id);
        if was_active {
            let first = self.arrangements[0].id.clone();
            self.apply_arrangement(&first)?;
        }
        Ok(())
    }

    pub fn rename_arrangement(&mut self, id: &str, name: &str) -> Result<(), String> {
        let arr = self
            .arrangements
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("Arrangement not found: {id}"))?;
        arr.name = name.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{ClipContent, ClipPlacement, MidiClipRef};

    fn midi_clip(id: &str) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: id.into(),
                clip: hardwave_midi::MidiClip::new(id.into(), "c".into(), 960),
            }),
            track_id: String::new(),
            position_ticks: 0,
            length_ticks: 960,
            lane: 0,
        }
    }

    #[test]
    fn first_use_creates_default_arrangement() {
        let mut p = Project::default();
        assert!(p.arrangements.is_empty());
        p.ensure_arrangements();
        assert_eq!(p.arrangements.len(), 1);
        assert_eq!(p.arrangements[0].name, "Arrangement 1");
        assert_eq!(p.active_arrangement, p.arrangements[0].id);
    }

    #[test]
    fn switching_swaps_timeline_and_preserves_both() {
        let mut p = Project::default();
        let tid = p.tracks[1].id.clone(); // first insert track
        p.tracks[1].clips.push(midi_clip("a"));
        p.ensure_arrangements(); // arrangement 1 captures clip "a" lazily? no — captured on demand
                                 // Create a new empty arrangement (captures current into arr1, switches to empty arr2).
        let arr2 = p.create_arrangement("Arrangement 2", false);
        // arr2 is empty → the live track has no clips now.
        assert!(
            p.tracks[1].clips.is_empty(),
            "new empty arrangement clears timeline"
        );
        // Add a different clip in arr2.
        p.tracks[1].clips.push(midi_clip("b"));
        // Switch back to arr1 → clip "a" returns.
        let arr1 = p
            .arrangements
            .iter()
            .find(|a| a.id != arr2)
            .unwrap()
            .id
            .clone();
        p.capture_active_arrangement();
        p.apply_arrangement(&arr1).unwrap();
        assert_eq!(p.tracks[1].clips.len(), 1);
        assert_eq!(p.tracks[1].clips[0].length_ticks, 960);
        let _ = tid;
        // Switch to arr2 → clip "b" returns.
        p.capture_active_arrangement();
        p.apply_arrangement(&arr2).unwrap();
        assert_eq!(p.tracks[1].clips.len(), 1);
    }

    #[test]
    fn cannot_delete_last_arrangement() {
        let mut p = Project::default();
        p.ensure_arrangements();
        let id = p.arrangements[0].id.clone();
        assert!(p.delete_arrangement(&id).is_err());
    }

    #[test]
    fn delete_active_switches_to_remaining() {
        let mut p = Project::default();
        p.ensure_arrangements();
        let arr1 = p.arrangements[0].id.clone();
        let arr2 = p.create_arrangement("Two", false); // active = arr2
        assert_eq!(p.active_arrangement, arr2);
        p.delete_arrangement(&arr2).unwrap();
        assert_eq!(p.active_arrangement, arr1);
        assert_eq!(p.arrangements.len(), 1);
    }
}
