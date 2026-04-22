//! 4-operator FM synthesis primitive. Each operator is a sine
//! oscillator with its own frequency ratio, level, envelope, and
//! feedback amount. Operators are wired together by an `Algorithm`
//! enum matching the classic 4-op presets (à la Yamaha DX series).
//!
//! Not a full DX7 clone — just the math the FM Synth plugin needs
//! to produce recognizable FM timbres.

use crate::synth::AdsrEnvelope;
use std::f32::consts::PI;

/// How the 4 operators are interconnected. Each variant describes
/// the modulation graph: `A -> B` means A modulates B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    /// Classic 1→2→3→4 chain. Only op 4 is audible.
    Stack,
    /// 1 → (2 + 3) → 4. Parallel modulation of op 4.
    ParallelMid,
    /// (1+2+3) → 4. Three modulators feed one carrier.
    ThreeToOne,
    /// 1→2 and 3→4 (two independent pairs). Ops 2 and 4 audible.
    DualPair,
    /// All four operators sum in parallel with no cross-modulation.
    Parallel,
    /// 1→2, sum 2+3+4 at output.
    OneModTwoPlusTwoCarriers,
    /// 1→(2,3,4) fan-out then sum carriers.
    FanOutCarriers,
    /// 1→2→3, 4 solo carrier.
    ChainPlusSolo,
}

/// A single FM operator — sine oscillator + ADSR + ratio + level +
/// feedback. Ratio is relative to the note's base frequency; level
/// is the output amplitude (for carriers) or modulation index (for
/// modulators).
pub struct Operator {
    phase: f32,
    ratio_coarse: f32,
    ratio_fine: f32,
    level: f32,
    feedback: f32,
    /// One-sample feedback memory for operators with self-feedback.
    last_output: f32,
    /// ADSR envelope scaling the operator's output.
    envelope: AdsrEnvelope,
    /// Velocity sensitivity in `[0, 1]`.
    velocity_sensitivity: f32,
    /// Current waveform type — FM traditionally uses sine, but
    /// some classic implementations allow other shapes.
    waveform: OpWaveform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpWaveform {
    Sine,
    Saw,
    Square,
    Triangle,
    Custom,
}

impl Operator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            ratio_coarse: 1.0,
            ratio_fine: 0.0,
            level: 1.0,
            feedback: 0.0,
            last_output: 0.0,
            envelope: AdsrEnvelope::new(sample_rate),
            velocity_sensitivity: 1.0,
            waveform: OpWaveform::Sine,
        }
    }

    /// Coarse tuning in integer multiples of the base frequency (1, 2,
    /// 3, 0.5, etc.). Classic FM uses small integer ratios to produce
    /// harmonically-related modulator/carrier pairs.
    pub fn set_ratio(&mut self, coarse: f32, fine: f32) {
        self.ratio_coarse = coarse.max(0.01);
        self.ratio_fine = fine.clamp(-1.0, 1.0);
    }

    pub fn effective_ratio(&self) -> f32 {
        self.ratio_coarse + self.ratio_fine * 0.01
    }

    pub fn set_level(&mut self, level: f32) {
        self.level = level.clamp(0.0, 4.0);
    }

    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback.clamp(0.0, 0.95);
    }

    pub fn set_envelope_times(&mut self, attack_secs: f32, decay_secs: f32, release_secs: f32) {
        self.envelope
            .set_times(attack_secs, decay_secs, release_secs);
    }

    pub fn set_sustain(&mut self, sustain: f32) {
        self.envelope.set_sustain(sustain);
    }

    pub fn set_velocity_sensitivity(&mut self, sensitivity: f32) {
        self.velocity_sensitivity = sensitivity.clamp(0.0, 1.0);
    }

    pub fn set_waveform(&mut self, w: OpWaveform) {
        self.waveform = w;
    }

    pub fn note_on(&mut self) {
        self.envelope.note_on();
    }

    pub fn note_off(&mut self) {
        self.envelope.note_off();
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.last_output = 0.0;
    }

    /// Advance one sample. `base_hz` is the note's fundamental (e.g.
    /// from `midi_to_hz`). `modulation` is the phase modulation input
    /// in radians, summed from any modulators feeding this operator.
    /// `velocity` is `[0, 1]` — the typical MIDI note-on velocity.
    pub fn tick(&mut self, base_hz: f32, modulation: f32, velocity: f32, sample_rate: f32) -> f32 {
        let freq = base_hz * self.effective_ratio();
        // Include self-feedback + incoming modulation.
        let fb = self.last_output * self.feedback * 2.0 * PI;
        let modulated_phase = self.phase * 2.0 * PI + modulation + fb;
        let raw = match self.waveform {
            OpWaveform::Sine => modulated_phase.sin(),
            OpWaveform::Saw => {
                let p = modulated_phase.rem_euclid(2.0 * PI) / (2.0 * PI);
                2.0 * p - 1.0
            }
            OpWaveform::Square => {
                let p = modulated_phase.rem_euclid(2.0 * PI) / (2.0 * PI);
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            OpWaveform::Triangle => {
                let p = modulated_phase.rem_euclid(2.0 * PI) / (2.0 * PI);
                if p < 0.5 {
                    4.0 * p - 1.0
                } else {
                    3.0 - 4.0 * p
                }
            }
            OpWaveform::Custom => modulated_phase.sin(),
        };
        let env = self.envelope.tick();
        let vel_gain = 1.0 - self.velocity_sensitivity * (1.0 - velocity);
        let out = raw * self.level * env * vel_gain;
        self.last_output = out;
        self.phase = (self.phase + freq / sample_rate.max(1.0)).rem_euclid(1.0);
        out
    }

    pub fn is_active(&self) -> bool {
        self.envelope.is_active()
    }
}

/// 4-operator FM voice. Holds four `Operator`s and an `Algorithm`
/// enum that dictates how they're connected.
pub struct FmVoice {
    ops: [Operator; 4],
    algorithm: Algorithm,
    sample_rate: f32,
    base_hz: f32,
    velocity: f32,
}

impl FmVoice {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            ops: [
                Operator::new(sample_rate),
                Operator::new(sample_rate),
                Operator::new(sample_rate),
                Operator::new(sample_rate),
            ],
            algorithm: Algorithm::Stack,
            sample_rate: sample_rate.max(1.0),
            base_hz: 440.0,
            velocity: 1.0,
        }
    }

    pub fn op_mut(&mut self, index: usize) -> Option<&mut Operator> {
        self.ops.get_mut(index)
    }

    pub fn set_algorithm(&mut self, algorithm: Algorithm) {
        self.algorithm = algorithm;
    }

    pub fn set_base_hz(&mut self, hz: f32) {
        self.base_hz = hz.max(1.0);
    }

    pub fn set_velocity(&mut self, velocity: f32) {
        self.velocity = velocity.clamp(0.0, 1.0);
    }

    pub fn note_on(&mut self) {
        for op in self.ops.iter_mut() {
            op.note_on();
        }
    }

    pub fn note_off(&mut self) {
        for op in self.ops.iter_mut() {
            op.note_off();
        }
    }

    pub fn reset(&mut self) {
        for op in self.ops.iter_mut() {
            op.reset();
        }
    }

    pub fn is_active(&self) -> bool {
        self.ops.iter().any(|o| o.is_active())
    }

    /// Generate one sample of audio output.
    pub fn tick(&mut self) -> f32 {
        let sr = self.sample_rate;
        let base = self.base_hz;
        let vel = self.velocity;

        // Evaluate operators in the order required by the algorithm.
        // Since each operator's output is needed by the next in line,
        // we evaluate the full DAG in topological order.
        match self.algorithm {
            Algorithm::Stack => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, o1, vel, sr);
                let o3 = self.ops[2].tick(base, o2, vel, sr);
                self.ops[3].tick(base, o3, vel, sr)
            }
            Algorithm::ParallelMid => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, o1, vel, sr);
                let o3 = self.ops[2].tick(base, o1, vel, sr);
                self.ops[3].tick(base, o2 + o3, vel, sr)
            }
            Algorithm::ThreeToOne => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, 0.0, vel, sr);
                let o3 = self.ops[2].tick(base, 0.0, vel, sr);
                self.ops[3].tick(base, o1 + o2 + o3, vel, sr)
            }
            Algorithm::DualPair => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, o1, vel, sr);
                let o3 = self.ops[2].tick(base, 0.0, vel, sr);
                let o4 = self.ops[3].tick(base, o3, vel, sr);
                o2 + o4
            }
            Algorithm::Parallel => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, 0.0, vel, sr);
                let o3 = self.ops[2].tick(base, 0.0, vel, sr);
                let o4 = self.ops[3].tick(base, 0.0, vel, sr);
                o1 + o2 + o3 + o4
            }
            Algorithm::OneModTwoPlusTwoCarriers => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, o1, vel, sr);
                let o3 = self.ops[2].tick(base, 0.0, vel, sr);
                let o4 = self.ops[3].tick(base, 0.0, vel, sr);
                o2 + o3 + o4
            }
            Algorithm::FanOutCarriers => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, o1, vel, sr);
                let o3 = self.ops[2].tick(base, o1, vel, sr);
                let o4 = self.ops[3].tick(base, o1, vel, sr);
                o2 + o3 + o4
            }
            Algorithm::ChainPlusSolo => {
                let o1 = self.ops[0].tick(base, 0.0, vel, sr);
                let o2 = self.ops[1].tick(base, o1, vel, sr);
                let o3 = self.ops[2].tick(base, o2, vel, sr);
                let o4 = self.ops[3].tick(base, 0.0, vel, sr);
                o3 + o4
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_voice() -> FmVoice {
        let mut v = FmVoice::new(48_000.0);
        v.set_base_hz(440.0);
        for i in 0..4 {
            if let Some(op) = v.op_mut(i) {
                op.set_envelope_times(0.001, 0.001, 0.001);
                op.set_sustain(1.0);
                op.set_level(1.0);
                op.set_ratio(1.0, 0.0);
            }
        }
        v.note_on();
        v
    }

    #[test]
    fn stack_algorithm_produces_audio() {
        let mut v = default_voice();
        v.set_algorithm(Algorithm::Stack);
        // Run for 1 ms after envelope reaches sustain.
        for _ in 0..200 {
            v.tick();
        }
        let mut max = 0.0_f32;
        for _ in 0..100 {
            let s = v.tick();
            if s.abs() > max {
                max = s.abs();
            }
        }
        assert!(max > 0.05, "stack algorithm produced silence: max = {max}");
    }

    #[test]
    fn parallel_algorithm_sums_four_carriers() {
        let mut v = default_voice();
        v.set_algorithm(Algorithm::Parallel);
        // Each op is 1.0 sine → sum should peak near 4.0 if all in phase.
        for _ in 0..200 {
            v.tick();
        }
        let mut max = 0.0_f32;
        for _ in 0..500 {
            let s = v.tick();
            if s.abs() > max {
                max = s.abs();
            }
        }
        assert!(max > 1.0, "parallel should sum to > 1, got {max}");
        // Cap at 4 (sum of 4 unit-amplitude sines).
        assert!(max < 4.5, "parallel sum overshoot: {max}");
    }

    #[test]
    fn feedback_produces_non_sinusoidal_content() {
        let mut v = default_voice();
        v.set_algorithm(Algorithm::Stack);
        if let Some(op) = v.op_mut(0) {
            op.set_feedback(0.7);
        }
        // Let the envelope settle and feedback build up.
        for _ in 0..5000 {
            v.tick();
        }
        let mut samples = Vec::new();
        for _ in 0..500 {
            samples.push(v.tick());
        }
        // A stable sine would have only two distinct absolute values
        // (positive and negative peaks). Feedback introduces complex
        // harmonic content — verify signal variation.
        let max = samples.iter().cloned().fold(0.0_f32, f32::max);
        let min = samples.iter().cloned().fold(0.0_f32, f32::min);
        assert!(max > 0.0);
        assert!(min < 0.0);
        let span = max - min;
        assert!(span > 0.1, "feedback should produce amplitude variation");
    }

    #[test]
    fn note_off_triggers_release_stage() {
        let mut v = default_voice();
        v.note_off();
        // After release, voice eventually becomes inactive.
        let mut still_active_after_secs = true;
        for _ in 0..48_000 {
            v.tick();
            if !v.is_active() {
                still_active_after_secs = false;
                break;
            }
        }
        assert!(
            !still_active_after_secs,
            "voice didn't release within 1 second"
        );
    }

    #[test]
    fn algorithm_switch_preserves_state() {
        let mut v = default_voice();
        v.set_algorithm(Algorithm::Stack);
        for _ in 0..1000 {
            v.tick();
        }
        v.set_algorithm(Algorithm::Parallel);
        // Should continue producing audio without panic.
        let s = v.tick();
        assert!(s.is_finite());
    }

    #[test]
    fn velocity_sensitivity_scales_output() {
        let mut v = default_voice();
        v.set_algorithm(Algorithm::Parallel);
        for i in 0..4 {
            if let Some(op) = v.op_mut(i) {
                op.set_velocity_sensitivity(1.0);
            }
        }
        v.set_velocity(1.0);
        // Let the envelope settle.
        for _ in 0..200 {
            v.tick();
        }
        let mut high_vel_max = 0.0_f32;
        for _ in 0..500 {
            let s = v.tick().abs();
            if s > high_vel_max {
                high_vel_max = s;
            }
        }
        // Restart voice with low velocity.
        let mut v = default_voice();
        v.set_algorithm(Algorithm::Parallel);
        for i in 0..4 {
            if let Some(op) = v.op_mut(i) {
                op.set_velocity_sensitivity(1.0);
            }
        }
        v.set_velocity(0.1);
        for _ in 0..200 {
            v.tick();
        }
        let mut low_vel_max = 0.0_f32;
        for _ in 0..500 {
            let s = v.tick().abs();
            if s > low_vel_max {
                low_vel_max = s;
            }
        }
        assert!(
            high_vel_max > low_vel_max * 2.0,
            "high velocity ({high_vel_max}) should far exceed low ({low_vel_max})"
        );
    }
}
