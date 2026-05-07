//! Native reverb plug-in — wraps `hardwave_dsp::reverb::AlgorithmicReverb`
//! in HostedPlugin. Mirrors Fruity Reverb 2's main controls.

use hardwave_dsp::reverb::AlgorithmicReverb;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_ROOM_SIZE: u32 = 0;
const PARAM_DECAY: u32 = 1;
const PARAM_DAMPING: u32 = 2;
const PARAM_PRE_DELAY: u32 = 3;
const PARAM_MIX: u32 = 4;
const PARAM_COUNT: u32 = 5;

pub struct NativeReverb {
    descriptor: PluginDescriptor,
    rev: AlgorithmicReverb,
    sample_rate: f32,
    room_size: f32,
    decay_secs: f32,
    damping: f32,
    pre_delay_ms: f32,
    mix: f32,
    active: bool,
}

impl NativeReverb {
    pub const ID: &'static str = "hardwave.native.reverb";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Reverb".into(),
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
            rev: AlgorithmicReverb::new(48_000.0),
            sample_rate: 48_000.0,
            room_size: 0.6,
            decay_secs: 2.0,
            damping: 0.5,
            pre_delay_ms: 20.0,
            mix: 0.3,
            active: false,
        }
    }

    fn refresh(&mut self) {
        self.rev.set_room_size(self.room_size);
        self.rev.set_decay_time(self.decay_secs);
        self.rev.set_damping(self.damping);
        self.rev.set_pre_delay_ms(self.pre_delay_ms);
        self.rev.set_mix(self.mix);
    }
}

impl Default for NativeReverb {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeReverb {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.rev = AlgorithmicReverb::new(self.sample_rate);
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
        match index {
            PARAM_ROOM_SIZE => Some(ParameterInfo {
                id: PARAM_ROOM_SIZE, name: "Size".into(),
                default_value: 0.6, min: 0.0, max: 1.0, unit: "".into(), automatable: true,
            }),
            PARAM_DECAY => Some(ParameterInfo {
                id: PARAM_DECAY, name: "Decay".into(),
                default_value: ((2.0_f64 - 0.1) / 9.9).clamp(0.0, 1.0),
                min: 0.0, max: 1.0, unit: "s".into(), automatable: true,
            }),
            PARAM_DAMPING => Some(ParameterInfo {
                id: PARAM_DAMPING, name: "Damping".into(),
                default_value: 0.5, min: 0.0, max: 1.0, unit: "".into(), automatable: true,
            }),
            PARAM_PRE_DELAY => Some(ParameterInfo {
                id: PARAM_PRE_DELAY, name: "Pre-Delay".into(),
                default_value: (20.0_f64 / 200.0).clamp(0.0, 1.0),
                min: 0.0, max: 1.0, unit: "ms".into(), automatable: true,
            }),
            PARAM_MIX => Some(ParameterInfo {
                id: PARAM_MIX, name: "Mix".into(),
                default_value: 0.3, min: 0.0, max: 1.0, unit: "%".into(), automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_ROOM_SIZE => self.room_size as f64,
            PARAM_DECAY => ((self.decay_secs - 0.1) / 9.9).clamp(0.0, 1.0) as f64,
            PARAM_DAMPING => self.damping as f64,
            PARAM_PRE_DELAY => (self.pre_delay_ms / 200.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_ROOM_SIZE => self.room_size = v as f32,
            PARAM_DECAY => self.decay_secs = (0.1 + v * 9.9) as f32,
            PARAM_DAMPING => self.damping = v as f32,
            PARAM_PRE_DELAY => self.pre_delay_ms = (v * 200.0) as f32,
            PARAM_MIX => self.mix = v as f32,
            _ => {}
        }
        self.refresh();
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"size\":{},\"decay\":{},\"damping\":{},\"pre\":{},\"mix\":{}}}",
            self.room_size, self.decay_secs, self.damping, self.pre_delay_ms, self.mix
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
        if let Some(v) = read("size") { self.room_size = v; }
        if let Some(v) = read("decay") { self.decay_secs = v; }
        if let Some(v) = read("damping") { self.damping = v; }
        if let Some(v) = read("pre") { self.pre_delay_ms = v; }
        if let Some(v) = read("mix") { self.mix = v; }
        self.refresh();
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
