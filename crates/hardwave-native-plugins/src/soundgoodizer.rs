//! Native one-knob mastering finisher — saturation + soft limit chain.
//! Mirrors FL Studio Soundgoodizer / Waves L1 baseline. Single Amount
//! knob plus output trim. Distinct from individual Saturator + Limiter
//! by being preset + one-shot for non-mastering producers.

use hardwave_dsp::dynamics::{compressor_gain_reduction_db, db_to_linear, linear_to_db, DetectMode, EnvelopeFollower};
use hardwave_dsp::synth_extras::FilterDrive;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_AMOUNT: u32 = 0;
const PARAM_OUTPUT: u32 = 1;
const PARAM_COUNT: u32 = 2;

pub struct NativeSoundgoodizer {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    drive_l: FilterDrive,
    drive_r: FilterDrive,
    env_l: EnvelopeFollower,
    env_r: EnvelopeFollower,
    amount: f32,
    output_db: f32,
    active: bool,
}

impl NativeSoundgoodizer {
    pub const ID: &'static str = "hardwave.native.soundgoodizer";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Master".into(),
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
        env_l.set_times(0.5, 50.0, sr);
        env_r.set_times(0.5, 50.0, sr);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            drive_l: FilterDrive::new(),
            drive_r: FilterDrive::new(),
            env_l,
            env_r,
            amount: 0.4,
            output_db: 0.0,
            active: false,
        }
    }

    fn refresh(&mut self) {
        // Amount drives saturation; chained limiter has fixed -1 dB ceiling.
        self.drive_l.set_amount(self.amount);
        self.drive_r.set_amount(self.amount);
    }
}

impl Default for NativeSoundgoodizer {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeSoundgoodizer {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.env_l.set_times(0.5, 50.0, self.sample_rate);
        self.env_r.set_times(0.5, 50.0, self.sample_rate);
        self.refresh();
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
        let ceiling_db = -1.0_f32;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            // Saturate
            let sat_l = self.drive_l.process(in_l);
            let sat_r = self.drive_r.process(in_r);
            // Track peak envelope
            let env_l = self.env_l.process(sat_l);
            let env_r = self.env_r.process(sat_r);
            let env_db_l = linear_to_db(env_l.max(1e-6));
            let env_db_r = linear_to_db(env_r.max(1e-6));
            // Soft brick-wall limit
            let red_l = compressor_gain_reduction_db(env_db_l, ceiling_db, 100.0, 0.5);
            let red_r = compressor_gain_reduction_db(env_db_r, ceiling_db, 100.0, 0.5);
            let lim_l = sat_l * db_to_linear(red_l);
            let lim_r = sat_r * db_to_linear(red_r);
            outputs[0][i] = lim_l * out_lin;
            outputs[1][i] = lim_r * out_lin;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_AMOUNT => ("Amount", 0.4, "%"),
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
            PARAM_AMOUNT => self.amount as f64,
            PARAM_OUTPUT => ((self.output_db + 12.0) / 24.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_AMOUNT => {
                self.amount = v as f32;
                self.refresh();
            }
            PARAM_OUTPUT => self.output_db = (v * 24.0 - 12.0) as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!("{{\"amt\":{},\"out\":{}}}", self.amount, self.output_db).into_bytes()
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
        if let Some(v) = read("amt") { self.amount = v.clamp(0.0, 1.0); }
        if let Some(v) = read("out") { self.output_db = v.clamp(-12.0, 12.0); }
        self.refresh();
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
