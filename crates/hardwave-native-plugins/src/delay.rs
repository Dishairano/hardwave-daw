//! Native delay plug-in — wraps `hardwave_dsp::delay_line::StereoDelayLine`.
//! Mirrors Fruity Delay 3's main controls: time, feedback, mix, ping-pong.

use hardwave_dsp::delay_line::StereoDelayLine;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_TIME_MS: u32 = 0;
const PARAM_FEEDBACK: u32 = 1;
const PARAM_MIX: u32 = 2;
const PARAM_PING_PONG: u32 = 3;
const PARAM_COUNT: u32 = 4;

const MAX_DELAY_MS: f32 = 2000.0;

pub struct NativeDelay {
    descriptor: PluginDescriptor,
    line: StereoDelayLine,
    sample_rate: f32,
    time_ms: f32,
    feedback: f32,
    mix: f32,
    ping_pong: bool,
    active: bool,
}

impl NativeDelay {
    pub const ID: &'static str = "hardwave.native.delay";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Delay".into(),
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
        let cap = ((MAX_DELAY_MS / 1000.0) * sr) as usize + 16;
        let mut line = StereoDelayLine::new(cap);
        line.set_delay(((250.0 / 1000.0) * sr) as usize);
        line.set_feedback(0.35);
        line.set_ping_pong(false);
        Self {
            descriptor: Self::descriptor(),
            line,
            sample_rate: sr,
            time_ms: 250.0,
            feedback: 0.35,
            mix: 0.3,
            ping_pong: false,
            active: false,
        }
    }

    fn refresh_time(&mut self) {
        let s = ((self.time_ms / 1000.0) * self.sample_rate).max(1.0) as usize;
        self.line.set_delay(s);
    }
}

impl Default for NativeDelay {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeDelay {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        let cap = ((MAX_DELAY_MS / 1000.0) * self.sample_rate) as usize + 16;
        self.line = StereoDelayLine::new(cap);
        self.refresh_time();
        self.line.set_feedback(self.feedback);
        self.line.set_ping_pong(self.ping_pong);
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
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let (wet_l, wet_r) = self.line.process(in_l, in_r);
            outputs[0][i] = in_l * dry + wet_l * mix;
            outputs[1][i] = in_r * dry + wet_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        match index {
            PARAM_TIME_MS => Some(ParameterInfo {
                id: PARAM_TIME_MS, name: "Time".into(),
                default_value: (250.0_f64 / MAX_DELAY_MS as f64).clamp(0.0, 1.0),
                min: 0.0, max: 1.0, unit: "ms".into(), automatable: true,
            }),
            PARAM_FEEDBACK => Some(ParameterInfo {
                id: PARAM_FEEDBACK, name: "Feedback".into(),
                default_value: 0.35, min: 0.0, max: 1.0, unit: "%".into(), automatable: true,
            }),
            PARAM_MIX => Some(ParameterInfo {
                id: PARAM_MIX, name: "Mix".into(),
                default_value: 0.3, min: 0.0, max: 1.0, unit: "%".into(), automatable: true,
            }),
            PARAM_PING_PONG => Some(ParameterInfo {
                id: PARAM_PING_PONG, name: "Ping-Pong".into(),
                default_value: 0.0, min: 0.0, max: 1.0, unit: "".into(), automatable: false,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_TIME_MS => (self.time_ms / MAX_DELAY_MS).clamp(0.0, 1.0) as f64,
            PARAM_FEEDBACK => self.feedback.clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            PARAM_PING_PONG => if self.ping_pong { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_TIME_MS => {
                self.time_ms = (v * MAX_DELAY_MS as f64) as f32;
                self.refresh_time();
            }
            PARAM_FEEDBACK => {
                self.feedback = v as f32;
                self.line.set_feedback(self.feedback);
            }
            PARAM_MIX => self.mix = v as f32,
            PARAM_PING_PONG => {
                self.ping_pong = v >= 0.5;
                self.line.set_ping_pong(self.ping_pong);
            }
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"time\":{},\"fb\":{},\"mix\":{},\"pp\":{}}}",
            self.time_ms, self.feedback, self.mix, if self.ping_pong { 1 } else { 0 }
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
        if let Some(v) = read("time") { self.time_ms = v.clamp(1.0, MAX_DELAY_MS); }
        if let Some(v) = read("fb") { self.feedback = v.clamp(0.0, 1.0); }
        if let Some(v) = read("mix") { self.mix = v.clamp(0.0, 1.0); }
        if let Some(v) = read("pp") { self.ping_pong = v >= 0.5; }
        self.refresh_time();
        self.line.set_feedback(self.feedback);
        self.line.set_ping_pong(self.ping_pong);
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
