//! Native hard clipper — dedicated brick-wall instant clipper.
//! Distinct from NativeLimiter (lookahead, smooth) and NativeDistortion
//! (multi-mode coloured) by being literal: drive, ceiling, instant clip.

use hardwave_dsp::distortion::hard_clip;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_DRIVE: u32 = 0;
const PARAM_CEILING: u32 = 1;
const PARAM_AUTO_GAIN: u32 = 2;
const PARAM_COUNT: u32 = 3;

pub struct NativeClipper {
    descriptor: PluginDescriptor,
    drive_db: f32,
    ceiling: f32,
    auto_gain: bool,
    active: bool,
}

impl NativeClipper {
    pub const ID: &'static str = "hardwave.native.clipper";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Clipper".into(),
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
        Self {
            descriptor: Self::descriptor(),
            drive_db: 0.0,
            ceiling: 0.95,
            auto_gain: true,
            active: false,
        }
    }
}

impl Default for NativeClipper {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeClipper {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, _sr: f64, _max: u32) -> Result<(), String> {
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
        let drive_lin = 10.0_f32.powf(self.drive_db / 20.0);
        // Auto-gain compensates for drive so clipper doesn't get louder.
        let comp = if self.auto_gain { 1.0 / drive_lin.max(1e-6) } else { 1.0 };
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            outputs[0][i] = hard_clip(in_l * drive_lin, self.ceiling) * comp;
            outputs[1][i] = hard_clip(in_r * drive_lin, self.ceiling) * comp;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_DRIVE => ("Drive", 0.0, "dB"),
            PARAM_CEILING => ("Ceiling", 0.95, ""),
            PARAM_AUTO_GAIN => ("Auto-Gain", 1.0, ""),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            // 0..36 dB
            PARAM_DRIVE => (self.drive_db / 36.0).clamp(0.0, 1.0) as f64,
            PARAM_CEILING => self.ceiling.clamp(0.0, 1.0) as f64,
            PARAM_AUTO_GAIN => if self.auto_gain { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_DRIVE => self.drive_db = (v * 36.0) as f32,
            PARAM_CEILING => self.ceiling = v.max(0.01) as f32,
            PARAM_AUTO_GAIN => self.auto_gain = v >= 0.5,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"drive\":{},\"ceil\":{},\"auto\":{}}}",
            self.drive_db, self.ceiling, if self.auto_gain {1} else {0}
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
        if let Some(v) = read("drive") { self.drive_db = v.clamp(0.0, 36.0); }
        if let Some(v) = read("ceil") { self.ceiling = v.clamp(0.01, 1.0); }
        if let Some(v) = read("auto") { self.auto_gain = v >= 0.5; }
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
