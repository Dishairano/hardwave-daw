//! Track freeze / unfreeze — off-line render a track's full output
//! to a rendered audio clip, swap the clip in for the live chain to
//! cut CPU, and restore the live chain on unfreeze.
//!
//! The crate-level type is state-only; the actual render happens on
//! a worker thread and feeds results back via `FrozenRender`.

use serde::{Deserialize, Serialize};

/// Per-track freeze state — the UI + audio engine both consult this
/// to know whether to play the live chain or the frozen clip.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum FreezeState {
    /// Live chain: plugins + clips process in real time.
    #[default]
    Live,
    /// Render in progress — UI shows a progress bar; audio still
    /// plays the live chain.
    Rendering { progress_percent: u8 },
    /// Frozen — audio engine plays the rendered clip, plugins are
    /// muted / disabled to save CPU.
    Frozen,
}

impl FreezeState {
    pub fn is_frozen(&self) -> bool {
        matches!(self, FreezeState::Frozen)
    }

    pub fn is_rendering(&self) -> bool {
        matches!(self, FreezeState::Rendering { .. })
    }

    pub fn progress(&self) -> Option<u8> {
        if let FreezeState::Rendering { progress_percent } = self {
            Some(*progress_percent)
        } else {
            None
        }
    }
}

/// The rendered output of a freeze pass — one stereo buffer that
/// replaces the track's live chain in the audio graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenRender {
    pub track_id: String,
    pub sample_rate: f32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl FrozenRender {
    pub fn duration_secs(&self) -> f32 {
        self.left.len() as f32 / self.sample_rate.max(1.0)
    }

    pub fn is_stereo(&self) -> bool {
        self.left.len() == self.right.len()
    }
}

/// Per-track registry — holds the current state plus the last
/// `FrozenRender` if one exists, so unfreezing preserves the render
/// until the user edits the chain.
#[derive(Debug, Clone, Default)]
pub struct FreezeRegistry {
    entries: Vec<FreezeEntry>,
}

#[derive(Debug, Clone)]
struct FreezeEntry {
    track_id: String,
    state: FreezeState,
    render: Option<FrozenRender>,
}

impl FreezeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state_of(&self, track_id: &str) -> FreezeState {
        self.entries
            .iter()
            .find(|e| e.track_id == track_id)
            .map(|e| e.state.clone())
            .unwrap_or_default()
    }

    pub fn is_frozen(&self, track_id: &str) -> bool {
        self.state_of(track_id).is_frozen()
    }

    /// Begin a freeze pass. State transitions `Live → Rendering`.
    pub fn begin_freeze(&mut self, track_id: &str) {
        let entry = self.entry(track_id);
        entry.state = FreezeState::Rendering {
            progress_percent: 0,
        };
        entry.render = None;
    }

    pub fn update_progress(&mut self, track_id: &str, percent: u8) {
        let entry = self.entry(track_id);
        if let FreezeState::Rendering { progress_percent } = &mut entry.state {
            *progress_percent = percent.min(100);
        }
    }

    /// Commit the finished render. `Rendering → Frozen` and the
    /// `FrozenRender` is stored.
    pub fn commit_render(&mut self, render: FrozenRender) {
        let entry = self.entry(&render.track_id.clone());
        entry.state = FreezeState::Frozen;
        entry.render = Some(render);
    }

    /// Unfreeze — go back to the live chain. The cached render stays
    /// so a re-freeze with unchanged plugins can short-circuit.
    pub fn unfreeze(&mut self, track_id: &str) {
        let entry = self.entry(track_id);
        entry.state = FreezeState::Live;
    }

    /// Drop the cached render — call when the user edits the plugin
    /// chain so a stale freeze can't be silently reused.
    pub fn invalidate(&mut self, track_id: &str) {
        let entry = self.entry(track_id);
        entry.render = None;
        if matches!(entry.state, FreezeState::Frozen) {
            entry.state = FreezeState::Live;
        }
    }

    pub fn render_for(&self, track_id: &str) -> Option<&FrozenRender> {
        self.entries
            .iter()
            .find(|e| e.track_id == track_id)
            .and_then(|e| e.render.as_ref())
    }

    fn entry(&mut self, track_id: &str) -> &mut FreezeEntry {
        if let Some(idx) = self.entries.iter().position(|e| e.track_id == track_id) {
            &mut self.entries[idx]
        } else {
            self.entries.push(FreezeEntry {
                track_id: track_id.to_string(),
                state: FreezeState::Live,
                render: None,
            });
            self.entries.last_mut().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_render(id: &str, n: usize) -> FrozenRender {
        FrozenRender {
            track_id: id.to_string(),
            sample_rate: 48_000.0,
            left: vec![0.5; n],
            right: vec![0.5; n],
        }
    }

    #[test]
    fn freeze_state_default_is_live() {
        let reg = FreezeRegistry::new();
        assert_eq!(reg.state_of("track-1"), FreezeState::Live);
        assert!(!reg.is_frozen("track-1"));
    }

    #[test]
    fn begin_and_commit_cycle_transitions_states() {
        let mut reg = FreezeRegistry::new();
        reg.begin_freeze("t");
        assert!(reg.state_of("t").is_rendering());
        reg.update_progress("t", 50);
        assert_eq!(reg.state_of("t").progress(), Some(50));
        reg.commit_render(mk_render("t", 1_024));
        assert!(reg.is_frozen("t"));
        assert!(reg.render_for("t").is_some());
    }

    #[test]
    fn unfreeze_preserves_render_cache() {
        let mut reg = FreezeRegistry::new();
        reg.begin_freeze("t");
        reg.commit_render(mk_render("t", 512));
        reg.unfreeze("t");
        assert_eq!(reg.state_of("t"), FreezeState::Live);
        assert!(reg.render_for("t").is_some());
    }

    #[test]
    fn invalidate_drops_cached_render() {
        let mut reg = FreezeRegistry::new();
        reg.begin_freeze("t");
        reg.commit_render(mk_render("t", 256));
        reg.invalidate("t");
        assert!(reg.render_for("t").is_none());
        assert_eq!(reg.state_of("t"), FreezeState::Live);
    }

    #[test]
    fn update_progress_clamps_to_100() {
        let mut reg = FreezeRegistry::new();
        reg.begin_freeze("t");
        reg.update_progress("t", 240);
        assert_eq!(reg.state_of("t").progress(), Some(100));
    }

    #[test]
    fn frozen_render_duration_matches_sample_rate() {
        let render = mk_render("x", 48_000);
        assert!((render.duration_secs() - 1.0).abs() < 1e-3);
        assert!(render.is_stereo());
    }
}
