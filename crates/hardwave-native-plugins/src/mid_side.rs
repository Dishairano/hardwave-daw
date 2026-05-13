//! Native mid/side processor — independent gain on Mid (mono content)
//! and Side (stereo content), plus M-only and S-only solo for tonal
//! work. Distinct from NativeStereo which exposes width and balance.
//! Mastering staple for tightening centre or widening sides.

use hardwave_dsp::stereo::{decode_ms, encode_ms};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_MID_GAIN: u32 = 0;
const PARAM_SIDE_GAIN: u32 = 1;
const PARAM_SOLO: u32 = 2;
const PARAM_COUNT: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Solo {
    Off,
    Mid,
    Side,
}

fn solo_from_norm(v: f32) -> Solo {
    let i = ((v.clamp(0.0, 1.0) * 3.0).floor() as i32).clamp(0, 2);
    match i {
        0 => Solo::Off,
        1 => Solo::Mid,
        _ => Solo::Side,
    }
}

fn solo_to_norm(s: Solo) -> f64 {
    let i = match s {
        Solo::Off => 0,
        Solo::Mid => 1,
        Solo::Side => 2,
    };
    (i as f64 + 0.5) / 3.0
}

pub struct NativeMidSide {
    descriptor: PluginDescriptor,
    mid_gain_db: f32,
    side_gain_db: f32,
    solo: Solo,
    active: bool,
}

impl NativeMidSide {
    pub const ID: &'static str = "hardwave.native.mid_side";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave M/S".into(),
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
            mid_gain_db: 0.0,
            side_gain_db: 0.0,
            solo: Solo::Off,
            active: false,
        }
    }
}

impl Default for NativeMidSide {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeMidSide {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, _sr: f64, _max: u32) -> Result<(), String> {
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
        let mid_lin = 10.0_f32.powf(self.mid_gain_db / 20.0);
        let side_lin = 10.0_f32.powf(self.side_gain_db / 20.0);
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let (mut mid, mut side) = encode_ms(in_l, in_r);
            mid *= mid_lin;
            side *= side_lin;
            match self.solo {
                Solo::Mid => side = 0.0,
                Solo::Side => mid = 0.0,
                Solo::Off => {}
            }
            let (l, r) = decode_ms(mid, side);
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_MID_GAIN => ("Mid", 0.5, "dB"), // 0.5 = unity in -24..+24 mapping
            PARAM_SIDE_GAIN => ("Side", 0.5, "dB"),
            PARAM_SOLO => ("Solo", solo_to_norm(Solo::Off), ""),
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
            // -24..=+24 dB mapped to 0..=1, 0.5 = unity
            PARAM_MID_GAIN => ((self.mid_gain_db + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_SIDE_GAIN => ((self.side_gain_db + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_SOLO => solo_to_norm(self.solo),
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_MID_GAIN => self.mid_gain_db = (v * 48.0 - 24.0) as f32,
            PARAM_SIDE_GAIN => self.side_gain_db = (v * 48.0 - 24.0) as f32,
            PARAM_SOLO => self.solo = solo_from_norm(v as f32),
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"mid\":{},\"side\":{},\"solo\":{}}}",
            self.mid_gain_db,
            self.side_gain_db,
            solo_to_norm(self.solo)
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
        if let Some(v) = read("mid") {
            self.mid_gain_db = v.clamp(-24.0, 24.0);
        }
        if let Some(v) = read("side") {
            self.side_gain_db = v.clamp(-24.0, 24.0);
        }
        if let Some(v) = read("solo") {
            self.solo = solo_from_norm(v);
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
