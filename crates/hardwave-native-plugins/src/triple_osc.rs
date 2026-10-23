//! Native 3xOSC — three detunable oscillators sharing one ADSR.
//! Mirrors FL's 3xOSC stock instrument.

use crate::format;
use crate::voices::{self, Glide, HeldNotes, MonoStep, VoiceMode, VoiceSettings};
use hardwave_dsp::synth::{AdsrEnvelope, AdsrStage, Oscillator, Waveform};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_OSC1_WAVE: u32 = 0;
const PARAM_OSC1_DETUNE: u32 = 1;
const PARAM_OSC1_LEVEL: u32 = 2;
const PARAM_OSC2_WAVE: u32 = 3;
const PARAM_OSC2_DETUNE: u32 = 4;
const PARAM_OSC2_LEVEL: u32 = 5;
const PARAM_OSC3_WAVE: u32 = 6;
const PARAM_OSC3_DETUNE: u32 = 7;
const PARAM_OSC3_LEVEL: u32 = 8;
const PARAM_ATTACK: u32 = 9;
const PARAM_DECAY: u32 = 10;
const PARAM_SUSTAIN: u32 = 11;
const PARAM_RELEASE: u32 = 12;
const PARAM_VOICE_MODE: u32 = 13;
const PARAM_VOICES: u32 = 14;
const PARAM_GLIDE: u32 = 15;
const PARAM_COUNT: u32 = 16;

fn wave_from_norm(v: f32) -> Waveform {
    let i = ((v * 5.0).floor() as i32).clamp(0, 4);
    match i {
        0 => Waveform::Sine,
        1 => Waveform::Saw,
        2 => Waveform::Square,
        3 => Waveform::Triangle,
        _ => Waveform::Noise,
    }
}

fn wave_to_norm(w: Waveform) -> f32 {
    match w {
        Waveform::Sine => 0.1,
        Waveform::Saw => 0.3,
        Waveform::Square => 0.5,
        Waveform::Triangle => 0.7,
        Waveform::Noise => 0.9,
    }
}

fn wave_index(w: Waveform) -> u8 {
    match w {
        Waveform::Sine => 0,
        Waveform::Saw => 1,
        Waveform::Square => 2,
        Waveform::Triangle => 3,
        Waveform::Noise => 4,
    }
}

/// Wave names by `wave_index`, which is also the order `wave_from_norm`
/// picks them in, so choice i sits at i / 4 on the knob.
const WAVE_NAMES: [&str; 5] = ["Sine", "Saw", "Square", "Triangle", "Noise"];

/// Detune in semitones (-24..+24) from the 0..1 knob. The setter and the
/// text both use it so what is shown is what is set.
fn detune_from_norm(v: f64) -> f32 {
    (v * 48.0 - 24.0) as f32
}

/// Envelope time in seconds (0..5) from the 0..1 knob.
fn env_secs_from_norm(v: f64) -> f32 {
    (v * 5.0) as f32
}

fn stage_priority(s: AdsrStage) -> u8 {
    match s {
        AdsrStage::Idle => 0,
        AdsrStage::Release => 1,
        AdsrStage::Sustain => 2,
        AdsrStage::Decay => 3,
        AdsrStage::Attack => 4,
    }
}

struct Voice {
    oscs: [Oscillator; 3],
    env: AdsrEnvelope,
    note: u8,
    velocity: f32,
    active: bool,
    /// The voice's pitch, which slides in Mono and Legato.
    glide: Glide,
    /// Each oscillator's detune, taken when the note started.
    detune: [f32; 3],
}

impl Voice {
    fn new(sr: f32) -> Self {
        Self {
            oscs: [
                Oscillator::new(sr),
                Oscillator::new(sr),
                Oscillator::new(sr),
            ],
            env: AdsrEnvelope::new(sr),
            note: 0,
            velocity: 0.0,
            active: false,
            glide: Glide::default(),
            detune: [0.0; 3],
        }
    }

    fn set_pitch(&mut self, note: f32) {
        for i in 0..3 {
            self.oscs[i].set_pitch_midi(note - 69.0 + self.detune[i], 0.0);
        }
    }
}

pub struct NativeTripleOsc {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    waves: [Waveform; 3],
    detune: [f32; 3],
    levels: [f32; 3],
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    voices: Vec<Voice>,
    active: bool,
    voice_settings: VoiceSettings,
    held: HeldNotes,
}

impl NativeTripleOsc {
    pub const ID: &'static str = "hardwave.native.tripleosc";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave 3xOSC".into(),
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
        let voices = (0..voices::MAX_VOICES).map(|_| Voice::new(sr)).collect();
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            waves: [Waveform::Saw, Waveform::Square, Waveform::Sine],
            detune: [0.0, -0.05, 0.05],
            levels: [0.6, 0.5, 0.4],
            attack: 0.005,
            decay: 0.1,
            sustain: 0.7,
            release: 0.15,
            voices,
            active: false,
            voice_settings: VoiceSettings::default(),
            held: HeldNotes::default(),
        }
    }

    fn alloc_voice(&mut self) -> usize {
        voices::pick_voice(
            &self.voices,
            self.voice_settings.voices,
            |v| !v.active,
            |v| stage_priority(v.env.stage()),
        )
    }

    /// Start a note on voice `idx`: the current sound, the envelope from the
    /// top. `slide` glides the pitch there from where the voice is.
    fn start_note(&mut self, idx: usize, note: u8, velocity: f32, slide: bool) {
        let (waves, detune, levels) = (self.waves, self.detune, self.levels);
        let (attack, decay, sustain, release) =
            (self.attack, self.decay, self.sustain, self.release);
        let glide_s = self.voice_settings.glide_s;
        let sr = self.sample_rate;
        let voice = &mut self.voices[idx];
        let sounding = voice.active;
        voice.note = note;
        voice.velocity = velocity;
        voice.detune = detune;
        for i in 0..3 {
            voice.oscs[i].set_waveform(waves[i]);
            voice.oscs[i].set_level(levels[i]);
        }
        if slide && sounding {
            voice.glide.slide(note as f32, glide_s, sr);
        } else {
            voice.glide.jump(note as f32);
            for osc in voice.oscs.iter_mut() {
                osc.reset_phase();
            }
        }
        voice.set_pitch(voice.glide.pitch());
        voice.env.set_times(attack, decay, release);
        voice.env.set_sustain(sustain);
        voice.env.note_on();
        voice.active = true;
    }

    /// Mono and Legato play on voice 0.
    fn mono(&mut self, step: MonoStep, velocity: f32) {
        let glide_s = self.voice_settings.glide_s;
        let sr = self.sample_rate;
        match step {
            MonoStep::Start(note) => self.start_note(0, note, velocity, true),
            MonoStep::Glide(note) => {
                let v = &mut self.voices[0];
                v.note = note;
                v.glide.slide(note as f32, glide_s, sr);
            }
            MonoStep::Release => self.voices[0].env.note_off(),
            MonoStep::Nothing => {}
        }
    }
}

impl Default for NativeTripleOsc {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeTripleOsc {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.voices = (0..voices::MAX_VOICES)
            .map(|_| Voice::new(self.sample_rate))
            .collect();
        self.held.clear();
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
                        // Voice 0 follows the held keys; any other voice
                        // still sounding from before the mode changed is
                        // let go by its own key.
                        1
                    };
                    for v in self.voices.iter_mut().skip(first) {
                        if v.active && v.note == *note {
                            v.env.note_off();
                        }
                    }
                }
                _ => {}
            }
        }

        for v in self.voices.iter_mut() {
            if !v.active {
                continue;
            }
            for i in 0..num_samples {
                let env = v.env.tick();
                if env <= 0.0 && v.env.stage() == AdsrStage::Idle {
                    v.active = false;
                    break;
                }
                if v.glide.gliding() {
                    let pitch = v.glide.advance();
                    v.set_pitch(pitch);
                }
                let s = v.oscs[0].tick() + v.oscs[1].tick() + v.oscs[2].tick();
                let sample = s * env * v.velocity * 0.33;
                outputs[0][i] += sample;
                outputs[1][i] += sample;
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let name = match index {
            PARAM_OSC1_WAVE => "Osc1 Wave",
            PARAM_OSC1_DETUNE => "Osc1 Detune",
            PARAM_OSC1_LEVEL => "Osc1 Level",
            PARAM_OSC2_WAVE => "Osc2 Wave",
            PARAM_OSC2_DETUNE => "Osc2 Detune",
            PARAM_OSC2_LEVEL => "Osc2 Level",
            PARAM_OSC3_WAVE => "Osc3 Wave",
            PARAM_OSC3_DETUNE => "Osc3 Detune",
            PARAM_OSC3_LEVEL => "Osc3 Level",
            PARAM_ATTACK => "Attack",
            PARAM_DECAY => "Decay",
            PARAM_SUSTAIN => "Sustain",
            PARAM_RELEASE => "Release",
            PARAM_VOICE_MODE => "Voice mode",
            PARAM_VOICES => "Voices",
            PARAM_GLIDE => "Glide",
            _ => return None,
        };
        Some(ParameterInfo {
            id: index,
            name: name.into(),
            default_value: self.get_parameter_value(index),
            min: 0.0,
            max: 1.0,
            unit: "".into(),
            automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_OSC1_WAVE => wave_to_norm(self.waves[0]) as f64,
            PARAM_OSC2_WAVE => wave_to_norm(self.waves[1]) as f64,
            PARAM_OSC3_WAVE => wave_to_norm(self.waves[2]) as f64,
            PARAM_OSC1_DETUNE => ((self.detune[0] + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_OSC2_DETUNE => ((self.detune[1] + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_OSC3_DETUNE => ((self.detune[2] + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            PARAM_OSC1_LEVEL => self.levels[0] as f64,
            PARAM_OSC2_LEVEL => self.levels[1] as f64,
            PARAM_OSC3_LEVEL => self.levels[2] as f64,
            PARAM_ATTACK => (self.attack / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_DECAY => (self.decay / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_SUSTAIN => self.sustain as f64,
            PARAM_RELEASE => (self.release / 5.0).clamp(0.0, 1.0) as f64,
            PARAM_VOICE_MODE => VoiceSettings::mode_to_norm(self.voice_settings.mode),
            PARAM_VOICES => VoiceSettings::voices_to_norm(self.voice_settings.voices),
            PARAM_GLIDE => VoiceSettings::glide_to_norm(self.voice_settings.glide_s),
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_OSC1_WAVE => self.waves[0] = wave_from_norm(v as f32),
            PARAM_OSC2_WAVE => self.waves[1] = wave_from_norm(v as f32),
            PARAM_OSC3_WAVE => self.waves[2] = wave_from_norm(v as f32),
            PARAM_OSC1_DETUNE => self.detune[0] = detune_from_norm(v),
            PARAM_OSC2_DETUNE => self.detune[1] = detune_from_norm(v),
            PARAM_OSC3_DETUNE => self.detune[2] = detune_from_norm(v),
            PARAM_OSC1_LEVEL => self.levels[0] = v as f32,
            PARAM_OSC2_LEVEL => self.levels[1] = v as f32,
            PARAM_OSC3_LEVEL => self.levels[2] = v as f32,
            PARAM_ATTACK => self.attack = env_secs_from_norm(v),
            PARAM_DECAY => self.decay = env_secs_from_norm(v),
            PARAM_SUSTAIN => self.sustain = v as f32,
            PARAM_RELEASE => self.release = env_secs_from_norm(v),
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
            PARAM_OSC1_WAVE | PARAM_OSC2_WAVE | PARAM_OSC3_WAVE => {
                WAVE_NAMES[wave_index(wave_from_norm(v as f32)) as usize].to_string()
            }
            PARAM_OSC1_DETUNE | PARAM_OSC2_DETUNE | PARAM_OSC3_DETUNE => {
                format::num(detune_from_norm(v) as f64, 2, "st")
            }
            PARAM_OSC1_LEVEL | PARAM_OSC2_LEVEL | PARAM_OSC3_LEVEL | PARAM_SUSTAIN => {
                format::pct(v)
            }
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
            PARAM_OSC1_WAVE | PARAM_OSC2_WAVE | PARAM_OSC3_WAVE => {
                Some(WAVE_NAMES.iter().map(|s| s.to_string()).collect())
            }
            PARAM_VOICE_MODE => Some(voices::MODE_NAMES.iter().map(|s| s.to_string()).collect()),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"w\":[{},{},{}],\"d\":[{},{},{}],\"l\":[{},{},{}],\"adsr\":[{},{},{},{}],{}}}",
            wave_index(self.waves[0]),
            wave_index(self.waves[1]),
            wave_index(self.waves[2]),
            self.detune[0],
            self.detune[1],
            self.detune[2],
            self.levels[0],
            self.levels[1],
            self.levels[2],
            self.attack,
            self.decay,
            self.sustain,
            self.release,
            self.voice_settings.state_fields()
        )
        .into_bytes()
    }

    /// Read back what `get_state` wrote. This did nothing, so a song
    /// reopened with 3xOSC on it had the synth's default sound.
    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let array = |key: &str| -> Vec<f32> {
            let needle = format!("\"{key}\":[");
            let Some(i) = s.find(&needle) else {
                return Vec::new();
            };
            let rest = &s[i + needle.len()..];
            let end = rest.find(']').unwrap_or(rest.len());
            rest[..end]
                .split(',')
                .filter_map(|x| x.trim().parse::<f32>().ok())
                .collect()
        };
        let waves = array("w");
        for (i, w) in waves.iter().take(3).enumerate() {
            let index = (*w as usize).min(WAVE_NAMES.len() - 1);
            self.waves[i] = wave_from_norm(index as f32 / 5.0 + 0.1);
        }
        for (i, d) in array("d").iter().take(3).enumerate() {
            self.detune[i] = d.clamp(-24.0, 24.0);
        }
        for (i, l) in array("l").iter().take(3).enumerate() {
            self.levels[i] = l.clamp(0.0, 1.0);
        }
        if let [a, d, su, r] = array("adsr")[..] {
            self.attack = a.clamp(0.0, 5.0);
            self.decay = d.clamp(0.0, 5.0);
            self.sustain = su.clamp(0.0, 1.0);
            self.release = r.clamp(0.0, 5.0);
        }
        self.voice_settings.read_state(|key| {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        });
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
        let mut p = NativeTripleOsc::new();
        let shown = |p: &NativeTripleOsc, id: u32| p.parameter_text(id, p.get_parameter_value(id));
        assert_eq!(shown(&p, PARAM_OSC1_WAVE).as_deref(), Some("Saw"));
        assert_eq!(shown(&p, PARAM_OSC2_DETUNE).as_deref(), Some("-0.05 st"));
        assert_eq!(shown(&p, PARAM_OSC1_LEVEL).as_deref(), Some("60 %"));
        assert_eq!(shown(&p, PARAM_ATTACK).as_deref(), Some("5.00 ms"));

        let options = p.parameter_options(PARAM_OSC1_WAVE).unwrap();
        let last = (options.len() - 1) as f64;
        for (i, label) in options.iter().enumerate() {
            let v = i as f64 / last;
            assert_eq!(p.parameter_text(PARAM_OSC1_WAVE, v).as_ref(), Some(label));
            p.set_parameter_value(PARAM_OSC1_WAVE, v);
            assert_eq!(shown(&p, PARAM_OSC1_WAVE).as_ref(), Some(label));
        }
        for id in 0..PARAM_COUNT {
            assert!(
                p.parameter_text(id, 0.5).is_some(),
                "parameter {id} has no text"
            );
        }
    }

    fn play(p: &mut NativeTripleOsc, events: &[MidiEvent], samples: usize) -> Vec<f32> {
        let mut out = vec![Vec::new(), Vec::new()];
        p.process(&[], &mut out, events, &mut Vec::new(), samples);
        out.remove(0)
    }

    fn on(note: u8) -> MidiEvent {
        MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note,
            velocity: 0.8,
        }
    }

    fn sounding(p: &NativeTripleOsc) -> usize {
        p.voices.iter().filter(|v| v.active).count()
    }

    #[test]
    fn poly_plays_as_many_notes_as_it_has_voices() {
        let mut p = NativeTripleOsc::new();
        p.activate(48_000.0, 512).unwrap();
        p.set_parameter_value(PARAM_VOICES, VoiceSettings::voices_to_norm(2));
        play(&mut p, &[on(60), on(64), on(67)], 64);
        assert_eq!(sounding(&p), 2, "a third note steals within two voices");
    }

    #[test]
    fn mono_plays_one_note_and_legato_slides_without_starting_again() {
        let mut p = NativeTripleOsc::new();
        p.activate(48_000.0, 512).unwrap();
        p.set_parameter_value(
            PARAM_VOICE_MODE,
            VoiceSettings::mode_to_norm(VoiceMode::Mono),
        );
        play(&mut p, &[on(60), on(64), on(67)], 64);
        assert_eq!(sounding(&p), 1);
        assert_eq!(p.voices[0].note, 67);

        p.set_parameter_value(
            PARAM_VOICE_MODE,
            VoiceSettings::mode_to_norm(VoiceMode::Legato),
        );
        p.set_parameter_value(PARAM_GLIDE, VoiceSettings::glide_to_norm(0.1));
        play(&mut p, &[on(48)], 4_800);
        let stage = p.voices[0].env.stage();
        play(&mut p, &[on(55)], 64);
        assert!(p.voices[0].glide.gliding(), "slides to the new note");
        assert_eq!(
            p.voices[0].env.stage(),
            stage,
            "the envelope is not started again"
        );
    }

    #[test]
    fn a_saved_sound_comes_back() {
        let mut p = NativeTripleOsc::new();
        p.set_parameter_value(PARAM_OSC2_WAVE, 0.0);
        p.set_parameter_value(PARAM_OSC1_LEVEL, 0.25);
        p.set_parameter_value(PARAM_RELEASE, 0.4);
        p.set_parameter_value(PARAM_VOICE_MODE, 1.0);
        let state = p.get_state();
        let mut back = NativeTripleOsc::new();
        back.set_state(&state).unwrap();
        for id in 0..PARAM_COUNT {
            assert!(
                (back.get_parameter_value(id) - p.get_parameter_value(id)).abs() < 1e-4,
                "parameter {id} came back different"
            );
        }
    }
}
