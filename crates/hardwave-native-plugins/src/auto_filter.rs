//! Native auto-filter — envelope-followed cutoff biquad. Filter
//! cutoff sweeps up/down with input level for talking-bass / wah-wah
//! effects. Distinct from NativeFilter (static cutoff).

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use hardwave_dsp::dynamics::{DetectMode, EnvelopeFollower};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_BASE: u32 = 0;
const PARAM_RANGE: u32 = 1;
const PARAM_SENSITIVITY: u32 = 2;
const PARAM_RESONANCE: u32 = 3;
const PARAM_ATTACK: u32 = 4;
const PARAM_RELEASE: u32 = 5;
const PARAM_COUNT: u32 = 6;

pub struct NativeAutoFilter {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    biquad_l: Biquad,
    biquad_r: Biquad,
    env_l: EnvelopeFollower,
    env_r: EnvelopeFollower,
    base_hz: f32,
    /// Octaves the envelope can sweep above base.
    range_octaves: f32,
    /// 0..=1; how much input level moves cutoff
    sensitivity: f32,
    resonance: f32,
    attack_ms: f32,
    release_ms: f32,
    block_counter: u32,
    active: bool,
}

impl NativeAutoFilter {
    pub const ID: &'static str = "hardwave.native.auto_filter";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Auto-Filter".into(),
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
        let mut env_l = EnvelopeFollower::default();
        let mut env_r = EnvelopeFollower::default();
        env_l.set_mode(DetectMode::Peak);
        env_r.set_mode(DetectMode::Peak);
        env_l.set_times(10.0, 100.0, sr);
        env_r.set_times(10.0, 100.0, sr);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            biquad_l: Biquad::default(),
            biquad_r: Biquad::default(),
            env_l,
            env_r,
            base_hz: 200.0,
            range_octaves: 4.0,
            sensitivity: 0.7,
            resonance: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            block_counter: 0,
            active: false,
        }
    }
}

impl Default for NativeAutoFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeAutoFilter {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.env_l
            .set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.env_r
            .set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.biquad_l.reset();
        self.biquad_r.reset();
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
        // Envelope-follow at sample rate, but recompute filter coefs
        // every 16 samples — recoef is float-mathy and doesn't need
        // sample-accurate updates for an auto-filter.
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let env_l = self.env_l.process(in_l);
            let env_r = self.env_r.process(in_r);
            let env_avg = (env_l + env_r) * 0.5;
            self.block_counter = self.block_counter.wrapping_add(1);
            if self.block_counter % 16 == 0 {
                let octave_offset = env_avg.clamp(0.0, 1.0) * self.sensitivity * self.range_octaves;
                let cutoff = (self.base_hz * 2.0_f32.powf(octave_offset)).clamp(20.0, 18_000.0);
                self.biquad_l.set(
                    BiquadKind::LowPass,
                    self.sample_rate,
                    cutoff,
                    self.resonance,
                    0.0,
                );
                self.biquad_r.set(
                    BiquadKind::LowPass,
                    self.sample_rate,
                    cutoff,
                    self.resonance,
                    0.0,
                );
            }
            outputs[0][i] = self.biquad_l.process_mono(in_l);
            outputs[1][i] = self.biquad_r.process_mono(in_r);
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_BASE => (
                "Base",
                ((200.0_f64.log10() - 50.0_f64.log10()) / (2_000.0_f64.log10() - 50.0_f64.log10()))
                    .clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_RANGE => ("Range", 4.0 / 6.0, "oct"),
            PARAM_SENSITIVITY => ("Sense", 0.7, "%"),
            PARAM_RESONANCE => (
                "Resonance",
                (4.0_f64.log10() / 10.0_f64.log10()).clamp(0.0, 1.0),
                "",
            ),
            PARAM_ATTACK => (
                "Attack",
                ((10.0_f64.log10() - 0.5_f64.log10()) / (200.0_f64.log10() - 0.5_f64.log10()))
                    .clamp(0.0, 1.0),
                "ms",
            ),
            PARAM_RELEASE => (
                "Release",
                ((100.0_f64.log10() - 5.0_f64.log10()) / (1000.0_f64.log10() - 5.0_f64.log10()))
                    .clamp(0.0, 1.0),
                "ms",
            ),
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
            PARAM_BASE => {
                let lo = 50.0_f32.log10();
                let hi = 2_000.0_f32.log10();
                ((self.base_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_RANGE => (self.range_octaves / 6.0).clamp(0.0, 1.0) as f64,
            PARAM_SENSITIVITY => self.sensitivity as f64,
            PARAM_RESONANCE => (self.resonance.log10() / 10.0_f32.log10()).clamp(0.0, 1.0) as f64,
            PARAM_ATTACK => {
                let lo = 0.5_f32.log10();
                let hi = 200.0_f32.log10();
                ((self.attack_ms.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_RELEASE => {
                let lo = 5.0_f32.log10();
                let hi = 1000.0_f32.log10();
                ((self.release_ms.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        let mut update_env = false;
        match id {
            PARAM_BASE => {
                let lo = 50.0_f32.log10();
                let hi = 2_000.0_f32.log10();
                self.base_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
            }
            PARAM_RANGE => self.range_octaves = (v * 6.0) as f32,
            PARAM_SENSITIVITY => self.sensitivity = v as f32,
            PARAM_RESONANCE => self.resonance = 10.0_f32.powf(v as f32),
            PARAM_ATTACK => {
                let lo = 0.5_f32.log10();
                let hi = 200.0_f32.log10();
                self.attack_ms = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                update_env = true;
            }
            PARAM_RELEASE => {
                let lo = 5.0_f32.log10();
                let hi = 1000.0_f32.log10();
                self.release_ms = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                update_env = true;
            }
            _ => {}
        }
        if update_env {
            self.env_l
                .set_times(self.attack_ms, self.release_ms, self.sample_rate);
            self.env_r
                .set_times(self.attack_ms, self.release_ms, self.sample_rate);
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"base\":{},\"range\":{},\"sens\":{},\"res\":{},\"a\":{},\"r\":{}}}",
            self.base_hz,
            self.range_octaves,
            self.sensitivity,
            self.resonance,
            self.attack_ms,
            self.release_ms
        )
        .into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest
                .find(|c: char| c == ',' || c == '}')
                .unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("base") {
            self.base_hz = v.clamp(50.0, 2_000.0);
        }
        if let Some(v) = read("range") {
            self.range_octaves = v.clamp(0.0, 6.0);
        }
        if let Some(v) = read("sens") {
            self.sensitivity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("res") {
            self.resonance = v.clamp(0.1, 10.0);
        }
        if let Some(v) = read("a") {
            self.attack_ms = v.clamp(0.5, 200.0);
        }
        if let Some(v) = read("r") {
            self.release_ms = v.clamp(5.0, 1000.0);
        }
        self.env_l
            .set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.env_r
            .set_times(self.attack_ms, self.release_ms, self.sample_rate);
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
