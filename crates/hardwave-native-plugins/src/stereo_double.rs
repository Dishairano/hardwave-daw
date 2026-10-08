//! Native stereo doubler — fakes "double-tracking" by delaying and
//! slightly detuning the right channel relative to the left. Mirrors
//! Eventide MicroPitch / Waves Doubler at the basic level. Distinct
//! from NativeChorus (symmetric LFO modulation on both channels) and
//! NativeVibrato (pure pitch wobble, no doubling).

use crate::format;
use hardwave_dsp::modulation::ModulatedDelay;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const MAX_DELAY_SAMPLES: usize = 2048;

const PARAM_DELAY: u32 = 0;
const PARAM_DETUNE: u32 = 1;
const PARAM_SPREAD: u32 = 2;
const PARAM_MIX: u32 = 3;
const PARAM_COUNT: u32 = 4;

pub struct NativeStereoDouble {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    delay_r: ModulatedDelay,
    delay_ms: f32,
    detune_amount: f32,
    spread: f32,
    mix: f32,
    active: bool,
}

impl NativeStereoDouble {
    pub const ID: &'static str = "hardwave.native.stereo_double";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Doubler".into(),
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
        let mut delay_r = ModulatedDelay::new(MAX_DELAY_SAMPLES, sr);
        delay_r.set_base_delay(0.012 * sr); // 12 ms
        delay_r.set_lfo_rate(2.0);
        delay_r.set_feedback(0.0);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            delay_r,
            delay_ms: 12.0,
            detune_amount: 0.3,
            spread: 0.7,
            mix: 0.5,
            active: false,
        }
    }

    /// The real value behind a 0..1 knob: milliseconds for the delay,
    /// the amount itself otherwise. Setting a parameter and showing it
    /// both go through here, so the label is always what the sound is
    /// doing.
    fn from_normalised(id: u32, value: f64) -> f32 {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_DELAY => (v * 50.0) as f32,
            _ => v as f32,
        }
    }

    fn refresh(&mut self) {
        self.delay_r
            .set_base_delay((self.delay_ms / 1000.0) * self.sample_rate);
        // Detune via tiny LFO depth — depth in samples scales with desired pitch shift
        let depth_samples = self.detune_amount * 0.0008 * self.sample_rate;
        self.delay_r.set_lfo_depth(depth_samples);
    }
}

impl Default for NativeStereoDouble {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeStereoDouble {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.delay_r = ModulatedDelay::new(MAX_DELAY_SAMPLES, self.sample_rate);
        self.delay_r.set_lfo_rate(2.0);
        self.delay_r.set_feedback(0.0);
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
        let spread = self.spread;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            // Sum input to mono and feed through the doubling delay
            let mono = (in_l + in_r) * 0.5;
            let doubled = self.delay_r.process(mono);
            // L gets dry input, R gets dry + doubled (or spread mix)
            let wet_l = in_l * (1.0 - spread) + doubled * (1.0 - spread);
            let wet_r = in_r * (1.0 - spread) + doubled * spread;
            outputs[0][i] = in_l * dry + wet_l * mix;
            outputs[1][i] = in_r * dry + wet_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_DELAY => ("Delay", (12.0_f64 / 50.0).clamp(0.0, 1.0), "ms"),
            PARAM_DETUNE => ("Detune", 0.3, "ct"),
            PARAM_SPREAD => ("Spread", 0.7, "%"),
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
            PARAM_DELAY => (self.delay_ms / 50.0).clamp(0.0, 1.0) as f64,
            PARAM_DETUNE => self.detune_amount as f64,
            PARAM_SPREAD => self.spread as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let real = Self::from_normalised(id, value);
        match id {
            PARAM_DELAY => {
                self.delay_ms = real;
                self.refresh();
            }
            PARAM_DETUNE => {
                self.detune_amount = real;
                self.refresh();
            }
            PARAM_SPREAD => self.spread = real,
            PARAM_MIX => self.mix = real,
            _ => {}
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        let real = Self::from_normalised(id, value) as f64;
        Some(match id {
            PARAM_DELAY => format::ms(real),
            // Detune is a depth for the wobble, not a set number of cents:
            // how far the pitch moves depends on the delay line, so it
            // reads as an amount.
            PARAM_DETUNE | PARAM_SPREAD | PARAM_MIX => format::pct(real),
            _ => return None,
        })
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"d\":{},\"det\":{},\"sp\":{},\"mix\":{}}}",
            self.delay_ms, self.detune_amount, self.spread, self.mix
        )
        .into_bytes()
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
        if let Some(v) = read("d") {
            self.delay_ms = v.clamp(0.0, 50.0);
        }
        if let Some(v) = read("det") {
            self.detune_amount = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("sp") {
            self.spread = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_reads_in_ms_and_the_rest_in_percent() {
        let sd = NativeStereoDouble::new();
        let delay = sd.get_parameter_info(PARAM_DELAY).unwrap().default_value;
        assert_eq!(
            sd.parameter_text(PARAM_DELAY, delay).as_deref(),
            Some("12.0 ms")
        );
        assert_eq!(
            sd.parameter_text(PARAM_DELAY, 1.0).as_deref(),
            Some("50.0 ms")
        );
        assert_eq!(
            sd.parameter_text(PARAM_DETUNE, 0.3).as_deref(),
            Some("30 %")
        );
        assert_eq!(
            sd.parameter_text(PARAM_SPREAD, 0.7).as_deref(),
            Some("70 %")
        );
        for id in 0..PARAM_COUNT {
            assert!(sd.parameter_options(id).is_none());
            for step in 0..=10 {
                assert!(sd.parameter_text(id, step as f64 / 10.0).is_some());
            }
        }
    }
}
