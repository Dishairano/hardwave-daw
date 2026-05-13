//! Native stereo enhancer — wraps `hardwave_dsp::stereo` width / balance
//! / bass-mono primitives in a HostedPlugin. Mirrors Fruity Stereo
//! Enhancer's main controls.

use hardwave_dsp::stereo::{apply_ms_balance, apply_width, BassMono};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_WIDTH: u32 = 0;
const PARAM_BALANCE: u32 = 1;
const PARAM_BASS_MONO: u32 = 2;
const PARAM_CROSSOVER: u32 = 3;
const PARAM_COUNT: u32 = 4;

pub struct NativeStereo {
    descriptor: PluginDescriptor,
    bass_mono: BassMono,
    sample_rate: f32,
    width: f32,
    balance: f32,
    bass_mono_on: bool,
    crossover_hz: f32,
    active: bool,
}

impl NativeStereo {
    pub const ID: &'static str = "hardwave.native.stereo";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Stereo".into(),
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
            bass_mono: BassMono::new(48_000.0, 120.0),
            sample_rate: 48_000.0,
            width: 1.0,
            balance: 0.0,
            bass_mono_on: false,
            crossover_hz: 120.0,
            active: false,
        }
    }
}

impl Default for NativeStereo {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeStereo {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.bass_mono = BassMono::new(self.sample_rate, self.crossover_hz);
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
        for i in 0..num_samples {
            let mut l = inputs[0].get(i).copied().unwrap_or(0.0);
            let mut r = inputs[1].get(i).copied().unwrap_or(0.0);
            let (wl, wr) = apply_width(l, r, self.width);
            l = wl;
            r = wr;
            let (bl, br) = apply_ms_balance(l, r, self.balance);
            l = bl;
            r = br;
            if self.bass_mono_on {
                let (ml, mr) = self.bass_mono.process(l, r);
                l = ml;
                r = mr;
            }
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        match index {
            PARAM_WIDTH => Some(ParameterInfo {
                id: PARAM_WIDTH,
                name: "Width".into(),
                default_value: 0.5,
                min: 0.0,
                max: 1.0,
                unit: "".into(),
                automatable: true,
            }),
            PARAM_BALANCE => Some(ParameterInfo {
                id: PARAM_BALANCE,
                name: "Balance".into(),
                default_value: 0.5,
                min: 0.0,
                max: 1.0,
                unit: "".into(),
                automatable: true,
            }),
            PARAM_BASS_MONO => Some(ParameterInfo {
                id: PARAM_BASS_MONO,
                name: "Bass Mono".into(),
                default_value: 0.0,
                min: 0.0,
                max: 1.0,
                unit: "".into(),
                automatable: false,
            }),
            PARAM_CROSSOVER => Some(ParameterInfo {
                id: PARAM_CROSSOVER,
                name: "Crossover".into(),
                default_value: ((120.0_f64.log10() - 20.0_f64.log10())
                    / (500.0_f64.log10() - 20.0_f64.log10()))
                .clamp(0.0, 1.0),
                min: 0.0,
                max: 1.0,
                unit: "Hz".into(),
                automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            // width 0..=2 mapped to 0..=1: 0.5 = unity
            PARAM_WIDTH => (self.width / 2.0).clamp(0.0, 1.0) as f64,
            // balance -1..=1 mapped to 0..=1: 0.5 = centre
            PARAM_BALANCE => ((self.balance + 1.0) * 0.5).clamp(0.0, 1.0) as f64,
            PARAM_BASS_MONO => {
                if self.bass_mono_on {
                    1.0
                } else {
                    0.0
                }
            }
            // crossover 20..=500 Hz log
            PARAM_CROSSOVER => {
                let v = (self.crossover_hz.log10() - 20.0_f32.log10())
                    / (500.0_f32.log10() - 20.0_f32.log10());
                v.clamp(0.0, 1.0) as f64
            }
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_WIDTH => self.width = (v * 2.0) as f32,
            PARAM_BALANCE => self.balance = (v * 2.0 - 1.0) as f32,
            PARAM_BASS_MONO => self.bass_mono_on = v >= 0.5,
            PARAM_CROSSOVER => {
                let lo = 20.0_f32.log10();
                let hi = 500.0_f32.log10();
                self.crossover_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                self.bass_mono
                    .set_crossover(self.sample_rate, self.crossover_hz);
            }
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"width\":{},\"balance\":{},\"bm\":{},\"cross\":{}}}",
            self.width,
            self.balance,
            if self.bass_mono_on { 1 } else { 0 },
            self.crossover_hz
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
        if let Some(v) = read("width") {
            self.width = v.clamp(0.0, 2.0);
        }
        if let Some(v) = read("balance") {
            self.balance = v.clamp(-1.0, 1.0);
        }
        if let Some(v) = read("bm") {
            self.bass_mono_on = v >= 0.5;
        }
        if let Some(v) = read("cross") {
            self.crossover_hz = v.clamp(20.0, 500.0);
            self.bass_mono
                .set_crossover(self.sample_rate, self.crossover_hz);
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
