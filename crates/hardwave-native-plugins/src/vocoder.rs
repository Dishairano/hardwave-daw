//! Native vocoder — a classic N-band channel vocoder.
//!
//! The track signal is the **modulator**; an internal oscillator
//! (saw / pulse / noise) is the **carrier**, so it works without
//! sidechain routing (the engine only feeds plugins their own 2
//! channels). Each of `NUM_BANDS` log-spaced bands extracts the
//! modulator's envelope and imprints it on the carrier's matching band;
//! the bands sum to the output. A saw/pulse carrier gives the robotic
//! talk-box voice; the noise carrier gives a whisper/breath effect.

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use hardwave_dsp::dynamics::{DetectMode, EnvelopeFollower};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const NUM_BANDS: usize = 16;
const BAND_LOW_HZ: f32 = 120.0;
const BAND_HIGH_HZ: f32 = 7500.0;
const BAND_Q: f32 = 5.0;
/// Band products are quiet; lift the summed output back to a usable level.
const MAKEUP: f32 = 6.0;

const PARAM_WET: u32 = 0;
const PARAM_CARRIER: u32 = 1; // 0=saw, 1=pulse, 2=noise
const PARAM_TONE: u32 = 2; // carrier base pitch
const PARAM_ATTACK: u32 = 3;
const PARAM_RELEASE: u32 = 4;
const PARAM_COUNT: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Carrier {
    Saw,
    Pulse,
    Noise,
}

/// Log-spaced band center frequencies for an `n`-band vocoder.
fn band_centers(n: usize, low: f32, high: f32) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![(low * high).sqrt()];
    }
    let ratio = (high / low).powf(1.0 / (n - 1) as f32);
    (0..n).map(|i| low * ratio.powi(i as i32)).collect()
}

/// One carrier sample for `phase` in `[0, 1)`. `rng` advances only for noise.
fn carrier_sample(kind: Carrier, phase: f32, rng: &mut u32) -> f32 {
    match kind {
        Carrier::Saw => 2.0 * phase - 1.0,
        Carrier::Pulse => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        Carrier::Noise => {
            // xorshift32 → [-1, 1]
            *rng ^= *rng << 13;
            *rng ^= *rng >> 17;
            *rng ^= *rng << 5;
            (*rng as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
    }
}

struct Band {
    mod_bp: Biquad,
    car_bp: Biquad,
    env: EnvelopeFollower,
}

pub struct NativeVocoder {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    active: bool,
    bands: Vec<Band>,
    centers: Vec<f32>,
    carrier: Carrier,
    carrier_freq: f32,
    phase: f32,
    rng: u32,
    wet: f32,
    attack_ms: f32,
    release_ms: f32,
}

impl NativeVocoder {
    pub const ID: &'static str = "hardwave.native.vocoder";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Vocoder".into(),
            vendor: "Hardwave".into(),
            version: "1.0.0".into(),
            format: PluginFormat::Clap,
            path: PathBuf::from("<native>"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    pub fn new() -> Self {
        let sr = 48_000.0_f32;
        let centers = band_centers(NUM_BANDS, BAND_LOW_HZ, BAND_HIGH_HZ);
        let attack_ms = 5.0;
        let release_ms = 60.0;
        let bands = centers
            .iter()
            .map(|&c| Band {
                mod_bp: Self::make_bp(sr, c),
                car_bp: Self::make_bp(sr, c),
                env: Self::make_env(sr, attack_ms, release_ms),
            })
            .collect();
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            active: false,
            bands,
            centers,
            carrier: Carrier::Saw,
            carrier_freq: 110.0,
            phase: 0.0,
            rng: 0x1234_5678,
            wet: 1.0,
            attack_ms,
            release_ms,
        }
    }

    fn make_bp(sr: f32, center: f32) -> Biquad {
        let mut b = Biquad::default();
        b.set(BiquadKind::BandPass, sr, center, BAND_Q, 0.0);
        b
    }

    fn make_env(sr: f32, attack_ms: f32, release_ms: f32) -> EnvelopeFollower {
        let mut e = EnvelopeFollower::default();
        e.set_mode(DetectMode::Peak);
        e.set_times(attack_ms, release_ms, sr);
        e
    }

    fn rebuild_bands(&mut self) {
        for (band, &c) in self.bands.iter_mut().zip(self.centers.iter()) {
            band.mod_bp = Self::make_bp(self.sample_rate, c);
            band.car_bp = Self::make_bp(self.sample_rate, c);
            band.env = Self::make_env(self.sample_rate, self.attack_ms, self.release_ms);
        }
    }
}

impl Default for NativeVocoder {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeVocoder {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = (sr as f32).max(1.0);
        self.rebuild_bands();
        self.active = true;
        Ok(())
    }
    fn deactivate(&mut self) {
        self.active = false;
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[MidiEvent],
        _midi_out: &mut Vec<MidiEvent>,
        num_samples: usize,
    ) {
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        if !self.active || inputs.len() < 2 || outputs.len() < 2 {
            for ch in 0..outputs.len().min(inputs.len()) {
                let n = inputs[ch].len().min(num_samples);
                outputs[ch][..n].copy_from_slice(&inputs[ch][..n]);
            }
            return;
        }
        let phase_inc = self.carrier_freq / self.sample_rate;
        for i in 0..num_samples {
            let l = inputs[0].get(i).copied().unwrap_or(0.0);
            let r = inputs[1].get(i).copied().unwrap_or(0.0);
            let modulator = (l + r) * 0.5;

            let carrier = carrier_sample(self.carrier, self.phase, &mut self.rng);
            self.phase += phase_inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }

            let mut voc = 0.0_f32;
            for band in self.bands.iter_mut() {
                let m = band.mod_bp.process_mono(modulator);
                let env = band.env.process(m.abs());
                let c = band.car_bp.process_mono(carrier);
                voc += c * env;
            }
            voc = (voc * MAKEUP).clamp(-1.0, 1.0);
            let out = modulator * (1.0 - self.wet) + voc * self.wet;
            outputs[0][i] = out;
            outputs[1][i] = out;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_WET => ("Wet", 1.0, ""),
            PARAM_CARRIER => ("Carrier", 0.0, ""), // 0=saw
            PARAM_TONE => ("Tone", 0.5, "Hz"),
            PARAM_ATTACK => ("Attack", 5.0 / 100.0, "ms"),
            PARAM_RELEASE => ("Release", 60.0 / 500.0, "ms"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index,
            name: name.into(),
            default_value: default,
            min: 0.0,
            max: 1.0,
            unit: unit.into(),
            automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_WET => self.wet as f64,
            PARAM_CARRIER => match self.carrier {
                Carrier::Saw => 0.0,
                Carrier::Pulse => 0.5,
                Carrier::Noise => 1.0,
            },
            // Tone maps 55..220 Hz onto 0..1.
            PARAM_TONE => ((self.carrier_freq - 55.0) / (220.0 - 55.0)).clamp(0.0, 1.0) as f64,
            PARAM_ATTACK => (self.attack_ms / 100.0).clamp(0.0, 1.0) as f64,
            PARAM_RELEASE => (self.release_ms / 500.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0) as f32;
        match id {
            PARAM_WET => self.wet = v,
            PARAM_CARRIER => {
                self.carrier = if v < 0.33 {
                    Carrier::Saw
                } else if v < 0.66 {
                    Carrier::Pulse
                } else {
                    Carrier::Noise
                };
            }
            PARAM_TONE => self.carrier_freq = 55.0 + v * (220.0 - 55.0),
            PARAM_ATTACK => {
                self.attack_ms = (v * 100.0).max(0.1);
                self.rebuild_bands();
            }
            PARAM_RELEASE => {
                self.release_ms = (v * 500.0).max(1.0);
                self.rebuild_bands();
            }
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        let carrier = match self.carrier {
            Carrier::Saw => 0,
            Carrier::Pulse => 1,
            Carrier::Noise => 2,
        };
        format!(
            "{{\"wet\":{},\"car\":{},\"freq\":{},\"a\":{},\"r\":{}}}",
            self.wet, carrier, self.carrier_freq, self.attack_ms, self.release_ms
        )
        .into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("wet") {
            self.wet = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("car") {
            self.carrier = match v as i32 {
                1 => Carrier::Pulse,
                2 => Carrier::Noise,
                _ => Carrier::Saw,
            };
        }
        if let Some(v) = read("freq") {
            self.carrier_freq = v.clamp(20.0, 2000.0);
        }
        if let Some(v) = read("a") {
            self.attack_ms = v.clamp(0.1, 100.0);
        }
        if let Some(v) = read("r") {
            self.release_ms = v.clamp(1.0, 500.0);
        }
        self.rebuild_bands();
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        0
    }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool {
        false
    }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_centers_ascend_and_span_range() {
        let c = band_centers(NUM_BANDS, BAND_LOW_HZ, BAND_HIGH_HZ);
        assert_eq!(c.len(), NUM_BANDS);
        assert!((c[0] - BAND_LOW_HZ).abs() < 1.0);
        assert!((c[NUM_BANDS - 1] - BAND_HIGH_HZ).abs() < 1.0);
        for w in c.windows(2) {
            assert!(w[1] > w[0], "centers must ascend");
        }
    }

    #[test]
    fn carrier_shapes_are_in_range() {
        let mut rng = 1u32;
        for &kind in &[Carrier::Saw, Carrier::Pulse, Carrier::Noise] {
            for p in 0..100 {
                let v = carrier_sample(kind, p as f32 / 100.0, &mut rng);
                assert!((-1.0..=1.0).contains(&v));
            }
        }
    }

    fn run(v: &mut NativeVocoder, modulator: &[f32]) -> Vec<f32> {
        let l = modulator.to_vec();
        let r = modulator.to_vec();
        let inputs: [&[f32]; 2] = [&l, &r];
        let mut outputs = vec![vec![0.0; modulator.len()], vec![0.0; modulator.len()]];
        v.process(&inputs, &mut outputs, &[], &mut Vec::new(), modulator.len());
        outputs[0].clone()
    }

    #[test]
    fn silence_in_yields_silence_out() {
        let mut v = NativeVocoder::new();
        v.activate(48_000.0, 512).unwrap();
        let out = run(&mut v, &vec![0.0; 1024]);
        assert!(
            out.iter().all(|s| s.abs() < 1e-6),
            "silent modulator → silent"
        );
    }

    #[test]
    fn broadband_modulator_produces_output() {
        let mut v = NativeVocoder::new();
        v.activate(48_000.0, 512).unwrap();
        // White-ish noise modulator excites all bands.
        let mut rng = 99u32;
        let modu: Vec<f32> = (0..4096)
            .map(|_| carrier_sample(Carrier::Noise, 0.0, &mut rng) * 0.5)
            .collect();
        let out = run(&mut v, &modu);
        let energy: f32 = out.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "voiced modulator should produce output");
    }

    #[test]
    fn inactive_passes_through() {
        let mut v = NativeVocoder::new(); // not activated
        let out = run(&mut v, &[0.1, -0.2, 0.3, -0.4]);
        assert_eq!(out, vec![0.1, -0.2, 0.3, -0.4]);
    }
}
