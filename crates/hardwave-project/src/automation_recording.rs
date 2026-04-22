//! Automation recording — capture knob/slider movements during
//! playback and convert them into automation points. Supports the
//! classic Write / Touch / Latch modes plus an Overwrite mode that
//! replaces existing points in the recorded range.
//!
//! This is a pure data-layer primitive: the UI / engine feeds
//! parameter values + playhead timestamps in, and the primitive
//! produces `AutomationPoint`s that can be attached to a lane.

use crate::automation::{AutomationPoint, CurveMode};

/// Automation recording mode. Matches the classic DAW terminology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Recording is off — values pass through without being captured.
    Off,
    /// Read — current automation plays back, knob movements are ignored.
    Read,
    /// Write — all knob movements are captured while playing; existing
    /// automation in the recorded range is replaced.
    Write,
    /// Touch — only captures values while the user is actively touching
    /// the control (i.e. between `begin_touch` and `end_touch`).
    Touch,
    /// Latch — starts capturing on first touch, continues until stop.
    Latch,
}

/// A running automation-recording session for one parameter. Collects
/// `(tick, value)` samples and produces `AutomationPoint`s on demand.
pub struct AutomationRecorder {
    mode: WriteMode,
    samples: Vec<(u64, f64)>,
    touching: bool,
    recording: bool,
}

impl Default for AutomationRecorder {
    fn default() -> Self {
        Self {
            mode: WriteMode::Off,
            samples: Vec::new(),
            touching: false,
            recording: false,
        }
    }
}

impl AutomationRecorder {
    pub fn set_mode(&mut self, mode: WriteMode) {
        self.mode = mode;
        if matches!(mode, WriteMode::Off | WriteMode::Read) {
            self.recording = false;
            self.touching = false;
        }
    }

    pub fn mode(&self) -> WriteMode {
        self.mode
    }

    /// Transport starts playing — recording becomes active for Write
    /// mode; Touch and Latch modes still need a begin_touch() to
    /// actually start capturing.
    pub fn on_transport_play(&mut self) {
        match self.mode {
            WriteMode::Write => self.recording = true,
            _ => self.recording = false,
        }
    }

    /// Transport stopped — all capture halts. Latch mode's recording
    /// state is also cleared even if the user is still holding the
    /// control.
    pub fn on_transport_stop(&mut self) {
        self.recording = false;
        self.touching = false;
    }

    /// User began touching the control — for Touch and Latch modes,
    /// this is when capture actually starts.
    pub fn begin_touch(&mut self) {
        self.touching = true;
        match self.mode {
            WriteMode::Touch => self.recording = true,
            WriteMode::Latch => {
                // Latch only starts recording on first touch during
                // playback; once started, it stays on until stop.
                self.recording = true;
            }
            _ => {}
        }
    }

    /// User released the control — Touch mode stops capturing here;
    /// Latch mode keeps going.
    pub fn end_touch(&mut self) {
        self.touching = false;
        if self.mode == WriteMode::Touch {
            self.recording = false;
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Feed one `(tick, value)` sample. No-op unless the recorder is
    /// actively recording.
    pub fn push_sample(&mut self, tick: u64, value: f64) {
        if self.recording {
            self.samples.push((tick, value.clamp(0.0, 1.0)));
        }
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Thin consecutive samples whose values differ by less than
    /// `tolerance`. Keeps endpoints. Useful after a live capture
    /// dumps thousands of samples that compress into a handful of
    /// meaningful inflection points.
    pub fn thin(&mut self, tolerance: f64) {
        if self.samples.len() <= 2 {
            return;
        }
        let mut kept: Vec<(u64, f64)> = Vec::with_capacity(self.samples.len());
        kept.push(self.samples[0]);
        for i in 1..self.samples.len() - 1 {
            let prev_val = kept.last().unwrap().1;
            if (self.samples[i].1 - prev_val).abs() >= tolerance {
                kept.push(self.samples[i]);
            }
        }
        if let Some(&last) = self.samples.last() {
            kept.push(last);
        }
        self.samples = kept;
    }

    /// Convert captured samples into `AutomationPoint`s with the
    /// specified curve mode. Caller typically calls `thin` first to
    /// reduce density before emitting.
    pub fn into_points(&self, curve: CurveMode) -> Vec<AutomationPoint> {
        self.samples
            .iter()
            .map(|(tick, value)| AutomationPoint {
                tick: *tick,
                value: *value,
                curve,
                tension: 0.0,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_mode_never_records() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Off);
        rec.on_transport_play();
        for tick in 0..100 {
            rec.push_sample(tick * 100, (tick as f64) * 0.01);
        }
        assert_eq!(rec.sample_count(), 0);
    }

    #[test]
    fn write_mode_captures_during_playback() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Write);
        rec.on_transport_play();
        assert!(rec.is_recording());
        for tick in 0..100 {
            rec.push_sample(tick * 100, (tick as f64) * 0.01);
        }
        assert_eq!(rec.sample_count(), 100);
    }

    #[test]
    fn touch_mode_requires_active_touch() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Touch);
        rec.on_transport_play();
        // No touch yet — not recording.
        rec.push_sample(0, 0.5);
        assert_eq!(rec.sample_count(), 0);
        // Touch begins — now recording.
        rec.begin_touch();
        rec.push_sample(100, 0.6);
        rec.push_sample(200, 0.7);
        assert_eq!(rec.sample_count(), 2);
        // Touch ends — recording stops.
        rec.end_touch();
        rec.push_sample(300, 0.8);
        assert_eq!(rec.sample_count(), 2);
    }

    #[test]
    fn latch_mode_records_from_first_touch_until_stop() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Latch);
        rec.on_transport_play();
        rec.push_sample(0, 0.5);
        assert_eq!(rec.sample_count(), 0);
        rec.begin_touch();
        rec.push_sample(100, 0.6);
        rec.end_touch();
        // Latch keeps recording even after touch ends.
        rec.push_sample(200, 0.7);
        assert_eq!(rec.sample_count(), 2);
        rec.on_transport_stop();
        rec.push_sample(300, 0.8);
        assert_eq!(rec.sample_count(), 2);
    }

    #[test]
    fn thin_keeps_endpoints_and_collapses_near_constants() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Write);
        rec.on_transport_play();
        // 10 samples at 0.5 with tiny variations.
        rec.push_sample(0, 0.1);
        for i in 1..10 {
            rec.push_sample(i * 100, 0.5 + (i as f64) * 0.001);
        }
        rec.push_sample(1000, 0.9);
        assert_eq!(rec.sample_count(), 11);
        rec.thin(0.05);
        // After thinning: kept[0] = first (0, 0.1), then one of the
        // mid samples with step > 0.05 from 0.1 (which is 0.5+), then
        // the endpoint (1000, 0.9). Details depend on thin() exactly.
        assert!(rec.sample_count() < 11);
        assert!(rec.sample_count() >= 2); // at minimum the endpoints
                                          // First and last must survive.
        let points = rec.into_points(CurveMode::Linear);
        assert_eq!(points.first().unwrap().tick, 0);
        assert_eq!(points.last().unwrap().tick, 1000);
    }

    #[test]
    fn into_points_produces_automation_points() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Write);
        rec.on_transport_play();
        rec.push_sample(0, 0.1);
        rec.push_sample(100, 0.5);
        rec.push_sample(200, 0.9);
        let points = rec.into_points(CurveMode::Bezier);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].tick, 0);
        assert_eq!(points[0].value, 0.1);
        assert!(matches!(points[1].curve, CurveMode::Bezier));
    }

    #[test]
    fn transport_stop_halts_recording() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Write);
        rec.on_transport_play();
        rec.push_sample(0, 0.5);
        rec.on_transport_stop();
        rec.push_sample(100, 0.8);
        assert_eq!(rec.sample_count(), 1);
    }

    #[test]
    fn clear_resets_samples() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Write);
        rec.on_transport_play();
        rec.push_sample(0, 0.5);
        rec.push_sample(100, 0.8);
        assert_eq!(rec.sample_count(), 2);
        rec.clear();
        assert_eq!(rec.sample_count(), 0);
    }

    #[test]
    fn push_clamps_values_to_normalized_range() {
        let mut rec = AutomationRecorder::default();
        rec.set_mode(WriteMode::Write);
        rec.on_transport_play();
        rec.push_sample(0, -1.0);
        rec.push_sample(100, 2.0);
        let pts = rec.into_points(CurveMode::Linear);
        assert_eq!(pts[0].value, 0.0);
        assert_eq!(pts[1].value, 1.0);
    }
}
