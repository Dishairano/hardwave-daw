//! Native FM synth — wraps `hardwave_dsp::fm_synth::FmVoice` (4-op FM)
//! into an 8-voice polyphonic instrument. Mirrors FL Studio FM7/DX10
//! at the basics level: algorithm picker, 4 op ratios + levels, shared
//! ADSR. Hardstyle producers use this for screech leads + bell stabs.

use crate::format;
use crate::voices::{self, Glide, HeldNotes, MonoStep, VoiceMode, VoiceSettings};
use hardwave_dsp::fm_synth::{Algorithm, FmVoice};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

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
const PARAM_VOICE_MODE: u32 = 14;
const PARAM_VOICES: u32 = 15;
const PARAM_GLIDE: u32 = 16;
const PARAM_COUNT: u32 = 17;

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

fn algo_index(a: Algorithm) -> usize {
    match a {
        Algorithm::Stack => 0,
        Algorithm::ParallelMid => 1,
        Algorithm::ThreeToOne => 2,
        Algorithm::DualPair => 3,
        Algorithm::Parallel => 4,
        Algorithm::OneModTwoPlusTwoCarriers => 5,
        Algorithm::FanOutCarriers => 6,
        Algorithm::ChainPlusSolo => 7,
    }
}

fn algo_to_norm(a: Algorithm) -> f64 {
    (algo_index(a) as f64 + 0.5) / 8.0
}

/// Algorithm names by `algo_index`, which is also the order
/// `algo_from_norm` picks them in, so choice i sits at i / 7.
const ALGO_NAMES: [&str; 8] = [
    "Stack",
    "Parallel Mid",
    "3 to 1",
    "Dual Pair",
    "Parallel",
    "Pair + 2",
    "Fan Out",
    "Chain + Solo",
];

/// Operator frequency ratio (0.01..8) from the 0..1 knob. The setter and
/// the text both use it so what is shown is what is set.
fn ratio_from_norm(v: f64) -> f32 {
    (v * 8.0).max(0.01) as f32
}

/// Envelope time in seconds (1 ms..5 s) from the 0..1 knob.
fn env_secs_from_norm(v: f64) -> f32 {
    (v * 5.0).max(0.001) as f32
}

struct VoiceSlot {
    voice: FmVoice,
    note: u8,
    active: bool,
    velocity: f32,
    /// The voice's pitch, which slides in Mono and Legato.
    glide: Glide,
}

impl VoiceSlot {
    fn new(sr: f32) -> Self {
        Self {
            voice: FmVoice::new(sr),
            note: 0,
            active: false,
            velocity: 0.0,
            glide: Glide::default(),
        }
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
    voice_settings: VoiceSettings,
    held: HeldNotes,
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
        let voices = (0..voices::MAX_VOICES)
            .map(|_| VoiceSlot::new(sr))
            .collect();
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
            voice_settings: VoiceSettings::default(),
            held: HeldNotes::default(),
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

    fn alloc_voice(&mut self) -> usize {
        // No idle slot: steal one whose sound has ended, else the first.
        voices::pick_voice(
            &self.voices,
            self.voice_settings.voices,
            |v| !v.active,
            |v| u8::from(v.voice.is_active()),
        )
    }

    /// Start a note on slot `idx` with the current sound, envelopes from
    /// the top. `slide` glides the pitch there from where the slot is.
    fn start_note(&mut self, idx: usize, note: u8, velocity: f32, slide: bool) {
        let glide_s = self.voice_settings.glide_s;
        let sr = self.sample_rate;
        let (algorithm, ratios, levels) = (self.algorithm, self.op_ratios, self.op_levels);
        let (attack, decay, sustain, release) =
            (self.attack, self.decay, self.sustain, self.release);
        let slot = &mut self.voices[idx];
        if slide && slot.active {
            slot.glide.slide(note as f32, glide_s, sr);
        } else {
            slot.glide.jump(note as f32);
        }
        slot.note = note;
        slot.active = true;
        slot.velocity = velocity;
        slot.voice.set_base_hz(voices::note_hz(slot.glide.pitch()));
        slot.voice.set_velocity(velocity);
        slot.voice.set_algorithm(algorithm);
        for i in 0..4 {
            if let Some(op) = slot.voice.op_mut(i) {
                op.set_ratio(ratios[i], 0.0);
                op.set_level(levels[i]);
                op.set_envelope_times(attack, decay, release);
                op.set_sustain(sustain);
            }
        }
        slot.voice.note_on();
    }

    /// Mono and Legato play on slot 0.
    fn mono(&mut self, step: MonoStep, velocity: f32) {
        let glide_s = self.voice_settings.glide_s;
        let sr = self.sample_rate;
        match step {
            MonoStep::Start(note) => self.start_note(0, note, velocity, true),
            MonoStep::Glide(note) => {
                let slot = &mut self.voices[0];
                slot.note = note;
                slot.glide.slide(note as f32, glide_s, sr);
            }
            MonoStep::Release => self.voices[0].voice.note_off(),
            MonoStep::Nothing => {}
        }
    }
}

impl Default for NativeFmSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeFmSynth {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.voices = (0..voices::MAX_VOICES)
            .map(|_| VoiceSlot::new(self.sample_rate))
            .collect();
        self.held.clear();
        self.refresh_static();
        self.active = true;
        Ok(())
    }
    fn deactivate(&mut self) {
        self.active = false;
    }

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
        if !self.active || outputs.len() < 2 {
            return;
        }

        for ev in midi_in {
            match ev {
                MidiEvent::NoteOn { note, velocity, .. } if *velocity > 0.0 => {
                    let mode = self.voice_settings.mode;
                    if mode == VoiceMode::Poly {
                        let idx = self.alloc_voice();
                        self.start_note(idx, *note, *velocity, false);
                    } else {
                        let step = self.held.press(mode, *note);
                        self.mono(step, *velocity);
                    }
                }
                MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note, .. } => {
                    let mode = self.voice_settings.mode;
                    let first = if mode == VoiceMode::Poly {
                        0
                    } else {
                        let step = self.held.lift(mode, *note);
                        let velocity = self.voices[0].velocity;
                        self.mono(step, velocity);
                        1
                    };
                    for slot in self.voices.iter_mut().skip(first) {
                        if slot.active && slot.note == *note {
                            slot.voice.note_off();
                        }
                    }
                }
                _ => {}
            }
        }

        for slot in self.voices.iter_mut() {
            if !slot.active {
                continue;
            }
            for i in 0..num_samples {
                if slot.glide.gliding() {
                    let pitch = slot.glide.advance();
                    slot.voice.set_base_hz(voices::note_hz(pitch));
                }
                let s = slot.voice.tick() * self.master_gain;
                outputs[0][i] += s;
                outputs[1][i] += s;
            }
            if !slot.voice.is_active() {
                slot.active = false;
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

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
            PARAM_VOICE_MODE => ("Voice mode", 0.0, ""),
            PARAM_VOICES => ("Voices", VoiceSettings::voices_to_norm(8), ""),
            PARAM_GLIDE => ("Glide", VoiceSettings::glide_to_norm(0.05), "s"),
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
            PARAM_VOICE_MODE => VoiceSettings::mode_to_norm(self.voice_settings.mode),
            PARAM_VOICES => VoiceSettings::voices_to_norm(self.voice_settings.voices),
            PARAM_GLIDE => VoiceSettings::glide_to_norm(self.voice_settings.glide_s),
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_ALGORITHM => self.algorithm = algo_from_norm(v as f32),
            PARAM_OP1_RATIO => self.op_ratios[0] = ratio_from_norm(v),
            PARAM_OP1_LEVEL => self.op_levels[0] = v as f32,
            PARAM_OP2_RATIO => self.op_ratios[1] = ratio_from_norm(v),
            PARAM_OP2_LEVEL => self.op_levels[1] = v as f32,
            PARAM_OP3_RATIO => self.op_ratios[2] = ratio_from_norm(v),
            PARAM_OP3_LEVEL => self.op_levels[2] = v as f32,
            PARAM_OP4_RATIO => self.op_ratios[3] = ratio_from_norm(v),
            PARAM_OP4_LEVEL => self.op_levels[3] = v as f32,
            PARAM_ATTACK => self.attack = env_secs_from_norm(v),
            PARAM_DECAY => self.decay = env_secs_from_norm(v),
            PARAM_SUSTAIN => self.sustain = v as f32,
            PARAM_RELEASE => self.release = env_secs_from_norm(v),
            PARAM_MASTER => self.master_gain = v as f32,
            PARAM_VOICE_MODE => {
                let mode = VoiceSettings::mode_from_norm(v);
                if mode != self.voice_settings.mode {
                    self.held.clear();
                    self.voice_settings.mode = mode;
                }
            }
            PARAM_VOICES => self.voice_settings.voices = VoiceSettings::voices_from_norm(v),
            PARAM_GLIDE => self.voice_settings.glide_s = VoiceSettings::glide_from_norm(v),
            _ => {}
        }
        self.refresh_static();
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        // Same clamp as the setter, so the text names what the value sets.
        let v = value.clamp(0.0, 1.0);
        let text = match id {
            PARAM_ALGORITHM => ALGO_NAMES[algo_index(algo_from_norm(v as f32))].to_string(),
            PARAM_OP1_RATIO | PARAM_OP2_RATIO | PARAM_OP3_RATIO | PARAM_OP4_RATIO => {
                format::num(ratio_from_norm(v) as f64, 2, "x")
            }
            PARAM_OP1_LEVEL | PARAM_OP2_LEVEL | PARAM_OP3_LEVEL | PARAM_OP4_LEVEL
            | PARAM_SUSTAIN | PARAM_MASTER => format::pct(v),
            PARAM_ATTACK | PARAM_DECAY | PARAM_RELEASE => {
                format::secs(env_secs_from_norm(v) as f64)
            }
            PARAM_VOICE_MODE => return VoiceSettings::text(0, v),
            PARAM_VOICES => return VoiceSettings::text(1, v),
            PARAM_GLIDE => return VoiceSettings::text(2, v),
            _ => return None,
        };
        Some(text)
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match id {
            PARAM_ALGORITHM => Some(ALGO_NAMES.iter().map(|s| s.to_string()).collect()),
            PARAM_VOICE_MODE => Some(voices::MODE_NAMES.iter().map(|s| s.to_string()).collect()),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"alg\":{},\"r1\":{},\"l1\":{},\"r2\":{},\"l2\":{},\"r3\":{},\"l3\":{},\"r4\":{},\"l4\":{},\"a\":{},\"d\":{},\"s\":{},\"r\":{},\"m\":{},{}}}",
            algo_to_norm(self.algorithm),
            self.op_ratios[0], self.op_levels[0],
            self.op_ratios[1], self.op_levels[1],
            self.op_ratios[2], self.op_levels[2],
            self.op_ratios[3], self.op_levels[3],
            self.attack, self.decay, self.sustain, self.release, self.master_gain,
            self.voice_settings.state_fields()
        ).into_bytes()
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
        if let Some(v) = read("alg") {
            self.algorithm = algo_from_norm(v);
        }
        for (i, key) in [("r1", "l1"), ("r2", "l2"), ("r3", "l3"), ("r4", "l4")]
            .iter()
            .enumerate()
        {
            if let Some(v) = read(key.0) {
                self.op_ratios[i] = v.max(0.01);
            }
            if let Some(v) = read(key.1) {
                self.op_levels[i] = v.clamp(0.0, 1.0);
            }
        }
        if let Some(v) = read("a") {
            self.attack = v.max(0.001);
        }
        if let Some(v) = read("d") {
            self.decay = v.max(0.001);
        }
        if let Some(v) = read("s") {
            self.sustain = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("r") {
            self.release = v.max(0.001);
        }
        if let Some(v) = read("m") {
            self.master_gain = v.clamp(0.0, 1.0);
        }
        self.voice_settings.read_state(read);
        self.refresh_static();
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
    fn values_read_as_text_and_choices_match_the_setter() {
        let mut p = NativeFmSynth::new();
        let shown = |p: &NativeFmSynth, id: u32| p.parameter_text(id, p.get_parameter_value(id));
        assert_eq!(shown(&p, PARAM_ALGORITHM).as_deref(), Some("Parallel Mid"));
        assert_eq!(shown(&p, PARAM_OP2_RATIO).as_deref(), Some("2.00 x"));
        assert_eq!(shown(&p, PARAM_MASTER).as_deref(), Some("60 %"));
        assert_eq!(shown(&p, PARAM_RELEASE).as_deref(), Some("300 ms"));

        let options = p.parameter_options(PARAM_ALGORITHM).unwrap();
        let last = (options.len() - 1) as f64;
        for (i, label) in options.iter().enumerate() {
            let v = i as f64 / last;
            assert_eq!(p.parameter_text(PARAM_ALGORITHM, v).as_ref(), Some(label));
            p.set_parameter_value(PARAM_ALGORITHM, v);
            assert_eq!(shown(&p, PARAM_ALGORITHM).as_ref(), Some(label));
        }
        for id in 0..PARAM_COUNT {
            assert!(
                p.parameter_text(id, 0.5).is_some(),
                "parameter {id} has no text"
            );
        }
    }

    #[test]
    fn voice_settings_play_and_come_back() {
        let on = |note| MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note,
            velocity: 0.8,
        };
        let mut p = NativeFmSynth::new();
        p.activate(48_000.0, 512).unwrap();
        p.set_parameter_value(PARAM_VOICES, VoiceSettings::voices_to_norm(2));
        let mut out = vec![Vec::new(), Vec::new()];
        p.process(
            &[],
            &mut out,
            &[on(60), on(64), on(67)],
            &mut Vec::new(),
            64,
        );
        assert_eq!(p.voices.iter().filter(|v| v.active).count(), 2);

        p.set_parameter_value(
            PARAM_VOICE_MODE,
            VoiceSettings::mode_to_norm(VoiceMode::Legato),
        );
        p.process(&[], &mut out, &[on(48), on(55)], &mut Vec::new(), 64);
        assert!(p.voices[0].glide.gliding(), "Legato slides to the new note");

        let state = p.get_state();
        let mut back = NativeFmSynth::new();
        back.set_state(&state).unwrap();
        for id in [PARAM_VOICE_MODE, PARAM_VOICES, PARAM_GLIDE] {
            assert!((back.get_parameter_value(id) - p.get_parameter_value(id)).abs() < 1e-4);
        }
    }
}
