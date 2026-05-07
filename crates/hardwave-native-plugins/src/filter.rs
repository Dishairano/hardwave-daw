//! Native filter plug-in — wraps `hardwave_dsp::biquad::Biquad` in a
//! HostedPlugin with the four most-used shapes (LP / HP / BP / Notch)
//! plus cutoff + resonance. Mirrors Fruity Filter's basic mode.
//!
//! Param map: Mode (4 shapes), Cutoff (20Hz..20kHz log), Q
//! (0.1..=10), Mix (dry/wet). Tone shaping with peaking + shelves
//! gets its own plug-in (the parametric EQ already covers them).

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_MODE: u32 = 0;
const PARAM_CUTOFF: u32 = 1;
const PARAM_Q: u32 = 2;
const PARAM_MIX: u32 = 3;
const PARAM_COUNT: u32 = 4;

/// We expose 4 shapes through one knob — the parametric EQ plug-in
/// covers the gain-bearing ones (peak/shelves).
fn mode_from_normalised(v: f32) -> BiquadKind {
    let idx = ((v * 4.0).floor() as i32).clamp(0, 3);
    match idx {
        0 => BiquadKind::LowPass,
        1 => BiquadKind::HighPass,
        2 => BiquadKind::BandPass,
        _ => BiquadKind::Notch,
    }
}

fn mode_to_normalised(k: BiquadKind) -> f32 {
    match k {
        BiquadKind::LowPass => 0.125,
        BiquadKind::HighPass => 0.375,
        BiquadKind::BandPass => 0.625,
        BiquadKind::Notch => 0.875,
        // Peaking + shelves don't show in this plug-in's mode list;
        // round to LowPass when round-tripped via state.
        _ => 0.125,
    }
}

pub struct NativeFilter {
    descriptor: PluginDescriptor,
    biquad: Biquad,
    sample_rate: f32,
    mode: BiquadKind,
    cutoff_hz: f32,
    q: f32,
    mix: f32,
    active: bool,
    needs_recoef: bool,
}

impl NativeFilter {
    pub const ID: &'static str = "hardwave.native.filter";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Filter".into(),
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
            biquad: Biquad::default(),
            sample_rate: 48_000.0,
            mode: BiquadKind::LowPass,
            cutoff_hz: 1_000.0,
            q: 0.7071,
            mix: 1.0,
            active: false,
            needs_recoef: true,
        }
    }

    fn ensure_coefs(&mut self) {
        if !self.needs_recoef { return; }
        self.biquad.set(self.mode, self.sample_rate, self.cutoff_hz, self.q, 0.0);
        self.needs_recoef = false;
    }

    /// Cutoff is exposed log-scaled so the knob feels useful across
    /// the full 20 Hz..=20 kHz range.
    fn cutoff_from_normalised(v: f64) -> f32 {
        let v = v.clamp(0.0, 1.0) as f32;
        20.0 * (1000.0_f32).powf(v) // 20 → 20k
    }

    fn cutoff_to_normalised(hz: f32) -> f64 {
        let hz = hz.clamp(20.0, 20_000.0);
        ((hz / 20.0).log10() / 3.0).clamp(0.0, 1.0) as f64
    }

    fn q_from_normalised(v: f64) -> f32 {
        let v = v.clamp(0.0, 1.0) as f32;
        // 0.1 → 10 log
        0.1 * 100.0_f32.powf(v)
    }

    fn q_to_normalised(q: f32) -> f64 {
        let q = q.clamp(0.1, 10.0);
        ((q / 0.1).log10() / 2.0).clamp(0.0, 1.0) as f64
    }
}

impl Default for NativeFilter {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeFilter {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.biquad.reset();
        self.needs_recoef = true;
        self.ensure_coefs();
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
        self.ensure_coefs();
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let (wet_l, wet_r) = self.biquad.process_stereo(in_l, in_r);
            outputs[0][i] = in_l * dry + wet_l * mix;
            outputs[1][i] = in_r * dry + wet_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        match index {
            PARAM_MODE => Some(ParameterInfo {
                id: PARAM_MODE, name: "Mode".into(),
                default_value: mode_to_normalised(BiquadKind::LowPass) as f64,
                min: 0.0, max: 1.0, unit: "".into(), automatable: true,
            }),
            PARAM_CUTOFF => Some(ParameterInfo {
                id: PARAM_CUTOFF, name: "Cutoff".into(),
                default_value: Self::cutoff_to_normalised(1_000.0),
                min: 0.0, max: 1.0, unit: "Hz".into(), automatable: true,
            }),
            PARAM_Q => Some(ParameterInfo {
                id: PARAM_Q, name: "Q".into(),
                default_value: Self::q_to_normalised(0.7071),
                min: 0.0, max: 1.0, unit: "".into(), automatable: true,
            }),
            PARAM_MIX => Some(ParameterInfo {
                id: PARAM_MIX, name: "Mix".into(),
                default_value: 1.0,
                min: 0.0, max: 1.0, unit: "%".into(), automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_MODE => mode_to_normalised(self.mode) as f64,
            PARAM_CUTOFF => Self::cutoff_to_normalised(self.cutoff_hz),
            PARAM_Q => Self::q_to_normalised(self.q),
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        match id {
            PARAM_MODE => {
                self.mode = mode_from_normalised(value as f32);
                self.needs_recoef = true;
            }
            PARAM_CUTOFF => {
                self.cutoff_hz = Self::cutoff_from_normalised(value);
                self.needs_recoef = true;
            }
            PARAM_Q => {
                self.q = Self::q_from_normalised(value);
                self.needs_recoef = true;
            }
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0) as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"mode\":{},\"cutoff\":{},\"q\":{},\"mix\":{}}}",
            self.mode as u32, self.cutoff_hz, self.q, self.mix
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
        if let Some(v) = read("mode") {
            self.mode = match v as u32 {
                1 => BiquadKind::HighPass,
                2 => BiquadKind::BandPass,
                3 => BiquadKind::Notch,
                _ => BiquadKind::LowPass,
            };
        }
        if let Some(v) = read("cutoff") { self.cutoff_hz = v; }
        if let Some(v) = read("q") { self.q = v; }
        if let Some(v) = read("mix") { self.mix = v.clamp(0.0, 1.0); }
        self.needs_recoef = true;
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
