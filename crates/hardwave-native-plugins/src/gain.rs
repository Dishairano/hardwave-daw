//! Native gain utility — volume in dB + pan + phase invert.
//! Mirrors Fruity Balance / Pro Q "Utility" plugin essentials.

use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::f32::consts::FRAC_PI_4;
use std::path::PathBuf;

const PARAM_GAIN: u32 = 0;
const PARAM_PAN: u32 = 1;
const PARAM_INVERT_L: u32 = 2;
const PARAM_INVERT_R: u32 = 3;
const PARAM_MUTE: u32 = 4;
const PARAM_COUNT: u32 = 5;

pub struct NativeGain {
    descriptor: PluginDescriptor,
    gain_db: f32,
    pan: f32,
    invert_l: bool,
    invert_r: bool,
    muted: bool,
    active: bool,
}

impl NativeGain {
    pub const ID: &'static str = "hardwave.native.gain";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Gain".into(),
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
            gain_db: 0.0,
            pan: 0.0,
            invert_l: false,
            invert_r: false,
            muted: false,
            active: false,
        }
    }
}

impl Default for NativeGain {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeGain {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, _sr: f64, _max: u32) -> Result<(), String> {
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
        if !self.active || self.muted {
            return;
        }
        if inputs.len() < 2 || outputs.len() < 2 {
            for ch in 0..outputs.len().min(inputs.len()) {
                let n = inputs[ch].len().min(num_samples);
                outputs[ch][..n].copy_from_slice(&inputs[ch][..n]);
            }
            return;
        }
        let gain_lin = 10.0_f32.powf(self.gain_db / 20.0);
        let angle = (self.pan + 1.0) * FRAC_PI_4;
        let l_gain = angle.cos() * gain_lin * if self.invert_l { -1.0 } else { 1.0 };
        let r_gain = angle.sin() * gain_lin * if self.invert_r { -1.0 } else { 1.0 };
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            outputs[0][i] = in_l * l_gain;
            outputs[1][i] = in_r * r_gain;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_GAIN => ("Gain", 0.5, "dB"), // 0.5 = unity in -36..+12 mapping
            PARAM_PAN => ("Pan", 0.5, ""),     // 0.5 = centre
            PARAM_INVERT_L => ("Invert L", 0.0, ""),
            PARAM_INVERT_R => ("Invert R", 0.0, ""),
            PARAM_MUTE => ("Mute", 0.0, ""),
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
            // -36..=+12 dB linear mapping, 0.5 = unity (0 dB)
            PARAM_GAIN => ((self.gain_db + 36.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_PAN => ((self.pan + 1.0) * 0.5).clamp(0.0, 1.0) as f64,
            PARAM_INVERT_L => {
                if self.invert_l {
                    1.0
                } else {
                    0.0
                }
            }
            PARAM_INVERT_R => {
                if self.invert_r {
                    1.0
                } else {
                    0.0
                }
            }
            PARAM_MUTE => {
                if self.muted {
                    1.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_GAIN => self.gain_db = (v * 48.0 - 36.0) as f32,
            PARAM_PAN => self.pan = (v * 2.0 - 1.0) as f32,
            PARAM_INVERT_L => self.invert_l = v >= 0.5,
            PARAM_INVERT_R => self.invert_r = v >= 0.5,
            PARAM_MUTE => self.muted = v >= 0.5,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"gain\":{},\"pan\":{},\"invl\":{},\"invr\":{},\"mute\":{}}}",
            self.gain_db,
            self.pan,
            if self.invert_l { 1 } else { 0 },
            if self.invert_r { 1 } else { 0 },
            if self.muted { 1 } else { 0 }
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
        if let Some(v) = read("gain") {
            self.gain_db = v.clamp(-36.0, 12.0);
        }
        if let Some(v) = read("pan") {
            self.pan = v.clamp(-1.0, 1.0);
        }
        if let Some(v) = read("invl") {
            self.invert_l = v >= 0.5;
        }
        if let Some(v) = read("invr") {
            self.invert_r = v >= 0.5;
        }
        if let Some(v) = read("mute") {
            self.muted = v >= 0.5;
        }
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
