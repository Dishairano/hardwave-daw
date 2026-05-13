//! Native saturator — single-knob warm tube/tape character via
//! `FilterDrive`. Distinct from NativeDistortion (multi-mode hot
//! clipper) by keeping it transparent at low settings and gluing
//! tracks together when pushed.

use hardwave_dsp::synth_extras::FilterDrive;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_AMOUNT: u32 = 0;
const PARAM_OUTPUT: u32 = 1;
const PARAM_MIX: u32 = 2;
const PARAM_COUNT: u32 = 3;

pub struct NativeSaturator {
    descriptor: PluginDescriptor,
    drive_l: FilterDrive,
    drive_r: FilterDrive,
    output_db: f32,
    mix: f32,
    active: bool,
}

impl NativeSaturator {
    pub const ID: &'static str = "hardwave.native.saturator";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Saturator".into(),
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
            drive_l: FilterDrive::new(),
            drive_r: FilterDrive::new(),
            output_db: 0.0,
            mix: 1.0,
            active: false,
        }
    }
}

impl Default for NativeSaturator {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeSaturator {
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
        if !self.active || inputs.len() < 2 || outputs.len() < 2 {
            for ch in 0..outputs.len().min(inputs.len()) {
                let n = inputs[ch].len().min(num_samples);
                outputs[ch][..n].copy_from_slice(&inputs[ch][..n]);
            }
            return;
        }
        let out_lin = 10.0_f32.powf(self.output_db / 20.0);
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let wet_l = self.drive_l.process(in_l) * out_lin;
            let wet_r = self.drive_r.process(in_r) * out_lin;
            outputs[0][i] = in_l * dry + wet_l * mix;
            outputs[1][i] = in_r * dry + wet_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_AMOUNT => ("Amount", 0.3, "%"),
            PARAM_OUTPUT => ("Output", 0.5, "dB"), // 0.5 = unity
            PARAM_MIX => ("Mix", 1.0, "%"),
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
            PARAM_AMOUNT => self.drive_l.amount() as f64,
            // -24..=+12 dB
            PARAM_OUTPUT => ((self.output_db + 24.0) / 36.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_AMOUNT => {
                self.drive_l.set_amount(v as f32);
                self.drive_r.set_amount(v as f32);
            }
            PARAM_OUTPUT => self.output_db = (v * 36.0 - 24.0) as f32,
            PARAM_MIX => self.mix = v as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"amt\":{},\"out\":{},\"mix\":{}}}",
            self.drive_l.amount(),
            self.output_db,
            self.mix
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
        if let Some(v) = read("amt") {
            let amt = v.clamp(0.0, 1.0);
            self.drive_l.set_amount(amt);
            self.drive_r.set_amount(amt);
        }
        if let Some(v) = read("out") {
            self.output_db = v.clamp(-24.0, 12.0);
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
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
