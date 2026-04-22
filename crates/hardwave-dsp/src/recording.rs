//! Recording state machine + ring buffer primitive. Encodes the
//! Idle → Armed → Recording → Stopped transitions and accumulates
//! captured audio samples into a `Vec<f32>` (interleaved stereo).
//! The plugin / engine layer wraps this with audio callback routing
//! and WAV file writing; the primitive itself is just the data
//! model + state transitions + timing book-keeping.

/// Recording state machine. Transitions are explicit: `Idle` →
/// `arm()` → `Armed` → `start()` → `Recording` → `stop()` →
/// `Stopped`. `cancel()` jumps from any state back to `Idle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Armed,
    Recording,
    Stopped,
}

/// A recording session. Holds the captured samples plus transport
/// metadata for placing the resulting clip at the right timeline
/// position.
pub struct RecordingSession {
    state: RecordingState,
    /// Interleaved stereo samples captured so far.
    captured: Vec<f32>,
    /// Engine sample position when recording started.
    start_sample: u64,
    /// Sample rate for timing conversions.
    sample_rate: u32,
    /// Number of channels in the captured buffer (1 or 2).
    channels: u8,
}

impl RecordingSession {
    pub fn new(sample_rate: u32, channels: u8) -> Self {
        Self {
            state: RecordingState::Idle,
            captured: Vec::new(),
            start_sample: 0,
            sample_rate,
            channels: channels.clamp(1, 2),
        }
    }

    pub fn state(&self) -> RecordingState {
        self.state
    }

    /// Move from Idle to Armed. No-op if already Armed or Recording.
    pub fn arm(&mut self) {
        match self.state {
            RecordingState::Idle | RecordingState::Stopped => {
                self.state = RecordingState::Armed;
            }
            _ => {}
        }
    }

    /// Disarm back to Idle. No-op if not Armed.
    pub fn disarm(&mut self) {
        if self.state == RecordingState::Armed {
            self.state = RecordingState::Idle;
        }
    }

    /// Begin recording at the given engine sample position.
    /// Requires Armed state.
    pub fn start(&mut self, position_samples: u64) {
        if self.state == RecordingState::Armed {
            self.state = RecordingState::Recording;
            self.captured.clear();
            self.start_sample = position_samples;
        }
    }

    /// Stop recording. Moves to Stopped; captured buffer retains
    /// the recording for the caller to consume.
    pub fn stop(&mut self) {
        if self.state == RecordingState::Recording {
            self.state = RecordingState::Stopped;
        }
    }

    /// Cancel a recording — discard captured samples, back to Idle.
    pub fn cancel(&mut self) {
        self.captured.clear();
        self.state = RecordingState::Idle;
    }

    /// Push one stereo frame to the capture buffer. Only active
    /// in Recording state; silently ignored otherwise.
    pub fn push_stereo(&mut self, l: f32, r: f32) {
        if self.state != RecordingState::Recording {
            return;
        }
        if self.channels == 1 {
            self.captured.push((l + r) * 0.5);
        } else {
            self.captured.push(l);
            self.captured.push(r);
        }
    }

    /// Push a mono sample. Duplicates to both channels for stereo
    /// captures; stored directly for mono.
    pub fn push_mono(&mut self, sample: f32) {
        if self.state != RecordingState::Recording {
            return;
        }
        self.captured.push(sample);
        if self.channels == 2 {
            self.captured.push(sample);
        }
    }

    /// Number of audio frames captured (not samples — 1 frame = 1
    /// stereo pair for channels=2, 1 mono sample for channels=1).
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.captured.len() / (self.channels as usize)
        }
    }

    /// Captured duration in seconds based on frame count / sample rate.
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frame_count() as f32 / self.sample_rate as f32
        }
    }

    /// Sample position the recording started at.
    pub fn start_sample(&self) -> u64 {
        self.start_sample
    }

    pub fn channels(&self) -> u8 {
        self.channels
    }

    pub fn captured(&self) -> &[f32] {
        &self.captured
    }

    /// Consume the session: return the captured samples and reset.
    pub fn take_captured(&mut self) -> Vec<f32> {
        let out = std::mem::take(&mut self.captured);
        self.state = RecordingState::Idle;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_idle() {
        let s = RecordingSession::new(48_000, 2);
        assert_eq!(s.state(), RecordingState::Idle);
    }

    #[test]
    fn arm_transitions_idle_to_armed() {
        let mut s = RecordingSession::new(48_000, 2);
        s.arm();
        assert_eq!(s.state(), RecordingState::Armed);
        // Arm from Armed is a no-op.
        s.arm();
        assert_eq!(s.state(), RecordingState::Armed);
    }

    #[test]
    fn cannot_start_from_idle() {
        let mut s = RecordingSession::new(48_000, 2);
        s.start(12345);
        assert_eq!(s.state(), RecordingState::Idle);
    }

    #[test]
    fn full_lifecycle_idle_armed_recording_stopped() {
        let mut s = RecordingSession::new(48_000, 2);
        assert_eq!(s.state(), RecordingState::Idle);
        s.arm();
        assert_eq!(s.state(), RecordingState::Armed);
        s.start(1000);
        assert_eq!(s.state(), RecordingState::Recording);
        assert_eq!(s.start_sample(), 1000);
        for i in 0..100 {
            s.push_stereo(i as f32 * 0.01, -(i as f32 * 0.01));
        }
        s.stop();
        assert_eq!(s.state(), RecordingState::Stopped);
        assert_eq!(s.frame_count(), 100);
        assert_eq!(s.captured().len(), 200); // stereo = 2 samples per frame
    }

    #[test]
    fn cancel_discards_captured_audio() {
        let mut s = RecordingSession::new(48_000, 2);
        s.arm();
        s.start(0);
        for _ in 0..50 {
            s.push_stereo(1.0, 1.0);
        }
        assert_eq!(s.frame_count(), 50);
        s.cancel();
        assert_eq!(s.state(), RecordingState::Idle);
        assert_eq!(s.frame_count(), 0);
    }

    #[test]
    fn mono_session_sums_stereo_to_mono() {
        let mut s = RecordingSession::new(48_000, 1);
        s.arm();
        s.start(0);
        s.push_stereo(0.8, 0.4);
        s.stop();
        assert_eq!(s.captured().len(), 1);
        assert!((s.captured()[0] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn stereo_session_duplicates_mono_input() {
        let mut s = RecordingSession::new(48_000, 2);
        s.arm();
        s.start(0);
        s.push_mono(0.5);
        s.stop();
        assert_eq!(s.captured(), &[0.5, 0.5]);
    }

    #[test]
    fn push_outside_recording_is_ignored() {
        let mut s = RecordingSession::new(48_000, 2);
        // Idle — push should do nothing.
        s.push_stereo(1.0, 1.0);
        assert_eq!(s.captured().len(), 0);
        // Armed — push should still do nothing (transport hasn't started).
        s.arm();
        s.push_stereo(1.0, 1.0);
        assert_eq!(s.captured().len(), 0);
    }

    #[test]
    fn take_captured_resets_to_idle() {
        let mut s = RecordingSession::new(48_000, 2);
        s.arm();
        s.start(0);
        for _ in 0..10 {
            s.push_stereo(0.5, 0.5);
        }
        let data = s.take_captured();
        assert_eq!(data.len(), 20);
        assert_eq!(s.state(), RecordingState::Idle);
        assert_eq!(s.frame_count(), 0);
    }

    #[test]
    fn duration_secs_matches_frame_count() {
        let mut s = RecordingSession::new(48_000, 2);
        s.arm();
        s.start(0);
        for _ in 0..24_000 {
            s.push_stereo(0.0, 0.0);
        }
        s.stop();
        let dur = s.duration_secs();
        assert!((dur - 0.5).abs() < 1e-3, "duration = {dur}");
    }
}
