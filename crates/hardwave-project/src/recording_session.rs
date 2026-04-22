//! Recording session model — tracks armed state, the in-progress
//! take on each armed track, live waveform peaks for the UI, elapsed
//! timer, and transport-stop / spacebar shortcut handling.
//!
//! The audio thread feeds samples into `push_samples`; the UI polls
//! `elapsed_secs`, `waveform_peaks`, and `takes` to draw the record
//! transport, timer, and per-track waveform preview.

use serde::{Deserialize, Serialize};

/// High-level recording state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Armed,
    Recording,
    Stopped,
}

/// A single armed track's in-progress take — the buffer of samples,
/// a downsampled peak trace for the waveform preview, and the
/// clip-placement info that will be committed when recording stops.
#[derive(Debug, Clone)]
pub struct RecordingTake {
    pub track_id: String,
    pub samples: Vec<f32>,
    pub waveform_peaks: Vec<(f32, f32)>,
    pub start_playhead_tick: u64,
    pub sample_rate: f32,
}

impl RecordingTake {
    pub fn new(track_id: impl Into<String>, start_playhead_tick: u64, sample_rate: f32) -> Self {
        Self {
            track_id: track_id.into(),
            samples: Vec::new(),
            waveform_peaks: Vec::new(),
            start_playhead_tick,
            sample_rate: sample_rate.max(1.0),
        }
    }

    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate
    }
}

/// Stop-reason returned by `stop()` — useful for the UI to show the
/// right messaging / highlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    RecordButton,
    TransportStop,
    Spacebar,
    Canceled,
}

/// Outcome returned by `stop()` — the per-track clip placements the
/// caller should commit to the project. Each `CommittedClip` is one
/// new audio clip on an armed track, positioned at the playhead tick
/// where recording began.
#[derive(Debug, Clone)]
pub struct CommittedClip {
    pub track_id: String,
    pub playhead_tick: u64,
    pub samples: Vec<f32>,
    pub sample_rate: f32,
}

/// The live recording session — holds armed tracks, state, and the
/// accumulating takes. Cheap to construct / destroy; new session per
/// recording pass.
#[derive(Debug, Clone)]
pub struct RecordingSession {
    state: RecordingState,
    takes: Vec<RecordingTake>,
    samples_recorded: usize,
    sample_rate: f32,
    peak_block_samples: usize,
    peak_accumulator: f32,
    peak_min: f32,
    peak_max: f32,
    peak_block_pos: usize,
}

impl RecordingSession {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            state: RecordingState::Idle,
            takes: Vec::new(),
            samples_recorded: 0,
            sample_rate: sample_rate.max(1.0),
            peak_block_samples: (sample_rate.max(1.0) / 60.0).max(64.0) as usize,
            peak_accumulator: 0.0,
            peak_min: f32::MAX,
            peak_max: f32::MIN,
            peak_block_pos: 0,
        }
    }

    pub fn state(&self) -> RecordingState {
        self.state
    }

    pub fn is_armed(&self) -> bool {
        matches!(
            self.state,
            RecordingState::Armed | RecordingState::Recording
        )
    }

    /// Arm a track. Called when the user clicks the track's record
    /// button. `playhead_tick` is where the clip will land when the
    /// session stops.
    pub fn arm_track(&mut self, track_id: impl Into<String>, playhead_tick: u64) {
        let tid = track_id.into();
        if !self.takes.iter().any(|t| t.track_id == tid) {
            self.takes
                .push(RecordingTake::new(tid, playhead_tick, self.sample_rate));
        }
        if self.state == RecordingState::Idle {
            self.state = RecordingState::Armed;
        }
    }

    pub fn armed_track_ids(&self) -> Vec<&str> {
        self.takes.iter().map(|t| t.track_id.as_str()).collect()
    }

    /// Start the recording — transitions `Armed → Recording`. No-op
    /// if not armed.
    pub fn start(&mut self) -> bool {
        if self.state == RecordingState::Armed {
            self.state = RecordingState::Recording;
            true
        } else {
            false
        }
    }

    /// Feed a block of input samples to the session. Appended to the
    /// first armed take's buffer and accumulated into the waveform
    /// peak block. Multi-track recording uses `push_samples_for` to
    /// feed each take independently.
    pub fn push_samples(&mut self, samples: &[f32]) {
        if self.state != RecordingState::Recording {
            return;
        }
        if let Some(take) = self.takes.first_mut() {
            take.samples.extend_from_slice(samples);
        }
        self.samples_recorded += samples.len();
        for &s in samples {
            self.accumulate_peak(s);
        }
    }

    /// Multi-track variant — feed `samples` into the take for
    /// `track_id`. If the take doesn't exist yet (not armed), the
    /// block is discarded.
    pub fn push_samples_for(&mut self, track_id: &str, samples: &[f32]) {
        if self.state != RecordingState::Recording {
            return;
        }
        if let Some(take) = self.takes.iter_mut().find(|t| t.track_id == track_id) {
            take.samples.extend_from_slice(samples);
            // Only count per-track once for the timer / waveform so
            // multi-track sessions don't double-advance the elapsed.
            if track_id == self.takes[0].track_id {
                self.samples_recorded += samples.len();
                for &s in samples {
                    self.accumulate_peak(s);
                }
            }
        }
    }

    /// Stop recording. Returns one `CommittedClip` per armed track —
    /// empty takes (zero samples) are dropped.
    pub fn stop(&mut self, reason: StopReason) -> Vec<CommittedClip> {
        if !matches!(
            self.state,
            RecordingState::Armed | RecordingState::Recording
        ) {
            return Vec::new();
        }
        if matches!(reason, StopReason::Canceled) {
            self.state = RecordingState::Stopped;
            self.takes.clear();
            return Vec::new();
        }
        self.state = RecordingState::Stopped;
        let takes = std::mem::take(&mut self.takes);
        takes
            .into_iter()
            .filter(|t| !t.samples.is_empty())
            .map(|t| CommittedClip {
                track_id: t.track_id,
                playhead_tick: t.start_playhead_tick,
                samples: t.samples,
                sample_rate: t.sample_rate,
            })
            .collect()
    }

    pub fn elapsed_secs(&self) -> f32 {
        self.samples_recorded as f32 / self.sample_rate
    }

    /// Format elapsed as `MM:SS.mmm` for the transport display.
    pub fn elapsed_text(&self) -> String {
        let total_ms = (self.elapsed_secs() * 1000.0).round() as u64;
        let minutes = total_ms / 60_000;
        let seconds = (total_ms % 60_000) / 1000;
        let millis = total_ms % 1000;
        format!("{:02}:{:02}.{:03}", minutes, seconds, millis)
    }

    /// Iterate the live waveform peaks for the first armed take.
    /// Each peak is `(min, max)` of a ~16 ms block of samples — tuned
    /// to ~60 frames/second so the UI can draw smoothly.
    pub fn waveform_peaks(&self) -> Option<&[(f32, f32)]> {
        self.takes.first().map(|t| t.waveform_peaks.as_slice())
    }

    pub fn takes(&self) -> &[RecordingTake] {
        &self.takes
    }

    fn accumulate_peak(&mut self, s: f32) {
        if s < self.peak_min {
            self.peak_min = s;
        }
        if s > self.peak_max {
            self.peak_max = s;
        }
        self.peak_accumulator += s * s;
        self.peak_block_pos += 1;
        if self.peak_block_pos >= self.peak_block_samples {
            if let Some(take) = self.takes.first_mut() {
                take.waveform_peaks.push((self.peak_min, self.peak_max));
            }
            self.peak_min = f32::MAX;
            self.peak_max = f32::MIN;
            self.peak_accumulator = 0.0;
            self.peak_block_pos = 0;
        }
    }
}

/// Transport event from the UI shortcut layer. Any of these should
/// stop an in-progress recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportEvent {
    Stop,
    Spacebar,
    Play,
    Seek,
}

impl TransportEvent {
    pub fn stops_recording(self) -> bool {
        matches!(self, TransportEvent::Stop | TransportEvent::Spacebar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_idle_moves_to_armed() {
        let mut s = RecordingSession::new(48_000.0);
        assert_eq!(s.state(), RecordingState::Idle);
        s.arm_track("track-1", 0);
        assert_eq!(s.state(), RecordingState::Armed);
        assert!(s.is_armed());
    }

    #[test]
    fn start_requires_armed_state() {
        let mut s = RecordingSession::new(48_000.0);
        assert!(!s.start());
        s.arm_track("track-1", 0);
        assert!(s.start());
        assert_eq!(s.state(), RecordingState::Recording);
    }

    #[test]
    fn push_samples_only_during_recording() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("t", 0);
        s.push_samples(&[0.5; 100]);
        assert_eq!(s.takes()[0].samples.len(), 0); // armed, not recording
        s.start();
        s.push_samples(&[0.5; 100]);
        assert_eq!(s.takes()[0].samples.len(), 100);
    }

    #[test]
    fn elapsed_tracks_recorded_sample_count() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("t", 0);
        s.start();
        let block = vec![0.0; 48_000]; // 1 s of samples
        s.push_samples(&block);
        assert!((s.elapsed_secs() - 1.0).abs() < 1e-3);
        assert!(s.elapsed_text().starts_with("00:01"));
    }

    #[test]
    fn waveform_peaks_accumulate_per_block() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("t", 0);
        s.start();
        // Peak block size is 800 samples (48_000 / 60). Push 10 blocks.
        for _ in 0..10 {
            s.push_samples(&[0.5; 800]);
        }
        let peaks = s.waveform_peaks().expect("peaks");
        assert_eq!(peaks.len(), 10);
        for (mn, mx) in peaks {
            assert!((*mx - 0.5).abs() < 1e-3);
            assert!((*mn - 0.5).abs() < 1e-3);
        }
    }

    #[test]
    fn stop_on_transport_commits_nonempty_takes() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("a", 100);
        s.arm_track("b", 100);
        s.start();
        s.push_samples_for("a", &[0.1; 50]);
        let clips = s.stop(StopReason::TransportStop);
        // Only track `a` has samples; `b` stays empty + filters out.
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].track_id, "a");
        assert_eq!(clips[0].samples.len(), 50);
        assert_eq!(clips[0].playhead_tick, 100);
        assert_eq!(s.state(), RecordingState::Stopped);
    }

    #[test]
    fn stop_canceled_throws_away_all_takes() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("a", 0);
        s.start();
        s.push_samples(&[0.3; 200]);
        let clips = s.stop(StopReason::Canceled);
        assert_eq!(clips.len(), 0);
        assert!(s.takes().is_empty());
    }

    #[test]
    fn transport_events_classify_stops() {
        assert!(TransportEvent::Stop.stops_recording());
        assert!(TransportEvent::Spacebar.stops_recording());
        assert!(!TransportEvent::Play.stops_recording());
        assert!(!TransportEvent::Seek.stops_recording());
    }

    #[test]
    fn multi_track_push_is_per_track() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("a", 0);
        s.arm_track("b", 0);
        s.start();
        s.push_samples_for("a", &[0.1; 100]);
        s.push_samples_for("b", &[0.2; 50]);
        assert_eq!(s.takes()[0].samples.len(), 100);
        assert_eq!(s.takes()[1].samples.len(), 50);
    }

    #[test]
    fn arming_same_track_twice_is_idempotent() {
        let mut s = RecordingSession::new(48_000.0);
        s.arm_track("a", 0);
        s.arm_track("a", 0);
        assert_eq!(s.takes().len(), 1);
    }
}
