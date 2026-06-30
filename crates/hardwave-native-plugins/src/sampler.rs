//! Native sampler — load an audio file and play it chromatically across
//! the keyboard, polyphonically. The track's MIDI notes drive it (it's an
//! `Instrument`-category plug-in, so the engine routes notes to it).
//!
//! Sample ingress reuses the existing plug-in-state path: the decoded
//! audio *is* the plug-in's state. The `load_sampler` command decodes a
//! file off the audio thread and ships the bytes via `SetState`, and the
//! same bytes round-trip through project save/load — so a project with a
//! sampler is self-contained (the sample travels with it).

use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const MAX_VOICES: usize = 16;
const PARAM_GAIN: u32 = 0;
const PARAM_ATTACK: u32 = 1;
const PARAM_RELEASE: u32 = 2;
const PARAM_COUNT: u32 = 3;

/// Decoded sample, addressable by MIDI note relative to `base_note`.
#[derive(Clone, Default)]
pub struct SampleData {
    pub sample_rate: u32,
    /// MIDI note at which the sample plays back at its natural pitch.
    pub base_note: u8,
    /// Per-channel PCM (1 = mono, 2 = stereo). Other counts are folded
    /// to stereo on load.
    pub channels: Vec<Vec<f32>>,
}

impl SampleData {
    fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }

    /// Encode to the plug-in-state byte format (little-endian):
    /// `base_note u8 | sample_rate u32 | num_channels u32 | num_frames u32
    /// | f32 PCM per channel`.
    fn to_bytes(&self) -> Vec<u8> {
        let nch = self.channels.len() as u32;
        let nfr = self.frames() as u32;
        let mut out = Vec::with_capacity(13 + (nch as usize * nfr as usize * 4));
        out.push(self.base_note);
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&nch.to_le_bytes());
        out.extend_from_slice(&nfr.to_le_bytes());
        for ch in &self.channels {
            for &s in ch {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
        out
    }

    fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 13 {
            return None;
        }
        let base_note = b[0];
        let sample_rate = u32::from_le_bytes(b[1..5].try_into().ok()?);
        let nch = u32::from_le_bytes(b[5..9].try_into().ok()?) as usize;
        let nfr = u32::from_le_bytes(b[9..13].try_into().ok()?) as usize;
        let need = 13 + nch * nfr * 4;
        if nch == 0 || b.len() < need {
            return None;
        }
        let mut channels = Vec::with_capacity(nch);
        let mut off = 13;
        for _ in 0..nch {
            let mut ch = Vec::with_capacity(nfr);
            for _ in 0..nfr {
                let s = f32::from_le_bytes(b[off..off + 4].try_into().ok()?);
                ch.push(s);
                off += 4;
            }
            channels.push(ch);
        }
        Some(SampleData {
            sample_rate,
            base_note,
            channels,
        })
    }
}

#[derive(Clone, Copy)]
enum EnvStage {
    Attack,
    Sustain,
    Release,
    Idle,
}

struct Voice {
    /// Fractional read position in source frames.
    pos: f64,
    /// Frames advanced per output sample (pitch ratio × SR conversion).
    step: f64,
    velocity: f32,
    note: u8,
    env: f32,
    stage: EnvStage,
}

pub struct NativeSampler {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    active: bool,
    sample: Option<SampleData>,
    voices: Vec<Voice>,
    gain: f32,
    attack_ms: f32,
    release_ms: f32,
}

fn pitch_ratio(note: u8, base: u8) -> f64 {
    2.0_f64.powf((note as f64 - base as f64) / 12.0)
}

impl NativeSampler {
    pub const ID: &'static str = "hardwave.native.sampler";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Sampler".into(),
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
        Self {
            descriptor: Self::descriptor(),
            sample_rate: 48_000.0,
            active: false,
            sample: None,
            voices: Vec::new(),
            gain: 1.0,
            attack_ms: 2.0,
            release_ms: 60.0,
        }
    }

    fn start_voice(&mut self, note: u8, velocity: f32) {
        let Some(sample) = &self.sample else { return };
        // Pitch ratio × (sample SR / engine SR) so playback speed is
        // correct regardless of the file's native rate.
        let sr_ratio = sample.sample_rate as f64 / self.sample_rate.max(1.0) as f64;
        let step = pitch_ratio(note, sample.base_note) * sr_ratio;
        if self.voices.len() >= MAX_VOICES {
            let steal = self
                .voices
                .iter()
                .position(|v| matches!(v.stage, EnvStage::Idle | EnvStage::Release))
                .unwrap_or(0);
            self.voices.remove(steal);
        }
        self.voices.push(Voice {
            pos: 0.0,
            step,
            velocity: velocity.clamp(0.0, 1.0),
            note,
            env: 0.0,
            stage: EnvStage::Attack,
        });
    }

    /// Linearly-interpolated sample for channel `ch` at fractional `pos`.
    fn read(sample: &SampleData, ch: usize, pos: f64) -> f32 {
        let data = match sample.channels.get(ch) {
            Some(d) => d,
            None => return 0.0,
        };
        let i = pos.floor() as usize;
        if i + 1 >= data.len() {
            return data.get(i).copied().unwrap_or(0.0);
        }
        let frac = (pos - i as f64) as f32;
        data[i] * (1.0 - frac) + data[i + 1] * frac
    }
}

impl Default for NativeSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeSampler {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = (sr as f32).max(1.0);
        Ok(())
    }
    fn deactivate(&mut self) {
        self.active = false;
        self.voices.clear();
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
        self.active = true;
        for ev in midi_in {
            match *ev {
                MidiEvent::NoteOn { note, velocity, .. } if velocity > 0.0 => {
                    self.start_voice(note, velocity)
                }
                MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note, .. } => {
                    for v in self.voices.iter_mut() {
                        if v.note == note && !matches!(v.stage, EnvStage::Release | EnvStage::Idle)
                        {
                            v.stage = EnvStage::Release;
                        }
                    }
                }
                _ => {}
            }
        }
        let Some(sample) = self.sample.clone() else {
            return;
        };
        if outputs.len() < 2 {
            return;
        }
        let frames = sample.frames();
        if frames == 0 {
            return;
        }
        let two_ch = sample.channels.len() >= 2;
        let atk = (self.attack_ms * 0.001 * self.sample_rate).max(1.0);
        let rel = (self.release_ms * 0.001 * self.sample_rate).max(1.0);
        let atk_inc = 1.0 / atk;
        let rel_dec = 1.0 / rel;

        for i in 0..num_samples {
            let mut l = 0.0_f32;
            let mut r = 0.0_f32;
            for v in self.voices.iter_mut() {
                if matches!(v.stage, EnvStage::Idle) {
                    continue;
                }
                // Envelope.
                match v.stage {
                    EnvStage::Attack => {
                        v.env += atk_inc;
                        if v.env >= 1.0 {
                            v.env = 1.0;
                            v.stage = EnvStage::Sustain;
                        }
                    }
                    EnvStage::Release => {
                        v.env -= rel_dec;
                        if v.env <= 0.0 {
                            v.env = 0.0;
                            v.stage = EnvStage::Idle;
                        }
                    }
                    _ => {}
                }
                if v.pos as usize >= frames {
                    v.stage = EnvStage::Idle;
                    continue;
                }
                let g = v.env * v.velocity * self.gain;
                let sl = Self::read(&sample, 0, v.pos);
                let sr_s = if two_ch {
                    Self::read(&sample, 1, v.pos)
                } else {
                    sl
                };
                l += sl * g;
                r += sr_s * g;
                v.pos += v.step;
            }
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
        self.voices.retain(|v| !matches!(v.stage, EnvStage::Idle));
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_GAIN => ("Gain", 1.0, ""),
            PARAM_ATTACK => ("Attack", 2.0 / 200.0, "ms"),
            PARAM_RELEASE => ("Release", 60.0 / 1000.0, "ms"),
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
            PARAM_GAIN => self.gain as f64,
            PARAM_ATTACK => (self.attack_ms / 200.0).clamp(0.0, 1.0) as f64,
            PARAM_RELEASE => (self.release_ms / 1000.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0) as f32;
        match id {
            PARAM_GAIN => self.gain = v,
            PARAM_ATTACK => self.attack_ms = (v * 200.0).max(0.1),
            PARAM_RELEASE => self.release_ms = (v * 1000.0).max(1.0),
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        // The sample itself is the state, so a saved project carries it.
        self.sample
            .as_ref()
            .map(|s| s.to_bytes())
            .unwrap_or_default()
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        match SampleData::from_bytes(bytes) {
            Some(s) => {
                self.sample = Some(s);
                self.voices.clear();
                Ok(())
            }
            None => Err("sampler: malformed sample state".into()),
        }
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

/// Public helper for the `load_sampler` command: build the state bytes a
/// `NativeSampler` expects from decoded channels. Other channel counts
/// are folded to mono so the sampler always has at least one channel.
pub fn encode_sample_state(sample_rate: u32, base_note: u8, channels: Vec<Vec<f32>>) -> Vec<u8> {
    let channels = if channels.is_empty() {
        vec![Vec::new()]
    } else {
        channels
    };
    SampleData {
        sample_rate,
        base_note,
        channels,
    }
    .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dc_sample() -> Vec<u8> {
        // 100 frames of DC = 0.5, stereo, base note 60.
        let ch = vec![0.5_f32; 100];
        encode_sample_state(48_000, 60, vec![ch.clone(), ch])
    }

    #[test]
    fn state_round_trips() {
        let bytes = dc_sample();
        let s = SampleData::from_bytes(&bytes).unwrap();
        assert_eq!(s.sample_rate, 48_000);
        assert_eq!(s.base_note, 60);
        assert_eq!(s.channels.len(), 2);
        assert_eq!(s.frames(), 100);
        assert!((s.channels[0][0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn plays_loaded_sample_on_note() {
        let mut s = NativeSampler::new();
        s.activate(48_000.0, 512).unwrap();
        s.set_state(&dc_sample()).unwrap();
        let mut out = vec![vec![0.0; 64], vec![0.0; 64]];
        let notes = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60, // base note → natural pitch
            velocity: 1.0,
        }];
        s.process(&[], &mut out, &notes, &mut Vec::new(), 64);
        let peak = out[0].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(
            peak > 0.0,
            "loaded sample must sound on note-on, got {peak}"
        );
    }

    #[test]
    fn silent_without_sample() {
        let mut s = NativeSampler::new();
        s.activate(48_000.0, 512).unwrap();
        let mut out = vec![vec![0.0; 32], vec![0.0; 32]];
        let notes = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 1.0,
        }];
        s.process(&[], &mut out, &notes, &mut Vec::new(), 32);
        assert!(out[0].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn higher_note_plays_faster() {
        // An octave up should advance the read position twice as fast.
        let mut s = NativeSampler::new();
        s.activate(48_000.0, 512).unwrap();
        s.set_state(&dc_sample()).unwrap();
        s.start_voice(72, 1.0); // +12 semitones
        let step = s.voices[0].step;
        assert!((step - 2.0).abs() < 1e-6, "octave up → 2x step, got {step}");
    }
}
