//! Native convolution reverb — wraps `ConvolutionReverb` plus the
//! procedural IR library (LargeHall / SmallRoom / Plate / Spring).
//! Mirrors Fruity Convolver / Logic Space Designer at the basic-
//! features level: pick a space, set mix, pre-delay, tail tone.

use hardwave_dsp::convolution::ConvolutionReverb;
use hardwave_dsp::ir_library::{synthesize_ir, IrPreset};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_PRESET: u32 = 0;
const PARAM_PRE_DELAY: u32 = 1;
const PARAM_LOW_CPU: u32 = 2;
const PARAM_LOW_CUT: u32 = 3;
const PARAM_HIGH_CUT: u32 = 4;
const PARAM_WIDTH: u32 = 5;
const PARAM_MIX: u32 = 6;
const PARAM_COUNT: u32 = 7;

fn preset_from_norm(v: f32) -> IrPreset {
    let i = ((v.clamp(0.0, 1.0) * 4.0).floor() as i32).clamp(0, 3);
    match i {
        0 => IrPreset::LargeHall,
        1 => IrPreset::SmallRoom,
        2 => IrPreset::Plate,
        _ => IrPreset::Spring,
    }
}

fn preset_to_norm(p: IrPreset) -> f64 {
    let idx = match p {
        IrPreset::LargeHall => 0,
        IrPreset::SmallRoom => 1,
        IrPreset::Plate => 2,
        IrPreset::Spring => 3,
    };
    (idx as f64 + 0.5) / 4.0
}

pub struct NativeConvReverb {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    rev: ConvolutionReverb,
    preset: IrPreset,
    pre_delay_ms: f32,
    low_cpu: f32,
    low_cut_hz: f32,
    high_cut_hz: f32,
    width: f32,
    mix: f32,
    active: bool,
}

impl NativeConvReverb {
    pub const ID: &'static str = "hardwave.native.conv_reverb";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Conv Reverb".into(),
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
        let mut rev = ConvolutionReverb::new(sr);
        let (l, r) = synthesize_ir(IrPreset::Plate, sr);
        rev.load_ir_stereo(&l, &r);
        rev.set_mix(0.3);
        rev.set_pre_delay_ms(15.0);
        rev.set_stereo_width(1.0);
        rev.set_tail_eq(120.0, 12_000.0);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            rev,
            preset: IrPreset::Plate,
            pre_delay_ms: 15.0,
            low_cpu: 1.0,
            low_cut_hz: 120.0,
            high_cut_hz: 12_000.0,
            width: 1.0,
            mix: 0.3,
            active: false,
        }
    }

    fn rebuild_ir(&mut self) {
        let (l, r) = synthesize_ir(self.preset, self.sample_rate);
        self.rev.load_ir_stereo(&l, &r);
        self.rev.set_low_cpu_mode(self.low_cpu);
    }

    fn refresh(&mut self) {
        self.rev.set_mix(self.mix);
        self.rev.set_pre_delay_ms(self.pre_delay_ms);
        self.rev.set_stereo_width(self.width);
        self.rev.set_tail_eq(self.low_cut_hz, self.high_cut_hz);
    }
}

impl Default for NativeConvReverb {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeConvReverb {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.rev = ConvolutionReverb::new(self.sample_rate);
        self.rebuild_ir();
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
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let (l, r) = self.rev.process(in_l, in_r);
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_PRESET => ("Preset", preset_to_norm(IrPreset::Plate), ""),
            PARAM_PRE_DELAY => ("Pre-Delay", (15.0_f64 / 200.0).clamp(0.0, 1.0), "ms"),
            PARAM_LOW_CPU => ("Low-CPU", 1.0, ""),
            PARAM_LOW_CUT => (
                "Low Cut",
                ((120.0_f64.log10() - 20.0_f64.log10()) / (1_000.0_f64.log10() - 20.0_f64.log10())).clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_HIGH_CUT => (
                "High Cut",
                ((12_000.0_f64.log10() - 1_000.0_f64.log10()) / (20_000.0_f64.log10() - 1_000.0_f64.log10())).clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_WIDTH => ("Width", 0.5, ""),
            PARAM_MIX => ("Mix", 0.3, "%"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_PRESET => preset_to_norm(self.preset),
            PARAM_PRE_DELAY => (self.pre_delay_ms / 200.0).clamp(0.0, 1.0) as f64,
            PARAM_LOW_CPU => self.low_cpu.clamp(0.0, 1.0) as f64,
            PARAM_LOW_CUT => {
                let lo = 20.0_f32.log10();
                let hi = 1_000.0_f32.log10();
                ((self.low_cut_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_HIGH_CUT => {
                let lo = 1_000.0_f32.log10();
                let hi = 20_000.0_f32.log10();
                ((self.high_cut_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_WIDTH => (self.width / 2.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_PRESET => {
                self.preset = preset_from_norm(v as f32);
                self.rebuild_ir();
                self.refresh();
            }
            PARAM_PRE_DELAY => {
                self.pre_delay_ms = (v * 200.0) as f32;
                self.rev.set_pre_delay_ms(self.pre_delay_ms);
            }
            PARAM_LOW_CPU => {
                self.low_cpu = v.clamp(0.1, 1.0) as f32;
                self.rev.set_low_cpu_mode(self.low_cpu);
            }
            PARAM_LOW_CUT => {
                let lo = 20.0_f32.log10();
                let hi = 1_000.0_f32.log10();
                self.low_cut_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                self.rev.set_tail_eq(self.low_cut_hz, self.high_cut_hz);
            }
            PARAM_HIGH_CUT => {
                let lo = 1_000.0_f32.log10();
                let hi = 20_000.0_f32.log10();
                self.high_cut_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                self.rev.set_tail_eq(self.low_cut_hz, self.high_cut_hz);
            }
            PARAM_WIDTH => {
                self.width = (v * 2.0) as f32;
                self.rev.set_stereo_width(self.width);
            }
            PARAM_MIX => {
                self.mix = v as f32;
                self.rev.set_mix(self.mix);
            }
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"preset\":{},\"pre\":{},\"cpu\":{},\"lc\":{},\"hc\":{},\"w\":{},\"mix\":{}}}",
            preset_to_norm(self.preset), self.pre_delay_ms, self.low_cpu,
            self.low_cut_hz, self.high_cut_hz, self.width, self.mix
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
        if let Some(v) = read("preset") { self.preset = preset_from_norm(v); }
        if let Some(v) = read("pre") { self.pre_delay_ms = v.clamp(0.0, 200.0); }
        if let Some(v) = read("cpu") { self.low_cpu = v.clamp(0.1, 1.0); }
        if let Some(v) = read("lc") { self.low_cut_hz = v.clamp(20.0, 1_000.0); }
        if let Some(v) = read("hc") { self.high_cut_hz = v.clamp(1_000.0, 20_000.0); }
        if let Some(v) = read("w") { self.width = v.clamp(0.0, 2.0); }
        if let Some(v) = read("mix") { self.mix = v.clamp(0.0, 1.0); }
        self.rebuild_ir();
        self.refresh();
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
