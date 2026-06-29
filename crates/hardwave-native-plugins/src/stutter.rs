//! Native beat-repeat / stutter — captures a slice of incoming audio and
//! loops it for a few repeats with optional per-repeat decay, producing
//! the glitch/stutter effect from the Gross-Beat family.
//!
//! This is the **internal-rate** sibling: the slice length is set in
//! milliseconds rather than synced to the host bar, because native
//! plugins aren't handed the transport/tempo in `process`. A
//! host-synced, curve-editable Gross-Beat proper is a follow-up that
//! needs transport plumbed into the plugin process path.

use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_MIX: u32 = 0;
const PARAM_SLICE: u32 = 1; // 30..500 ms
const PARAM_REPEATS: u32 = 2; // 1..8
const PARAM_DECAY: u32 = 3; // per-repeat gain 0.5..1.0
const PARAM_COUNT: u32 = 4;

const SLICE_MIN_MS: f32 = 30.0;
const SLICE_MAX_MS: f32 = 500.0;
const MAX_REPEATS: u32 = 8;

pub struct NativeStutter {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    active: bool,
    // Hold buffer for the captured slice (stereo, interleaved L,R).
    hold_l: Vec<f32>,
    hold_r: Vec<f32>,
    slice_len: usize,
    pos_in_slice: usize,
    slice_in_group: u32,
    // Params
    mix: f32,
    slice_ms: f32,
    repeats: u32,
    decay: f32,
}

impl NativeStutter {
    pub const ID: &'static str = "hardwave.native.stutter";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Stutter".into(),
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
        let slice_ms = 125.0;
        let slice_len = ((slice_ms / 1000.0) * sr) as usize;
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            active: false,
            hold_l: vec![0.0; slice_len.max(1)],
            hold_r: vec![0.0; slice_len.max(1)],
            slice_len: slice_len.max(1),
            pos_in_slice: 0,
            slice_in_group: 0,
            mix: 0.5,
            slice_ms,
            repeats: 4,
            decay: 0.85,
        }
    }

    fn recompute_slice(&mut self) {
        let len = (((self.slice_ms / 1000.0) * self.sample_rate) as usize).max(1);
        self.slice_len = len;
        self.hold_l.resize(len, 0.0);
        self.hold_r.resize(len, 0.0);
        if self.pos_in_slice >= len {
            self.pos_in_slice = 0;
        }
    }
}

impl Default for NativeStutter {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeStutter {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = (sr as f32).max(1.0);
        self.recompute_slice();
        self.pos_in_slice = 0;
        self.slice_in_group = 0;
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
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);

            let (wet_l, wet_r) = if self.slice_in_group == 0 {
                // First slice of the group: pass through AND record it.
                self.hold_l[self.pos_in_slice] = in_l;
                self.hold_r[self.pos_in_slice] = in_r;
                (in_l, in_r)
            } else {
                // Subsequent slices: replay the captured slice, decayed.
                let g = self.decay.powi(self.slice_in_group as i32);
                (
                    self.hold_l[self.pos_in_slice] * g,
                    self.hold_r[self.pos_in_slice] * g,
                )
            };

            outputs[0][i] = in_l * (1.0 - mix) + wet_l * mix;
            outputs[1][i] = in_r * (1.0 - mix) + wet_r * mix;

            self.pos_in_slice += 1;
            if self.pos_in_slice >= self.slice_len {
                self.pos_in_slice = 0;
                self.slice_in_group += 1;
                if self.slice_in_group >= self.repeats {
                    self.slice_in_group = 0;
                }
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_MIX => ("Mix", 0.5, ""),
            PARAM_SLICE => (
                "Slice",
                ((125.0 - SLICE_MIN_MS) / (SLICE_MAX_MS - SLICE_MIN_MS)) as f64,
                "ms",
            ),
            PARAM_REPEATS => ("Repeats", 3.0 / (MAX_REPEATS - 1) as f64, ""),
            PARAM_DECAY => ("Decay", (0.85 - 0.5) / 0.5, ""),
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
            PARAM_MIX => self.mix as f64,
            PARAM_SLICE => ((self.slice_ms - SLICE_MIN_MS) / (SLICE_MAX_MS - SLICE_MIN_MS))
                .clamp(0.0, 1.0) as f64,
            PARAM_REPEATS => ((self.repeats - 1) as f32 / (MAX_REPEATS - 1) as f32) as f64,
            PARAM_DECAY => ((self.decay - 0.5) / 0.5).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0) as f32;
        match id {
            PARAM_MIX => self.mix = v,
            PARAM_SLICE => {
                self.slice_ms = SLICE_MIN_MS + v * (SLICE_MAX_MS - SLICE_MIN_MS);
                self.recompute_slice();
            }
            PARAM_REPEATS => {
                self.repeats = 1 + (v * (MAX_REPEATS - 1) as f32).round() as u32;
            }
            PARAM_DECAY => self.decay = 0.5 + v * 0.5,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"mix\":{},\"slice\":{},\"rep\":{},\"dec\":{}}}",
            self.mix, self.slice_ms, self.repeats, self.decay
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
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("slice") {
            self.slice_ms = v.clamp(SLICE_MIN_MS, SLICE_MAX_MS);
        }
        if let Some(v) = read("rep") {
            self.repeats = (v as u32).clamp(1, MAX_REPEATS);
        }
        if let Some(v) = read("dec") {
            self.decay = v.clamp(0.5, 1.0);
        }
        self.recompute_slice();
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

    fn run(s: &mut NativeStutter, l: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let r = l.to_vec();
        let inputs: [&[f32]; 2] = [l, &r];
        let mut outputs = vec![vec![0.0; l.len()], vec![0.0; l.len()]];
        s.process(&inputs, &mut outputs, &[], &mut Vec::new(), l.len());
        (outputs[0].clone(), outputs[1].clone())
    }

    #[test]
    fn silence_in_silence_out() {
        let mut s = NativeStutter::new();
        s.activate(48_000.0, 512).unwrap();
        let (o, _) = run(&mut s, &vec![0.0; 4096]);
        assert!(o.iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn first_slice_passes_through_fully_wet() {
        let mut s = NativeStutter::new();
        s.mix = 1.0;
        s.slice_ms = 1.0; // tiny slice for the test
        s.repeats = 2;
        s.activate(48_000.0, 512).unwrap();
        // slice_len at 48k, 1ms = 48 samples. First slice = pass-through.
        let input: Vec<f32> = (0..s.slice_len).map(|i| (i as f32 * 0.01).sin()).collect();
        let (o, _) = run(&mut s, &input);
        for (a, b) in o.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6, "first slice must pass through");
        }
    }

    #[test]
    fn second_slice_repeats_first_with_decay() {
        let mut s = NativeStutter::new();
        s.mix = 1.0;
        s.repeats = 2;
        s.decay = 0.5;
        s.activate(48_000.0, 512).unwrap();
        let len = s.slice_len;
        // Feed slice 1 = constant 1.0, slice 2 = constant 0.0.
        let mut input = vec![1.0; len];
        input.extend(vec![0.0; len]);
        let (o, _) = run(&mut s, &input);
        // Slice 1 passes 1.0; slice 2 replays slice 1 (=1.0) * decay 0.5.
        assert!((o[0] - 1.0).abs() < 1e-6);
        assert!((o[len] - 0.5).abs() < 1e-6, "second slice = first * decay");
    }

    #[test]
    fn inactive_passes_through() {
        let mut s = NativeStutter::new();
        let (o, _) = run(&mut s, &[0.1, -0.2, 0.3]);
        assert_eq!(o, vec![0.1, -0.2, 0.3]);
    }
}
