//! Native distortion plug-in — wraps `hardwave_dsp::distortion`'s
//! soft-clip / hard-clip / tape / tube / bitcrush curves into a
//! switchable HostedPlugin.
//!
//! Hardstyle producers want a one-stop "make it dirty" insert: this
//! is that. Param map mirrors Fruity Blood Overdrive: Drive,
//! Mode (curve type), Mix, Output. Tone-shaping (post-distortion EQ)
//! lands in a follow-up commit.

use hardwave_dsp::distortion::{
    bitcrush, drive_compensate, hard_clip, parallel_mix, soft_clip, tape_saturation, tube_emulation,
};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_DRIVE: u32 = 0;
const PARAM_MODE: u32 = 1;
const PARAM_MIX: u32 = 2;
const PARAM_OUTPUT: u32 = 3;
const PARAM_COUNT: u32 = 4;

/// Curve to apply. Stored as a normalised parameter so the
/// automation pipeline doesn't need a separate enum codec.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Mode {
    Soft = 0,
    Hard = 1,
    Tape = 2,
    Tube = 3,
    Bitcrush = 4,
}

impl Mode {
    /// 5 modes spread across 0..=1: 0..0.2 = Soft, 0.2..0.4 = Hard, …
    fn from_normalised(v: f32) -> Self {
        let idx = ((v * 5.0).floor() as i32).clamp(0, 4);
        match idx {
            0 => Mode::Soft,
            1 => Mode::Hard,
            2 => Mode::Tape,
            3 => Mode::Tube,
            _ => Mode::Bitcrush,
        }
    }

    fn to_normalised(self) -> f32 {
        // Land in the centre of the band so re-reads round-trip.
        match self {
            Mode::Soft => 0.10,
            Mode::Hard => 0.30,
            Mode::Tape => 0.50,
            Mode::Tube => 0.70,
            Mode::Bitcrush => 0.90,
        }
    }
}

pub struct NativeDistortion {
    descriptor: PluginDescriptor,
    drive_db: f32, // 0..=24
    mode: Mode,
    mix: f32,       // 0..=1 (0 = dry, 1 = wet)
    output_db: f32, // -24..=24
    active: bool,
}

impl NativeDistortion {
    pub const ID: &'static str = "hardwave.native.distortion";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Distortion".into(),
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
            drive_db: 6.0,
            mode: Mode::Soft,
            mix: 1.0,
            output_db: 0.0,
            active: false,
        }
    }

    fn apply_curve(&self, sample: f32, drive_lin: f32) -> f32 {
        match self.mode {
            Mode::Soft => soft_clip(sample, drive_lin),
            Mode::Hard => hard_clip(sample, drive_lin),
            Mode::Tape => tape_saturation(sample, drive_lin),
            Mode::Tube => tube_emulation(sample, drive_lin),
            Mode::Bitcrush => {
                // Map drive 0..=24 dB → 16..=2 bits so cranking drive
                // crushes harder. Inverse so the knob feels right.
                let bits = (16.0 - (self.drive_db / 24.0).clamp(0.0, 1.0) * 14.0).round() as u8;
                bitcrush(sample, bits.max(1))
            }
        }
    }

    fn from_normalised(id: u32, v: f64) -> f64 {
        let v = v.clamp(0.0, 1.0);
        match id {
            PARAM_DRIVE => v * 24.0,
            PARAM_MODE => v,
            PARAM_MIX => v,
            PARAM_OUTPUT => -24.0 + v * 48.0,
            _ => v,
        }
    }

    fn to_normalised(id: u32, v: f64) -> f64 {
        match id {
            PARAM_DRIVE => (v / 24.0).clamp(0.0, 1.0),
            PARAM_MODE => v.clamp(0.0, 1.0),
            PARAM_MIX => v.clamp(0.0, 1.0),
            PARAM_OUTPUT => ((v + 24.0) / 48.0).clamp(0.0, 1.0),
            _ => v.clamp(0.0, 1.0),
        }
    }
}

impl Default for NativeDistortion {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeDistortion {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    fn activate(&mut self, _sr: f64, _max_block: u32) -> Result<(), String> {
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
        let output_lin = 10.0_f32.powf(self.output_db / 20.0);
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let dist_l = self.apply_curve(in_l, drive_lin);
            let dist_r = self.apply_curve(in_r, drive_lin);
            // Compensate so increasing drive doesn't blast out volume —
            // approximates the perceived loudness of clean signal at
            // matched RMS.
            let comp_l = drive_compensate(dist_l, self.drive_db);
            let comp_r = drive_compensate(dist_r, self.drive_db);
            outputs[0][i] = parallel_mix(in_l, comp_l, self.mix) * output_lin;
            outputs[1][i] = parallel_mix(in_r, comp_r, self.mix) * output_lin;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        match index {
            PARAM_DRIVE => Some(ParameterInfo {
                id: PARAM_DRIVE,
                name: "Drive".into(),
                default_value: Self::to_normalised(PARAM_DRIVE, 6.0),
                min: 0.0,
                max: 1.0,
                unit: "dB".into(),
                automatable: true,
            }),
            PARAM_MODE => Some(ParameterInfo {
                id: PARAM_MODE,
                name: "Mode".into(),
                default_value: Mode::Soft.to_normalised() as f64,
                min: 0.0,
                max: 1.0,
                unit: "".into(),
                automatable: true,
            }),
            PARAM_MIX => Some(ParameterInfo {
                id: PARAM_MIX,
                name: "Mix".into(),
                default_value: 1.0,
                min: 0.0,
                max: 1.0,
                unit: "%".into(),
                automatable: true,
            }),
            PARAM_OUTPUT => Some(ParameterInfo {
                id: PARAM_OUTPUT,
                name: "Output".into(),
                default_value: Self::to_normalised(PARAM_OUTPUT, 0.0),
                min: 0.0,
                max: 1.0,
                unit: "dB".into(),
                automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_DRIVE => Self::to_normalised(id, self.drive_db as f64),
            PARAM_MODE => self.mode.to_normalised() as f64,
            PARAM_MIX => self.mix as f64,
            PARAM_OUTPUT => Self::to_normalised(id, self.output_db as f64),
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        match id {
            PARAM_DRIVE => self.drive_db = Self::from_normalised(id, value) as f32,
            PARAM_MODE => self.mode = Mode::from_normalised(value as f32),
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0) as f32,
            PARAM_OUTPUT => self.output_db = Self::from_normalised(id, value) as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"drive\":{},\"mode\":{},\"mix\":{},\"output\":{}}}",
            self.drive_db, self.mode as u32, self.mix, self.output_db
        )
        .into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest
                .find(|c: char| c == ',' || c == '}')
                .unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("drive") {
            self.drive_db = v;
        }
        if let Some(v) = read("mode") {
            self.mode = match v as u32 {
                1 => Mode::Hard,
                2 => Mode::Tape,
                3 => Mode::Tube,
                4 => Mode::Bitcrush,
                _ => Mode::Soft,
            };
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("output") {
            self.output_db = v;
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
