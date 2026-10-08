//! Native bit-crusher — sample-rate decimation + bit-depth quantization.
//! Distinct from NativeDistortion's bitcrush mode by giving direct
//! control over both reduction axes plus a wet/dry mix and pre-gain.

use crate::format;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_BIT_DEPTH: u32 = 0;
const PARAM_RATE: u32 = 1;
const PARAM_DRIVE: u32 = 2;
const PARAM_MIX: u32 = 3;
const PARAM_COUNT: u32 = 4;

pub struct NativeBitcrush {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    bit_depth: f32,      // 1..=16
    rate_reduction: f32, // 1..=64 (every Nth sample held)
    drive_db: f32,
    mix: f32,
    hold_l: f32,
    hold_r: f32,
    counter: f32,
    active: bool,
}

impl NativeBitcrush {
    pub const ID: &'static str = "hardwave.native.bitcrush";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Bitcrush".into(),
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
            bit_depth: 8.0,
            rate_reduction: 1.0,
            drive_db: 0.0,
            mix: 1.0,
            hold_l: 0.0,
            hold_r: 0.0,
            counter: 0.0,
            active: false,
        }
    }

    /// The real value behind a 0..1 setting: bits, hold factor, dB, or
    /// the mix itself. Setting a parameter and showing it both go through
    /// here, so the label is always what the sound is doing.
    fn from_normalised(id: u32, value: f64) -> f32 {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_BIT_DEPTH => (1.0 + v * 15.0) as f32,
            PARAM_RATE => (1.0 + v * 63.0) as f32,
            PARAM_DRIVE => (v * 48.0 - 24.0) as f32,
            _ => v as f32,
        }
    }

    fn quantize(value: f32, bit_depth: f32) -> f32 {
        let levels = 2.0_f32.powf(bit_depth.clamp(1.0, 16.0));
        (value * levels).round() / levels
    }
}

impl Default for NativeBitcrush {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeBitcrush {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.hold_l = 0.0;
        self.hold_r = 0.0;
        self.counter = 0.0;
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
        let drive_lin = 10.0_f32.powf(self.drive_db / 20.0);
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            // Sample-and-hold for rate reduction.
            self.counter += 1.0;
            if self.counter >= self.rate_reduction {
                self.counter -= self.rate_reduction;
                self.hold_l = Self::quantize((in_l * drive_lin).clamp(-1.0, 1.0), self.bit_depth);
                self.hold_r = Self::quantize((in_r * drive_lin).clamp(-1.0, 1.0), self.bit_depth);
            }
            outputs[0][i] = in_l * dry + self.hold_l * mix;
            outputs[1][i] = in_r * dry + self.hold_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_BIT_DEPTH => ("Bits", ((8.0 - 1.0) / 15.0_f64).clamp(0.0, 1.0), "bit"),
            PARAM_RATE => ("Rate", (1.0 / 64.0_f64).clamp(0.0, 1.0), "x"),
            PARAM_DRIVE => ("Drive", 0.5, "dB"),
            PARAM_MIX => ("Mix", 1.0, "%"),
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
            PARAM_BIT_DEPTH => ((self.bit_depth - 1.0) / 15.0).clamp(0.0, 1.0) as f64,
            PARAM_RATE => ((self.rate_reduction - 1.0) / 63.0).clamp(0.0, 1.0) as f64,
            PARAM_DRIVE => ((self.drive_db + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let real = Self::from_normalised(id, value);
        match id {
            PARAM_BIT_DEPTH => self.bit_depth = real,
            PARAM_RATE => self.rate_reduction = real,
            PARAM_DRIVE => self.drive_db = real,
            PARAM_MIX => self.mix = real,
            _ => {}
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        let real = Self::from_normalised(id, value) as f64;
        Some(match id {
            PARAM_BIT_DEPTH => format::num(real, 1, "bit"),
            // How many samples each held sample stands in for.
            PARAM_RATE => format::num(real, 1, "x"),
            PARAM_DRIVE => format::db(real),
            PARAM_MIX => format::pct(real),
            _ => return None,
        })
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"bits\":{},\"rate\":{},\"drive\":{},\"mix\":{}}}",
            self.bit_depth, self.rate_reduction, self.drive_db, self.mix
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
        if let Some(v) = read("bits") {
            self.bit_depth = v.clamp(1.0, 16.0);
        }
        if let Some(v) = read("rate") {
            self.rate_reduction = v.clamp(1.0, 64.0);
        }
        if let Some(v) = read("drive") {
            self.drive_db = v.clamp(-24.0, 24.0);
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
    fn parameters_read_as_bits_hold_db_and_percent() {
        let b = NativeBitcrush::new();
        let bits = b.get_parameter_info(PARAM_BIT_DEPTH).unwrap().default_value;
        assert_eq!(
            b.parameter_text(PARAM_BIT_DEPTH, bits).as_deref(),
            Some("8.0 bit")
        );
        assert_eq!(b.parameter_text(PARAM_RATE, 0.0).as_deref(), Some("1.0 x"));
        assert_eq!(
            b.parameter_text(PARAM_DRIVE, 0.5).as_deref(),
            Some("0.0 dB")
        );
        assert_eq!(b.parameter_text(PARAM_MIX, 1.0).as_deref(), Some("100 %"));
        for id in 0..PARAM_COUNT {
            assert!(b.parameter_options(id).is_none());
            for step in 0..=10 {
                assert!(b.parameter_text(id, step as f64 / 10.0).is_some());
            }
        }
        assert!(b.parameter_text(PARAM_COUNT, 0.5).is_none());
    }
}
