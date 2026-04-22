//! Native parametric EQ plugin — wraps `hardwave_dsp::biquad` in the
//! `HostedPlugin` trait so the audio engine can host it like any
//! external VST3 / CLAP plugin.

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const NUM_BANDS: usize = 7;
const PARAMS_PER_BAND: u32 = 4;
const PARAMS_GLOBAL: u32 = 1;

/// One EQ band — shared-coefficient biquad + user-facing params.
struct Band {
    kind: BiquadKind,
    enabled: bool,
    frequency_hz: f64,
    gain_db: f64,
    q: f64,
    biquad: Biquad,
}

impl Band {
    fn new(kind: BiquadKind, freq: f64) -> Self {
        Self {
            kind,
            enabled: false,
            frequency_hz: freq,
            gain_db: 0.0,
            q: 1.0,
            biquad: Biquad::default(),
        }
    }

    fn update(&mut self, sr: f64) {
        if !self.enabled {
            return;
        }
        self.biquad.set(
            self.kind,
            sr as f32,
            self.frequency_hz as f32,
            self.q as f32,
            self.gain_db as f32,
        );
    }
}

/// Native EQ plugin — 7 bands (low shelf / 5 peaks / high shelf).
pub struct NativeEq {
    descriptor: PluginDescriptor,
    bands: [Band; NUM_BANDS],
    output_gain_db: f64,
    sample_rate: f64,
    active: bool,
}

impl NativeEq {
    pub const ID: &'static str = "hardwave.native.eq";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave EQ".into(),
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
            bands: [
                Band::new(BiquadKind::LowShelf, 80.0),
                Band::new(BiquadKind::Peak, 200.0),
                Band::new(BiquadKind::Peak, 500.0),
                Band::new(BiquadKind::Peak, 1_000.0),
                Band::new(BiquadKind::Peak, 3_000.0),
                Band::new(BiquadKind::Peak, 6_000.0),
                Band::new(BiquadKind::HighShelf, 10_000.0),
            ],
            output_gain_db: 0.0,
            sample_rate: 48_000.0,
            active: false,
        }
    }

    fn total_params(&self) -> u32 {
        NUM_BANDS as u32 * PARAMS_PER_BAND + PARAMS_GLOBAL
    }

    fn refresh_coeffs(&mut self) {
        for band in self.bands.iter_mut() {
            band.update(self.sample_rate);
        }
    }
}

impl Default for NativeEq {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeEq {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, _max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate.max(1.0);
        self.active = true;
        self.refresh_coeffs();
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
        if outputs.len() < 2 || inputs.len() < 2 {
            return;
        }
        let left_in = &inputs[0][..num_samples.min(inputs[0].len())];
        let right_in = &inputs[1][..num_samples.min(inputs[1].len())];
        outputs[0].clear();
        outputs[1].clear();
        outputs[0].extend_from_slice(left_in);
        outputs[1].extend_from_slice(right_in);
        for band in self.bands.iter_mut() {
            if !band.enabled {
                continue;
            }
            for i in 0..outputs[0].len().min(outputs[1].len()) {
                let l = outputs[0][i];
                let r = outputs[1][i];
                let (yl, yr) = band.biquad.process_stereo(l, r);
                outputs[0][i] = yl;
                outputs[1][i] = yr;
            }
        }
        let gain = 10f64.powf(self.output_gain_db / 20.0) as f32;
        if (gain - 1.0).abs() > 1e-6 {
            let (left_out, rest) = outputs.split_at_mut(1);
            for v in left_out[0].iter_mut() {
                *v *= gain;
            }
            for v in rest[0].iter_mut() {
                *v *= gain;
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        self.total_params()
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let bands = NUM_BANDS as u32;
        if index < bands * PARAMS_PER_BAND {
            let band_index = index / PARAMS_PER_BAND;
            let param_index = index % PARAMS_PER_BAND;
            let (name, min, max, unit, default) = match param_index {
                0 => ("Enabled", 0.0, 1.0, "toggle", 0.0),
                1 => ("Frequency", 20.0, 20_000.0, "Hz", 1_000.0),
                2 => ("Gain", -24.0, 24.0, "dB", 0.0),
                3 => ("Q", 0.1, 10.0, "", 1.0),
                _ => return None,
            };
            return Some(ParameterInfo {
                id: index,
                name: format!("Band {} {}", band_index + 1, name),
                default_value: default,
                min,
                max,
                unit: unit.into(),
                automatable: true,
            });
        }
        if index == bands * PARAMS_PER_BAND {
            return Some(ParameterInfo {
                id: index,
                name: "Output Gain".into(),
                default_value: 0.0,
                min: -24.0,
                max: 24.0,
                unit: "dB".into(),
                automatable: true,
            });
        }
        None
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        let bands = NUM_BANDS as u32;
        if id < bands * PARAMS_PER_BAND {
            let band = &self.bands[(id / PARAMS_PER_BAND) as usize];
            match id % PARAMS_PER_BAND {
                0 => {
                    if band.enabled {
                        1.0
                    } else {
                        0.0
                    }
                }
                1 => band.frequency_hz,
                2 => band.gain_db,
                3 => band.q,
                _ => 0.0,
            }
        } else if id == bands * PARAMS_PER_BAND {
            self.output_gain_db
        } else {
            0.0
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let bands = NUM_BANDS as u32;
        if id < bands * PARAMS_PER_BAND {
            let band = &mut self.bands[(id / PARAMS_PER_BAND) as usize];
            match id % PARAMS_PER_BAND {
                0 => band.enabled = value >= 0.5,
                1 => band.frequency_hz = value.clamp(20.0, 20_000.0),
                2 => band.gain_db = value.clamp(-24.0, 24.0),
                3 => band.q = value.clamp(0.1, 10.0),
                _ => {}
            }
        } else if id == bands * PARAMS_PER_BAND {
            self.output_gain_db = value.clamp(-24.0, 24.0);
        }
        self.refresh_coeffs();
    }

    fn get_state(&self) -> Vec<u8> {
        let mut state = Vec::with_capacity(1 + NUM_BANDS * 32 + 8);
        state.extend_from_slice(&1u32.to_le_bytes());
        for band in self.bands.iter() {
            state.push(u8::from(band.enabled));
            state.extend_from_slice(&(band.frequency_hz as f32).to_le_bytes());
            state.extend_from_slice(&(band.gain_db as f32).to_le_bytes());
            state.extend_from_slice(&(band.q as f32).to_le_bytes());
        }
        state.extend_from_slice(&(self.output_gain_db as f32).to_le_bytes());
        state
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() < 4 {
            return Err("state too short".into());
        }
        let mut cursor = 4usize;
        for band in self.bands.iter_mut() {
            if cursor + 13 > bytes.len() {
                return Err("state truncated".into());
            }
            band.enabled = bytes[cursor] != 0;
            cursor += 1;
            band.frequency_hz =
                f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as f64;
            cursor += 4;
            band.gain_db = f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as f64;
            cursor += 4;
            band.q = f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as f64;
            cursor += 4;
        }
        if cursor + 4 <= bytes.len() {
            self.output_gain_db =
                f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as f64;
        }
        self.refresh_coeffs();
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        0
    }

    fn open_editor(&mut self, _parent: RawWindowHandle) -> bool {
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
    fn new_eq_passes_through_when_all_bands_disabled() {
        let mut eq = NativeEq::new();
        eq.activate(48_000.0, 512).unwrap();
        let input_l: Vec<f32> = (0..128).map(|i| (i as f32 / 128.0).sin()).collect();
        let input_r = input_l.clone();
        let mut outputs = vec![Vec::new(), Vec::new()];
        let mut midi_out = Vec::new();
        eq.process(&[&input_l, &input_r], &mut outputs, &[], &mut midi_out, 128);
        for (a, b) in input_l.iter().zip(outputs[0].iter()) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn enabled_band_modifies_output() {
        let mut eq = NativeEq::new();
        eq.activate(48_000.0, 512).unwrap();
        eq.set_parameter_value(0, 1.0); // enable band 1 (low shelf 80 Hz)
        eq.set_parameter_value(2, 12.0); // +12 dB
        let input_l: Vec<f32> = (0..4096)
            .map(|i| (2.0 * std::f32::consts::PI * 80.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let input_r = input_l.clone();
        let mut outputs = vec![Vec::new(), Vec::new()];
        let mut midi_out = Vec::new();
        eq.process(
            &[&input_l, &input_r],
            &mut outputs,
            &[],
            &mut midi_out,
            4096,
        );
        let in_peak = input_l.iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        let out_peak = outputs[0].iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        assert!(out_peak > in_peak * 1.5, "shelf boost should lift peak");
    }

    #[test]
    fn param_count_matches_formula() {
        let eq = NativeEq::new();
        assert_eq!(eq.get_parameter_count(), NUM_BANDS as u32 * 4 + 1);
    }

    #[test]
    fn set_parameter_clamps_to_range() {
        let mut eq = NativeEq::new();
        eq.set_parameter_value(2, 100.0);
        assert!((eq.get_parameter_value(2) - 24.0).abs() < 1e-6);
        eq.set_parameter_value(1, 0.0);
        assert!((eq.get_parameter_value(1) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn state_round_trips_through_serialization() {
        let mut eq = NativeEq::new();
        eq.set_parameter_value(0, 1.0);
        eq.set_parameter_value(2, 6.0);
        eq.set_parameter_value(NUM_BANDS as u32 * 4, -3.0);
        let state = eq.get_state();
        let mut eq2 = NativeEq::new();
        eq2.set_state(&state).unwrap();
        assert!((eq2.get_parameter_value(0) - 1.0).abs() < 1e-6);
        assert!((eq2.get_parameter_value(2) - 6.0).abs() < 1e-6);
        assert!((eq2.get_parameter_value(NUM_BANDS as u32 * 4) + 3.0).abs() < 1e-6);
    }
}
