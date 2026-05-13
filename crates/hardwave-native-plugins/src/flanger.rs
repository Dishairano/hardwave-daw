//! Native flanger — ModulatedDelay with very short base (0.1-5 ms) and
//! optionally negative feedback for resonant comb filtering.
//! Distinct from Chorus (longer delay, low feedback) by focusing on
//! the metallic comb-filter character.

use hardwave_dsp::modulation::ModulatedDelay;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const MAX_DELAY_SAMPLES: usize = 1024;

const PARAM_RATE: u32 = 0;
const PARAM_DEPTH: u32 = 1;
const PARAM_FEEDBACK: u32 = 2;
const PARAM_MIX: u32 = 3;
const PARAM_INVERT: u32 = 4;
const PARAM_COUNT: u32 = 5;

pub struct NativeFlanger {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    delay_l: ModulatedDelay,
    delay_r: ModulatedDelay,
    rate_hz: f32,
    depth_ms: f32,
    feedback: f32,
    mix: f32,
    invert: bool,
    active: bool,
}

impl NativeFlanger {
    pub const ID: &'static str = "hardwave.native.flanger";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Flanger".into(),
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
        let mut delay_l = ModulatedDelay::new(MAX_DELAY_SAMPLES, sr);
        let mut delay_r = ModulatedDelay::new(MAX_DELAY_SAMPLES, sr);
        delay_l.set_base_delay(0.001 * sr); // 1 ms
        delay_r.set_base_delay(0.001 * sr);
        delay_l.set_lfo_phase_offset(0.0);
        delay_r.set_lfo_phase_offset(0.5);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            delay_l,
            delay_r,
            rate_hz: 0.3,
            depth_ms: 0.5,
            feedback: 0.6,
            mix: 0.5,
            invert: false,
            active: false,
        }
    }

    fn refresh(&mut self) {
        self.delay_l.set_lfo_rate(self.rate_hz);
        self.delay_r.set_lfo_rate(self.rate_hz);
        let depth_samples = (self.depth_ms / 1000.0) * self.sample_rate;
        self.delay_l.set_lfo_depth(depth_samples);
        self.delay_r.set_lfo_depth(depth_samples);
        let fb = if self.invert {
            -self.feedback
        } else {
            self.feedback
        };
        self.delay_l.set_feedback(fb);
        self.delay_r.set_feedback(fb);
    }
}

impl Default for NativeFlanger {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeFlanger {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.delay_l = ModulatedDelay::new(MAX_DELAY_SAMPLES, self.sample_rate);
        self.delay_r = ModulatedDelay::new(MAX_DELAY_SAMPLES, self.sample_rate);
        self.delay_l.set_base_delay(0.001 * self.sample_rate);
        self.delay_r.set_base_delay(0.001 * self.sample_rate);
        self.delay_l.set_lfo_phase_offset(0.0);
        self.delay_r.set_lfo_phase_offset(0.5);
        self.refresh();
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
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let wet_l = self.delay_l.process(in_l);
            let wet_r = self.delay_r.process(in_r);
            outputs[0][i] = in_l * dry + wet_l * mix;
            outputs[1][i] = in_r * dry + wet_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_RATE => ("Rate", (0.3_f64 / 5.0).clamp(0.0, 1.0), "Hz"),
            PARAM_DEPTH => ("Depth", 0.5 / 5.0, "ms"),
            PARAM_FEEDBACK => ("Feedback", 0.6, "%"),
            PARAM_MIX => ("Mix", 0.5, "%"),
            PARAM_INVERT => ("Invert", 0.0, ""),
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
            PARAM_RATE => (self.rate_hz / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_DEPTH => (self.depth_ms / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_FEEDBACK => self.feedback as f64,
            PARAM_MIX => self.mix as f64,
            PARAM_INVERT => {
                if self.invert {
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
            PARAM_RATE => self.rate_hz = (v * 5.0) as f32,
            PARAM_DEPTH => self.depth_ms = (v * 5.0) as f32,
            PARAM_FEEDBACK => self.feedback = v.clamp(0.0, 0.95) as f32,
            PARAM_MIX => self.mix = v as f32,
            PARAM_INVERT => self.invert = v >= 0.5,
            _ => {}
        }
        self.refresh();
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"rate\":{},\"depth\":{},\"fb\":{},\"mix\":{},\"inv\":{}}}",
            self.rate_hz,
            self.depth_ms,
            self.feedback,
            self.mix,
            if self.invert { 1 } else { 0 }
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
        if let Some(v) = read("rate") {
            self.rate_hz = v.clamp(0.0, 5.0);
        }
        if let Some(v) = read("depth") {
            self.depth_ms = v.clamp(0.0, 5.0);
        }
        if let Some(v) = read("fb") {
            self.feedback = v.clamp(0.0, 0.95);
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("inv") {
            self.invert = v >= 0.5;
        }
        self.refresh();
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
