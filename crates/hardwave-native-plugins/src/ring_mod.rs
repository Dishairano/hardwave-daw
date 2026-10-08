//! Native ring modulator — multiplies input by an internal sine LFO.
//! Produces inharmonic sum/difference tones ideal for metallic FX,
//! robot voices, science-fiction risers. Distinct from tremolo
//! (tremolo modulates amplitude with positive-only LFO, ring-mod uses
//! bipolar -1..+1 LFO so sign-flips create new partials).

use crate::format;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::f32::consts::TAU;
use std::path::PathBuf;

const PARAM_FREQUENCY: u32 = 0;
const PARAM_MIX: u32 = 1;
const PARAM_COUNT: u32 = 2;

pub struct NativeRingMod {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    frequency_hz: f32,
    mix: f32,
    phase: f32,
    active: bool,
}

impl NativeRingMod {
    pub const ID: &'static str = "hardwave.native.ring_mod";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Ring Mod".into(),
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
            frequency_hz: 200.0,
            mix: 0.5,
            phase: 0.0,
            active: false,
        }
    }

    /// The real value behind a 0..1 knob: hertz for the frequency, the
    /// mix itself otherwise. Setting a parameter and showing it both go
    /// through here, so the label is always what the sound is doing.
    fn from_normalised(id: u32, value: f64) -> f32 {
        let v = value.clamp(0.0, 1.0);
        match id {
            // 1..2000 Hz log
            PARAM_FREQUENCY => {
                let lo = 1.0_f32.log10();
                let hi = 2_000.0_f32.log10();
                10.0_f32.powf(lo + (hi - lo) * v as f32)
            }
            _ => v as f32,
        }
    }
}

impl Default for NativeRingMod {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeRingMod {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.phase = 0.0;
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
        let dphase = self.frequency_hz / self.sample_rate;
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let osc = (TAU * self.phase).sin();
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            outputs[0][i] = in_l * dry + (in_l * osc) * mix;
            outputs[1][i] = in_r * dry + (in_r * osc) * mix;
            self.phase = (self.phase + dphase) % 1.0;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            // 1..2000 Hz log
            PARAM_FREQUENCY => (
                "Frequency",
                ((200.0_f64.log10() - 1.0_f64.log10()) / (2_000.0_f64.log10() - 1.0_f64.log10()))
                    .clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_MIX => ("Mix", 0.5, "%"),
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
            PARAM_FREQUENCY => {
                let lo = 1.0_f32.log10();
                let hi = 2_000.0_f32.log10();
                ((self.frequency_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let real = Self::from_normalised(id, value);
        match id {
            PARAM_FREQUENCY => self.frequency_hz = real,
            PARAM_MIX => self.mix = real,
            _ => {}
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        let real = Self::from_normalised(id, value) as f64;
        Some(match id {
            PARAM_FREQUENCY => format::hz(real),
            PARAM_MIX => format::pct(real),
            _ => return None,
        })
    }

    fn get_state(&self) -> Vec<u8> {
        format!("{{\"freq\":{},\"mix\":{}}}", self.frequency_hz, self.mix).into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("freq") {
            self.frequency_hz = v.clamp(1.0, 2_000.0);
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_reads_in_hertz_and_mix_in_percent() {
        let rm = NativeRingMod::new();
        let freq = rm
            .get_parameter_info(PARAM_FREQUENCY)
            .unwrap()
            .default_value;
        assert_eq!(
            rm.parameter_text(PARAM_FREQUENCY, freq).as_deref(),
            Some("200 Hz")
        );
        assert_eq!(
            rm.parameter_text(PARAM_FREQUENCY, 0.0).as_deref(),
            Some("1.0 Hz")
        );
        assert_eq!(
            rm.parameter_text(PARAM_FREQUENCY, 1.0).as_deref(),
            Some("2.00 kHz")
        );
        assert_eq!(rm.parameter_text(PARAM_MIX, 0.5).as_deref(), Some("50 %"));
        for id in 0..PARAM_COUNT {
            assert!(rm.parameter_options(id).is_none());
            for step in 0..=10 {
                assert!(rm.parameter_text(id, step as f64 / 10.0).is_some());
            }
        }
    }
}
