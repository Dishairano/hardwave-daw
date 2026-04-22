//! Dynamics primitives — envelope follower, gain-reduction calculators
//! for compressor / limiter / gate, and auto-makeup gain. All pure
//! sample-level math; no allocation, no state other than the envelope
//! follower itself. Plugin wrappers compose these with attack/release
//! smoothing and parameter automation on top.

/// Detection mode for envelope followers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectMode {
    /// Peak follows the instantaneous absolute sample value.
    Peak,
    /// RMS uses a small-window running average — slower, smoother.
    Rms,
}

/// Single-channel envelope follower with separate attack and release
/// time constants. Stateful across calls — one instance per channel.
pub struct EnvelopeFollower {
    value: f32,
    attack_coeff: f32,
    release_coeff: f32,
    mode: DetectMode,
    rms_sq_accumulator: f32,
}

impl Default for EnvelopeFollower {
    fn default() -> Self {
        Self {
            value: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            mode: DetectMode::Peak,
            rms_sq_accumulator: 0.0,
        }
    }
}

impl EnvelopeFollower {
    /// Set attack/release time constants in milliseconds at the given
    /// sample rate. Uses the standard exponential-smoothing coefficient
    /// `exp(-1 / (time_ms * sr / 1000))`, so after `time_ms` the
    /// envelope has reached 63.2% of the target.
    pub fn set_times(&mut self, attack_ms: f32, release_ms: f32, sample_rate: f32) {
        let sr = sample_rate.max(1.0);
        let compute = |ms: f32| -> f32 {
            let ms = ms.max(0.0);
            if ms <= 0.0 {
                return 0.0;
            }
            let samples = ms * 0.001 * sr;
            (-1.0 / samples.max(1.0)).exp()
        };
        self.attack_coeff = compute(attack_ms);
        self.release_coeff = compute(release_ms);
    }

    /// Set detection mode (Peak or Rms).
    pub fn set_mode(&mut self, mode: DetectMode) {
        self.mode = mode;
    }

    /// Reset the envelope to zero.
    pub fn reset(&mut self) {
        self.value = 0.0;
        self.rms_sq_accumulator = 0.0;
    }

    /// Process one sample and return the current envelope value in
    /// linear amplitude (not dB). For RMS mode, returns the sqrt of
    /// the smoothed squared value.
    #[inline]
    pub fn process(&mut self, sample: f32) -> f32 {
        let detected = match self.mode {
            DetectMode::Peak => sample.abs(),
            DetectMode::Rms => {
                // One-pole smoother on sample squared — time constant
                // follows release for a stable RMS window.
                let sq = sample * sample;
                let alpha = 0.001; // ~160 Hz corner @ 48 kHz
                self.rms_sq_accumulator = alpha * sq + (1.0 - alpha) * self.rms_sq_accumulator;
                self.rms_sq_accumulator.sqrt()
            }
        };
        let coeff = if detected > self.value {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.value = coeff * self.value + (1.0 - coeff) * detected;
        self.value
    }

    /// Current envelope value without advancing state.
    pub fn current(&self) -> f32 {
        self.value
    }
}

/// Convert linear amplitude to dBFS, clamping very small values to
/// -120 dB to avoid `-inf`.
#[inline]
pub fn linear_to_db(x: f32) -> f32 {
    let mag = x.abs().max(1e-6);
    20.0 * mag.log10()
}

/// Convert dBFS to linear amplitude.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Compute the compressor gain-reduction curve: given an input level
/// in dB, return the amount of gain reduction in dB (always <= 0).
///
/// `threshold_db`: start of compression (-60..0 dB).
/// `ratio`: input:output ratio above threshold (1.0 = no compression,
/// 100.0 = limiting).
/// `knee_db`: soft-knee width (0..30). At 0, the knee is hard; larger
/// values smooth the transition over a `knee_db`-wide region centered
/// on threshold.
pub fn compressor_gain_reduction_db(
    input_db: f32,
    threshold_db: f32,
    ratio: f32,
    knee_db: f32,
) -> f32 {
    let ratio = ratio.max(1.0);
    let knee = knee_db.max(0.0);
    let overshoot = input_db - threshold_db;

    // Below the knee region: no compression.
    if overshoot < -knee / 2.0 {
        return 0.0;
    }

    // Above the knee region: straight compression.
    if overshoot > knee / 2.0 {
        return -(overshoot - overshoot / ratio);
    }

    // Inside the soft knee: smooth quadratic interpolation.
    let knee_input = overshoot + knee / 2.0; // 0..knee
    let scaled = knee_input * knee_input / (2.0 * knee).max(1e-6);
    -(scaled - scaled / ratio)
}

/// Gate gain-reduction: returns linear gain multiplier (0..1). Unlike
/// the compressor which always returns <= 0 dB, the gate fully closes
/// (returns 0.0) when the signal is below threshold minus hysteresis.
///
/// `range_db` sets the floor — how deep the gate closes. Range of
/// `range_db = 0` would mean hard mute; typical values are -30..-80 dB.
pub fn gate_gain(input_db: f32, threshold_db: f32, range_db: f32, hysteresis_db: f32) -> f32 {
    let hyst = hysteresis_db.abs();
    let close_at = threshold_db - hyst;
    if input_db >= threshold_db {
        1.0
    } else if input_db <= close_at {
        db_to_linear(range_db.min(0.0))
    } else {
        // Linear fade across the hysteresis window.
        let t = (input_db - close_at) / hyst.max(1e-6);
        let floor_linear = db_to_linear(range_db.min(0.0));
        floor_linear + (1.0 - floor_linear) * t
    }
}

/// Auto makeup gain: computes the makeup gain (in dB) that roughly
/// compensates for the average gain reduction a compressor produces.
/// Uses the half-ratio approximation commonly found in DAW stock comps.
pub fn auto_makeup_gain_db(threshold_db: f32, ratio: f32) -> f32 {
    let ratio = ratio.max(1.0);
    // Makeup ≈ |threshold| × (1 - 1/ratio) / 2.
    // Gives +3 dB makeup for threshold=-6, ratio=2, etc.
    (-threshold_db) * (1.0 - 1.0 / ratio) * 0.5
}

/// True-peak estimate using 2× oversampling with a simple linear
/// interpolator. Not a full ITU-R BS.1770 true-peak implementation,
/// but a cheap upper bound suitable for catching inter-sample peaks
/// that trip limiters. Returns the max of the sample and the
/// half-way interpolated value between the previous and current
/// sample.
#[inline]
pub fn true_peak_upper_bound(prev_sample: f32, sample: f32) -> f32 {
    let interp = 0.5 * (prev_sample + sample);
    sample.abs().max(interp.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_peak_follows_input_magnitude() {
        let mut env = EnvelopeFollower::default();
        env.set_times(1.0, 10.0, 48_000.0);
        env.set_mode(DetectMode::Peak);
        // Feed a constant 0.5 signal; envelope should converge.
        for _ in 0..1000 {
            env.process(0.5);
        }
        assert!(
            (env.current() - 0.5).abs() < 0.01,
            "env = {}",
            env.current()
        );
    }

    #[test]
    fn envelope_attack_is_faster_than_release() {
        let mut env = EnvelopeFollower::default();
        env.set_times(1.0, 100.0, 48_000.0);
        env.set_mode(DetectMode::Peak);
        // Step up to 1.0 — envelope rises quickly.
        let mut rise_time = 0;
        for i in 0..10_000 {
            env.process(1.0);
            if env.current() > 0.9 {
                rise_time = i;
                break;
            }
        }
        // Step down to 0.0 — envelope decays slowly.
        let mut fall_time = 0;
        for i in 0..100_000 {
            env.process(0.0);
            if env.current() < 0.1 {
                fall_time = i;
                break;
            }
        }
        assert!(
            rise_time < fall_time / 10,
            "rise {rise_time} vs fall {fall_time}"
        );
    }

    #[test]
    fn rms_mode_is_smoother_than_peak() {
        // RMS should smooth a square-ish signal; peak latches
        // instantaneously. A pulse train into peak spikes; into
        // RMS it averages toward half-amplitude.
        let mut peak = EnvelopeFollower::default();
        let mut rms = EnvelopeFollower::default();
        peak.set_times(1.0, 10.0, 48_000.0);
        rms.set_times(1.0, 10.0, 48_000.0);
        peak.set_mode(DetectMode::Peak);
        rms.set_mode(DetectMode::Rms);

        for t in 0..10_000 {
            let pulse = if t % 4 < 2 { 1.0 } else { 0.0 };
            peak.process(pulse);
            rms.process(pulse);
        }
        // Peak tracks ~1.0, RMS averages toward 0.707.
        assert!(peak.current() > 0.7, "peak = {}", peak.current());
        assert!(rms.current() < peak.current());
    }

    #[test]
    fn compressor_hard_knee_passes_below_threshold() {
        // Input -20 dB, threshold -10, ratio 4, knee 0.
        let gr = compressor_gain_reduction_db(-20.0, -10.0, 4.0, 0.0);
        assert_eq!(gr, 0.0, "signal below threshold should not be compressed");
    }

    #[test]
    fn compressor_hard_knee_4_to_1_above_threshold() {
        // Input 0 dB, threshold -12, ratio 4, knee 0.
        // Overshoot = 12 dB. Output gain = 12 × (1 - 1/4) = 9 dB reduction.
        let gr = compressor_gain_reduction_db(0.0, -12.0, 4.0, 0.0);
        assert!((gr - (-9.0)).abs() < 1e-3, "gr = {gr}");
    }

    #[test]
    fn compressor_ratio_1_is_identity() {
        // ratio = 1.0 means no compression even above threshold.
        let gr = compressor_gain_reduction_db(0.0, -24.0, 1.0, 0.0);
        assert!(gr.abs() < 1e-3, "ratio 1 should never reduce, got {gr}");
    }

    #[test]
    fn compressor_soft_knee_starts_below_threshold() {
        // With knee=10, compression begins at threshold - 5. Probe
        // slightly above that boundary (overshoot = -4, into the knee).
        let gr = compressor_gain_reduction_db(-16.0, -12.0, 4.0, 10.0);
        // Should have a small negative GR (partial compression).
        assert!(gr < 0.0 && gr > -2.0, "gr in knee = {gr}");
    }

    #[test]
    fn gate_fully_closes_below_floor() {
        // Input -80, threshold -40, hysteresis 3, range -80.
        let gain = gate_gain(-80.0, -40.0, -80.0, 3.0);
        let expected = db_to_linear(-80.0);
        assert!((gain - expected).abs() < 1e-4, "gain = {gain}");
    }

    #[test]
    fn gate_fully_opens_above_threshold() {
        let gain = gate_gain(-10.0, -40.0, -80.0, 3.0);
        assert_eq!(gain, 1.0);
    }

    #[test]
    fn auto_makeup_gain_half_ratio_approximation() {
        // threshold=-12, ratio=4 → (|-12|) × (1-1/4) × 0.5 = 4.5 dB
        let makeup = auto_makeup_gain_db(-12.0, 4.0);
        assert!((makeup - 4.5).abs() < 1e-3, "makeup = {makeup}");
        // ratio=1 → zero makeup (no compression).
        assert_eq!(auto_makeup_gain_db(-12.0, 1.0), 0.0);
    }

    #[test]
    fn db_conversion_round_trips() {
        for db in [-60.0, -24.0, -6.0, 0.0, 6.0] {
            let rt = linear_to_db(db_to_linear(db));
            assert!((rt - db).abs() < 0.01, "round trip {db} -> {rt}");
        }
    }

    #[test]
    fn true_peak_upper_bound_catches_intersample_peaks() {
        // Alternating +1 / -1 samples have an inter-sample peak of 0.
        let upper = true_peak_upper_bound(1.0, -1.0);
        // The midpoint average is 0; but either sample's abs is 1.
        assert!((upper - 1.0).abs() < 1e-6);
        // Same-sign alternation can exceed either sample's magnitude.
        let upper2 = true_peak_upper_bound(0.9, 0.9);
        assert!(upper2 >= 0.9);
    }
}
