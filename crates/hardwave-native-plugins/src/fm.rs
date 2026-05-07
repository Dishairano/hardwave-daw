//! Native FM synth — wraps `hardwave_dsp::fm_synth::FmVoice` (4-op FM)
//! into an 8-voice polyphonic instrument. Mirrors FL Studio FM7/DX10
//! at the basics level: algorithm picker, 4 op ratios + levels, shared
//! ADSR. Hardstyle producers use this for screech leads + bell stabs.

use hardwave_dsp::fm_synth::{Algorithm, FmVoice};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const POLYPHONY: usize = 8;

const PARAM_ALGORITHM: u32 = 0;
const PARAM_OP1_RATIO: u32 = 1;
const PARAM_OP1_LEVEL: u32 = 2;
const PARAM_OP2_RATIO: u32 = 3;
const PARAM_OP2_LEVEL: u32 = 4;
const PARAM_OP3_RATIO: u32 = 5;
const PARAM_OP3_LEVEL: u32 = 6;
const PARAM_OP4_RATIO: u32 = 7;
const PARAM_OP4_LEVEL: u32 = 8;
const PARAM_ATTACK: u32 = 9;
const PARAM_DECAY: u32 = 10;
const PARAM_SUSTAIN: u32 = 11;
const PARAM_RELEASE: u32 = 12;
const PARAM_MASTER: u32 = 13;
const PARAM_COUNT: u32 = 14;

fn algo_from_norm(v: f32) -> Algorithm {
    let i = ((v.clamp(0.0, 1.0) * 8.0).floor() as i32).clamp(0, 7);
    match i {
        0 => Algorithm::Stack,
        1 => Algorithm::ParallelMid,
        2 => Algorithm::ThreeToOne,
        3 => Algorithm::DualPair,
        4 => Algorithm::Parallel,
        5 => Algorithm::OneModTwoPlusTwoCarriers,
        6 => Algorithm::FanOutCarriers,
        _ => Algorithm::ChainPlusSolo,
    }
}

fn algo_to_norm(a: Algorithm) -> f64 {
    let idx = match a {
        Algorithm::Stack => 0,
        Algorithm::ParallelMid => 1,
        Algorithm::ThreeToOne => 2,
        Algorithm::DualPair => 3,
        Algorithm::Parallel => 4,
        Algorithm::OneModTwoPlusTwoCarriers => 5,
        Algorithm::FanOutCarriers => 6,
        Algorithm::ChainPlusSolo => 7,
    };
    (idx as f64 + 0.5) / 8.0
}

struct VoiceSlot {
    voice: FmVoice,
    note: u8,
    active: bool,
}

impl VoiceSlot {
    fn new(sr: f32) -> Self {
        Self { voice: FmVoice::new(sr), note: 0, active: false }
    }
}

pub struct NativeFmSynth {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    voices: Vec<VoiceSlot>,
    algorithm: Algorithm,
    op_ratios: [f32; 4],
    op_levels: [f32; 4],
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    master_gain: f32,
    active: bool,
}

impl NativeFmSynth {
    pub const ID: &'static str = "hardwave.native.fm";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave FM".into(),
            vendor: "Hardwave".into(),
            version: "1.0.0".into(),
            format: PluginFormat::Clap,
            path: PathBuf::from("<native>"),
            category: PluginCategory::Instrument,
            num_inputs: 0,
            num_outputs: 2,
            has_midi_input: true,
            has_editor: false,
        }
    }

    pub fn new() -> Self {
        let sr = 48_000.0_f32;
        let voices = (0..POLYPHONY).map(|_| VoiceSlot::new(sr)).collect();
        let mut s = Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            voices,
            algorithm: Algorithm::ParallelMid,
            op_ratios: [1.0, 2.0, 3.0, 1.0],
            op_levels: [0.8, 0.7, 0.6, 1.0],
            attack: 0.005,
            decay: 0.15,
            sustain: 0.7,
            release: 0.3,
            master_gain: 0.6,
            active: false,
        };
        s.refresh_static();
        s
    }

    /// Apply non-per-voice settings (ratios/levels/algorithm/ADSR) to
    /// each voice template. Re-applied on parameter changes.
    fn refresh_static(&mut self) {
        for slot in self.voices.iter_mut() {
            slot.voice.set_algorithm(self.algorithm);
            for i in 0..4 {
                if let Some(op) = slot.voice.op_mut(i) {
                    op.set_ratio(self.op_ratios[i], 0.0);
                    op.set_level(self.op_levels[i]);
                    op.set_envelope_times(self.attack, self.decay, self.release);
                    op.set_sustain(self.sustain);
                }
            }
        }
    }

    fn note_to_freq(midi_note: u8) -> f32 {
        let semis = midi_note as f32 - 69.0;
        440.0 * (2.0_f32).powf(semis / 12.0)
    }

    fn alloc_voice(&mut self) -> usize {
        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            return idx;
        }
        // No idle slot — steal the longest-running voice.
        self.voices
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let an = if a.1.voice.is_active() { 1 } else { 0 };
                let bn = if b.1.voice.is_active() { 1 } else { 0 };
                an.cmp(&bn)
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

impl Default for NativeFmSynth {
    fn default() -> Self { Self::new() }
}

impl HostedPlugin for NativeFmSynth {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.voices = (0..POLYPHONY).map(|_| VoiceSlot::new(self.sample_rate)).collect();
        self.refresh_static();
        self.active = true;
        Ok(())
    }
    fn deactivate(&mut self) { self.active = false; }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[MidiEvent],
        _midi_out: &mut Vec<MidiEvent>,
        num_samples: usize,
    ) {
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        if !self.active || outputs.len() < 2 { return; }

        for ev in midi_in {
            match ev {
                MidiEvent::NoteOn { note, velocity, .. } if *velocity > 0.0 => {
                    let idx = self.alloc_voice();
                    let f = Self::note_to_freq(*note);
                    let slot = &mut self.voices[idx];
                    slot.note = *note;
                    slot.active = true;
                    slot.voice.set_base_hz(f);
                    slot.voice.set_velocity(*velocity);
                    slot.voice.set_algorithm(self.algorithm);
                    for i in 0..4 {
                        if let Some(op) = slot.voice.op_mut(i) {
                            op.set_ratio(self.op_ratios[i], 0.0);
                            op.set_level(self.op_levels[i]);
                            op.set_envelope_times(self.attack, self.decay, self.release);
                            op.set_sustain(self.sustain);
                        }
                    }
                    slot.voice.note_on();
                }
                MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note, .. } => {
                    for slot in self.voices.iter_mut() {
                        if slot.active && slot.note == *note {
                            slot.voice.note_off();
                        }
                    }
                }
                _ => {}
            }
        }

        for slot in self.voices.iter_mut() {
            if !slot.active { continue; }
            for i in 0..num_samples {
                let s = slot.voice.tick() * self.master_gain;
                outputs[0][i] += s;
                outputs[1][i] += s;
            }
            if !slot.voice.is_active() {
                slot.active = false;
            }
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_ALGORITHM => ("Algorithm", algo_to_norm(Algorithm::ParallelMid), ""),
            PARAM_OP1_RATIO => ("Op1 Ratio", 0.125, "x"),
            PARAM_OP1_LEVEL => ("Op1 Level", 0.8, "%"),
            PARAM_OP2_RATIO => ("Op2 Ratio", 0.25, "x"),
            PARAM_OP2_LEVEL => ("Op2 Level", 0.7, "%"),
            PARAM_OP3_RATIO => ("Op3 Ratio", 0.375, "x"),
            PARAM_OP3_LEVEL => ("Op3 Level", 0.6, "%"),
            PARAM_OP4_RATIO => ("Op4 Ratio", 0.125, "x"),
            PARAM_OP4_LEVEL => ("Op4 Level", 1.0, "%"),
            PARAM_ATTACK => ("Attack", 0.005 / 5.0, "s"),
            PARAM_DECAY => ("Decay", 0.15 / 5.0, "s"),
            PARAM_SUSTAIN => ("Sustain", 0.7, ""),
            PARAM_RELEASE => ("Release", 0.3 / 5.0, "s"),
            PARAM_MASTER => ("Master", 0.6, "%"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index, name: name.into(), default_value: default,
            min: 0.0, max: 1.0, unit: unit.into(), automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_ALGORITHM => algo_to_norm(self.algorithm),
            PARAM_OP1_RATIO => (self.op_ratios[0] / 8.0).clamp(0.0, 1.0) as f64,
            PARAM_OP1_LEVEL => self.op_levels[0] as f64,
            PARAM_OP2_RATIO => (self.op_ratios[1] / 8.0).clamp(0.0, 1.0) as f64,
            PARAM_OP2_LEVEL => self.op_levels[1] as f64,
            PARAM_OP3_RATIO => (self.op_ratios[2] / 8.0).clamp(0.0, 1.0) as f64,
            PARAM_OP3_LEVEL => self.op_levels[2] as f64,
            PARAM_OP4_RATIO => (self.op_ratios[3] / 8.0).clamp(0.0, 1.0) as f64,
            PARAM_OP4_LEVEL => self.op_levels[3] as f64,
            PARAM_ATTACK => (self.attack / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_DECAY => (self.decay / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_SUSTAIN => self.sustain as f64,
            PARAM_RELEASE => (self.release / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_MASTER => self.master_gain as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_ALGORITHM => self.algorithm = algo_from_norm(v as f32),
            PARAM_OP1_RATIO => self.op_ratios[0] = (v * 8.0).max(0.01) as f32,
            PARAM_OP1_LEVEL => self.op_levels[0] = v as f32,
            PARAM_OP2_RATIO => self.op_ratios[1] = (v * 8.0).max(0.01) as f32,
            PARAM_OP2_LEVEL => self.op_levels[1] = v as f32,
            PARAM_OP3_RATIO => self.op_ratios[2] = (v * 8.0).max(0.01) as f32,
            PARAM_OP3_LEVEL => self.op_levels[2] = v as f32,
            PARAM_OP4_RATIO => self.op_ratios[3] = (v * 8.0).max(0.01) as f32,
            PARAM_OP4_LEVEL => self.op_levels[3] = v as f32,
            PARAM_ATTACK => self.attack = (v * 5.0).max(0.001) as f32,
            PARAM_DECAY => self.decay = (v * 5.0).max(0.001) as f32,
            PARAM_SUSTAIN => self.sustain = v as f32,
            PARAM_RELEASE => self.release = (v * 5.0).max(0.001) as f32,
            PARAM_MASTER => self.master_gain = v as f32,
            _ => {}
        }
        self.refresh_static();
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"alg\":{},\"r1\":{},\"l1\":{},\"r2\":{},\"l2\":{},\"r3\":{},\"l3\":{},\"r4\":{},\"l4\":{},\"a\":{},\"d\":{},\"s\":{},\"r\":{},\"m\":{}}}",
            algo_to_norm(self.algorithm),
            self.op_ratios[0], self.op_levels[0],
            self.op_ratios[1], self.op_levels[1],
            self.op_ratios[2], self.op_levels[2],
            self.op_ratios[3], self.op_levels[3],
            self.attack, self.decay, self.sustain, self.release, self.master_gain
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
        if let Some(v) = read("alg") { self.algorithm = algo_from_norm(v); }
        for (i, key) in [("r1","l1"),("r2","l2"),("r3","l3"),("r4","l4")].iter().enumerate() {
            if let Some(v) = read(key.0) { self.op_ratios[i] = v.max(0.01); }
            if let Some(v) = read(key.1) { self.op_levels[i] = v.clamp(0.0, 1.0); }
        }
        if let Some(v) = read("a") { self.attack = v.max(0.001); }
        if let Some(v) = read("d") { self.decay = v.max(0.001); }
        if let Some(v) = read("s") { self.sustain = v.clamp(0.0, 1.0); }
        if let Some(v) = read("r") { self.release = v.max(0.001); }
        if let Some(v) = read("m") { self.master_gain = v.clamp(0.0, 1.0); }
        self.refresh_static();
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
