//! Native auto-pan — sine LFO sweeping the L/R balance. Distinct from
//! Tremolo (which modulates volume of both channels equally).

use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::f32::consts::{FRAC_PI_4, TAU};
use std::path::PathBuf;

const PARAM_RATE: u32 = 0;
const PARAM_DEPTH: u32 = 1;
const PARAM_SHAPE: u32 = 2;
const PARAM_COUNT: u32 = 3;

#[derive(Clone, Copy)]
enum Shape { Sine, Triangle, Square }

fn shape_from_norm(v: f32) -> Shape {
    let i = ((v.clamp(0.0, 1.0) * 3.0).floor() as i32).clamp(0, 2);
    match i {
        0 => Shape::Sine,
        1 => Shape::Triangle,
        _ => Shape::Square,
    }
}

fn shape_to_norm(s: Shape) -> f64 {
    let i = match s { Shape::Sine => 0, Shape::Triangle => 1, Shape::Square => 2 };
    (i as f64 + 0.5) / 3.0
}

fn lfo(shape: Shape, phase: f32) -> f32 {
    let p = phase - phase.floor();
    match shape {
        Shape::Sine => (TAU * p).sin(),
        Shape::Triangle => 4.0 * (p - (p + 0.5).floor()).abs() - 1.0,
        Shape::Square => if p < 0.5 { 1.0 } else { -1.0 },
    }
}

pub struct NativeAutoPan {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    rate_hz: f32,
    depth: f32,
    shape: Shape,
    phase: f32,
    active: bool,
}

impl NativeAutoPan {
    pub const ID: &'static str = "hardwave.native.auto_pan";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Auto-Pan".into(),
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
            sample_rate: 48_000.0,
            rate_hz: 1.0,
            depth: 0.7,
            shape: Shape::Sine,
            phase: 0.0,
            active: false,
        }
    }
}

impl Default for NativeAutoPan {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeAutoPan {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.phase = 0.0;
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
        let dphase = self.rate_hz / self.sample_rate;
        for i in 0..num_samples {
            let pan = lfo(self.shape, self.phase) * self.depth; // -depth..=depth
            // Equal-power pan — pan in [-1, 1], 0 = centre.
            let angle = (pan + 1.0) * FRAC_PI_4;
            let l_gain = angle.cos();
            let r_gain = angle.sin();
            // Mix L+R to mono first so both channels pan together.
            let mid = (inputs[0].get(i).copied().unwrap_or(0.0)
                    + inputs[1].get(i).copied().unwrap_or(0.0)) * 0.5;
            outputs[0][i] = mid * l_gain;
            outputs[1][i] = mid * r_gain;
            self.phase = (self.phase + dphase) % 1.0;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_RATE => ("Rate", (1.0_f64 / 20.0).clamp(0.0, 1.0), "Hz"),
            PARAM_DEPTH => ("Depth", 0.7, "%"),
            PARAM_SHAPE => ("Shape", shape_to_norm(Shape::Sine), ""),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_RATE => (self.rate_hz / 20.0).clamp(0.0, 1.0) as f64,
            PARAM_DEPTH => self.depth as f64,
            PARAM_SHAPE => shape_to_norm(self.shape),
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_RATE => self.rate_hz = (v * 20.0) as f32,
            PARAM_DEPTH => self.depth = v as f32,
            PARAM_SHAPE => self.shape = shape_from_norm(v as f32),
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"rate\":{},\"depth\":{},\"shape\":{}}}",
            self.rate_hz, self.depth, shape_to_norm(self.shape)
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
        if let Some(v) = read("rate") { self.rate_hz = v.clamp(0.0, 20.0); }
        if let Some(v) = read("depth") { self.depth = v.clamp(0.0, 1.0); }
        if let Some(v) = read("shape") { self.shape = shape_from_norm(v); }
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
