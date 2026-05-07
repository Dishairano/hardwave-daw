//! Native transient designer — boost or attenuate the attack and
//! sustain portions of a signal independently. Mirrors SPL Transient
//! Designer / FabFilter Pro-MB transient mode using a fast/slow
//! envelope follower differential.
//!
//! Algorithm: track two envelopes — fast (1 ms attack) and slow
//! (50 ms attack). The difference (fast - slow) is high during
//! transients and low during sustain. Modulate gain accordingly.

use hardwave_dsp::dynamics::{linear_to_db, DetectMode, EnvelopeFollower};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_ATTACK: u32 = 0;
const PARAM_SUSTAIN: u32 = 1;
const PARAM_OUTPUT: u32 = 2;
const PARAM_COUNT: u32 = 3;

pub struct NativeTransient {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    env_fast_l: EnvelopeFollower,
    env_fast_r: EnvelopeFollower,
    env_slow_l: EnvelopeFollower,
    env_slow_r: EnvelopeFollower,
    /// -12..=+12 dB shift for transient portion
    attack_db: f32,
    /// -12..=+12 dB shift for sustain portion
    sustain_db: f32,
    output_db: f32,
    active: bool,
}

impl NativeTransient {
    pub const ID: &'static str = "hardwave.native.transient";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Transient".into(),
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
        let mut env_fast_l = EnvelopeFollower::default();
        let mut env_fast_r = EnvelopeFollower::default();
        env_fast_l.set_mode(DetectMode::Peak);
        env_fast_r.set_mode(DetectMode::Peak);
        env_fast_l.set_times(1.0, 30.0, sr);
        env_fast_r.set_times(1.0, 30.0, sr);
        let mut env_slow_l = EnvelopeFollower::default();
        let mut env_slow_r = EnvelopeFollower::default();
        env_slow_l.set_mode(DetectMode::Peak);
        env_slow_r.set_mode(DetectMode::Peak);
        env_slow_l.set_times(50.0, 200.0, sr);
        env_slow_r.set_times(50.0, 200.0, sr);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            env_fast_l,
            env_fast_r,
            env_slow_l,
            env_slow_r,
            attack_db: 0.0,
            sustain_db: 0.0,
            output_db: 0.0,
            active: false,
        }
    }
}

impl Default for NativeTransient {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeTransient {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.env_fast_l.set_times(1.0, 30.0, self.sample_rate);
        self.env_fast_r.set_times(1.0, 30.0, self.sample_rate);
        self.env_slow_l.set_times(50.0, 200.0, self.sample_rate);
        self.env_slow_r.set_times(50.0, 200.0, self.sample_rate);
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
        let out_lin = 10.0_f32.powf(self.output_db / 20.0);
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let fast_l = self.env_fast_l.process(in_l);
            let fast_r = self.env_fast_r.process(in_r);
            let slow_l = self.env_slow_l.process(in_l);
            let slow_r = self.env_slow_r.process(in_r);
            // Transient amount = how much fast > slow (clamped 0..1)
            let trans_l = ((linear_to_db(fast_l) - linear_to_db(slow_l) + 6.0) / 12.0).clamp(0.0, 1.0);
            let trans_r = ((linear_to_db(fast_r) - linear_to_db(slow_r) + 6.0) / 12.0).clamp(0.0, 1.0);
            // Apply attack gain to transient portion, sustain gain to rest.
            let gain_l_db = trans_l * self.attack_db + (1.0 - trans_l) * self.sustain_db;
            let gain_r_db = trans_r * self.attack_db + (1.0 - trans_r) * self.sustain_db;
            let gain_l = 10.0_f32.powf(gain_l_db / 20.0) * out_lin;
            let gain_r = 10.0_f32.powf(gain_r_db / 20.0) * out_lin;
            outputs[0][i] = in_l * gain_l;
            outputs[1][i] = in_r * gain_r;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_ATTACK => ("Attack", 0.5, "dB"),  // 0.5 = unity (-12..+12 mapping)
            PARAM_SUSTAIN => ("Sustain", 0.5, "dB"),
            PARAM_OUTPUT => ("Output", 0.5, "dB"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            // -12..+12 dB → 0..1, 0.5 = unity
            PARAM_ATTACK => ((self.attack_db + 12.0) / 24.0).clamp(0.0, 1.0) as f64,
            PARAM_SUSTAIN => ((self.sustain_db + 12.0) / 24.0).clamp(0.0, 1.0) as f64,
            PARAM_OUTPUT => ((self.output_db + 12.0) / 24.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_ATTACK => self.attack_db = (v * 24.0 - 12.0) as f32,
            PARAM_SUSTAIN => self.sustain_db = (v * 24.0 - 12.0) as f32,
            PARAM_OUTPUT => self.output_db = (v * 24.0 - 12.0) as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"a\":{},\"s\":{},\"o\":{}}}",
            self.attack_db, self.sustain_db, self.output_db
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
        if let Some(v) = read("a") { self.attack_db = v.clamp(-12.0, 12.0); }
        if let Some(v) = read("s") { self.sustain_db = v.clamp(-12.0, 12.0); }
        if let Some(v) = read("o") { self.output_db = v.clamp(-12.0, 12.0); }
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
