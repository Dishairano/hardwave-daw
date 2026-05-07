//! Native noise gate — wraps `gate_gain` with an envelope follower.
//! Mirrors Fruity Limiter's gate mode at the basic level: threshold,
//! range (dB of attenuation), attack/release, hysteresis.

use hardwave_dsp::dynamics::{gate_gain, linear_to_db, DetectMode, EnvelopeFollower};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_THRESHOLD: u32 = 0;
const PARAM_RANGE: u32 = 1;
const PARAM_ATTACK: u32 = 2;
const PARAM_RELEASE: u32 = 3;
const PARAM_HYSTERESIS: u32 = 4;
const PARAM_COUNT: u32 = 5;

pub struct NativeGate {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    env_l: EnvelopeFollower,
    env_r: EnvelopeFollower,
    threshold_db: f32,
    range_db: f32,
    attack_ms: f32,
    release_ms: f32,
    hysteresis_db: f32,
    active: bool,
}

impl NativeGate {
    pub const ID: &'static str = "hardwave.native.gate";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Gate".into(),
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
        env_l.set_times(1.0, 50.0, sr);
        env_r.set_times(1.0, 50.0, sr);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            env_l,
            env_r,
            threshold_db: -40.0,
            range_db: 60.0,
            attack_ms: 1.0,
            release_ms: 50.0,
            hysteresis_db: 3.0,
            active: false,
        }
    }
}

impl Default for NativeGate {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeGate {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.env_l.set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.env_r.set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.active = true;
        Ok(())
    }
    fn deactivate(&mut self) { self.active = false; }

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
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let env_l = self.env_l.process(in_l);
            let env_r = self.env_r.process(in_r);
            let level_db = linear_to_db(env_l.max(env_r));
            let gain = gate_gain(level_db, self.threshold_db, self.range_db, self.hysteresis_db);
            outputs[0][i] = in_l * gain;
            outputs[1][i] = in_r * gain;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            // -80..0 mapped to 0..1
            PARAM_THRESHOLD => ("Threshold", ((-40.0_f64 + 80.0) / 80.0).clamp(0.0, 1.0), "dB"),
            // 0..80 dB
            PARAM_RANGE => ("Range", 60.0_f64 / 80.0, "dB"),
            // 0.1..200 ms log
            PARAM_ATTACK => ("Attack", ((1.0_f64.log10() - 0.1_f64.log10()) / (200.0_f64.log10() - 0.1_f64.log10())).clamp(0.0, 1.0), "ms"),
            PARAM_RELEASE => ("Release", ((50.0_f64.log10() - 0.1_f64.log10()) / (2000.0_f64.log10() - 0.1_f64.log10())).clamp(0.0, 1.0), "ms"),
            PARAM_HYSTERESIS => ("Hysteresis", 3.0 / 12.0, "dB"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_THRESHOLD => ((self.threshold_db + 80.0) / 80.0).clamp(0.0, 1.0) as f64,
            PARAM_RANGE => (self.range_db / 80.0).clamp(0.0, 1.0) as f64,
            PARAM_ATTACK => {
                let lo = 0.1_f32.log10();
                let hi = 200.0_f32.log10();
                ((self.attack_ms.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_RELEASE => {
                let lo = 0.1_f32.log10();
                let hi = 2000.0_f32.log10();
                ((self.release_ms.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_HYSTERESIS => (self.hysteresis_db / 12.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        let mut update_env = false;
        match id {
            PARAM_THRESHOLD => self.threshold_db = (v * 80.0 - 80.0) as f32,
            PARAM_RANGE => self.range_db = (v * 80.0) as f32,
            PARAM_ATTACK => {
                let lo = 0.1_f32.log10();
                let hi = 200.0_f32.log10();
                self.attack_ms = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                update_env = true;
            }
            PARAM_RELEASE => {
                let lo = 0.1_f32.log10();
                let hi = 2000.0_f32.log10();
                self.release_ms = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                update_env = true;
            }
            PARAM_HYSTERESIS => self.hysteresis_db = (v * 12.0) as f32,
            _ => {}
        }
        if update_env {
            self.env_l.set_times(self.attack_ms, self.release_ms, self.sample_rate);
            self.env_r.set_times(self.attack_ms, self.release_ms, self.sample_rate);
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"thr\":{},\"rng\":{},\"a\":{},\"r\":{},\"hys\":{}}}",
            self.threshold_db, self.range_db, self.attack_ms, self.release_ms, self.hysteresis_db
        ).into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest.find(|c: char| c == ',' || c == '}').unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("thr") { self.threshold_db = v.clamp(-80.0, 0.0); }
        if let Some(v) = read("rng") { self.range_db = v.clamp(0.0, 80.0); }
        if let Some(v) = read("a") { self.attack_ms = v.clamp(0.1, 200.0); }
        if let Some(v) = read("r") { self.release_ms = v.clamp(0.1, 2000.0); }
        if let Some(v) = read("hys") { self.hysteresis_db = v.clamp(0.0, 12.0); }
        self.env_l.set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.env_r.set_times(self.attack_ms, self.release_ms, self.sample_rate);
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
