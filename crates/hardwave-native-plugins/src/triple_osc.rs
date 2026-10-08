//! Native 3xOSC — three detunable oscillators sharing one ADSR.
//! Mirrors FL's 3xOSC stock instrument.

use crate::format;
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
const PARAM_COUNT: u32 = 13;

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
}

impl NativeTripleOsc {
    pub const ID: &'static str = "hardwave.native.tripleosc";
    const POLYPHONY: usize = 8;

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
        let voices = (0..Self::POLYPHONY).map(|_| Voice::new(sr)).collect();
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
        }
    }

    fn alloc_voice(&mut self) -> usize {
        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            return idx;
        }
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| stage_priority(v.env.stage()))
            .map(|(i, _)| i)
            .unwrap_or(0)
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
        self.voices = (0..Self::POLYPHONY)
            .map(|_| Voice::new(self.sample_rate))
            .collect();
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
                MidiEvent::NoteOn { note, velocity, .. } => {
                    let waves = self.waves;
                    let detune = self.detune;
                    let levels = self.levels;
                    let attack = self.attack;
                    let decay = self.decay;
                    let sustain = self.sustain;
                    let release = self.release;
                    let idx = self.alloc_voice();
                    let voice = &mut self.voices[idx];
                    voice.note = *note;
                    voice.velocity = *velocity;
                    for i in 0..3 {
                        voice.oscs[i].set_waveform(waves[i]);
                        voice.oscs[i].set_level(levels[i]);
                        voice.oscs[i].set_pitch_midi(*note as f32 - 69.0 + detune[i], 0.0);
                        voice.oscs[i].reset_phase();
                    }
                    voice.env.set_times(attack, decay, release);
                    voice.env.set_sustain(sustain);
                    voice.env.note_on();
                    voice.active = true;
                }
                MidiEvent::NoteOff { note, .. } => {
                    for v in self.voices.iter_mut() {
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
            _ => return None,
        };
        Some(text)
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match id {
            PARAM_OSC1_WAVE | PARAM_OSC2_WAVE | PARAM_OSC3_WAVE => {
                Some(WAVE_NAMES.iter().map(|s| s.to_string()).collect())
            }
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"w\":[{},{},{}],\"d\":[{},{},{}],\"l\":[{},{},{}],\"adsr\":[{},{},{},{}]}}",
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
            self.release
        )
        .into_bytes()
    }

    fn set_state(&mut self, _state: &[u8]) -> Result<(), String> {
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
}
