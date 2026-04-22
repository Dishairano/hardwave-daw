//! Drum machine primitives — 16 pads, each playing a sample with
//! per-pad volume / pan / pitch / reverse, ADSR envelope, and an
//! optional low-pass/high-pass filter. Pads participate in choke
//! groups so hitting one pad silences another in the same group
//! (classic 808/909 hihat behavior).

use crate::biquad::{Biquad, BiquadKind};
use crate::synth::AdsrEnvelope;

pub const PAD_COUNT: usize = 16;

/// Per-pad sample data + playback parameters.
pub struct DrumPad {
    /// Loaded sample. Empty until `load_sample` is called.
    sample: Vec<f32>,
    /// Playback position (fractional for pitch/resample).
    play_pos: f32,
    /// Pitch ratio (1.0 = unity, 2.0 = one octave up).
    pitch_ratio: f32,
    /// Playback state.
    playing: bool,
    /// Volume multiplier.
    volume: f32,
    /// Pan in [-1, 1].
    pan: f32,
    /// If true, sample plays backward.
    reverse: bool,
    /// Choke group id (pads in the same group mute each other).
    /// 0 = no group.
    choke_group: u8,
    /// MIDI note this pad responds to.
    midi_note: u8,
    /// Envelope applied to the sample's amplitude.
    envelope: AdsrEnvelope,
    /// Optional filter stage.
    filter: Biquad,
    filter_kind: BiquadKind,
    filter_enabled: bool,
    filter_cutoff_hz: f32,
    filter_q: f32,
    sample_rate: f32,
}

impl DrumPad {
    pub fn new(sample_rate: f32, midi_note: u8) -> Self {
        let mut envelope = AdsrEnvelope::new(sample_rate);
        envelope.set_times(0.001, 0.100, 0.200);
        envelope.set_sustain(0.8);
        Self {
            sample: Vec::new(),
            play_pos: 0.0,
            pitch_ratio: 1.0,
            playing: false,
            volume: 1.0,
            pan: 0.0,
            reverse: false,
            choke_group: 0,
            midi_note,
            envelope,
            filter: Biquad::default(),
            filter_kind: BiquadKind::LowPass,
            filter_enabled: false,
            filter_cutoff_hz: 20_000.0,
            filter_q: 0.707,
            sample_rate: sample_rate.max(1.0),
        }
    }

    pub fn load_sample(&mut self, sample: Vec<f32>) {
        self.sample = sample;
        self.play_pos = 0.0;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 4.0);
    }

    pub fn set_pan(&mut self, pan: f32) {
        self.pan = pan.clamp(-1.0, 1.0);
    }

    pub fn set_pitch_semitones(&mut self, semitones: f32) {
        self.pitch_ratio = 2.0_f32.powf(semitones / 12.0);
    }

    pub fn set_reverse(&mut self, reverse: bool) {
        self.reverse = reverse;
    }

    pub fn set_choke_group(&mut self, group: u8) {
        self.choke_group = group;
    }

    pub fn choke_group(&self) -> u8 {
        self.choke_group
    }

    pub fn set_midi_note(&mut self, note: u8) {
        self.midi_note = note;
    }

    pub fn midi_note(&self) -> u8 {
        self.midi_note
    }

    pub fn set_envelope(&mut self, attack_secs: f32, decay_secs: f32, release_secs: f32) {
        self.envelope
            .set_times(attack_secs, decay_secs, release_secs);
    }

    pub fn set_sustain(&mut self, sustain: f32) {
        self.envelope.set_sustain(sustain);
    }

    pub fn set_filter(&mut self, kind: BiquadKind, cutoff_hz: f32, q: f32) {
        self.filter_kind = kind;
        self.filter_cutoff_hz = cutoff_hz.clamp(20.0, 20_000.0);
        self.filter_q = q.clamp(0.1, 10.0);
        self.filter.set(
            kind,
            self.sample_rate,
            self.filter_cutoff_hz,
            self.filter_q,
            0.0,
        );
    }

    pub fn set_filter_enabled(&mut self, enabled: bool) {
        self.filter_enabled = enabled;
    }

    pub fn trigger(&mut self, velocity: f32) {
        if self.sample.is_empty() {
            return;
        }
        self.play_pos = if self.reverse {
            (self.sample.len() - 1) as f32
        } else {
            0.0
        };
        self.playing = true;
        self.envelope.note_on();
        // Simple velocity-to-volume scaling.
        self.volume = self.volume.max(0.0) * velocity.clamp(0.0, 1.5);
        self.filter.reset();
    }

    pub fn release(&mut self) {
        self.envelope.note_off();
    }

    pub fn choke(&mut self) {
        self.playing = false;
        self.envelope.note_off();
        self.play_pos = 0.0;
    }

    pub fn is_playing(&self) -> bool {
        self.playing && self.envelope.is_active()
    }

    /// Tick one sample. Returns `(l, r)` after pan and filter.
    pub fn tick(&mut self) -> (f32, f32) {
        if !self.playing || self.sample.is_empty() {
            return (0.0, 0.0);
        }
        let idx = self.play_pos as usize;
        if idx >= self.sample.len() {
            self.playing = false;
            return (0.0, 0.0);
        }
        let frac = self.play_pos - idx as f32;
        let next_idx = (idx + 1).min(self.sample.len() - 1);
        let raw = self.sample[idx] + (self.sample[next_idx] - self.sample[idx]) * frac;

        let env = self.envelope.tick();
        let sample_val = raw * env * self.volume;

        let post_filter = if self.filter_enabled {
            self.filter.process_mono(sample_val)
        } else {
            sample_val
        };

        // Advance position in the correct direction.
        if self.reverse {
            self.play_pos -= self.pitch_ratio;
            if self.play_pos < 0.0 {
                self.playing = false;
            }
        } else {
            self.play_pos += self.pitch_ratio;
        }

        // Pan law: equal-power pan.
        let pan_angle = (self.pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let left_gain = pan_angle.cos();
        let right_gain = pan_angle.sin();
        (post_filter * left_gain, post_filter * right_gain)
    }
}

/// 16-pad drum machine with choke-group routing.
pub struct DrumMachine {
    pads: Vec<DrumPad>,
}

impl DrumMachine {
    pub fn new(sample_rate: f32) -> Self {
        let mut pads = Vec::with_capacity(PAD_COUNT);
        for i in 0..PAD_COUNT {
            pads.push(DrumPad::new(sample_rate, 36 + i as u8));
        }
        Self { pads }
    }

    pub fn pad_mut(&mut self, index: usize) -> Option<&mut DrumPad> {
        self.pads.get_mut(index)
    }

    pub fn pad_count(&self) -> usize {
        self.pads.len()
    }

    /// Trigger a pad by index, applying choke-group muting of other
    /// pads in the same group.
    pub fn trigger_pad(&mut self, index: usize, velocity: f32) {
        if index >= self.pads.len() {
            return;
        }
        let group = self.pads[index].choke_group;
        if group != 0 {
            for (i, pad) in self.pads.iter_mut().enumerate() {
                if i != index && pad.choke_group == group {
                    pad.choke();
                }
            }
        }
        self.pads[index].trigger(velocity);
    }

    /// Trigger a pad by its configured MIDI note.
    pub fn trigger_note(&mut self, midi_note: u8, velocity: f32) {
        if let Some(idx) = self.pads.iter().position(|p| p.midi_note == midi_note) {
            self.trigger_pad(idx, velocity);
        }
    }

    /// Generate one stereo frame by summing all playing pads.
    pub fn tick(&mut self) -> (f32, f32) {
        let mut sum_l = 0.0;
        let mut sum_r = 0.0;
        for pad in self.pads.iter_mut() {
            let (l, r) = pad.tick();
            sum_l += l;
            sum_r += r;
        }
        (sum_l, sum_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_sample() -> Vec<f32> {
        (0..100).map(|i| if i < 50 { 1.0 } else { 0.5 }).collect()
    }

    #[test]
    fn drum_pad_plays_after_trigger() {
        let mut pad = DrumPad::new(48_000.0, 36);
        pad.load_sample(make_test_sample());
        pad.set_envelope(0.0001, 0.0001, 0.0001);
        pad.set_sustain(1.0);
        assert!(!pad.is_playing());
        pad.trigger(1.0);
        assert!(pad.is_playing());
        // Process a few samples — should produce audio (give
        // envelope attack time to settle).
        let mut max = 0.0_f32;
        for _ in 0..60 {
            let (l, _r) = pad.tick();
            if l.abs() > max {
                max = l.abs();
            }
        }
        assert!(max > 0.3, "pad produced output < 0.3: {max}");
    }

    #[test]
    fn drum_pad_stops_at_sample_end() {
        let mut pad = DrumPad::new(48_000.0, 36);
        pad.load_sample(vec![1.0; 10]);
        pad.set_envelope(0.0001, 0.0001, 0.001);
        pad.set_sustain(1.0);
        pad.trigger(1.0);
        // Play past the end.
        for _ in 0..20 {
            pad.tick();
        }
        assert!(
            !pad.is_playing(),
            "pad should have stopped after sample end"
        );
    }

    #[test]
    fn drum_pad_pitch_up_plays_faster() {
        let mut pad_unity = DrumPad::new(48_000.0, 36);
        pad_unity.load_sample(vec![1.0; 1000]);
        pad_unity.set_envelope(0.0001, 0.0001, 0.001);
        pad_unity.set_sustain(1.0);
        pad_unity.set_pitch_semitones(0.0);
        pad_unity.trigger(1.0);

        let mut pad_octave = DrumPad::new(48_000.0, 36);
        pad_octave.load_sample(vec![1.0; 1000]);
        pad_octave.set_envelope(0.0001, 0.0001, 0.001);
        pad_octave.set_sustain(1.0);
        pad_octave.set_pitch_semitones(12.0);
        pad_octave.trigger(1.0);

        for _ in 0..600 {
            pad_unity.tick();
            pad_octave.tick();
        }
        // At +12 semitones, pad_octave consumed 2× faster — should
        // already be done by tick 600 (sample length 1000 / 2 = 500).
        assert!(
            !pad_octave.is_playing(),
            "octave-up pad should have finished"
        );
        assert!(
            pad_unity.is_playing(),
            "unity-pitch pad should still be playing"
        );
    }

    #[test]
    fn drum_machine_has_16_pads() {
        let dm = DrumMachine::new(48_000.0);
        assert_eq!(dm.pad_count(), PAD_COUNT);
    }

    #[test]
    fn choke_group_mutes_sibling_pads() {
        let mut dm = DrumMachine::new(48_000.0);
        // Pads 0 and 1 both in choke group 1 (hihats).
        if let Some(p) = dm.pad_mut(0) {
            p.load_sample(make_test_sample());
            p.set_envelope(0.0001, 0.0001, 0.001);
            p.set_sustain(1.0);
            p.set_choke_group(1);
        }
        if let Some(p) = dm.pad_mut(1) {
            p.load_sample(make_test_sample());
            p.set_envelope(0.0001, 0.0001, 0.001);
            p.set_sustain(1.0);
            p.set_choke_group(1);
        }
        dm.trigger_pad(0, 1.0);
        assert!(dm.pad_mut(0).unwrap().is_playing());
        // Triggering pad 1 should choke pad 0.
        dm.trigger_pad(1, 1.0);
        assert!(!dm.pad_mut(0).unwrap().is_playing());
        assert!(dm.pad_mut(1).unwrap().is_playing());
    }

    #[test]
    fn midi_note_mapping_triggers_correct_pad() {
        let mut dm = DrumMachine::new(48_000.0);
        if let Some(p) = dm.pad_mut(5) {
            p.load_sample(make_test_sample());
            p.set_envelope(0.0001, 0.0001, 0.001);
            p.set_sustain(1.0);
            p.set_midi_note(42);
        }
        dm.trigger_note(42, 1.0);
        assert!(dm.pad_mut(5).unwrap().is_playing());
    }

    #[test]
    fn reverse_pad_plays_backward() {
        let mut pad = DrumPad::new(48_000.0, 36);
        // Sample with increasing values so we can verify order.
        let sample: Vec<f32> = (0..500).map(|i| i as f32 / 500.0).collect();
        pad.load_sample(sample);
        // Minimum envelope time clamps to 0.001 s = 48 samples at
        // 48 kHz. Need enough samples to get past the attack ramp.
        pad.set_envelope(0.001, 0.001, 0.01);
        pad.set_sustain(1.0);
        pad.set_reverse(true);
        pad.trigger(1.0);
        // Skip past envelope attack transient (~60 samples at 48 kHz).
        for _ in 0..100 {
            pad.tick();
        }
        let (first, _) = pad.tick();
        for _ in 0..50 {
            pad.tick();
        }
        let (later, _) = pad.tick();
        assert!(
            first > later,
            "reverse: first tick {first} should exceed later {later}"
        );
    }

    #[test]
    fn equal_power_pan_to_right() {
        let mut pad = DrumPad::new(48_000.0, 36);
        pad.load_sample(vec![1.0; 100]);
        pad.set_envelope(0.0001, 0.0001, 0.001);
        pad.set_sustain(1.0);
        pad.set_pan(1.0); // Full right.
        pad.trigger(1.0);
        // Let envelope settle past attack.
        let mut last_l = 0.0;
        let mut last_r = 0.0;
        for _ in 0..30 {
            let (l, r) = pad.tick();
            last_l = l;
            last_r = r;
        }
        assert!(last_l.abs() < 0.01, "full-right L should be ~0: {last_l}");
        assert!(last_r.abs() > 0.5, "full-right R should be present: {last_r}");
    }
}
