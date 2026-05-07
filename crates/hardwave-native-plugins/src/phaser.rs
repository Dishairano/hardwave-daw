//! Native phaser — 6-stage allpass chain with per-block notch
//! modulation. Mirrors Fruity Phaser at the basic-features level.
//!
//! Notch frequencies are re-set every block from a sine LFO. Block
//! granularity (typically 64-256 samples) is fast enough at audible
//! LFO rates (0.1-10 Hz) to avoid stepping artifacts; sample-rate
//! modulation would need a Biquad cookbook recompute per sample,
//! which is too much for what is effectively a colour effect.

use hardwave_dsp::modulation::PhaserChain;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::f32::consts::TAU;
use std::path::PathBuf;

const STAGES: usize = 6;

const PARAM_RATE: u32 = 0;
const PARAM_DEPTH: u32 = 1;
const PARAM_BASE: u32 = 2;
const PARAM_SPREAD: u32 = 3;
const PARAM_MIX: u32 = 4;
const PARAM_COUNT: u32 = 5;

pub struct NativePhaser {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    chain_l: PhaserChain<STAGES>,
    chain_r: PhaserChain<STAGES>,
    rate_hz: f32,
    depth_octaves: f32,
    base_hz: f32,
    spread_octaves: f32,
    mix: f32,
    lfo_phase_l: f32,
    lfo_phase_r: f32,
    active: bool,
}

impl NativePhaser {
    pub const ID: &'static str = "hardwave.native.phaser";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Phaser".into(),
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
            chain_l: PhaserChain::default(),
            chain_r: PhaserChain::default(),
            rate_hz: 0.5,
            depth_octaves: 1.5,
            base_hz: 500.0,
            spread_octaves: 2.0,
            mix: 0.5,
            lfo_phase_l: 0.0,
            lfo_phase_r: 0.5, // 180° offset for stereo
            active: false,
        }
    }

    fn modulated_base(&self, lfo_phase: f32) -> f32 {
        let lfo = (TAU * lfo_phase).sin();
        let octave_offset = lfo * self.depth_octaves;
        self.base_hz * 2.0_f32.powf(octave_offset)
    }
}

impl Default for NativePhaser {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativePhaser {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.chain_l = PhaserChain::default();
        self.chain_r = PhaserChain::default();
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

        // Re-set notches once per block from current LFO phase. Cheap
        // at typical block sizes (64-256 samples).
        let base_l = self.modulated_base(self.lfo_phase_l);
        let base_r = self.modulated_base(self.lfo_phase_r);
        self.chain_l.set_notches(base_l, self.spread_octaves, self.sample_rate);
        self.chain_r.set_notches(base_r, self.spread_octaves, self.sample_rate);

        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let wet_l = self.chain_l.process(in_l);
            let wet_r = self.chain_r.process(in_r);
            outputs[0][i] = in_l * dry + wet_l * mix;
            outputs[1][i] = in_r * dry + wet_r * mix;
        }

        // Advance LFO phase by the block length.
        let dphase = self.rate_hz * num_samples as f32 / self.sample_rate;
        self.lfo_phase_l = (self.lfo_phase_l + dphase) % 1.0;
        self.lfo_phase_r = (self.lfo_phase_r + dphase) % 1.0;
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_RATE => ("Rate", (0.5_f64 / 8.0).clamp(0.0, 1.0), "Hz"),
            PARAM_DEPTH => ("Depth", 1.5 / 4.0, "oct"),
            PARAM_BASE => ("Base", ((500.0_f64.log10() - 100.0_f64.log10()) / (5_000.0_f64.log10() - 100.0_f64.log10())).clamp(0.0, 1.0), "Hz"),
            PARAM_SPREAD => ("Spread", 2.0 / 4.0, "oct"),
            PARAM_MIX => ("Mix", 0.5, "%"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_RATE => (self.rate_hz / 8.0).clamp(0.0, 1.0) as f64,
            PARAM_DEPTH => (self.depth_octaves / 4.0).clamp(0.0, 1.0) as f64,
            PARAM_BASE => {
                let lo = 100.0_f32.log10();
                let hi = 5_000.0_f32.log10();
                ((self.base_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_SPREAD => (self.spread_octaves / 4.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_RATE => self.rate_hz = (v * 8.0) as f32,
            PARAM_DEPTH => self.depth_octaves = (v * 4.0) as f32,
            PARAM_BASE => {
                let lo = 100.0_f32.log10();
                let hi = 5_000.0_f32.log10();
                self.base_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
            }
            PARAM_SPREAD => self.spread_octaves = (v * 4.0) as f32,
            PARAM_MIX => self.mix = v as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"rate\":{},\"depth\":{},\"base\":{},\"spread\":{},\"mix\":{}}}",
            self.rate_hz, self.depth_octaves, self.base_hz, self.spread_octaves, self.mix
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
        if let Some(v) = read("rate") { self.rate_hz = v.clamp(0.0, 8.0); }
        if let Some(v) = read("depth") { self.depth_octaves = v.clamp(0.0, 4.0); }
        if let Some(v) = read("base") { self.base_hz = v.clamp(20.0, 20_000.0); }
        if let Some(v) = read("spread") { self.spread_octaves = v.clamp(0.0, 4.0); }
        if let Some(v) = read("mix") { self.mix = v.clamp(0.0, 1.0); }
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
