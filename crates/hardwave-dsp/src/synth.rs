//! Synth primitives — oscillator with classic waveforms plus noise,
//! and an ADSR envelope. These are the building blocks every synth
//! plugin (Subtractive, Kick Synth, FM, etc.) composes together.
//!
//! All state is sample-accurate; no block-level approximation. Pure
//! f32 arithmetic.

use std::f32::consts::PI;

/// Oscillator waveform selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Saw,
    Square,
    Triangle,
    /// White noise — uniform in `[-1, 1]`, seeded per oscillator for
    /// deterministic playback.
    Noise,
}

/// Classic wavetable-free oscillator. Advances phase per sample,
/// evaluates the waveform shape, scales by `level`. Uses a naive
/// (non-bandlimited) implementation — the synth layer can add
/// oversampling on top if needed for high-pitch aliasing control.
pub struct Oscillator {
    waveform: Waveform,
    phase: f32,
    frequency_hz: f32,
    sample_rate: f32,
    level: f32,
    /// Pulse width for Square waveform in `[0.05, 0.95]`. Ignored
    /// by the other waveforms.
    pulse_width: f32,
    noise_state: u32,
}

impl Oscillator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            waveform: Waveform::Sine,
            phase: 0.0,
            frequency_hz: 440.0,
            sample_rate: sample_rate.max(1.0),
            level: 1.0,
            pulse_width: 0.5,
            noise_state: 0x12345678,
        }
    }

    pub fn set_waveform(&mut self, w: Waveform) {
        self.waveform = w;
    }

    /// Set the fundamental pitch in Hz. Clamped to at least 0.
    pub fn set_frequency(&mut self, hz: f32) {
        self.frequency_hz = hz.max(0.0);
    }

    /// Set frequency from a MIDI note number (69 = A4 = 440 Hz) with
    /// fine-tune offset in cents. Convenient for instrument layers
    /// where the UI exposes semitones + cents.
    pub fn set_pitch_midi(&mut self, semitones_above_a4: f32, fine_cents: f32) {
        let total_semitones = semitones_above_a4 + fine_cents / 100.0;
        let hz = 440.0 * 2.0_f32.powf(total_semitones / 12.0);
        self.set_frequency(hz);
    }

    pub fn set_level(&mut self, level: f32) {
        self.level = level.clamp(0.0, 4.0);
    }

    pub fn set_pulse_width(&mut self, pw: f32) {
        self.pulse_width = pw.clamp(0.05, 0.95);
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
    }

    /// Force phase to a specific value in `[0, 1)`. Used for
    /// oscillator sync (hard-sync Osc 2 to Osc 1 by calling this
    /// whenever Osc 1's phase wraps).
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    pub fn current_phase(&self) -> f32 {
        self.phase
    }

    /// Advance one sample and return the oscillator's output.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        let p = self.phase;
        let out = match self.waveform {
            Waveform::Sine => (2.0 * PI * p).sin(),
            Waveform::Saw => 2.0 * p - 1.0,
            Waveform::Square => {
                if p < self.pulse_width {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => {
                if p < 0.5 {
                    4.0 * p - 1.0
                } else {
                    3.0 - 4.0 * p
                }
            }
            Waveform::Noise => {
                // xorshift32 for cheap deterministic white noise.
                let mut x = self.noise_state;
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                self.noise_state = x;
                (x as f32 / u32::MAX as f32) * 2.0 - 1.0
            }
        };
        self.phase = (self.phase + self.frequency_hz / self.sample_rate).rem_euclid(1.0);
        out * self.level
    }
}

/// ADSR envelope stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdsrStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Classic Attack / Decay / Sustain / Release envelope. Times are in
/// seconds; sustain is a normalized level in `[0, 1]`. Outputs a
/// linear amplitude multiplier in `[0, 1]` per sample.
pub struct AdsrEnvelope {
    stage: AdsrStage,
    value: f32,
    attack_secs: f32,
    decay_secs: f32,
    sustain_level: f32,
    release_secs: f32,
    sample_rate: f32,
}

impl AdsrEnvelope {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            stage: AdsrStage::Idle,
            value: 0.0,
            attack_secs: 0.01,
            decay_secs: 0.1,
            sustain_level: 0.7,
            release_secs: 0.2,
            sample_rate: sample_rate.max(1.0),
        }
    }

    pub fn set_times(&mut self, attack_secs: f32, decay_secs: f32, release_secs: f32) {
        self.attack_secs = attack_secs.max(0.001);
        self.decay_secs = decay_secs.max(0.001);
        self.release_secs = release_secs.max(0.001);
    }

    pub fn set_sustain(&mut self, sustain_level: f32) {
        self.sustain_level = sustain_level.clamp(0.0, 1.0);
    }

    /// Gate on — start the attack stage from the current value.
    pub fn note_on(&mut self) {
        self.stage = AdsrStage::Attack;
    }

    /// Gate off — start the release stage.
    pub fn note_off(&mut self) {
        if !matches!(self.stage, AdsrStage::Idle) {
            self.stage = AdsrStage::Release;
        }
    }

    pub fn stage(&self) -> AdsrStage {
        self.stage
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.stage, AdsrStage::Idle)
    }

    /// Advance one sample and return the envelope's linear output.
    pub fn tick(&mut self) -> f32 {
        match self.stage {
            AdsrStage::Idle => {
                self.value = 0.0;
            }
            AdsrStage::Attack => {
                let step = 1.0 / (self.attack_secs * self.sample_rate);
                self.value += step;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = AdsrStage::Decay;
                }
            }
            AdsrStage::Decay => {
                let step = (1.0 - self.sustain_level) / (self.decay_secs * self.sample_rate);
                self.value -= step;
                if self.value <= self.sustain_level {
                    self.value = self.sustain_level;
                    self.stage = AdsrStage::Sustain;
                }
            }
            AdsrStage::Sustain => {
                self.value = self.sustain_level;
            }
            AdsrStage::Release => {
                // Linear decay: one unit of envelope per release_secs,
                // so a release from 1.0 to 0.0 takes exactly release_secs.
                // Releases starting from lower values (e.g. sustain=0.5)
                // reach zero faster, which matches musical expectation.
                let step = 1.0 / (self.release_secs * self.sample_rate);
                self.value -= step;
                if self.value <= 0.0 {
                    self.value = 0.0;
                    self.stage = AdsrStage::Idle;
                }
            }
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_oscillator_covers_unit_range() {
        let mut osc = Oscillator::new(48_000.0);
        osc.set_waveform(Waveform::Sine);
        osc.set_frequency(100.0);
        let mut max = -10.0_f32;
        let mut min = 10.0_f32;
        for _ in 0..2000 {
            let v = osc.tick();
            if v > max {
                max = v;
            }
            if v < min {
                min = v;
            }
        }
        assert!(max > 0.99, "sine max = {max}");
        assert!(min < -0.99, "sine min = {min}");
    }

    #[test]
    fn saw_ramps_from_minus_one_to_one() {
        let mut osc = Oscillator::new(48_000.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(50.0);
        let samples: Vec<f32> = (0..960).map(|_| osc.tick()).collect();
        // Saw should hit both -1 and +1 within a cycle.
        let mn = samples.iter().cloned().fold(f32::MAX, f32::min);
        let mx = samples.iter().cloned().fold(f32::MIN, f32::max);
        assert!(mn < -0.95 && mx > 0.95);
    }

    #[test]
    fn square_holds_two_values_only() {
        let mut osc = Oscillator::new(48_000.0);
        osc.set_waveform(Waveform::Square);
        osc.set_frequency(50.0);
        let samples: Vec<f32> = (0..2000).map(|_| osc.tick()).collect();
        // All samples should be ±1.0.
        for s in &samples {
            assert!(
                (s - 1.0).abs() < 1e-6 || (s + 1.0).abs() < 1e-6,
                "square produced off-value: {s}"
            );
        }
    }

    #[test]
    fn triangle_covers_full_unit_range() {
        let mut osc = Oscillator::new(48_000.0);
        osc.set_waveform(Waveform::Triangle);
        osc.set_frequency(50.0);
        let samples: Vec<f32> = (0..2000).map(|_| osc.tick()).collect();
        let mn = samples.iter().cloned().fold(f32::MAX, f32::min);
        let mx = samples.iter().cloned().fold(f32::MIN, f32::max);
        assert!(mn < -0.95 && mx > 0.95);
    }

    #[test]
    fn noise_is_deterministic_per_instance() {
        let mut a = Oscillator::new(48_000.0);
        let mut b = Oscillator::new(48_000.0);
        a.set_waveform(Waveform::Noise);
        b.set_waveform(Waveform::Noise);
        for _ in 0..100 {
            assert_eq!(a.tick(), b.tick(), "noise should be deterministic");
        }
    }

    #[test]
    fn set_pitch_midi_matches_a4_convention() {
        let mut osc = Oscillator::new(48_000.0);
        osc.set_pitch_midi(0.0, 0.0); // A4
                                      // Phase advance per sample = 440 / 48000.
        osc.tick();
        let step = osc.current_phase();
        let expected = 440.0 / 48_000.0;
        assert!(
            (step - expected).abs() < 1e-4,
            "A4 phase step {step} vs expected {expected}"
        );
    }

    #[test]
    fn set_pitch_midi_octave_up_doubles_frequency() {
        let mut a = Oscillator::new(48_000.0);
        let mut b = Oscillator::new(48_000.0);
        a.set_pitch_midi(0.0, 0.0);
        b.set_pitch_midi(12.0, 0.0);
        a.tick();
        b.tick();
        // B's phase advance should be 2x A's.
        let ratio = b.current_phase() / a.current_phase();
        assert!((ratio - 2.0).abs() < 1e-3, "octave ratio = {ratio}");
    }

    #[test]
    fn adsr_attack_stage_ramps_to_one() {
        let mut env = AdsrEnvelope::new(48_000.0);
        env.set_times(0.01, 0.1, 0.2);
        env.set_sustain(0.5);
        env.note_on();
        assert_eq!(env.stage(), AdsrStage::Attack);
        // Advance 10 ms at 48 kHz = 480 samples.
        let mut last = 0.0;
        for _ in 0..500 {
            last = env.tick();
        }
        assert!(
            last >= 0.5,
            "attack should have climbed well past sustain, got {last}"
        );
    }

    #[test]
    fn adsr_reaches_sustain_after_attack_and_decay() {
        let mut env = AdsrEnvelope::new(48_000.0);
        env.set_times(0.001, 0.001, 0.1);
        env.set_sustain(0.4);
        env.note_on();
        for _ in 0..10_000 {
            env.tick();
        }
        assert_eq!(env.stage(), AdsrStage::Sustain);
        assert!((env.tick() - 0.4).abs() < 1e-3);
    }

    #[test]
    fn adsr_release_decays_to_zero_and_idles() {
        let mut env = AdsrEnvelope::new(48_000.0);
        env.set_times(0.001, 0.001, 0.01);
        env.set_sustain(0.5);
        env.note_on();
        for _ in 0..5_000 {
            env.tick();
        }
        env.note_off();
        for _ in 0..5_000 {
            env.tick();
        }
        assert_eq!(env.stage(), AdsrStage::Idle);
        assert!(env.tick() < 1e-6);
    }

    #[test]
    fn oscillator_sync_forces_phase() {
        let mut osc = Oscillator::new(48_000.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(1000.0);
        for _ in 0..10 {
            osc.tick();
        }
        let before = osc.current_phase();
        assert!(before > 0.0);
        osc.set_phase(0.25);
        assert!((osc.current_phase() - 0.25).abs() < 1e-6);
    }
}
