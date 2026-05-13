//! Native wavetable synth — Harmor / Serum / Vital basic-feature clone.
//! Uses `hardwave_dsp::wavetable::WavetableOscillator` plus a built-in
//! Wavetable bank (Basic / Analog / Digital / Vocal / Noise). Voice
//! count: 8, monotimbral.

use hardwave_dsp::synth::AdsrEnvelope;
use hardwave_dsp::wavetable::{Wavetable, WavetableOscillator};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const POLYPHONY: usize = 8;

const PARAM_BANK: u32 = 0;
const PARAM_POSITION: u32 = 1;
const PARAM_UNISON_DETUNE: u32 = 2;
const PARAM_ATTACK: u32 = 3;
const PARAM_DECAY: u32 = 4;
const PARAM_SUSTAIN: u32 = 5;
const PARAM_RELEASE: u32 = 6;
const PARAM_MASTER: u32 = 7;
const PARAM_COUNT: u32 = 8;

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

fn bank_to_norm(b: Bank) -> f64 {
    let idx = match b {
        Bank::Basic => 0,
        Bank::Analog => 1,
        Bank::Digital => 2,
        Bank::Vocal => 3,
        Bank::Noise => 4,
    };
    (idx as f64 + 0.5) / 5.0
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
}

impl VoiceSlot {
    fn new(sr: f32) -> Self {
        Self {
            osc: WavetableOscillator::new(sr),
            env: AdsrEnvelope::new(sr),
            note: 0,
            velocity: 0.0,
            active: false,
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
        let voices = (0..POLYPHONY).map(|_| VoiceSlot::new(sr)).collect();
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
        }
    }

    fn note_to_freq(midi_note: u8, fine_semis: f32) -> f32 {
        let semis = midi_note as f32 - 69.0 + fine_semis;
        440.0 * (2.0_f32).powf(semis / 12.0)
    }

    fn alloc_voice(&mut self) -> usize {
        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            return idx;
        }
        0
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
        self.voices = (0..POLYPHONY)
            .map(|_| VoiceSlot::new(self.sample_rate))
            .collect();
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
                    let attack = self.attack;
                    let decay = self.decay;
                    let sustain = self.sustain;
                    let release = self.release;
                    let position = self.position;
                    let detune = self.unison_detune;
                    let idx = self.alloc_voice();
                    let slot = &mut self.voices[idx];
                    slot.note = *note;
                    slot.velocity = *velocity;
                    slot.osc.set_frequency(Self::note_to_freq(*note, detune));
                    slot.osc.set_level(1.0);
                    slot.osc.set_position(position);
                    slot.osc.reset_phase();
                    slot.env.set_times(attack, decay, release);
                    slot.env.set_sustain(sustain);
                    slot.env.note_on();
                    slot.active = true;
                }
                MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note, .. } => {
                    for slot in self.voices.iter_mut() {
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
            PARAM_UNISON_DETUNE => self.unison_detune = (v - 0.5) as f32,
            PARAM_ATTACK => self.attack = (v * 5.0).max(0.001) as f32,
            PARAM_DECAY => self.decay = (v * 5.0).max(0.001) as f32,
            PARAM_SUSTAIN => self.sustain = v as f32,
            PARAM_RELEASE => self.release = (v * 5.0).max(0.001) as f32,
            PARAM_MASTER => self.master_gain = v as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"bank\":{},\"pos\":{},\"det\":{},\"a\":{},\"d\":{},\"s\":{},\"r\":{},\"m\":{}}}",
            bank_to_norm(self.bank),
            self.position,
            self.unison_detune,
            self.attack,
            self.decay,
            self.sustain,
            self.release,
            self.master_gain
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
