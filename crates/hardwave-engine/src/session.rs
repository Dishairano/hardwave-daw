//! The clip launcher: playing loops on top of the song, in time.
//!
//! A timeline is for writing a song down. A grid of loops is for
//! finding one, and for playing live: hit a clip, it starts on the
//! next bar; hit a row, the whole row starts together.
//!
//! What the audio thread needs is small, so it is all atomics: which
//! slot a track is playing, which one it has been asked to play, and
//! when the current one started. No locks, no allocation, no waiting.

use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Nothing is playing, or nothing is queued.
pub const NONE: i32 = -1;
/// Queued: stop at the next boundary rather than start something.
pub const STOP: i32 = -2;

/// One track's place in the grid.
#[derive(Debug, Default)]
pub struct TrackSession {
    /// The slot being played, or `NONE`.
    pub playing: AtomicI32,
    /// What was asked for and is waiting for the next boundary.
    pub pending: AtomicI32,
    /// Where the playing slot started, in samples on the transport's
    /// clock, so a loop knows how far into itself it is.
    pub launch_sample: AtomicU64,
}

impl TrackSession {
    pub fn new() -> Self {
        Self {
            playing: AtomicI32::new(NONE),
            pending: AtomicI32::new(NONE),
            launch_sample: AtomicU64::new(0),
        }
    }

    /// Ask for a slot. It starts at the next boundary, which is what
    /// keeps a launcher in time with everything already going.
    pub fn queue(&self, slot: i32) {
        self.pending.store(slot, Ordering::Relaxed);
    }

    pub fn queue_stop(&self) {
        self.pending.store(STOP, Ordering::Relaxed);
    }

    /// Stop now, without waiting for the bar. For the stop button,
    /// where waiting is not what anyone means.
    pub fn stop_now(&self) {
        self.pending.store(NONE, Ordering::Relaxed);
        self.playing.store(NONE, Ordering::Relaxed);
    }

    pub fn playing_slot(&self) -> Option<usize> {
        let slot = self.playing.load(Ordering::Relaxed);
        (slot >= 0).then_some(slot as usize)
    }

    pub fn pending_slot(&self) -> i32 {
        self.pending.load(Ordering::Relaxed)
    }
}

/// The grid as a whole: every track's slot state, and how launches are
/// lined up.
#[derive(Debug, Default)]
pub struct SessionState {
    tracks: parking_lot::Mutex<std::collections::HashMap<String, Arc<TrackSession>>>,
    /// How a launch is lined up, in beats. Four is a bar in common
    /// time; one is a beat; zero is now, for people who mean now.
    /// Shared, because every track's node reads it each block.
    pub quantise_beats: Arc<AtomicU32>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            tracks: parking_lot::Mutex::new(std::collections::HashMap::new()),
            quantise_beats: Arc::new(AtomicU32::new(4)),
        }
    }

    /// The track's state, made the first time it is asked for. Called
    /// from commands and from graph building, never from the audio
    /// thread.
    pub fn track(&self, track_id: &str) -> Arc<TrackSession> {
        let mut tracks = self.tracks.lock();
        Arc::clone(
            tracks
                .entry(track_id.to_string())
                .or_insert_with(|| Arc::new(TrackSession::new())),
        )
    }

    /// Every track that has a state, for stopping everything and for
    /// telling the UI what is playing.
    pub fn all(&self) -> Vec<(String, Arc<TrackSession>)> {
        self.tracks
            .lock()
            .iter()
            .map(|(id, state)| (id.clone(), Arc::clone(state)))
            .collect()
    }

    /// Launch a whole row, so the tracks start together.
    pub fn queue_scene(&self, scene: usize, has_clip: impl Fn(&str) -> bool) {
        for (track_id, state) in self.all() {
            if has_clip(&track_id) {
                state.queue(scene as i32);
            } else {
                // A track with nothing in this row stops, which is what
                // makes a row a section rather than a pile.
                state.queue_stop();
            }
        }
    }

    pub fn stop_all(&self) {
        for (_, state) in self.all() {
            state.stop_now();
        }
    }
}

/// How many samples a launch is lined up to.
pub fn quantise_samples(beats: u32, tempo: f64, sample_rate: f64) -> u64 {
    if beats == 0 || tempo <= 0.0 {
        return 0;
    }
    (beats as f64 * 60.0 / tempo * sample_rate).round() as u64
}

/// The first boundary at or after `position`.
///
/// At zero the answer is the position itself: pressing a clip when the
/// transport is sitting on the bar line should not wait a whole bar.
pub fn next_boundary(position: u64, quantise: u64) -> u64 {
    if quantise == 0 {
        return position;
    }
    let past = position % quantise;
    if past == 0 {
        position
    } else {
        position + (quantise - past)
    }
}

/// Where in a loop a given moment falls.
///
/// `None` once the loop has been asked to stop and the moment is
/// before it started, which happens on the block a launch lands in.
pub fn position_in_loop(now: u64, launched_at: u64, length: u64) -> Option<u64> {
    if length == 0 || now < launched_at {
        return None;
    }
    Some((now - launched_at) % length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_at_one_twenty_is_two_seconds() {
        assert_eq!(quantise_samples(4, 120.0, 48_000.0), 96_000);
        assert_eq!(quantise_samples(1, 120.0, 48_000.0), 24_000);
        assert_eq!(quantise_samples(0, 120.0, 48_000.0), 0, "zero means now");
    }

    #[test]
    fn a_launch_waits_for_the_next_bar_but_not_a_whole_one() {
        assert_eq!(next_boundary(0, 96_000), 0, "on the line is now");
        assert_eq!(next_boundary(1, 96_000), 96_000);
        assert_eq!(next_boundary(95_999, 96_000), 96_000);
        assert_eq!(next_boundary(96_000, 96_000), 96_000);
        assert_eq!(next_boundary(96_001, 96_000), 192_000);
    }

    #[test]
    fn with_no_quantise_a_launch_is_now() {
        assert_eq!(next_boundary(12_345, 0), 12_345);
    }

    #[test]
    fn a_loop_wraps_at_its_length() {
        assert_eq!(position_in_loop(1_000, 1_000, 480), Some(0));
        assert_eq!(position_in_loop(1_479, 1_000, 480), Some(479));
        assert_eq!(position_in_loop(1_480, 1_000, 480), Some(0));
        assert_eq!(position_in_loop(999, 1_000, 480), None, "before it started");
        assert_eq!(position_in_loop(1_000, 1_000, 0), None, "a loop of nothing");
    }

    #[test]
    fn a_track_queues_then_plays() {
        let session = SessionState::new();
        let track = session.track("track-1");
        assert_eq!(track.playing_slot(), None);
        track.queue(2);
        assert_eq!(track.pending_slot(), 2);
        // The audio thread is what promotes pending to playing; here
        // we only check the handles are the same object.
        let again = session.track("track-1");
        assert_eq!(again.pending_slot(), 2);
    }

    #[test]
    fn stopping_now_clears_both() {
        let session = SessionState::new();
        let track = session.track("t");
        track.queue(1);
        track.playing.store(1, Ordering::Relaxed);
        track.stop_now();
        assert_eq!(track.playing_slot(), None);
        assert_eq!(track.pending_slot(), NONE);
    }

    #[test]
    fn a_scene_starts_the_tracks_that_have_a_clip_and_stops_the_rest() {
        let session = SessionState::new();
        let with_clip = session.track("has");
        let without = session.track("has-not");
        session.queue_scene(3, |id| id == "has");
        assert_eq!(with_clip.pending_slot(), 3);
        assert_eq!(without.pending_slot(), STOP);
    }
}
