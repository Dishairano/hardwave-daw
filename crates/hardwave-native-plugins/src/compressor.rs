//! Native compressor plugin — wraps `hardwave_dsp::dynamics` in the
//! `HostedPlugin` trait.

use hardwave_dsp::dynamics::{
    compressor_gain_reduction_db, db_to_linear, linear_to_db, DetectMode, EnvelopeFollower,
};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_THRESHOLD: u32 = 0;
const PARAM_RATIO: u32 = 1;
const PARAM_ATTACK: u32 = 2;
const PARAM_RELEASE: u32 = 3;
const PARAM_KNEE: u32 = 4;
const PARAM_MAKEUP: u32 = 5;
const PARAM_AUTO_MAKEUP: u32 = 6;
const PARAM_MODE: u32 = 7;
const PARAM_COUNT: u32 = 8;

pub struct NativeCompressor {
    descriptor: PluginDescriptor,
    env_l: EnvelopeFollower,
    env_r: EnvelopeFollower,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    knee_db: f32,
    makeup_db: f32,
    auto_makeup: bool,
    mode: DetectMode,
    sample_rate: f32,
    active: bool,
}

impl NativeCompressor {
    pub const ID: &'static str = "hardwave.native.compressor";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Compressor".into(),
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
            env_l: EnvelopeFollower::default(),
            env_r: EnvelopeFollower::default(),
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            knee_db: 6.0,
            makeup_db: 0.0,
            auto_makeup: false,
            mode: DetectMode::Peak,
            sample_rate: 48_000.0,
            active: false,
        }
    }

    fn update_envelope(&mut self) {
        self.env_l
            .set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.env_r
            .set_times(self.attack_ms, self.release_ms, self.sample_rate);
        self.env_l.set_mode(self.mode);
        self.env_r.set_mode(self.mode);
    }

    fn effective_makeup_db(&self) -> f32 {
        if self.auto_makeup {
            hardwave_dsp::dynamics::auto_makeup_gain_db(self.threshold_db, self.ratio)
                + self.makeup_db
        } else {
            self.makeup_db
        }
    }

    fn process_sample(&mut self, ch: usize, sample: f32) -> f32 {
        let env = if ch == 0 {
            self.env_l.process(sample)
        } else {
            self.env_r.process(sample)
        };
        let env_db = linear_to_db(env);
        // `compressor_gain_reduction_db` returns a non-positive dB
        // value (0 dB = no reduction, more negative = more reduction).
        let reduction_db =
            compressor_gain_reduction_db(env_db, self.threshold_db, self.ratio, self.knee_db);
        let makeup = self.effective_makeup_db();
        let gain = db_to_linear(reduction_db + makeup);
        sample * gain
    }
}

impl Default for NativeCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeCompressor {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, _max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate.max(1.0) as f32;
        self.update_envelope();
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
        if outputs.len() < 2 || inputs.len() < 2 {
            return;
        }
        let n = num_samples.min(inputs[0].len()).min(inputs[1].len());
        outputs[0].clear();
        outputs[1].clear();
        outputs[0].reserve(n);
        outputs[1].reserve(n);
        let left_in = &inputs[0][..n];
        let right_in = &inputs[1][..n];
        for (l, r) in left_in.iter().zip(right_in.iter()) {
            outputs[0].push(self.process_sample(0, *l));
            outputs[1].push(self.process_sample(1, *r));
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (id, name, min, max, unit, default) = match index {
            PARAM_THRESHOLD => (index, "Threshold", -60.0_f64, 0.0, "dB", -18.0),
            PARAM_RATIO => (index, "Ratio", 1.0, 100.0, ":1", 2.0),
            PARAM_ATTACK => (index, "Attack", 0.01, 100.0, "ms", 10.0),
            PARAM_RELEASE => (index, "Release", 1.0, 5_000.0, "ms", 100.0),
            PARAM_KNEE => (index, "Knee", 0.0, 30.0, "dB", 6.0),
            PARAM_MAKEUP => (index, "Makeup", 0.0, 30.0, "dB", 0.0),
            PARAM_AUTO_MAKEUP => (index, "Auto Makeup", 0.0, 1.0, "toggle", 0.0),
            PARAM_MODE => (index, "Detect Mode", 0.0, 1.0, "peak/rms", 0.0),
            _ => return None,
        };
        Some(ParameterInfo {
            id,
            name: name.into(),
            default_value: default,
            min,
            max,
            unit: unit.into(),
            automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_THRESHOLD => self.threshold_db as f64,
            PARAM_RATIO => self.ratio as f64,
            PARAM_ATTACK => self.attack_ms as f64,
            PARAM_RELEASE => self.release_ms as f64,
            PARAM_KNEE => self.knee_db as f64,
            PARAM_MAKEUP => self.makeup_db as f64,
            PARAM_AUTO_MAKEUP => {
                if self.auto_makeup {
                    1.0
                } else {
                    0.0
                }
            }
            PARAM_MODE => match self.mode {
                DetectMode::Peak => 0.0,
                DetectMode::Rms => 1.0,
            },
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        match id {
            PARAM_THRESHOLD => self.threshold_db = (value as f32).clamp(-60.0, 0.0),
            PARAM_RATIO => self.ratio = (value as f32).clamp(1.0, 100.0),
            PARAM_ATTACK => {
                self.attack_ms = (value as f32).clamp(0.01, 100.0);
                self.update_envelope();
            }
            PARAM_RELEASE => {
                self.release_ms = (value as f32).clamp(1.0, 5_000.0);
                self.update_envelope();
            }
            PARAM_KNEE => self.knee_db = (value as f32).clamp(0.0, 30.0),
            PARAM_MAKEUP => self.makeup_db = (value as f32).clamp(0.0, 30.0),
            PARAM_AUTO_MAKEUP => self.auto_makeup = value >= 0.5,
            PARAM_MODE => {
                self.mode = if value >= 0.5 {
                    DetectMode::Rms
                } else {
                    DetectMode::Peak
                };
                self.update_envelope();
            }
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(40);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&self.threshold_db.to_le_bytes());
        out.extend_from_slice(&self.ratio.to_le_bytes());
        out.extend_from_slice(&self.attack_ms.to_le_bytes());
        out.extend_from_slice(&self.release_ms.to_le_bytes());
        out.extend_from_slice(&self.knee_db.to_le_bytes());
        out.extend_from_slice(&self.makeup_db.to_le_bytes());
        out.push(u8::from(self.auto_makeup));
        out.push(match self.mode {
            DetectMode::Peak => 0,
            DetectMode::Rms => 1,
        });
        out
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() < 4 + 6 * 4 + 2 {
            return Err("state too short".into());
        }
        let mut cursor = 4usize;
        let read_f32 = |cursor: &mut usize, bytes: &[u8]| -> f32 {
            let v = f32::from_le_bytes(bytes[*cursor..*cursor + 4].try_into().unwrap());
            *cursor += 4;
            v
        };
        self.threshold_db = read_f32(&mut cursor, bytes);
        self.ratio = read_f32(&mut cursor, bytes);
        self.attack_ms = read_f32(&mut cursor, bytes);
        self.release_ms = read_f32(&mut cursor, bytes);
        self.knee_db = read_f32(&mut cursor, bytes);
        self.makeup_db = read_f32(&mut cursor, bytes);
        self.auto_makeup = bytes[cursor] != 0;
        cursor += 1;
        self.mode = if bytes[cursor] == 1 {
            DetectMode::Rms
        } else {
            DetectMode::Peak
        };
        self.update_envelope();
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
    fn param_count_is_correct() {
        let c = NativeCompressor::new();
        assert_eq!(c.get_parameter_count(), PARAM_COUNT);
    }

    #[test]
    fn threshold_below_passes_through() {
        let mut c = NativeCompressor::new();
        c.set_parameter_value(PARAM_THRESHOLD, -6.0);
        c.set_parameter_value(PARAM_RATIO, 4.0);
        c.activate(48_000.0, 512).unwrap();
        // Input at -24 dBFS (≈ 0.063) is well below -6 dB threshold.
        let level = 10f32.powf(-24.0 / 20.0);
        let input = vec![level; 256];
        let mut outputs = vec![Vec::new(), Vec::new()];
        let mut midi_out = Vec::new();
        c.process(&[&input, &input], &mut outputs, &[], &mut midi_out, 256);
        let peak_in = input.iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        let peak_out = outputs[0].iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        // Expect roughly unity (no compression applied).
        assert!((peak_in - peak_out).abs() < 0.01);
    }

    #[test]
    fn above_threshold_reduces_gain() {
        let mut c = NativeCompressor::new();
        c.set_parameter_value(PARAM_THRESHOLD, -18.0);
        c.set_parameter_value(PARAM_RATIO, 4.0);
        c.set_parameter_value(PARAM_ATTACK, 1.0);
        c.activate(48_000.0, 512).unwrap();
        // Hit with 0 dBFS.
        let input = vec![1.0_f32; 2048];
        let mut outputs = vec![Vec::new(), Vec::new()];
        let mut midi_out = Vec::new();
        c.process(&[&input, &input], &mut outputs, &[], &mut midi_out, 2048);
        let tail = &outputs[0][1024..];
        let peak_tail = tail.iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        assert!(
            peak_tail < 0.75,
            "compression should pull peak well below 1.0, got {peak_tail}"
        );
    }

    #[test]
    fn state_roundtrips() {
        let mut c = NativeCompressor::new();
        c.set_parameter_value(PARAM_THRESHOLD, -12.0);
        c.set_parameter_value(PARAM_RATIO, 8.0);
        c.set_parameter_value(PARAM_AUTO_MAKEUP, 1.0);
        c.set_parameter_value(PARAM_MODE, 1.0);
        let state = c.get_state();
        let mut c2 = NativeCompressor::new();
        c2.set_state(&state).unwrap();
        assert!((c2.get_parameter_value(PARAM_THRESHOLD) + 12.0).abs() < 1e-3);
        assert!((c2.get_parameter_value(PARAM_RATIO) - 8.0).abs() < 1e-3);
        assert_eq!(c2.get_parameter_value(PARAM_AUTO_MAKEUP), 1.0);
        assert_eq!(c2.get_parameter_value(PARAM_MODE), 1.0);
    }
}
