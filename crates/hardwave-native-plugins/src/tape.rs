//! Native tape emulator — slow wow modulation (pitch wobble) +
//! saturation + high-frequency loss. Mirrors Waves J37 / U-He Satin
//! at the basic-features level. Distinct from NativeVibrato (which
//! is purely modulation, no saturation/HF-loss colour).

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use hardwave_dsp::modulation::ModulatedDelay;
use hardwave_dsp::synth_extras::FilterDrive;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const MAX_DELAY_SAMPLES: usize = 2048;

const PARAM_WOW: u32 = 0;
const PARAM_DRIVE: u32 = 1;
const PARAM_HF_LOSS: u32 = 2;
const PARAM_OUTPUT: u32 = 3;
const PARAM_COUNT: u32 = 4;

pub struct NativeTape {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    wow_l: ModulatedDelay,
    wow_r: ModulatedDelay,
    drive_l: FilterDrive,
    drive_r: FilterDrive,
    lp_l: Biquad,
    lp_r: Biquad,
    wow_amount: f32,
    hf_loss_hz: f32,
    output_db: f32,
    needs_recoef: bool,
    active: bool,
}

impl NativeTape {
    pub const ID: &'static str = "hardwave.native.tape";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Tape".into(),
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
        let mut wow_l = ModulatedDelay::new(MAX_DELAY_SAMPLES, sr);
        let mut wow_r = ModulatedDelay::new(MAX_DELAY_SAMPLES, sr);
        let base = 0.005 * sr; // 5 ms
        wow_l.set_base_delay(base);
        wow_r.set_base_delay(base);
        wow_l.set_lfo_rate(0.5);
        wow_r.set_lfo_rate(0.5);
        wow_l.set_lfo_phase_offset(0.0);
        wow_r.set_lfo_phase_offset(0.5);
        wow_l.set_feedback(0.0);
        wow_r.set_feedback(0.0);
        let mut s = Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            wow_l,
            wow_r,
            drive_l: FilterDrive::new(),
            drive_r: FilterDrive::new(),
            lp_l: Biquad::default(),
            lp_r: Biquad::default(),
            wow_amount: 0.3,
            hf_loss_hz: 12_000.0,
            output_db: 0.0,
            needs_recoef: true,
            active: false,
        };
        s.drive_l.set_amount(0.4);
        s.drive_r.set_amount(0.4);
        s
    }

    fn ensure_coefs(&mut self) {
        if !self.needs_recoef {
            return;
        }
        self.lp_l.set(
            BiquadKind::LowPass,
            self.sample_rate,
            self.hf_loss_hz,
            0.707,
            0.0,
        );
        self.lp_r.set(
            BiquadKind::LowPass,
            self.sample_rate,
            self.hf_loss_hz,
            0.707,
            0.0,
        );
        self.needs_recoef = false;
    }

    fn refresh_wow(&mut self) {
        // Wow depth scales 0..1 → 0..3 ms peak deviation
        let depth_samples = self.wow_amount * 0.003 * self.sample_rate;
        self.wow_l.set_lfo_depth(depth_samples);
        self.wow_r.set_lfo_depth(depth_samples);
    }
}

impl Default for NativeTape {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeTape {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.wow_l = ModulatedDelay::new(MAX_DELAY_SAMPLES, self.sample_rate);
        self.wow_r = ModulatedDelay::new(MAX_DELAY_SAMPLES, self.sample_rate);
        let base = 0.005 * self.sample_rate;
        self.wow_l.set_base_delay(base);
        self.wow_r.set_base_delay(base);
        self.wow_l.set_lfo_rate(0.5);
        self.wow_r.set_lfo_rate(0.5);
        self.wow_l.set_lfo_phase_offset(0.0);
        self.wow_r.set_lfo_phase_offset(0.5);
        self.refresh_wow();
        self.lp_l.reset();
        self.lp_r.reset();
        self.needs_recoef = true;
        self.ensure_coefs();
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
        self.ensure_coefs();
        let out_lin = 10.0_f32.powf(self.output_db / 20.0);
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            // Wow → saturation → low-pass → output
            let wow_l = self.wow_l.process(in_l);
            let wow_r = self.wow_r.process(in_r);
            let sat_l = self.drive_l.process(wow_l);
            let sat_r = self.drive_r.process(wow_r);
            let lp_l = self.lp_l.process_mono(sat_l);
            let lp_r = self.lp_r.process_mono(sat_r);
            outputs[0][i] = lp_l * out_lin;
            outputs[1][i] = lp_r * out_lin;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_WOW => ("Wow", 0.3, "%"),
            PARAM_DRIVE => ("Drive", 0.4, "%"),
            PARAM_HF_LOSS => (
                "HF Loss",
                ((12_000.0_f64.log10() - 4_000.0_f64.log10())
                    / (20_000.0_f64.log10() - 4_000.0_f64.log10()))
                .clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_OUTPUT => ("Output", 0.5, "dB"),
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
            PARAM_WOW => self.wow_amount as f64,
            PARAM_DRIVE => self.drive_l.amount() as f64,
            PARAM_HF_LOSS => {
                let lo = 4_000.0_f32.log10();
                let hi = 20_000.0_f32.log10();
                ((self.hf_loss_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_OUTPUT => ((self.output_db + 12.0) / 24.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_WOW => {
                self.wow_amount = v as f32;
                self.refresh_wow();
            }
            PARAM_DRIVE => {
                self.drive_l.set_amount(v as f32);
                self.drive_r.set_amount(v as f32);
            }
            PARAM_HF_LOSS => {
                let lo = 4_000.0_f32.log10();
                let hi = 20_000.0_f32.log10();
                self.hf_loss_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                self.needs_recoef = true;
            }
            PARAM_OUTPUT => self.output_db = (v * 24.0 - 12.0) as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"wow\":{},\"drv\":{},\"hf\":{},\"out\":{}}}",
            self.wow_amount,
            self.drive_l.amount(),
            self.hf_loss_hz,
            self.output_db
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
        if let Some(v) = read("wow") {
            self.wow_amount = v.clamp(0.0, 1.0);
            self.refresh_wow();
        }
        if let Some(v) = read("drv") {
            let amt = v.clamp(0.0, 1.0);
            self.drive_l.set_amount(amt);
            self.drive_r.set_amount(amt);
        }
        if let Some(v) = read("hf") {
            self.hf_loss_hz = v.clamp(4_000.0, 20_000.0);
            self.needs_recoef = true;
        }
        if let Some(v) = read("out") {
            self.output_db = v.clamp(-12.0, 12.0);
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
