//! Recording state machine + ring buffer primitive. Encodes the
//! Idle → Armed → Recording → Stopped transitions and accumulates
//! captured audio samples into a `Vec<f32>` (interleaved stereo).
//! The plugin / engine layer wraps this with audio callback routing
//! and WAV file writing; the primitive itself is just the data
//! model + state transitions + timing book-keeping.
//!
//! Also provides `LoopMode` + `TakeList` for loop recording (overdub,
//! replace, stacked) and `write_wav` for dumping the captured buffer
//! to a 32-bit-float WAV file via the `hound` crate.

use hound::{SampleFormat, WavSpec, WavWriter};
use std::path::Path;

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

/// Loop recording behavior when the transport wraps mid-recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    /// No loop recording — treat each wrap as a continuation.
    Off,
    /// Overdub — later passes layer additively on top of earlier ones.
    Overdub,
    /// Replace — each new pass overwrites the previous captured audio.
    Replace,
    /// Stacked takes — each pass is saved as a separate take, selectable
    /// later via a take-comp UI.
    Stacked,
}

/// A single take captured during a loop-recording pass.
#[derive(Debug, Clone)]
pub struct Take {
    pub samples: Vec<f32>,
    pub start_sample: u64,
    pub end_sample: u64,
    pub channels: u8,
}

/// Accumulator for multi-pass loop recordings. Each call to
/// `on_loop_wrap(current_sample, mode)` finalizes the current pass
/// per the active `LoopMode` policy.
pub struct TakeList {
    takes: Vec<Take>,
    channels: u8,
    current: Vec<f32>,
    current_start: u64,
}

impl TakeList {
    pub fn new(_sample_rate: u32, channels: u8) -> Self {
        Self {
            takes: Vec::new(),
            channels: channels.clamp(1, 2),
            current: Vec::new(),
            current_start: 0,
        }
    }

    pub fn take_count(&self) -> usize {
        self.takes.len()
    }

    pub fn takes(&self) -> &[Take] {
        &self.takes
    }

    pub fn clear(&mut self) {
        self.takes.clear();
        self.current.clear();
        self.current_start = 0;
    }

    /// Start a new take at the given sample position.
    pub fn start_take(&mut self, start_sample: u64) {
        self.current.clear();
        self.current_start = start_sample;
    }

    pub fn push_stereo(&mut self, l: f32, r: f32) {
        if self.channels == 1 {
            self.current.push((l + r) * 0.5);
        } else {
            self.current.push(l);
            self.current.push(r);
        }
    }

    pub fn current_frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.current.len() / self.channels as usize
        }
    }

    /// Called when the loop wraps. `mode` decides what to do with the
    /// just-captured pass: Off finalizes and stops; Overdub mixes the
    /// new pass into the most recent take; Replace drops the previous
    /// take and keeps only this one; Stacked appends the new take to
    /// the take list.
    pub fn on_loop_wrap(&mut self, next_start_sample: u64, mode: LoopMode) {
        let end = self.current_start + self.current_frame_count() as u64;
        let new_take = Take {
            samples: std::mem::take(&mut self.current),
            start_sample: self.current_start,
            end_sample: end,
            channels: self.channels,
        };
        match mode {
            LoopMode::Off => {
                self.takes.push(new_take);
            }
            LoopMode::Overdub => {
                if let Some(last) = self.takes.last_mut() {
                    let n = last.samples.len().min(new_take.samples.len());
                    for i in 0..n {
                        last.samples[i] += new_take.samples[i];
                    }
                    if new_take.samples.len() > last.samples.len() {
                        last.samples.extend_from_slice(&new_take.samples[n..]);
                    }
                } else {
                    self.takes.push(new_take);
                }
            }
            LoopMode::Replace => {
                if !self.takes.is_empty() {
                    self.takes.pop();
                }
                self.takes.push(new_take);
            }
            LoopMode::Stacked => {
                self.takes.push(new_take);
            }
        }
        self.current_start = next_start_sample;
    }

    /// Finalize whatever is currently being captured as a final take.
    pub fn finalize(&mut self, mode: LoopMode) {
        if self.current.is_empty() {
            return;
        }
        // Reuse on_loop_wrap for the policy, treating the final pass
        // as another wrap with next_start = end of current take.
        let next = self.current_start + self.current_frame_count() as u64;
        self.on_loop_wrap(next, mode);
    }
}

/// Punch-in / punch-out window — the tick range during which the
/// recorder should actually capture audio. Outside the window, the
/// engine skips `push_stereo` calls even if the RecordingSession is
/// in the Recording state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PunchWindow {
    pub punch_in_tick: u64,
    pub punch_out_tick: u64,
    pub enabled: bool,
}

impl Default for PunchWindow {
    fn default() -> Self {
        Self {
            punch_in_tick: 0,
            punch_out_tick: u64::MAX,
            enabled: false,
        }
    }
}

impl PunchWindow {
    pub fn new(punch_in_tick: u64, punch_out_tick: u64) -> Self {
        Self {
            punch_in_tick,
            punch_out_tick: punch_out_tick.max(punch_in_tick),
            enabled: true,
        }
    }

    pub fn set_in(&mut self, tick: u64) {
        self.punch_in_tick = tick;
        if self.punch_out_tick < tick {
            self.punch_out_tick = tick;
        }
    }

    pub fn set_out(&mut self, tick: u64) {
        self.punch_out_tick = tick.max(self.punch_in_tick);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// True if recording should be active at this tick.
    pub fn should_record(&self, tick: u64) -> bool {
        if !self.enabled {
            return true;
        }
        tick >= self.punch_in_tick && tick < self.punch_out_tick
    }

    /// True if `current_tick` just crossed the punch-out boundary
    /// since `prev_tick` — the engine uses this to auto-stop the
    /// recording session when playback passes the punch-out mark.
    pub fn crossed_out(&self, prev_tick: u64, current_tick: u64) -> bool {
        self.enabled && prev_tick < self.punch_out_tick && current_tick >= self.punch_out_tick
    }

    /// True if `current_tick` just crossed the punch-in boundary —
    /// the engine uses this to auto-start recording.
    pub fn crossed_in(&self, prev_tick: u64, current_tick: u64) -> bool {
        self.enabled && prev_tick < self.punch_in_tick && current_tick >= self.punch_in_tick
    }
}

/// A single slice chosen from a take during comp assembly.
/// `(take_index, start_frame, length_frames)` — include this many
/// frames from this take starting at this frame offset.
#[derive(Debug, Clone, Copy)]
pub struct CompSlice {
    pub take_index: usize,
    pub start_frame: usize,
    pub length_frames: usize,
}

/// Assemble a final comp'd take from a list of `Take`s and a
/// selection of slices from each. Slices are concatenated in order
/// into one interleaved `Vec<f32>`. Frames that fall outside a
/// take's bounds are silenced. Channels are taken from the first
/// take; if later takes have different channel counts they're
/// zero-padded to match.
pub fn assemble_comp_take(takes: &[Take], slices: &[CompSlice]) -> Option<Take> {
    let first = takes.first()?;
    let channels = first.channels.max(1) as usize;
    let mut out_samples: Vec<f32> = Vec::new();
    let mut min_start: Option<u64> = None;
    let mut max_end: u64 = 0;
    for slice in slices {
        let Some(take) = takes.get(slice.take_index) else {
            // Skip invalid take index — silent.
            out_samples.resize(
                out_samples.len() + slice.length_frames * channels,
                0.0,
            );
            continue;
        };
        let take_channels = take.channels.max(1) as usize;
        let start_sample = slice.start_frame * take_channels;
        let slice_start_u64 = take.start_sample + slice.start_frame as u64;
        if min_start.is_none_or(|m| slice_start_u64 < m) {
            min_start = Some(slice_start_u64);
        }
        let slice_end_u64 = slice_start_u64 + slice.length_frames as u64;
        if slice_end_u64 > max_end {
            max_end = slice_end_u64;
        }
        for f in 0..slice.length_frames {
            let src_base = start_sample + f * take_channels;
            for ch in 0..channels {
                let src_idx = src_base + ch.min(take_channels - 1);
                let val = take.samples.get(src_idx).copied().unwrap_or(0.0);
                out_samples.push(val);
            }
        }
    }
    Some(Take {
        samples: out_samples,
        start_sample: min_start.unwrap_or(0),
        end_sample: max_end,
        channels: channels as u8,
    })
}

/// Write the captured samples from a RecordingSession as a 32-bit
/// float WAV file. Returns the number of samples written, or an error
/// if the file can't be created / written.
pub fn write_wav<P: AsRef<Path>>(session: &RecordingSession, path: P) -> Result<usize, String> {
    let spec = WavSpec {
        channels: session.channels() as u16,
        sample_rate: session.sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(path, spec).map_err(|e| format!("create wav: {e}"))?;
    let samples = session.captured();
    for &s in samples {
        writer
            .write_sample(s)
            .map_err(|e| format!("write sample: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("finalize wav: {e}"))?;
    Ok(samples.len())
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

    #[test]
    fn take_list_stacked_mode_preserves_all_passes() {
        let mut tl = TakeList::new(48_000, 1);
        tl.start_take(0);
        for _ in 0..10 {
            tl.push_stereo(1.0, 1.0);
        }
        tl.on_loop_wrap(10, LoopMode::Stacked);

        tl.start_take(10);
        for _ in 0..10 {
            tl.push_stereo(0.5, 0.5);
        }
        tl.on_loop_wrap(20, LoopMode::Stacked);

        assert_eq!(tl.take_count(), 2);
        assert_eq!(tl.takes()[0].samples[0], 1.0);
        assert_eq!(tl.takes()[1].samples[0], 0.5);
    }

    #[test]
    fn take_list_replace_mode_keeps_only_last_pass() {
        let mut tl = TakeList::new(48_000, 1);
        tl.start_take(0);
        for _ in 0..10 {
            tl.push_stereo(1.0, 1.0);
        }
        tl.on_loop_wrap(10, LoopMode::Replace);

        tl.start_take(10);
        for _ in 0..10 {
            tl.push_stereo(0.5, 0.5);
        }
        tl.on_loop_wrap(20, LoopMode::Replace);

        assert_eq!(tl.take_count(), 1);
        assert_eq!(tl.takes()[0].samples[0], 0.5);
    }

    #[test]
    fn take_list_overdub_mode_sums_passes() {
        let mut tl = TakeList::new(48_000, 1);
        tl.start_take(0);
        for _ in 0..10 {
            tl.push_stereo(1.0, 1.0);
        }
        tl.on_loop_wrap(10, LoopMode::Overdub);

        tl.start_take(10);
        for _ in 0..10 {
            tl.push_stereo(0.5, 0.5);
        }
        tl.on_loop_wrap(20, LoopMode::Overdub);

        assert_eq!(tl.take_count(), 1);
        // First sample: 1.0 + 0.5 = 1.5.
        assert!((tl.takes()[0].samples[0] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn assemble_comp_take_concatenates_slices_from_multiple_takes() {
        // Two mono takes, easy to identify by value.
        let take_a = Take {
            samples: vec![1.0; 100],
            start_sample: 0,
            end_sample: 100,
            channels: 1,
        };
        let take_b = Take {
            samples: vec![2.0; 100],
            start_sample: 0,
            end_sample: 100,
            channels: 1,
        };
        let takes = vec![take_a, take_b];
        let slices = vec![
            CompSlice {
                take_index: 0,
                start_frame: 0,
                length_frames: 50,
            },
            CompSlice {
                take_index: 1,
                start_frame: 50,
                length_frames: 50,
            },
        ];
        let comp = assemble_comp_take(&takes, &slices).expect("comp built");
        assert_eq!(comp.samples.len(), 100);
        // First 50 should be 1.0 (from take A), last 50 should be 2.0.
        for i in 0..50 {
            assert_eq!(comp.samples[i], 1.0);
        }
        for i in 50..100 {
            assert_eq!(comp.samples[i], 2.0);
        }
    }

    #[test]
    fn assemble_comp_take_with_invalid_index_produces_silence() {
        let take = Take {
            samples: vec![1.0; 50],
            start_sample: 0,
            end_sample: 50,
            channels: 1,
        };
        let slices = vec![CompSlice {
            take_index: 99,
            start_frame: 0,
            length_frames: 25,
        }];
        let comp = assemble_comp_take(&[take], &slices).expect("comp built");
        for s in &comp.samples {
            assert_eq!(*s, 0.0);
        }
        assert_eq!(comp.samples.len(), 25);
    }

    #[test]
    fn punch_window_only_records_in_range() {
        let w = PunchWindow::new(100, 500);
        assert!(!w.should_record(50));
        assert!(w.should_record(100));
        assert!(w.should_record(499));
        assert!(!w.should_record(500));
        assert!(!w.should_record(1000));
    }

    #[test]
    fn punch_window_disabled_always_records() {
        let w = PunchWindow::default();
        assert!(w.should_record(0));
        assert!(w.should_record(u64::MAX / 2));
    }

    #[test]
    fn punch_window_detects_boundary_crossings() {
        let w = PunchWindow::new(100, 500);
        assert!(w.crossed_in(99, 100));
        assert!(!w.crossed_in(98, 99));
        assert!(w.crossed_out(499, 500));
        assert!(!w.crossed_out(498, 499));
    }

    #[test]
    fn punch_window_sanitizes_reversed_range() {
        // If someone sets out < in, out clamps up to in.
        let mut w = PunchWindow::new(500, 200);
        assert_eq!(w.punch_out_tick, 500);
        // Setting a new in point above out also fixes out.
        w.set_in(700);
        assert_eq!(w.punch_out_tick, 700);
    }

    #[test]
    fn write_wav_round_trips_samples() {
        let mut s = RecordingSession::new(48_000, 2);
        s.arm();
        s.start(0);
        for i in 0..1000 {
            let v = (i as f32 / 1000.0).sin();
            s.push_stereo(v, v * 0.5);
        }
        s.stop();
        let tmp = std::env::temp_dir().join("hardwave_recording_roundtrip.wav");
        let n = write_wav(&s, &tmp).expect("wav write");
        assert_eq!(n, 2000); // 1000 frames × 2 channels
                             // Verify file exists and has non-zero size.
        let meta = std::fs::metadata(&tmp).expect("wav file metadata");
        assert!(meta.len() > 0);
        let _ = std::fs::remove_file(&tmp);
    }
}
