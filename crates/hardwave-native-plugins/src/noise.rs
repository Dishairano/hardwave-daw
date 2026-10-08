//! Native noise generator — white / pink / brown noise instrument.
//! Triggered by MIDI note-on (any note); note-off stops. Useful for
//! kick layering, FX risers, hi-hat synthesis, sound design.

use crate::format;
use hardwave_dsp::synth::AdsrEnvelope;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_COLOUR: u32 = 0;
const PARAM_LEVEL: u32 = 1;
const PARAM_ATTACK: u32 = 2;
const PARAM_DECAY: u32 = 3;
const PARAM_SUSTAIN: u32 = 4;
const PARAM_RELEASE: u32 = 5;
const PARAM_COUNT: u32 = 6;

#[derive(Clone, Copy)]
enum Colour {
    White,
    Pink,
    Brown,
}

fn colour_from_norm(v: f32) -> Colour {
    let i = ((v.clamp(0.0, 1.0) * 3.0).floor() as i32).clamp(0, 2);
    match i {
        0 => Colour::White,
        1 => Colour::Pink,
        _ => Colour::Brown,
    }
}

fn colour_index(c: Colour) -> usize {
    match c {
        Colour::White => 0,
        Colour::Pink => 1,
        Colour::Brown => 2,
    }
}

fn colour_to_norm(c: Colour) -> f64 {
    (colour_index(c) as f64 + 0.5) / 3.0
}

/// Colour names by `colour_index`, which is also the order
/// `colour_from_norm` picks them in, so choice i sits at i / 2.
const COLOUR_NAMES: [&str; 3] = ["White", "Pink", "Brown"];

/// Envelope time in seconds (1 ms..5 s) from the 0..1 knob. The setter
/// and the text both use it so what is shown is what is set.
fn env_secs_from_norm(v: f64) -> f32 {
    (v * 5.0).max(0.001) as f32
}

/// Voss-McCartney pink noise generator with 16 octaves of resolution.
struct PinkState {
    rows: [f32; 16],
    counter: u32,
}

impl PinkState {
    fn new() -> Self {
        Self {
            rows: [0.0; 16],
            counter: 0,
        }
    }
    fn tick(&mut self, white: f32) -> f32 {
        self.counter = self.counter.wrapping_add(1);
        let trailing_zeros = self.counter.trailing_zeros().min(15) as usize;
        self.rows[trailing_zeros] = white;
        let sum: f32 = self.rows.iter().sum();
        sum * (1.0 / 16.0)
    }
}

pub struct NativeNoise {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    colour: Colour,
    level: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    env: AdsrEnvelope,
    pink: PinkState,
    brown_state: f32,
    rng_state: u32,
    note_count: u8,
    active: bool,
}

impl NativeNoise {
    pub const ID: &'static str = "hardwave.native.noise";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Noise".into(),
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
        let mut env = AdsrEnvelope::new(sr);
        env.set_times(0.005, 0.1, 0.2);
        env.set_sustain(0.7);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            colour: Colour::White,
            level: 0.6,
            attack: 0.005,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
            env,
            pink: PinkState::new(),
            brown_state: 0.0,
            rng_state: 0xDEADBEEF,
            note_count: 0,
            active: false,
        }
    }

    fn next_white(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        // Map to [-1, 1]
        (x as i32 as f32) / (i32::MAX as f32)
    }
}

impl Default for NativeNoise {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeNoise {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.env = AdsrEnvelope::new(self.sample_rate);
        self.env.set_times(self.attack, self.decay, self.release);
        self.env.set_sustain(self.sustain);
        self.pink = PinkState::new();
        self.brown_state = 0.0;
        self.note_count = 0;
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
                MidiEvent::NoteOn { velocity, .. } if *velocity > 0.0 => {
                    self.note_count = self.note_count.saturating_add(1);
                    self.env.set_times(self.attack, self.decay, self.release);
                    self.env.set_sustain(self.sustain);
                    self.env.note_on();
                }
                MidiEvent::NoteOn { .. } | MidiEvent::NoteOff { .. } => {
                    self.note_count = self.note_count.saturating_sub(1);
                    if self.note_count == 0 {
                        self.env.note_off();
                    }
                }
                _ => {}
            }
        }

        if !self.env.is_active() {
            return;
        }

        for i in 0..num_samples {
            let env_v = self.env.tick();
            let white = self.next_white();
            let sample = match self.colour {
                Colour::White => white,
                Colour::Pink => self.pink.tick(white),
                Colour::Brown => {
                    self.brown_state = (self.brown_state + 0.02 * white).clamp(-1.0, 1.0);
                    self.brown_state * 3.5 // amplitude compensation
                }
            };
            let out = sample * env_v * self.level;
            outputs[0][i] = out;
            outputs[1][i] = out;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_COLOUR => ("Colour", colour_to_norm(Colour::White), ""),
            PARAM_LEVEL => ("Level", 0.6, "%"),
            PARAM_ATTACK => ("Attack", 0.005 / 5.0, "s"),
            PARAM_DECAY => ("Decay", 0.1 / 5.0, "s"),
            PARAM_SUSTAIN => ("Sustain", 0.7, ""),
            PARAM_RELEASE => ("Release", 0.2 / 5.0, "s"),
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
            PARAM_COLOUR => colour_to_norm(self.colour),
            PARAM_LEVEL => self.level as f64,
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
            PARAM_COLOUR => self.colour = colour_from_norm(v as f32),
            PARAM_LEVEL => self.level = v as f32,
            PARAM_ATTACK => {
                self.attack = env_secs_from_norm(v);
                self.env.set_times(self.attack, self.decay, self.release);
            }
            PARAM_DECAY => {
                self.decay = env_secs_from_norm(v);
                self.env.set_times(self.attack, self.decay, self.release);
            }
            PARAM_SUSTAIN => {
                self.sustain = v as f32;
                self.env.set_sustain(self.sustain);
            }
            PARAM_RELEASE => {
                self.release = env_secs_from_norm(v);
                self.env.set_times(self.attack, self.decay, self.release);
            }
            _ => {}
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        // Same clamp as the setter, so the text names what the value sets.
        let v = value.clamp(0.0, 1.0);
        let text = match id {
            PARAM_COLOUR => COLOUR_NAMES[colour_index(colour_from_norm(v as f32))].to_string(),
            PARAM_LEVEL | PARAM_SUSTAIN => format::pct(v),
            PARAM_ATTACK | PARAM_DECAY | PARAM_RELEASE => {
                format::secs(env_secs_from_norm(v) as f64)
            }
            _ => return None,
        };
        Some(text)
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match id {
            PARAM_COLOUR => Some(COLOUR_NAMES.iter().map(|s| s.to_string()).collect()),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"col\":{},\"lvl\":{},\"a\":{},\"d\":{},\"s\":{},\"r\":{}}}",
            colour_to_norm(self.colour),
            self.level,
            self.attack,
            self.decay,
            self.sustain,
            self.release
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
        if let Some(v) = read("col") {
            self.colour = colour_from_norm(v);
        }
        if let Some(v) = read("lvl") {
            self.level = v.clamp(0.0, 1.0);
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
        self.env.set_times(self.attack, self.decay, self.release);
        self.env.set_sustain(self.sustain);
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
        let mut p = NativeNoise::new();
        let shown = |p: &NativeNoise, id: u32| p.parameter_text(id, p.get_parameter_value(id));
        assert_eq!(shown(&p, PARAM_COLOUR).as_deref(), Some("White"));
        assert_eq!(shown(&p, PARAM_LEVEL).as_deref(), Some("60 %"));
        assert_eq!(shown(&p, PARAM_RELEASE).as_deref(), Some("200 ms"));

        let options = p.parameter_options(PARAM_COLOUR).unwrap();
        let last = (options.len() - 1) as f64;
        for (i, label) in options.iter().enumerate() {
            let v = i as f64 / last;
            assert_eq!(p.parameter_text(PARAM_COLOUR, v).as_ref(), Some(label));
            p.set_parameter_value(PARAM_COLOUR, v);
            assert_eq!(shown(&p, PARAM_COLOUR).as_ref(), Some(label));
        }
        for id in 0..PARAM_COUNT {
            assert!(
                p.parameter_text(id, 0.5).is_some(),
                "parameter {id} has no text"
            );
        }
    }
}
