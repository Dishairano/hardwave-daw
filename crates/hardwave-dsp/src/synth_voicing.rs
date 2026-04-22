//! Synth voicing primitives — LFOs, unison spread, portamento glide,
//! and a voice pool for mono / 4 / 8 / 16 / 32-voice polyphony with
//! basic voice stealing. Shared across the subtractive, wavetable,
//! and FM engines.

use std::f32::consts::PI;

/// LFO shape options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleAndHold,
}

/// Where the LFO output is routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoTarget {
    Pitch,
    FilterCutoff,
    Amp,
    Pan,
    OscLevel,
}

/// A single LFO with rate, shape, target, amount, and optional tempo
/// sync. `tick()` advances by one sample and returns the LFO value in
/// `[-amount, +amount]`.
pub struct SynthLfo {
    sample_rate: f32,
    rate_hz: f32,
    shape: LfoShape,
    target: LfoTarget,
    amount: f32,
    phase: f32,
    sh_value: f32,
    sh_phase_prev: f32,
    sh_state: u32,
    synced_bpm: Option<f32>,
    synced_divisor: f32,
}

impl SynthLfo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            rate_hz: 1.0,
            shape: LfoShape::Sine,
            target: LfoTarget::Pitch,
            amount: 0.0,
            phase: 0.0,
            sh_value: 0.0,
            sh_phase_prev: 0.0,
            sh_state: 0xC0FFEE,
            synced_bpm: None,
            synced_divisor: 4.0,
        }
    }

    pub fn set_rate_hz(&mut self, rate: f32) {
        self.rate_hz = rate.max(0.01);
        self.synced_bpm = None;
    }

    pub fn set_shape(&mut self, shape: LfoShape) {
        self.shape = shape;
    }

    pub fn set_target(&mut self, target: LfoTarget) {
        self.target = target;
    }

    pub fn target(&self) -> LfoTarget {
        self.target
    }

    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
    }

    /// Enable tempo sync. `divisor` is the musical division in beats —
    /// 0.25 = 16th note, 1.0 = quarter note, 4.0 = whole bar. The
    /// derived rate overrides any previous `set_rate_hz` call.
    pub fn set_tempo_sync(&mut self, bpm: f32, divisor: f32) {
        self.synced_bpm = Some(bpm.max(1.0));
        self.synced_divisor = divisor.max(0.0625);
        let seconds_per_beat = 60.0 / bpm.max(1.0);
        let cycle_seconds = seconds_per_beat * self.synced_divisor;
        self.rate_hz = 1.0 / cycle_seconds.max(1e-4);
    }

    pub fn is_tempo_synced(&self) -> bool {
        self.synced_bpm.is_some()
    }

    /// Advance one sample and return the modulation value scaled by
    /// `amount`.
    pub fn tick(&mut self) -> f32 {
        let step = self.rate_hz / self.sample_rate;
        self.phase = (self.phase + step).rem_euclid(1.0);
        let raw = match self.shape {
            LfoShape::Sine => (2.0 * PI * self.phase).sin(),
            LfoShape::Triangle => {
                if self.phase < 0.5 {
                    4.0 * self.phase - 1.0
                } else {
                    3.0 - 4.0 * self.phase
                }
            }
            LfoShape::Saw => 2.0 * self.phase - 1.0,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleAndHold => {
                // Re-roll when the phase wraps past 0.
                if self.phase < self.sh_phase_prev {
                    self.sh_state = self
                        .sh_state
                        .wrapping_mul(1_664_525)
                        .wrapping_add(1_013_904_223);
                    self.sh_value = (self.sh_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                }
                self.sh_phase_prev = self.phase;
                self.sh_value
            }
        };
        raw * self.amount
    }
}

/// Exponential portamento (glide) — smoothly chases a target value
/// with a time-constant controlled by `glide_seconds`. Useful for
/// pitch glide in monophonic patches.
pub struct Portamento {
    current: f32,
    target: f32,
    alpha: f32,
}

impl Portamento {
    pub fn new(sample_rate: f32, glide_seconds: f32) -> Self {
        let mut p = Self {
            current: 0.0,
            target: 0.0,
            alpha: 0.0,
        };
        p.set_time(sample_rate, glide_seconds);
        p
    }

    pub fn set_time(&mut self, sample_rate: f32, glide_seconds: f32) {
        let g = glide_seconds.max(0.0);
        if g <= 1e-4 {
            self.alpha = 1.0; // instant
        } else {
            // -6 dB / time-constant per g seconds.
            self.alpha = 1.0 - (-1.0 / (g * sample_rate.max(1.0))).exp();
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn force(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }

    pub fn tick(&mut self) -> f32 {
        self.current += (self.target - self.current) * self.alpha;
        self.current
    }
}

/// Generate a set of detune + pan offsets for `voices` unison voices.
/// `detune_cents` controls the outermost detune; inner voices
/// interpolate linearly from `-detune` to `+detune`. `spread` is the
/// stereo spread in `[-1, 1]`. Pan for voice `i` is
/// `(i / (N-1) * 2 - 1) * spread`.
pub fn unison_offsets(voices: usize, detune_cents: f32, spread: f32) -> Vec<(f32, f32)> {
    let voices = voices.max(1);
    let spread = spread.clamp(-1.0, 1.0);
    if voices == 1 {
        return vec![(0.0, 0.0)];
    }
    (0..voices)
        .map(|i| {
            let t = i as f32 / (voices - 1) as f32; // 0..1
            let detune = (t * 2.0 - 1.0) * detune_cents;
            let pan = (t * 2.0 - 1.0) * spread;
            (detune, pan)
        })
        .collect()
}

/// Mono mode — legato holds the current envelope on note-on if a
/// note is already playing; retrigger always restarts the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonoMode {
    Legato,
    Retrigger,
}

/// Polyphony configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polyphony {
    Mono(MonoMode),
    Poly(usize),
}

impl Polyphony {
    pub fn max_voices(&self) -> usize {
        match self {
            Polyphony::Mono(_) => 1,
            Polyphony::Poly(n) => (*n).clamp(1, 64),
        }
    }
}

/// Voice-allocation result returned by `VoicePool::note_on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceAssignment {
    pub voice_index: usize,
    pub stole: bool,
}

/// Minimal voice pool — tracks which voices are currently active and
/// what note each one holds. Handles note-on (allocate / steal),
/// note-off, and query. Doesn't hold the actual synth state; the
/// caller owns a parallel array of `Voice` structs and looks them
/// up by `voice_index`.
pub struct VoicePool {
    active: Vec<Option<u8>>, // None = idle, Some(note) = holding note.
    order: Vec<usize>,       // allocation order for stealing.
}

impl VoicePool {
    pub fn new(max_voices: usize) -> Self {
        let n = max_voices.clamp(1, 64);
        Self {
            active: vec![None; n],
            order: Vec::with_capacity(n),
        }
    }

    pub fn capacity(&self) -> usize {
        self.active.len()
    }

    pub fn active_count(&self) -> usize {
        self.active.iter().filter(|v| v.is_some()).count()
    }

    /// Assign a voice for `note`. If all voices are active, steal the
    /// oldest. If the note is already playing, return its existing
    /// voice (legato-style retrigger is the caller's choice).
    pub fn note_on(&mut self, note: u8) -> VoiceAssignment {
        if let Some((i, _)) = self
            .active
            .iter()
            .enumerate()
            .find(|(_, v)| **v == Some(note))
        {
            self.touch_order(i);
            return VoiceAssignment {
                voice_index: i,
                stole: false,
            };
        }
        if let Some((i, _)) = self.active.iter().enumerate().find(|(_, v)| v.is_none()) {
            self.active[i] = Some(note);
            self.touch_order(i);
            return VoiceAssignment {
                voice_index: i,
                stole: false,
            };
        }
        // All busy — steal the oldest.
        let victim = self.order.first().copied().unwrap_or(0);
        self.active[victim] = Some(note);
        self.touch_order(victim);
        VoiceAssignment {
            voice_index: victim,
            stole: true,
        }
    }

    pub fn note_off(&mut self, note: u8) -> Option<usize> {
        for (i, slot) in self.active.iter_mut().enumerate() {
            if *slot == Some(note) {
                *slot = None;
                self.order.retain(|&idx| idx != i);
                return Some(i);
            }
        }
        None
    }

    pub fn note_for(&self, voice_index: usize) -> Option<u8> {
        self.active.get(voice_index).copied().flatten()
    }

    fn touch_order(&mut self, index: usize) {
        self.order.retain(|&i| i != index);
        self.order.push(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfo_sine_spans_plus_minus_one_over_a_cycle() {
        let mut lfo = SynthLfo::new(48_000.0);
        lfo.set_rate_hz(10.0);
        lfo.set_shape(LfoShape::Sine);
        lfo.set_amount(1.0);
        let samples: Vec<f32> = (0..4_800).map(|_| lfo.tick()).collect();
        let max = samples.iter().cloned().fold(f32::MIN, f32::max);
        let min = samples.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max > 0.95 && min < -0.95);
    }

    #[test]
    fn lfo_amount_scales_output() {
        let mut lfo = SynthLfo::new(48_000.0);
        lfo.set_rate_hz(5.0);
        lfo.set_amount(0.25);
        let samples: Vec<f32> = (0..9_600).map(|_| lfo.tick()).collect();
        let max = samples.iter().cloned().fold(f32::MIN, f32::max);
        let min = samples.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max < 0.3 && max > 0.2, "max {}", max);
        assert!(min > -0.3 && min < -0.2, "min {}", min);
    }

    #[test]
    fn lfo_tempo_sync_sets_rate_from_bpm() {
        let mut lfo = SynthLfo::new(48_000.0);
        lfo.set_tempo_sync(120.0, 1.0); // quarter note @ 120 BPM = 0.5 s
        assert!(lfo.is_tempo_synced());
        // Rate should be 2 Hz (one cycle per 0.5 s).
        lfo.set_amount(1.0);
        let samples: Vec<f32> = (0..48_000).map(|_| lfo.tick()).collect();
        let zc = samples
            .windows(2)
            .filter(|w| (w[0] < 0.0 && w[1] >= 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
            .count();
        // 2 Hz sine over 1 s → 4 zero crossings ± small window effects.
        assert!((3..=6).contains(&zc), "zc {}", zc);
    }

    #[test]
    fn portamento_converges_to_target() {
        let mut p = Portamento::new(48_000.0, 0.1);
        p.set_target(100.0);
        for _ in 0..48_000 {
            p.tick();
        }
        assert!(p.tick() > 99.5);
    }

    #[test]
    fn portamento_zero_time_is_instant() {
        let mut p = Portamento::new(48_000.0, 0.0);
        p.set_target(42.0);
        let v = p.tick();
        assert!((v - 42.0).abs() < 0.01);
    }

    #[test]
    fn unison_offsets_symmetric_around_zero() {
        let offsets = unison_offsets(5, 12.0, 1.0);
        assert_eq!(offsets.len(), 5);
        assert!((offsets[0].0 + 12.0).abs() < 1e-4);
        assert!((offsets[4].0 - 12.0).abs() < 1e-4);
        assert!((offsets[0].1 + 1.0).abs() < 1e-4);
        assert!((offsets[4].1 - 1.0).abs() < 1e-4);
        // Middle voice sits at (0, 0).
        assert!(offsets[2].0.abs() < 1e-4);
        assert!(offsets[2].1.abs() < 1e-4);
    }

    #[test]
    fn voice_pool_allocates_and_frees() {
        let mut pool = VoicePool::new(4);
        let a = pool.note_on(60);
        assert!(!a.stole);
        assert_eq!(pool.active_count(), 1);
        let _b = pool.note_on(62);
        let _c = pool.note_on(64);
        let _d = pool.note_on(67);
        assert_eq!(pool.active_count(), 4);
        let e = pool.note_on(69); // should steal.
        assert!(e.stole);
        assert_eq!(pool.active_count(), 4);
        assert!(pool.note_off(69).is_some());
        assert_eq!(pool.active_count(), 3);
    }

    #[test]
    fn voice_pool_reuses_same_voice_for_same_note() {
        let mut pool = VoicePool::new(4);
        let a = pool.note_on(60);
        let b = pool.note_on(60);
        assert_eq!(a.voice_index, b.voice_index);
        assert_eq!(pool.active_count(), 1);
    }

    #[test]
    fn polyphony_max_voices_is_clamped() {
        assert_eq!(Polyphony::Mono(MonoMode::Legato).max_voices(), 1);
        assert_eq!(Polyphony::Poly(16).max_voices(), 16);
        assert_eq!(Polyphony::Poly(200).max_voices(), 64);
    }
}
