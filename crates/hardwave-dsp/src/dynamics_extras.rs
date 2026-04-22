//! Extras for the dynamics processors — makeup / input gain with
//! smoothing, pre/post peak + RMS meters, sidechain HPF/LPF pre-
//! filters, look-ahead delay for compressor / limiter side chains,
//! and gate hold-time state tracking.

use std::f32::consts::PI;

/// Smoothed dB gain control — re-uses a one-pole smoother on the
/// linear gain so ramp changes don't click. Works for both makeup
/// gain (post) and input gain (pre).
pub struct SmoothGain {
    target_linear: f32,
    current_linear: f32,
    alpha: f32,
}

impl SmoothGain {
    pub fn new(sample_rate: f32, smoothing_ms: f32, initial_db: f32) -> Self {
        let initial = db_to_linear(initial_db);
        let mut g = Self {
            target_linear: initial,
            current_linear: initial,
            alpha: 0.0,
        };
        g.set_smoothing(sample_rate, smoothing_ms);
        g
    }

    pub fn set_smoothing(&mut self, sample_rate: f32, smoothing_ms: f32) {
        let tau = (smoothing_ms * 0.001).max(1e-4);
        let dt = 1.0 / sample_rate.max(1.0);
        self.alpha = 1.0 - (-dt / tau).exp();
    }

    pub fn set_db(&mut self, db: f32) {
        self.target_linear = db_to_linear(db);
    }

    pub fn tick(&mut self, sample: f32) -> f32 {
        self.current_linear += (self.target_linear - self.current_linear) * self.alpha;
        sample * self.current_linear
    }

    pub fn current_db(&self) -> f32 {
        linear_to_db(self.current_linear.max(1e-9))
    }
}

/// Simple peak meter — max absolute value over a window with
/// exponential decay between samples. Drop-in for compressor /
/// limiter / mixer input-output strips.
pub struct PeakMeter {
    current: f32,
    decay: f32,
}

impl PeakMeter {
    pub fn new(sample_rate: f32, decay_ms: f32) -> Self {
        Self {
            current: 0.0,
            decay: decay_coefficient(sample_rate, decay_ms),
        }
    }

    pub fn set_decay(&mut self, sample_rate: f32, decay_ms: f32) {
        self.decay = decay_coefficient(sample_rate, decay_ms);
    }

    pub fn tick(&mut self, sample: f32) -> f32 {
        let a = sample.abs();
        if a > self.current {
            self.current = a;
        } else {
            self.current *= self.decay;
        }
        self.current
    }

    pub fn current_db(&self) -> f32 {
        linear_to_db(self.current.max(1e-9))
    }

    pub fn reset(&mut self) {
        self.current = 0.0;
    }
}

/// Windowed RMS meter with one-pole averaging on the squared signal.
pub struct RmsMeter {
    sum_sq: f32,
    alpha: f32,
}

impl RmsMeter {
    pub fn new(sample_rate: f32, window_ms: f32) -> Self {
        Self {
            sum_sq: 0.0,
            alpha: decay_coefficient(sample_rate, window_ms),
        }
    }

    pub fn tick(&mut self, sample: f32) -> f32 {
        let s2 = sample * sample;
        self.sum_sq = self.alpha * self.sum_sq + (1.0 - self.alpha) * s2;
        self.sum_sq.sqrt()
    }

    pub fn current_db(&self) -> f32 {
        linear_to_db(self.sum_sq.sqrt().max(1e-9))
    }
}

/// One-pole HPF — cheap sidechain pre-filter. Feed the detection
/// signal through this to stop kick / bass from triggering the
/// compressor on unrelated mid content.
pub struct OnePoleHpf {
    prev_in: f32,
    prev_out: f32,
    a: f32,
}

impl OnePoleHpf {
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self {
            prev_in: 0.0,
            prev_out: 0.0,
            a: one_pole_alpha(sample_rate, cutoff_hz),
        }
    }

    pub fn set_cutoff(&mut self, sample_rate: f32, cutoff_hz: f32) {
        self.a = one_pole_alpha(sample_rate, cutoff_hz);
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.a * (self.prev_out + x - self.prev_in);
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}

/// One-pole LPF — counterpart to `OnePoleHpf` for gate side chains
/// (e.g. key the gate only on drum low-end).
pub struct OnePoleLpf {
    prev_out: f32,
    a: f32,
}

impl OnePoleLpf {
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        let mut f = Self {
            prev_out: 0.0,
            a: 0.0,
        };
        f.set_cutoff(sample_rate, cutoff_hz);
        f
    }

    pub fn set_cutoff(&mut self, sample_rate: f32, cutoff_hz: f32) {
        // Canonical one-pole low-pass: alpha = dt / (RC + dt).
        let dt = 1.0 / sample_rate.max(1.0);
        let rc = 1.0 / (2.0 * PI * cutoff_hz.max(1e-3));
        self.a = dt / (rc + dt);
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.prev_out += self.a * (x - self.prev_out);
        self.prev_out
    }
}

/// Look-ahead delay — delays the audio stream by N samples so the
/// detection side chain (no delay) sees transients early enough for
/// the compressor / limiter to act on them. 0 samples = pass-through.
pub struct LookAheadDelay {
    buffer: Vec<f32>,
    write: usize,
    len: usize,
}

impl LookAheadDelay {
    pub fn new(max_samples: usize) -> Self {
        let cap = max_samples.max(1);
        Self {
            buffer: vec![0.0; cap],
            write: 0,
            len: 0,
        }
    }

    pub fn set_samples(&mut self, samples: usize) {
        self.len = samples.min(self.buffer.len());
    }

    pub fn process(&mut self, x: f32) -> f32 {
        if self.len == 0 {
            return x;
        }
        let cap = self.buffer.len();
        let read = (self.write + cap - self.len) % cap;
        let out = self.buffer[read];
        self.buffer[self.write] = x;
        self.write = (self.write + 1) % cap;
        out
    }

    pub fn reset(&mut self) {
        for v in self.buffer.iter_mut() {
            *v = 0.0;
        }
        self.write = 0;
    }
}

/// Gate state machine with hold time — once the envelope rises above
/// threshold the gate opens; when it drops below, the `hold_samples`
/// window must elapse before the gate closes. Emits `GateState` on
/// each sample so the UI can draw an open/close indicator without
/// touching the audio graph.
pub struct GateHoldState {
    state: GateState,
    hold_remaining: usize,
    hold_samples: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Closed,
    Opening,
    Open,
    Holding,
}

impl GateHoldState {
    pub fn new(sample_rate: f32, hold_ms: f32) -> Self {
        Self {
            state: GateState::Closed,
            hold_remaining: 0,
            hold_samples: ((hold_ms * 0.001) * sample_rate).max(0.0) as usize,
        }
    }

    pub fn set_hold(&mut self, sample_rate: f32, hold_ms: f32) {
        self.hold_samples = ((hold_ms * 0.001) * sample_rate).max(0.0) as usize;
    }

    pub fn tick(&mut self, envelope_db: f32, threshold_db: f32) -> GateState {
        let above = envelope_db >= threshold_db;
        self.state = match self.state {
            GateState::Closed => {
                if above {
                    GateState::Opening
                } else {
                    GateState::Closed
                }
            }
            GateState::Opening => {
                if above {
                    GateState::Open
                } else {
                    GateState::Closed
                }
            }
            GateState::Open => {
                if above {
                    GateState::Open
                } else {
                    self.hold_remaining = self.hold_samples;
                    GateState::Holding
                }
            }
            GateState::Holding => {
                if above {
                    GateState::Open
                } else if self.hold_remaining > 0 {
                    self.hold_remaining -= 1;
                    GateState::Holding
                } else {
                    GateState::Closed
                }
            }
        };
        self.state
    }

    pub fn state(&self) -> GateState {
        self.state
    }

    pub fn is_open(&self) -> bool {
        matches!(
            self.state,
            GateState::Open | GateState::Holding | GateState::Opening
        )
    }
}

fn linear_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn db_to_linear(db: f32) -> f32 {
    10_f32.powf(db / 20.0)
}

fn decay_coefficient(sample_rate: f32, decay_ms: f32) -> f32 {
    let tau = (decay_ms * 0.001).max(1e-4);
    let dt = 1.0 / sample_rate.max(1.0);
    (-dt / tau).exp()
}

fn one_pole_alpha(sample_rate: f32, cutoff_hz: f32) -> f32 {
    let dt = 1.0 / sample_rate.max(1.0);
    let rc = 1.0 / (2.0 * PI * cutoff_hz.max(1e-3));
    rc / (rc + dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_gain_approaches_target() {
        let mut g = SmoothGain::new(48_000.0, 10.0, 0.0);
        g.set_db(-6.0);
        for _ in 0..48_000 {
            g.tick(1.0);
        }
        assert!((g.current_db() - (-6.0)).abs() < 0.5);
    }

    #[test]
    fn smooth_gain_does_not_jump() {
        let mut g = SmoothGain::new(48_000.0, 50.0, 0.0);
        g.set_db(-24.0);
        // One-sample ramp should not exceed ~0.1 dB change.
        let a = g.tick(1.0);
        let b = g.tick(1.0);
        let delta_db = 20.0 * (a.abs().max(1e-9) / b.abs().max(1e-9)).log10();
        assert!(delta_db.abs() < 0.5);
    }

    #[test]
    fn peak_meter_responds_to_transients_and_decays() {
        let mut p = PeakMeter::new(48_000.0, 100.0);
        p.tick(0.5);
        assert!((p.current_db() - (-6.02)).abs() < 0.2);
        for _ in 0..48_000 {
            p.tick(0.0);
        }
        // After 1 s of silence the meter should have decayed many dB.
        assert!(p.current_db() < -60.0);
    }

    #[test]
    fn rms_meter_matches_sine_rms() {
        let sr = 48_000.0;
        let mut r = RmsMeter::new(sr, 50.0);
        for i in 0..sr as usize {
            let x = (2.0 * PI * 440.0 * i as f32 / sr).sin();
            r.tick(x);
        }
        // Full-scale sine → RMS ≈ 0.707 → -3.01 dBFS.
        assert!((r.current_db() - (-3.01)).abs() < 1.0);
    }

    #[test]
    fn hpf_attenuates_low_tones() {
        let sr = 48_000.0;
        let mut lo_filter = OnePoleHpf::new(sr, 200.0);
        let mut hi_filter = OnePoleHpf::new(sr, 200.0);
        let mut energy_lo = 0.0_f32;
        let mut energy_hi = 0.0_f32;
        for i in 0..sr as usize {
            let lo = (2.0 * PI * 50.0 * i as f32 / sr).sin();
            let hi = (2.0 * PI * 1000.0 * i as f32 / sr).sin();
            energy_lo += lo_filter.process(lo).powi(2);
            energy_hi += hi_filter.process(hi).powi(2);
        }
        assert!(
            energy_hi > energy_lo * 3.0,
            "hi {} lo {}",
            energy_hi,
            energy_lo
        );
    }

    #[test]
    fn lpf_attenuates_high_tones() {
        let sr = 48_000.0;
        let mut lo_filter = OnePoleLpf::new(sr, 200.0);
        let mut hi_filter = OnePoleLpf::new(sr, 200.0);
        let mut energy_lo = 0.0_f32;
        let mut energy_hi = 0.0_f32;
        for i in 0..sr as usize {
            let lo = (2.0 * PI * 50.0 * i as f32 / sr).sin();
            let hi = (2.0 * PI * 5000.0 * i as f32 / sr).sin();
            energy_lo += lo_filter.process(lo).powi(2);
            energy_hi += hi_filter.process(hi).powi(2);
        }
        assert!(
            energy_lo > energy_hi * 3.0,
            "lo {} hi {}",
            energy_lo,
            energy_hi
        );
    }

    #[test]
    fn look_ahead_delays_by_exactly_n_samples() {
        let mut la = LookAheadDelay::new(32);
        la.set_samples(4);
        // Push 10 known samples.
        let out: Vec<f32> = (0..10).map(|i| la.process(i as f32)).collect();
        // First 4 outputs should be 0 (pre-fill), then 0, 1, 2, 3, 4, 5.
        assert_eq!(&out[..4], &[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(out[4], 0.0);
        assert_eq!(out[5], 1.0);
        assert_eq!(out[9], 5.0);
    }

    #[test]
    fn look_ahead_zero_is_passthrough() {
        let mut la = LookAheadDelay::new(16);
        la.set_samples(0);
        assert_eq!(la.process(7.0), 7.0);
    }

    #[test]
    fn gate_opens_on_threshold_cross_and_holds() {
        let sr = 48_000.0;
        let mut g = GateHoldState::new(sr, 50.0);
        let threshold = -40.0_f32;
        // Rising above → Opening → Open.
        g.tick(-30.0, threshold);
        assert_eq!(g.state(), GateState::Opening);
        g.tick(-30.0, threshold);
        assert_eq!(g.state(), GateState::Open);
        // Drop below → Holding for hold window.
        g.tick(-60.0, threshold);
        assert_eq!(g.state(), GateState::Holding);
        assert!(g.is_open());
        let hold = ((50.0 * 0.001) * sr) as usize;
        for _ in 0..hold + 2 {
            g.tick(-60.0, threshold);
        }
        assert_eq!(g.state(), GateState::Closed);
        assert!(!g.is_open());
    }
}
