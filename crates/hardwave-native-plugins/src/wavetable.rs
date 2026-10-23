//! Native wavetable synth — Harmor / Serum / Vital basic-feature clone.
//! Uses `hardwave_dsp::wavetable::WavetableOscillator` plus a built-in
//! Wavetable bank (Basic / Analog / Digital / Vocal / Noise). Voice
//! count: 8, monotimbral.

use crate::format;
use crate::voices::{self, Glide, HeldNotes, MonoStep, VoiceMode, VoiceSettings};
use hardwave_dsp::synth::{AdsrEnvelope, AdsrStage};
use hardwave_dsp::wavetable::{Wavetable, WavetableOscillator};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_BANK: u32 = 0;
const PARAM_POSITION: u32 = 1;
const PARAM_UNISON_DETUNE: u32 = 2;
const PARAM_ATTACK: u32 = 3;
const PARAM_DECAY: u32 = 4;
const PARAM_SUSTAIN: u32 = 5;
const PARAM_RELEASE: u32 = 6;
const PARAM_MASTER: u32 = 7;
const PARAM_VOICE_MODE: u32 = 8;
const PARAM_VOICES: u32 = 9;
const PARAM_GLIDE: u32 = 10;
const PARAM_COUNT: u32 = 11;

#[derive(Clone, Copy)]
enum Bank {
    Basic,
    Analog,
    Digital,
    Vocal,
    Noise,
}

fn bank_from_norm(v: f32) -> Bank {
    let i = ((v.clamp(0.0, 1.0) * 5.0).floor() as i32).clamp(0, 4);
    match i {
        0 => Bank::Basic,
        1 => Bank::Analog,
        2 => Bank::Digital,
        3 => Bank::Vocal,
        _ => Bank::Noise,
    }
}

fn bank_index(b: Bank) -> usize {
    match b {
        Bank::Basic => 0,
        Bank::Analog => 1,
        Bank::Digital => 2,
        Bank::Vocal => 3,
        Bank::Noise => 4,
    }
}

fn bank_to_norm(b: Bank) -> f64 {
    (bank_index(b) as f64 + 0.5) / 5.0
}

/// Bank names by `bank_index`, which is also the order `bank_from_norm`
/// picks them in, so choice i sits at i / 4.
const BANK_NAMES: [&str; 5] = ["Basic", "Analog", "Digital", "Vocal", "Noise"];

/// Detune in semitones (-0.5..+0.5) from the 0..1 knob. The setter and
/// the text both use it so what is shown is what is set.
fn detune_from_norm(v: f64) -> f32 {
    (v - 0.5) as f32
}

/// Envelope time in seconds (1 ms..5 s) from the 0..1 knob.
fn env_secs_from_norm(v: f64) -> f32 {
    (v * 5.0).max(0.001) as f32
}

fn build_table(b: Bank) -> Wavetable {
    match b {
        Bank::Basic => Wavetable::basic(),
        Bank::Analog => Wavetable::analog(),
        Bank::Digital => Wavetable::digital(),
        Bank::Vocal => Wavetable::vocal(),
        Bank::Noise => Wavetable::noise(),
    }
}

struct VoiceSlot {
    osc: WavetableOscillator,
    env: AdsrEnvelope,
    note: u8,
    velocity: f32,
    active: bool,
    /// The voice's pitch, which slides in Mono and Legato.
    glide: Glide,
    /// The detune taken when the note started, in semitones.
    detune: f32,
}

/// A voice further into its sound is the one to steal: an idle one, then one
/// letting go, before one still sounding.
fn steal_rank(stage: AdsrStage) -> u8 {
    match stage {
        AdsrStage::Idle => 0,
        AdsrStage::Release => 1,
        AdsrStage::Sustain => 2,
        AdsrStage::Decay => 3,
        AdsrStage::Attack => 4,
    }
}

impl VoiceSlot {
    fn new(sr: f32) -> Self {
        Self {
            osc: WavetableOscillator::new(sr),
            env: AdsrEnvelope::new(sr),
            note: 0,
            velocity: 0.0,
            active: false,
            glide: Glide::default(),
            detune: 0.0,
        }
    }
}

pub struct NativeWavetable {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    voices: Vec<VoiceSlot>,
    table: Wavetable,
    bank: Bank,
    position: f32,
    unison_detune: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    master_gain: f32,
    active: bool,
    voice_settings: VoiceSettings,
    held: HeldNotes,
}

impl NativeWavetable {
    pub const ID: &'static str = "hardwave.native.wavetable";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Wavetable".into(),
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
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            voices,
            table: Wavetable::analog(),
            bank: Bank::Analog,
            position: 0.0,
            unison_detune: 0.05,
            attack: 0.01,
            decay: 0.2,
            sustain: 0.7,
            release: 0.4,
            master_gain: 0.6,
            active: false,
            voice_settings: VoiceSettings::default(),
            held: HeldNotes::default(),
        }
    }

    /// A full synth stole voice 0 every time, however loud it still was.
    fn alloc_voice(&mut self) -> usize {
        voices::pick_voice(
            &self.voices,
            self.voice_settings.voices,
            |v| !v.active,
            |v| steal_rank(v.env.stage()),
        )
    }

    /// Start a note on slot `idx` with the current sound, the envelope from
    /// the top. `slide` glides the pitch there from where the slot is.
    fn start_note(&mut self, idx: usize, note: u8, velocity: f32, slide: bool) {
        let (attack, decay, sustain, release) =
            (self.attack, self.decay, self.sustain, self.release);
        let (position, detune) = (self.position, self.unison_detune);
        let glide_s = self.voice_settings.glide_s;
        let sr = self.sample_rate;
        let slot = &mut self.voices[idx];
        if slide && slot.active {
            slot.glide.slide(note as f32, glide_s, sr);
        } else {
            slot.glide.jump(note as f32);
            slot.osc.reset_phase();
        }
        slot.note = note;
        slot.velocity = velocity;
        slot.detune = detune;
        slot.osc
            .set_frequency(voices::note_hz(slot.glide.pitch() + detune));
        slot.osc.set_level(1.0);
        slot.osc.set_position(position);
        slot.env.set_times(attack, decay, release);
        slot.env.set_sustain(sustain);
        slot.env.note_on();
        slot.active = true;
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
            MonoStep::Release => self.voices[0].env.note_off(),
            MonoStep::Nothing => {}
        }
    }
}

impl Default for NativeWavetable {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeWavetable {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.voices = (0..voices::MAX_VOICES)
            .map(|_| VoiceSlot::new(self.sample_rate))
            .collect();
        self.held.clear();
        for v in self.voices.iter_mut() {
            v.env.set_times(self.attack, self.decay, self.release);
            v.env.set_sustain(self.sustain);
        }
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
                            slot.env.note_off();
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
                let env_v = slot.env.tick();
                if !slot.env.is_active() {
                    slot.active = false;
                    break;
                }
                if slot.glide.gliding() {
                    let pitch = slot.glide.advance();
                    slot.osc.set_frequency(voices::note_hz(pitch + slot.detune));
                }
                let s = slot.osc.tick(&self.table) * env_v * slot.velocity * self.master_gain;
                outputs[0][i] += s;
                outputs[1][i] += s;
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_BANK => ("Bank", bank_to_norm(Bank::Analog), ""),
            PARAM_POSITION => ("Position", 0.0, ""),
            PARAM_UNISON_DETUNE => ("Detune", 0.5, "ct"),
            PARAM_ATTACK => ("Attack", 0.01 / 5.0, "s"),
            PARAM_DECAY => ("Decay", 0.2 / 5.0, "s"),
            PARAM_SUSTAIN => ("Sustain", 0.7, ""),
            PARAM_RELEASE => ("Release", 0.4 / 5.0, "s"),
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
            PARAM_BANK => bank_to_norm(self.bank),
            PARAM_POSITION => self.position as f64,
            PARAM_UNISON_DETUNE => ((self.unison_detune + 0.5) / 1.0).clamp(0.0, 1.0) as f64,
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
            PARAM_BANK => {
                self.bank = bank_from_norm(v as f32);
                self.table = build_table(self.bank);
            }
            PARAM_POSITION => self.position = v as f32,
            PARAM_UNISON_DETUNE => self.unison_detune = detune_from_norm(v),
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
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        // Same clamp as the setter, so the text names what the value sets.
        let v = value.clamp(0.0, 1.0);
        let text = match id {
            PARAM_BANK => BANK_NAMES[bank_index(bank_from_norm(v as f32))].to_string(),
            // Stored in semitones; half a semitone either way reads
            // better in cents.
            PARAM_UNISON_DETUNE => format::num(detune_from_norm(v) as f64 * 100.0, 0, "ct"),
            PARAM_POSITION | PARAM_SUSTAIN | PARAM_MASTER => format::pct(v),
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
            PARAM_BANK => Some(BANK_NAMES.iter().map(|s| s.to_string()).collect()),
            PARAM_VOICE_MODE => Some(voices::MODE_NAMES.iter().map(|s| s.to_string()).collect()),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"bank\":{},\"pos\":{},\"det\":{},\"a\":{},\"d\":{},\"s\":{},\"r\":{},\"m\":{},{}}}",
            bank_to_norm(self.bank),
            self.position,
            self.unison_detune,
            self.attack,
            self.decay,
            self.sustain,
            self.release,
            self.master_gain,
            self.voice_settings.state_fields()
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
        if let Some(v) = read("bank") {
            self.bank = bank_from_norm(v);
            self.table = build_table(self.bank);
        }
        if let Some(v) = read("pos") {
            self.position = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("det") {
            self.unison_detune = v.clamp(-0.5, 0.5);
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
        let mut p = NativeWavetable::new();
        let shown = |p: &NativeWavetable, id: u32| p.parameter_text(id, p.get_parameter_value(id));
        assert_eq!(shown(&p, PARAM_BANK).as_deref(), Some("Analog"));
        assert_eq!(shown(&p, PARAM_UNISON_DETUNE).as_deref(), Some("5 ct"));
        assert_eq!(shown(&p, PARAM_DECAY).as_deref(), Some("200 ms"));
        assert_eq!(shown(&p, PARAM_MASTER).as_deref(), Some("60 %"));

        let options = p.parameter_options(PARAM_BANK).unwrap();
        let last = (options.len() - 1) as f64;
        for (i, label) in options.iter().enumerate() {
            let v = i as f64 / last;
            assert_eq!(p.parameter_text(PARAM_BANK, v).as_ref(), Some(label));
            p.set_parameter_value(PARAM_BANK, v);
            assert_eq!(shown(&p, PARAM_BANK).as_ref(), Some(label));
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
        let mut p = NativeWavetable::new();
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
        let mut back = NativeWavetable::new();
        back.set_state(&state).unwrap();
        for id in [PARAM_VOICE_MODE, PARAM_VOICES, PARAM_GLIDE] {
            assert!((back.get_parameter_value(id) - p.get_parameter_value(id)).abs() < 1e-4);
        }
    }
}
