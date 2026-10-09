//! Native parametric EQ plugin — wraps `hardwave_dsp::biquad` in the
//! `HostedPlugin` trait so the audio engine can host it like any
//! external VST3 / CLAP plugin.

use crate::format;
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
/// Each band's shape, one parameter per band after Output Gain. Added after
/// the others so every older parameter keeps its id: automation and saved
/// songs address parameters by id.
const TYPE_BASE: u32 = NUM_BANDS as u32 * PARAMS_PER_BAND + PARAMS_GLOBAL;

/// The shapes a band can take, in the order the Type parameter counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    LowCut,
    LowShelf,
    Bell,
    Notch,
    HighShelf,
    HighCut,
}

const SHAPES: [Shape; 6] = [
    Shape::LowCut,
    Shape::LowShelf,
    Shape::Bell,
    Shape::Notch,
    Shape::HighShelf,
    Shape::HighCut,
];

impl Shape {
    fn from_value(v: f64) -> Self {
        SHAPES[(v.round().max(0.0) as usize).min(SHAPES.len() - 1)]
    }
    fn value(self) -> f64 {
        SHAPES.iter().position(|s| *s == self).unwrap_or(2) as f64
    }
    fn label(self) -> &'static str {
        match self {
            Shape::LowCut => "Low cut",
            Shape::LowShelf => "Low shelf",
            Shape::Bell => "Bell",
            Shape::Notch => "Notch",
            Shape::HighShelf => "High shelf",
            Shape::HighCut => "High cut",
        }
    }
    fn kind(self) -> BiquadKind {
        match self {
            Shape::LowCut => BiquadKind::HighPass,
            Shape::LowShelf => BiquadKind::LowShelf,
            Shape::Bell => BiquadKind::Peak,
            Shape::Notch => BiquadKind::Notch,
            Shape::HighShelf => BiquadKind::HighShelf,
            Shape::HighCut => BiquadKind::LowPass,
        }
    }
    /// The cuts are two filters in a row: 24 dB an octave, as an EQ's
    /// cut is expected to be, not the 12 of one biquad.
    fn stages(self) -> usize {
        matches!(self, Shape::LowCut | Shape::HighCut) as usize + 1
    }
}

/// One EQ band — shared-coefficient biquad + user-facing params.
struct Band {
    shape: Shape,
    enabled: bool,
    frequency_hz: f64,
    gain_db: f64,
    q: f64,
    biquad: Biquad,
    /// The second stage of a cut.
    biquad2: Biquad,
}

impl Band {
    fn new(shape: Shape, freq: f64) -> Self {
        Self {
            shape,
            enabled: false,
            frequency_hz: freq,
            gain_db: 0.0,
            q: 1.0,
            biquad: Biquad::default(),
            biquad2: Biquad::default(),
        }
    }

    fn update(&mut self, sr: f64) {
        if !self.enabled {
            return;
        }
        for b in [&mut self.biquad, &mut self.biquad2] {
            b.set(
                self.shape.kind(),
                sr as f32,
                self.frequency_hz as f32,
                self.q as f32,
                self.gain_db as f32,
            );
        }
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
                Band::new(Shape::LowShelf, 80.0),
                Band::new(Shape::Bell, 200.0),
                Band::new(Shape::Bell, 500.0),
                Band::new(Shape::Bell, 1_000.0),
                Band::new(Shape::Bell, 3_000.0),
                Band::new(Shape::Bell, 6_000.0),
                Band::new(Shape::HighShelf, 10_000.0),
            ],
            output_gain_db: 0.0,
            sample_rate: 48_000.0,
            active: false,
        }
    }

    fn total_params(&self) -> u32 {
        TYPE_BASE + NUM_BANDS as u32
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
                let (mut yl, mut yr) = band.biquad.process_stereo(l, r);
                if band.shape.stages() == 2 {
                    (yl, yr) = band.biquad2.process_stereo(yl, yr);
                }
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
            let start = Self::new().bands[band_index as usize].frequency_hz;
            let (name, min, max, unit, default) = match param_index {
                0 => ("Enabled", 0.0, 1.0, "toggle", 0.0),
                1 => ("Frequency", 20.0, 20_000.0, "Hz", start),
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
        if (TYPE_BASE..TYPE_BASE + bands).contains(&index) {
            let band = (index - TYPE_BASE) as usize;
            return Some(ParameterInfo {
                id: index,
                name: format!("Band {} Type", band + 1),
                default_value: Self::new().bands[band].shape.value(),
                min: 0.0,
                max: (SHAPES.len() - 1) as f64,
                unit: String::new(),
                automatable: false,
            });
        }
        None
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        let bands = NUM_BANDS as u32;
        if id < bands * PARAMS_PER_BAND {
            let band = &self.bands[(id / PARAMS_PER_BAND) as usize];
            match id % PARAMS_PER_BAND {
                0 if band.enabled => 1.0,
                1 => band.frequency_hz,
                2 => band.gain_db,
                3 => band.q,
                _ => 0.0,
            }
        } else if id == bands * PARAMS_PER_BAND {
            self.output_gain_db
        } else if (TYPE_BASE..TYPE_BASE + bands).contains(&id) {
            self.bands[(id - TYPE_BASE) as usize].shape.value()
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
        } else if (TYPE_BASE..TYPE_BASE + bands).contains(&id) {
            let band = &mut self.bands[(id - TYPE_BASE) as usize];
            let shape = Shape::from_value(value);
            if shape != band.shape {
                band.shape = shape;
                // A new shape starts from silence, not the old filter's memory.
                band.biquad.reset();
                band.biquad2.reset();
            }
        }
        self.refresh_coeffs();
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        // Values are real units here, not 0..1, so the text is the value
        // clamped to the range set_parameter_value clamps it to.
        let info = self.get_parameter_info(id)?;
        let v = value.clamp(info.min, info.max);
        if id == NUM_BANDS as u32 * PARAMS_PER_BAND {
            return Some(format::db(v));
        }
        if id >= TYPE_BASE {
            return Some(Shape::from_value(v).label().to_string());
        }
        Some(match id % PARAMS_PER_BAND {
            0 => format::on_off(v),
            1 => format::hz(v),
            2 => format::db(v),
            3 => format::num(v, 2, ""),
            _ => return None,
        })
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        (id >= TYPE_BASE && id < self.total_params())
            .then(|| SHAPES.iter().map(|s| s.label().to_string()).collect())
    }

    fn get_state(&self) -> Vec<u8> {
        let mut state = Vec::with_capacity(1 + NUM_BANDS * 32 + 8);
        state.extend_from_slice(&2u32.to_le_bytes());
        for band in self.bands.iter() {
            state.push(u8::from(band.enabled));
            state.extend_from_slice(&(band.frequency_hz as f32).to_le_bytes());
            state.extend_from_slice(&(band.gain_db as f32).to_le_bytes());
            state.extend_from_slice(&(band.q as f32).to_le_bytes());
        }
        state.extend_from_slice(&(self.output_gain_db as f32).to_le_bytes());
        // Version 2: each band's shape, after everything version 1 had.
        for band in self.bands.iter() {
            state.push(band.shape.value() as u8);
        }
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
            cursor += 4;
        }
        // Shapes came in version 2; an older state keeps the default ones.
        let defaults = Self::new();
        for (i, band) in self.bands.iter_mut().enumerate() {
            band.shape = bytes
                .get(cursor + i)
                .map(|b| Shape::from_value(*b as f64))
                .unwrap_or(defaults.bands[i].shape);
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
    fn parameter_text_reads_real_units() {
        let eq = NativeEq::new();
        assert_eq!(eq.parameter_text(1, 1_000.0).as_deref(), Some("1.00 kHz"));
        assert_eq!(eq.parameter_text(2, -6.0).as_deref(), Some("-6.0 dB"));
        assert_eq!(eq.parameter_text(3, 1.0).as_deref(), Some("1.00"));
        assert_eq!(eq.parameter_text(0, 1.0).as_deref(), Some("On"));
        // Out-of-range values read as what the plug-in would clamp them to.
        assert_eq!(eq.parameter_text(1, 5.0).as_deref(), Some("20.0 Hz"));
        for id in 0..eq.get_parameter_count() {
            let info = eq.get_parameter_info(id).unwrap();
            for v in [info.min, info.default_value, info.max] {
                assert!(eq.parameter_text(id, v).is_some(), "no text for {id}");
            }
            assert_eq!(
                eq.parameter_options(id).is_some(),
                id >= TYPE_BASE,
                "options only for the shapes"
            );
        }
    }

    #[test]
    fn param_count_matches_formula() {
        let eq = NativeEq::new();
        assert_eq!(
            eq.get_parameter_count(),
            NUM_BANDS as u32 * 4 + 1 + NUM_BANDS as u32
        );
        // The parameters that were there before keep their ids.
        assert_eq!(eq.get_parameter_info(28).unwrap().name, "Output Gain");
        assert_eq!(eq.get_parameter_info(29).unwrap().name, "Band 1 Type");
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

    /// RMS of a sine at `hz` through the EQ, in dB relative to the input.
    fn gain_at(eq: &mut NativeEq, hz: f32) -> f32 {
        let n = 16_384;
        let input: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let mut outputs = vec![Vec::new(), Vec::new()];
        eq.process(&[&input, &input], &mut outputs, &[], &mut Vec::new(), n);
        let rms =
            |v: &[f32]| (v[n / 2..].iter().map(|x| x * x).sum::<f32>() / (n / 2) as f32).sqrt();
        20.0 * (rms(&outputs[0]) / rms(&input)).log10()
    }

    #[test]
    fn a_low_cut_takes_out_the_lows_at_24_db_an_octave() {
        let mut eq = NativeEq::new();
        eq.activate(48_000.0, 512).unwrap();
        eq.set_parameter_value(TYPE_BASE, 0.0); // band 1: low cut
        eq.set_parameter_value(0, 1.0);
        eq.set_parameter_value(1, 200.0);
        eq.set_parameter_value(3, 0.71);
        let pass = gain_at(&mut eq, 2_000.0);
        let octave_down = gain_at(&mut eq, 100.0);
        assert!(pass.abs() < 0.5, "the highs pass: {pass}");
        assert!(
            octave_down < -18.0,
            "an octave below is about 24 dB down: {octave_down}"
        );
    }

    #[test]
    fn shapes_are_saved_and_older_states_keep_theirs() {
        let mut eq = NativeEq::new();
        eq.set_parameter_value(TYPE_BASE + 6, 5.0); // band 7: high cut
        let mut back = NativeEq::new();
        back.set_state(&eq.get_state()).unwrap();
        assert_eq!(back.get_parameter_value(TYPE_BASE + 6), 5.0);
        // A version 1 state: no shapes at the end.
        let mut v1 = eq.get_state();
        v1.truncate(v1.len() - NUM_BANDS);
        let mut old = NativeEq::new();
        old.set_state(&v1).unwrap();
        assert_eq!(
            old.get_parameter_value(TYPE_BASE + 6),
            Shape::HighShelf.value()
        );
        assert_eq!(
            old.parameter_text(TYPE_BASE, 0.0).as_deref(),
            Some("Low cut")
        );
    }
}
