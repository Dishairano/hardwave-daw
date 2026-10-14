//! Native sampler — load an audio file and play it chromatically across
//! the keyboard, polyphonically. The track's MIDI notes drive it (it's an
//! `Instrument`-category plug-in, so the engine routes notes to it).
//!
//! It plays a sample the way FL's sampler channel does: from a start
//! offset, looping between two points (straight or ping-pong) while the
//! note is held, through a volume envelope with delay, attack, hold,
//! decay, sustain and release when that envelope is switched on, and
//! straight through to the end of the sample when it is off. A channel
//! can be mono, cut itself on every new note, and slide between notes
//! (porta). The root note says which key plays the sample as recorded;
//! the pitch knob shifts the whole channel.
//!
//! Sample ingress reuses the existing plug-in-state path: the decoded
//! audio *is* the plug-in's state. The `load_sampler` command decodes a
//! file off the audio thread and ships the bytes via `SetState`, and the
//! same bytes round-trip through project save/load — so a project with a
//! sampler is self-contained (the sample travels with it). The settings
//! travel in the same bytes, after the audio, so a saved song plays back
//! with the envelope and loop it was made with.

use crate::format;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use hardwave_project::clip::DeclickMode;
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const MAX_VOICES: usize = 16;

pub const PARAM_GAIN: u32 = 0;
pub const PARAM_ATTACK: u32 = 1;
pub const PARAM_RELEASE: u32 = 2;
pub const PARAM_DELAY: u32 = 3;
pub const PARAM_HOLD: u32 = 4;
pub const PARAM_DECAY: u32 = 5;
pub const PARAM_SUSTAIN: u32 = 6;
pub const PARAM_ENVELOPE: u32 = 7;
pub const PARAM_OFFSET: u32 = 8;
pub const PARAM_LOOP_START: u32 = 9;
pub const PARAM_LOOP_END: u32 = 10;
pub const PARAM_LOOP: u32 = 11;
pub const PARAM_PING_PONG: u32 = 12;
pub const PARAM_MONO: u32 = 13;
pub const PARAM_CUT_ITSELF: u32 = 14;
pub const PARAM_PORTA: u32 = 15;
pub const PARAM_ROOT: u32 = 16;
pub const PARAM_PITCH: u32 = 17;
/// Where the sample starts in its file and how much of the rest it is
/// (FL's SMP START and length), 0..1 of the file.
pub const PARAM_SAMPLE_START: u32 = 18;
pub const PARAM_SAMPLE_LENGTH: u32 = 19;
/// The loop's crossfade, as a share of the loop.
pub const PARAM_XFADE: u32 = 20;
/// Cuts the sample's quiet tail: 0 is off, then the threshold rises from
/// -60 dB under the peak to the peak.
pub const PARAM_TRIM: u32 = 21;
pub const PARAM_FADE_STEREO: u32 = 22;
/// FL's declicking mode: the tiny fades at a note's start and at a cut.
pub const PARAM_DECLICK: u32 = 23;
pub const PARAM_COUNT: u32 = 24;

/// The time knobs run on a square law (fine near zero, as FL's do):
/// seconds = knob² × the longest they go.
const ATTACK_MAX_S: f32 = 5.0;
const DELAY_MAX_S: f32 = 5.0;
const HOLD_MAX_S: f32 = 5.0;
const DECAY_MAX_S: f32 = 10.0;
const RELEASE_MAX_S: f32 = 10.0;
/// A cut voice (mono, cut itself, a stolen voice) fades at least this long.
const CUT_FADE_S: f32 = 0.002;
/// The tail profile keeps the loudest sample of every block this long, so
/// the trim's cut is found without reading the audio again.
const PROFILE_STEP: usize = 64;
/// A note-off with the envelope off, on a looping sample, lets go over this long.
const LOOP_STOP_S: f32 = 0.005;
/// Porta slides from one note to the next over this long.
const PORTA_S: f32 = 0.12;
/// The pitch knob reaches this many semitones each way.
const PITCH_RANGE_ST: f32 = 24.0;

/// What the saved bytes carry after the audio: the settings, and the
/// file the sample came from, so the window can name it.
const TAIL_MAGIC: &[u8; 4] = b"HWSP";
const TAIL_VERSION: u8 = 1;

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
    pub fn frames(&self) -> usize {
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

    /// How many bytes the header and the audio take, when the header is
    /// sound and the bytes hold what it says.
    fn pcm_len(b: &[u8]) -> Option<usize> {
        if b.len() < 13 {
            return None;
        }
        let nch = u32::from_le_bytes(b[5..9].try_into().ok()?) as usize;
        let nfr = u32::from_le_bytes(b[9..13].try_into().ok()?) as usize;
        // The counts come from the saved state, which came from a file.
        // A sample has a handful of channels and what the bytes actually
        // hold; anything else is refused before memory is asked for.
        if nch == 0 || nch > 8 {
            return None;
        }
        let need = nch
            .checked_mul(nfr)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(13))?;
        (b.len() >= need).then_some(need)
    }

    /// The sample from the front of the bytes, and how many bytes it took.
    fn from_bytes(b: &[u8]) -> Option<(Self, usize)> {
        let need = Self::pcm_len(b)?;
        let base_note = b[0];
        let sample_rate = u32::from_le_bytes(b[1..5].try_into().ok()?);
        let nch = u32::from_le_bytes(b[5..9].try_into().ok()?) as usize;
        let nfr = u32::from_le_bytes(b[9..13].try_into().ok()?) as usize;
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
        Some((
            SampleData {
                sample_rate,
                base_note,
                channels,
            },
            need,
        ))
    }
}

/// Everything a sampler's state bytes hold: the sample, the settings
/// (parameter id and its 0..1 value) and the file the sample came from.
/// Bytes from before the settings travelled carry only the sample; they
/// still load, with the settings left as they are.
#[derive(Clone, Default)]
pub struct SamplerState {
    pub sample: SampleData,
    pub params: Vec<(u32, f32)>,
    pub path: String,
}

/// The settings block that follows the audio.
fn encode_tail(params: &[(u32, f32)], path: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + params.len() * 8 + 2 + path.len());
    out.extend_from_slice(TAIL_MAGIC);
    out.push(TAIL_VERSION);
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for &(id, v) in params {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }
    let path = path.as_bytes();
    let len = path.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&path[..len]);
    out
}

/// The settings and the file name behind the audio; nothing there means
/// bytes from before the settings travelled.
fn parse_tail(tail: &[u8]) -> Option<(Vec<(u32, f32)>, String)> {
    if tail.is_empty() {
        return Some((Vec::new(), String::new()));
    }
    // Anything after the sample is the settings block, whole, or the
    // bytes are not ours.
    if tail.len() < 4 + 1 + 4 || &tail[..4] != TAIL_MAGIC || tail[4] != TAIL_VERSION {
        return None;
    }
    let n = u32::from_le_bytes(tail[5..9].try_into().ok()?) as usize;
    if n > PARAM_COUNT as usize * 4 {
        return None;
    }
    let mut off = 9;
    let mut params = Vec::with_capacity(n);
    for _ in 0..n {
        if tail.len() < off + 8 {
            return None;
        }
        let id = u32::from_le_bytes(tail[off..off + 4].try_into().ok()?);
        let v = f32::from_le_bytes(tail[off + 4..off + 8].try_into().ok()?);
        params.push((id, v));
        off += 8;
    }
    if tail.len() < off + 2 {
        return None;
    }
    let len = u16::from_le_bytes(tail[off..off + 2].try_into().ok()?) as usize;
    off += 2;
    if tail.len() < off + len {
        return None;
    }
    let path = String::from_utf8_lossy(&tail[off..off + len]).into_owned();
    Some((params, path))
}

impl SamplerState {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.sample.to_bytes();
        out.extend(encode_tail(&self.params, &self.path));
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let (sample, used) = SampleData::from_bytes(b)?;
        let (params, path) = parse_tail(&b[used..])?;
        Some(SamplerState {
            sample,
            params,
            path,
        })
    }
}

/// The same state bytes with one setting changed, without decoding the
/// audio: the project's copy of a sampler follows its knobs this way, so
/// an undo brings a knob back and a save keeps it.
pub fn with_parameter(bytes: &[u8], id: u32, value: f32) -> Option<Vec<u8>> {
    if id >= PARAM_COUNT {
        return None;
    }
    let used = SampleData::pcm_len(bytes)?;
    let (mut params, path) = parse_tail(&bytes[used..])?;
    let value = value.clamp(0.0, 1.0);
    match params.iter_mut().find(|(i, _)| *i == id) {
        Some(p) => p.1 = value,
        None => params.push((id, value)),
    }
    let mut out = bytes[..used].to_vec();
    if id == PARAM_ROOT {
        // The header names the root too, for whoever reads only that.
        out[0] = (value * 127.0).round().clamp(0.0, 127.0) as u8;
    }
    out.extend(encode_tail(&params, &path));
    Some(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EnvStage {
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Idle,
}

struct Voice {
    /// Fractional read position in source frames.
    pos: f64,
    /// Frames advanced per output sample (pitch ratio × SR conversion).
    step: f64,
    /// Where a porta slide is heading, and how much `step` moves per
    /// sample to get there (0 when not sliding).
    target_step: f64,
    glide: f64,
    /// +1 forwards, -1 backwards (the way back of a ping-pong loop).
    dir: f64,
    velocity: f32,
    note: u8,
    env: f32,
    stage: EnvStage,
    /// Samples left in a timed stage (delay, hold).
    left: u64,
    /// How much the envelope drops per sample while releasing.
    rel_dec: f32,
    /// A voice that was cut fades out by this much per sample; 0 = not cut.
    cut: f32,
    cut_gain: f32,
    /// The declicking fade at the start: how much it rises per sample
    /// (0 = none) and where it is.
    attack: f32,
    attack_gain: f32,
}

impl Voice {
    fn held(&self) -> bool {
        self.cut == 0.0 && !matches!(self.stage, EnvStage::Release | EnvStage::Idle)
    }
}

pub struct NativeSampler {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    active: bool,
    sample: Option<SampleData>,
    /// The file the sample came from; what the window shows as its name.
    path: String,
    /// The loudest sample of every `PROFILE_STEP` frames, and the loudest
    /// of all: the trim finds its cut in these.
    profile: Vec<f32>,
    peak: f32,
    voices: Vec<Voice>,
    /// Every setting, 0..1, by parameter id.
    p: [f32; PARAM_COUNT as usize],
}

fn knob_seconds(v: f32, max_s: f32, floor_s: f32) -> f32 {
    (v * v * max_s).max(floor_s)
}

fn pitch_ratio(note: u8, base: u8) -> f64 {
    2.0_f64.powf((note as f64 - base as f64) / 12.0)
}

fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[(n % 12) as usize], (n / 12) as i32 - 1)
}

fn defaults() -> [f32; PARAM_COUNT as usize] {
    let mut p = [0.0; PARAM_COUNT as usize];
    p[PARAM_GAIN as usize] = 1.0;
    p[PARAM_ATTACK as usize] = (0.002_f32 / ATTACK_MAX_S).sqrt();
    p[PARAM_RELEASE as usize] = (0.06_f32 / RELEASE_MAX_S).sqrt();
    p[PARAM_DECAY as usize] = (0.5_f32 / DECAY_MAX_S).sqrt();
    p[PARAM_SUSTAIN as usize] = 1.0;
    p[PARAM_LOOP_END as usize] = 1.0;
    // FL's sampler channel: the envelope is off and every new note cuts
    // the one before, which is what a drum wants.
    p[PARAM_CUT_ITSELF as usize] = 1.0;
    p[PARAM_ROOT as usize] = 60.0 / 127.0;
    p[PARAM_PITCH as usize] = 0.5;
    p[PARAM_SAMPLE_LENGTH as usize] = 1.0;
    p
}

fn declick_of(v: f32) -> DeclickMode {
    let n = DeclickMode::ALL.len();
    DeclickMode::ALL[((v.clamp(0.0, 1.0) * (n - 1) as f32).round() as usize).min(n - 1)]
}

/// The loudest sample of every `PROFILE_STEP` frames over every channel,
/// and the loudest of all.
fn profile_of(sample: &SampleData) -> (Vec<f32>, f32) {
    let frames = sample.frames();
    let mut profile = vec![0.0f32; frames.div_ceil(PROFILE_STEP)];
    let mut peak = 0.0f32;
    for ch in &sample.channels {
        for (i, chunk) in ch.chunks(PROFILE_STEP).enumerate() {
            let m = chunk.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
            profile[i] = profile[i].max(m);
            peak = peak.max(m);
        }
    }
    (profile, peak)
}

impl NativeSampler {
    pub const ID: &'static str = "hardwave.native.sampler";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Sampler".into(),
            vendor: "Hardwave".into(),
            version: "1.1.0".into(),
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
            path: String::new(),
            profile: Vec::new(),
            peak: 0.0,
            voices: Vec::new(),
            p: defaults(),
        }
    }

    fn on(&self, id: u32) -> bool {
        self.p[id as usize] >= 0.5
    }
    fn root(&self) -> u8 {
        (self.p[PARAM_ROOT as usize] * 127.0)
            .round()
            .clamp(0.0, 127.0) as u8
    }
    fn pitch_semitones(&self) -> f32 {
        (self.p[PARAM_PITCH as usize] * 2.0 - 1.0) * PITCH_RANGE_ST
    }
    fn seconds(&self, id: u32) -> f32 {
        let v = self.p[id as usize];
        match id {
            PARAM_ATTACK => knob_seconds(v, ATTACK_MAX_S, 0.0001),
            PARAM_DELAY => knob_seconds(v, DELAY_MAX_S, 0.0),
            PARAM_HOLD => knob_seconds(v, HOLD_MAX_S, 0.0),
            PARAM_DECAY => knob_seconds(v, DECAY_MAX_S, 0.001),
            PARAM_RELEASE => knob_seconds(v, RELEASE_MAX_S, 0.001),
            _ => 0.0,
        }
    }
    fn samples(&self, seconds: f32) -> f64 {
        (seconds * self.sample_rate).max(1.0) as f64
    }

    /// The part of the file that is the sample, in frames: from the sample
    /// start, as long as the length says, and no further than where the
    /// trim cuts the tail.
    fn region(&self, frames: usize) -> (f64, f64) {
        let frames = frames as f64;
        let start = self.p[PARAM_SAMPLE_START as usize].clamp(0.0, 1.0) as f64 * frames;
        let mut end =
            start + self.p[PARAM_SAMPLE_LENGTH as usize].clamp(0.0, 1.0) as f64 * (frames - start);
        let trim = self.p[PARAM_TRIM as usize].clamp(0.0, 1.0);
        if trim > 0.0 && self.peak > 0.0 {
            let threshold = 10f32.powf(-60.0 * (1.0 - trim) / 20.0) * self.peak;
            let cut = self
                .profile
                .iter()
                .rposition(|m| *m > threshold)
                .map(|i| ((i + 1) * PROFILE_STEP) as f64)
                .unwrap_or(0.0);
            end = end.min(cut.min(frames));
        }
        (start, end.max(start))
    }

    fn declick(&self) -> DeclickMode {
        declick_of(self.p[PARAM_DECLICK as usize])
    }

    /// How much a cut voice drops per sample: the declicking mode's fade
    /// out, never shorter than the floor.
    fn cut_step(&self) -> f32 {
        let (_, out_ms) = self.declick().fades_ms();
        1.0 / ((out_ms as f32 / 1000.0).max(CUT_FADE_S) * self.sample_rate).max(1.0)
    }

    /// Frames per output sample for `note`: the distance from the root,
    /// the pitch knob, and the file's rate against the engine's.
    fn step_for(&self, note: u8, sample: &SampleData) -> f64 {
        let sr_ratio = sample.sample_rate as f64 / self.sample_rate.max(1.0) as f64;
        pitch_ratio(note, self.root())
            * 2.0_f64.powf(self.pitch_semitones() as f64 / 12.0)
            * sr_ratio
    }

    fn start_voice(&mut self, note: u8, velocity: f32) {
        let Some(sample) = &self.sample else { return };
        let step = self.step_for(note, sample);
        let (region_start, region_end) = self.region(sample.frames());
        let cut_step = self.cut_step();
        let (attack_ms, _) = self.declick().fades_ms();
        let attack = if attack_ms > 0.0 {
            1.0 / ((attack_ms as f32 / 1000.0) * self.sample_rate).max(1.0)
        } else {
            0.0
        };
        if self.on(PARAM_MONO) {
            if self.on(PARAM_PORTA) {
                // Legato: the held note slides to the new one and keeps its
                // envelope where it is.
                let glide_len = (PORTA_S * self.sample_rate).max(1.0) as f64;
                if let Some(v) = self.voices.iter_mut().find(|v| v.held()) {
                    v.target_step = step;
                    v.glide = (step - v.step) / glide_len;
                    v.note = note;
                    return;
                }
            }
            for v in self.voices.iter_mut() {
                if v.cut == 0.0 {
                    v.cut = cut_step;
                }
            }
        } else if self.on(PARAM_CUT_ITSELF) {
            for v in self.voices.iter_mut() {
                if v.cut == 0.0 {
                    v.cut = cut_step;
                }
            }
        }
        if self.voices.len() >= MAX_VOICES {
            let steal = self.voices.iter().position(|v| !v.held()).unwrap_or(0);
            self.voices.remove(steal);
        }
        let envelope = self.on(PARAM_ENVELOPE);
        let delay = self.seconds(PARAM_DELAY);
        let (stage, env, left) = if !envelope {
            (EnvStage::Sustain, 1.0, 0)
        } else if delay > 0.0 {
            (EnvStage::Delay, 0.0, self.samples(delay) as u64)
        } else {
            (EnvStage::Attack, 0.0, 0)
        };
        self.voices.push(Voice {
            pos: region_start
                + self.p[PARAM_OFFSET as usize].clamp(0.0, 1.0) as f64
                    * (region_end - region_start),
            step,
            target_step: step,
            glide: 0.0,
            dir: 1.0,
            velocity: velocity.clamp(0.0, 1.0),
            note,
            env,
            stage,
            left,
            rel_dec: 0.0,
            cut: 0.0,
            cut_gain: 1.0,
            attack,
            attack_gain: if attack > 0.0 { 0.0 } else { 1.0 },
        });
    }

    fn release_note(&mut self, note: u8) {
        let envelope = self.on(PARAM_ENVELOPE);
        let looping = self.on(PARAM_LOOP);
        let rel = self.samples(self.seconds(PARAM_RELEASE)) as f32;
        let stop = self.samples(LOOP_STOP_S) as f32;
        for v in self.voices.iter_mut() {
            if v.note != note || !v.held() {
                continue;
            }
            if envelope {
                v.rel_dec = v.env.max(1e-6) / rel;
                v.stage = EnvStage::Release;
            } else if looping {
                // No envelope: the sample plays to its end on its own, but
                // a loop would never end, so the key does.
                v.rel_dec = 1.0 / stop;
                v.stage = EnvStage::Release;
            }
        }
    }

    /// Linearly-interpolated sample for channel `ch` at fractional `pos`.
    fn read(sample: &SampleData, ch: usize, pos: f64) -> f32 {
        let data = match sample.channels.get(ch) {
            Some(d) => d,
            None => return 0.0,
        };
        if pos < 0.0 {
            return 0.0;
        }
        let i = pos.floor() as usize;
        if i + 1 >= data.len() {
            return data.get(i).copied().unwrap_or(0.0);
        }
        let frac = (pos - i as f64) as f32;
        data[i] * (1.0 - frac) + data[i + 1] * frac
    }

    fn state(&self) -> SamplerState {
        SamplerState {
            sample: self
                .sample
                .clone()
                .map(|mut s| {
                    s.base_note = self.root();
                    s
                })
                .unwrap_or_default(),
            params: (0..PARAM_COUNT)
                .map(|id| (id, self.p[id as usize]))
                .collect(),
            path: self.path.clone(),
        }
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
                    self.release_note(note)
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
        let envelope = self.on(PARAM_ENVELOPE);
        let gain = self.p[PARAM_GAIN as usize];
        let sustain = self.p[PARAM_SUSTAIN as usize];
        let atk_inc = 1.0 / self.samples(self.seconds(PARAM_ATTACK)) as f32;
        let dec_dec = (1.0 - sustain) / self.samples(self.seconds(PARAM_DECAY)) as f32;
        let hold = self.samples(self.seconds(PARAM_HOLD)) as u64;
        let looping = self.on(PARAM_LOOP);
        let ping_pong = self.on(PARAM_PING_PONG);
        // The sample is its region of the file; the loop points and the
        // offset count within it.
        let (rs, re) = self.region(frames);
        let span = (re - rs).max(1.0);
        let ls = rs + self.p[PARAM_LOOP_START as usize].clamp(0.0, 1.0) as f64 * span;
        let le = rs + self.p[PARAM_LOOP_END as usize].clamp(0.0, 1.0) as f64 * span;
        let loop_on = looping && le > ls + 1.0;
        let xfade = if loop_on {
            self.p[PARAM_XFADE as usize].clamp(0.0, 1.0) as f64 * (le - ls)
        } else {
            0.0
        };
        let fade_stereo = self.on(PARAM_FADE_STEREO);

        for i in 0..num_samples {
            let mut l = 0.0_f32;
            let mut r = 0.0_f32;
            for v in self.voices.iter_mut() {
                if v.stage == EnvStage::Idle {
                    continue;
                }
                if envelope {
                    match v.stage {
                        EnvStage::Delay => {
                            if v.left == 0 {
                                v.stage = EnvStage::Attack;
                            } else {
                                v.left -= 1;
                                continue;
                            }
                        }
                        EnvStage::Attack => {
                            v.env += atk_inc;
                            if v.env >= 1.0 {
                                v.env = 1.0;
                                v.stage = EnvStage::Hold;
                                v.left = hold;
                            }
                        }
                        EnvStage::Hold => {
                            if v.left == 0 {
                                v.stage = EnvStage::Decay;
                            } else {
                                v.left -= 1;
                            }
                        }
                        EnvStage::Decay => {
                            v.env -= dec_dec;
                            if v.env <= sustain {
                                v.env = sustain;
                                v.stage = EnvStage::Sustain;
                            }
                        }
                        EnvStage::Sustain => v.env = sustain,
                        EnvStage::Release => {
                            v.env -= v.rel_dec;
                            if v.env <= 0.0 {
                                v.env = 0.0;
                                v.stage = EnvStage::Idle;
                                continue;
                            }
                        }
                        EnvStage::Idle => continue,
                    }
                } else if v.stage == EnvStage::Release {
                    v.env -= v.rel_dec;
                    if v.env <= 0.0 {
                        v.env = 0.0;
                        v.stage = EnvStage::Idle;
                        continue;
                    }
                } else {
                    v.env = 1.0;
                }
                if v.cut > 0.0 {
                    v.cut_gain -= v.cut;
                    if v.cut_gain <= 0.0 {
                        v.stage = EnvStage::Idle;
                        continue;
                    }
                }
                if v.attack > 0.0 && v.attack_gain < 1.0 {
                    v.attack_gain = (v.attack_gain + v.attack).min(1.0);
                }
                if v.pos >= re || v.pos < rs {
                    v.stage = EnvStage::Idle;
                    continue;
                }
                let g = v.env * v.cut_gain * v.attack_gain * v.velocity * gain;
                let (mut sl, mut sr_s) = (Self::read(&sample, 0, v.pos), 0.0);
                if two_ch {
                    sr_s = Self::read(&sample, 1, v.pos);
                }
                // Into the loop's crossfade: blend with what comes before
                // the loop start, so the seam does not click.
                if xfade > 0.0 && v.dir > 0.0 && v.pos > le - xfade {
                    let other = v.pos - (le - ls);
                    if other >= rs {
                        let w = ((v.pos - (le - xfade)) / xfade).clamp(0.0, 1.0) as f32;
                        sl = sl * (1.0 - w) + Self::read(&sample, 0, other) * w;
                        if two_ch {
                            sr_s = sr_s * (1.0 - w) + Self::read(&sample, 1, other) * w;
                        }
                    }
                }
                if !two_ch {
                    sr_s = sl;
                }
                if fade_stereo {
                    let x = ((v.pos - rs) / span).clamp(0.0, 1.0) as f32;
                    sl *= 1.0 - x;
                    sr_s *= x;
                }
                l += sl * g;
                r += sr_s * g;
                if v.glide != 0.0 {
                    v.step += v.glide;
                    if (v.glide > 0.0 && v.step >= v.target_step)
                        || (v.glide < 0.0 && v.step <= v.target_step)
                    {
                        v.step = v.target_step;
                        v.glide = 0.0;
                    }
                }
                v.pos += v.step * v.dir;
                if loop_on {
                    if v.dir > 0.0 && v.pos >= le {
                        if ping_pong {
                            v.dir = -1.0;
                            v.pos = (le - (v.pos - le)).max(ls);
                        } else {
                            v.pos = ls + (v.pos - le) % (le - ls);
                        }
                    } else if v.dir < 0.0 && v.pos <= ls {
                        v.dir = 1.0;
                        v.pos = (ls + (ls - v.pos)).min(le);
                    }
                }
            }
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
        self.voices.retain(|v| v.stage != EnvStage::Idle);
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, unit) = match index {
            PARAM_GAIN => ("Gain", ""),
            PARAM_ATTACK => ("Attack", "ms"),
            PARAM_RELEASE => ("Release", "ms"),
            PARAM_DELAY => ("Delay", "ms"),
            PARAM_HOLD => ("Hold", "ms"),
            PARAM_DECAY => ("Decay", "ms"),
            PARAM_SUSTAIN => ("Sustain", ""),
            PARAM_ENVELOPE => ("Envelope", ""),
            PARAM_OFFSET => ("Offset", ""),
            PARAM_LOOP_START => ("Loop start", ""),
            PARAM_LOOP_END => ("Loop end", ""),
            PARAM_LOOP => ("Loop", ""),
            PARAM_PING_PONG => ("Ping pong", ""),
            PARAM_MONO => ("Mono", ""),
            PARAM_CUT_ITSELF => ("Cut itself", ""),
            PARAM_PORTA => ("Porta", ""),
            PARAM_ROOT => ("Root note", ""),
            PARAM_PITCH => ("Pitch", "st"),
            PARAM_SAMPLE_START => ("Sample start", ""),
            PARAM_SAMPLE_LENGTH => ("Sample length", ""),
            PARAM_XFADE => ("Loop crossfade", ""),
            PARAM_TRIM => ("Trim", ""),
            PARAM_FADE_STEREO => ("Fade stereo", ""),
            PARAM_DECLICK => ("Declicking", ""),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index,
            name: name.into(),
            default_value: defaults()[index as usize] as f64,
            min: 0.0,
            max: 1.0,
            unit: unit.into(),
            automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        self.p.get(id as usize).copied().unwrap_or(0.0) as f64
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        if let Some(slot) = self.p.get_mut(id as usize) {
            *slot = value.clamp(0.0, 1.0) as f32;
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        // Same clamp as the setter, so the text names what the value sets.
        let v = value.clamp(0.0, 1.0) as f32;
        let text = match id {
            PARAM_GAIN | PARAM_SUSTAIN => format::pct(v as f64),
            PARAM_ATTACK => format::secs(knob_seconds(v, ATTACK_MAX_S, 0.0001) as f64),
            PARAM_RELEASE => format::secs(knob_seconds(v, RELEASE_MAX_S, 0.001) as f64),
            PARAM_DELAY => format::secs(knob_seconds(v, DELAY_MAX_S, 0.0) as f64),
            PARAM_HOLD => format::secs(knob_seconds(v, HOLD_MAX_S, 0.0) as f64),
            PARAM_DECAY => format::secs(knob_seconds(v, DECAY_MAX_S, 0.001) as f64),
            PARAM_ENVELOPE | PARAM_LOOP | PARAM_PING_PONG | PARAM_MONO | PARAM_CUT_ITSELF
            | PARAM_PORTA | PARAM_FADE_STEREO => format::on_off(v as f64),
            PARAM_OFFSET | PARAM_LOOP_START | PARAM_LOOP_END | PARAM_SAMPLE_START
            | PARAM_SAMPLE_LENGTH | PARAM_XFADE => format::pct(v as f64),
            PARAM_TRIM => {
                if v <= 0.0 {
                    "Off".to_string()
                } else {
                    format!("{:.0} dB", -60.0 * (1.0 - v))
                }
            }
            PARAM_DECLICK => declick_of(v).name().to_string(),
            PARAM_ROOT => note_name((v * 127.0).round() as u8),
            PARAM_PITCH => {
                let st = (v * 2.0 - 1.0) * PITCH_RANGE_ST;
                format!("{}{:.2} st", if st > 0.0 { "+" } else { "" }, st)
            }
            _ => return None,
        };
        Some(text)
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match id {
            PARAM_ROOT => Some((0..=127).map(note_name).collect()),
            PARAM_DECLICK => Some(
                DeclickMode::ALL
                    .iter()
                    .map(|m| m.name().to_string())
                    .collect(),
            ),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        // The sample itself is the state, so a saved project carries it,
        // with the settings and the file's name behind it.
        if self.sample.is_none() {
            return Vec::new();
        }
        self.state().to_bytes()
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        match SamplerState::from_bytes(bytes) {
            Some(s) => {
                // Bytes with settings bring their own; a bare sample (a
                // new file, an old song) keeps the settings as they are,
                // and names its root from the sample.
                if s.params.is_empty() {
                    self.p[PARAM_ROOT as usize] = s.sample.base_note as f32 / 127.0;
                } else {
                    for (id, v) in s.params {
                        self.set_parameter_value(id, v as f64);
                    }
                }
                if !s.path.is_empty() {
                    self.path = s.path;
                }
                let (profile, peak) = profile_of(&s.sample);
                self.profile = profile;
                self.peak = peak;
                self.sample = Some(s.sample);
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
/// The settings are not in these bytes: a sampler loading them keeps its
/// own, as FL keeps a channel's envelope when a new sample drops in.
pub fn encode_sample_state(sample_rate: u32, base_note: u8, channels: Vec<Vec<f32>>) -> Vec<u8> {
    encode_sample_state_from(sample_rate, base_note, channels, "")
}

/// As `encode_sample_state`, naming the file the sample came from.
pub fn encode_sample_state_from(
    sample_rate: u32,
    base_note: u8,
    channels: Vec<Vec<f32>>,
    path: &str,
) -> Vec<u8> {
    let channels = if channels.is_empty() {
        vec![Vec::new()]
    } else {
        channels
    };
    let sample = SampleData {
        sample_rate,
        base_note,
        channels,
    };
    if path.is_empty() {
        return sample.to_bytes();
    }
    SamplerState {
        sample,
        params: Vec::new(),
        path: path.to_string(),
    }
    .to_bytes()
}

/// As `encode_sample_state_from`, with the settings the sampler starts
/// with (parameter id and its 0..1 value): an FL import brings the
/// channel's envelope, loop and voices this way.
pub fn encode_sample_state_with(
    sample_rate: u32,
    base_note: u8,
    channels: Vec<Vec<f32>>,
    path: &str,
    params: Vec<(u32, f32)>,
) -> Vec<u8> {
    let channels = if channels.is_empty() {
        vec![Vec::new()]
    } else {
        channels
    };
    SamplerState {
        sample: SampleData {
            sample_rate,
            base_note,
            channels,
        },
        params,
        path: path.to_string(),
    }
    .to_bytes()
}

/// The sampler's settings for an FL sampler channel: its volume envelope,
/// loop, voices, declicking, crossfade, trim, fade stereo and the sample's
/// start and length, and the root note. FL's knobs come as fractions of
/// their travel; they land on the same fraction of this sampler's knobs.
/// Normalize and DC removal are not settings here: the import does them
/// to the sample itself, as it does reverse and polarity.
pub fn settings_from_fl(
    setup: &hardwave_project::fl_import::FlSamplerSetup,
    root_note: u8,
) -> Vec<(u32, f32)> {
    let on = |b: bool| if b { 1.0 } else { 0.0 };
    let mut p = vec![
        (PARAM_ROOT, root_note.min(127) as f32 / 127.0),
        (PARAM_LOOP, on(setup.use_loop_points)),
        (PARAM_PING_PONG, on(setup.ping_pong)),
        (PARAM_MONO, on(setup.mono)),
        (PARAM_PORTA, on(setup.porta)),
        (PARAM_CUT_ITSELF, on(setup.cut_itself)),
        (
            PARAM_DECLICK,
            setup.declick.min(5) as f32 / (DeclickMode::ALL.len() - 1) as f32,
        ),
        (PARAM_XFADE, setup.crossfade.clamp(0.0, 1.0)),
        (PARAM_TRIM, setup.trim.clamp(0.0, 1.0)),
        (PARAM_FADE_STEREO, on(setup.fade_stereo)),
    ];
    if let Some(start) = setup.start {
        p.push((PARAM_SAMPLE_START, start.clamp(0.0, 1.0)));
    }
    if let Some(length) = setup.length {
        p.push((PARAM_SAMPLE_LENGTH, length.clamp(0.0, 1.0)));
    }
    match setup.envelope {
        Some(e) => p.extend([
            (PARAM_ENVELOPE, on(e.enabled)),
            (PARAM_DELAY, e.delay.clamp(0.0, 1.0)),
            (PARAM_ATTACK, e.attack.clamp(0.0, 1.0)),
            (PARAM_HOLD, e.hold.clamp(0.0, 1.0)),
            (PARAM_DECAY, e.decay.clamp(0.0, 1.0)),
            (PARAM_SUSTAIN, e.sustain.clamp(0.0, 1.0)),
            (PARAM_RELEASE, e.release.clamp(0.0, 1.0)),
        ]),
        // No envelope saved: FL plays the sample as it is.
        None => p.push((PARAM_ENVELOPE, 0.0)),
    }
    p
}

/// What a sampler's state bytes hold, for whoever shows them.
pub fn decode_sample_state(bytes: &[u8]) -> Option<SamplerState> {
    SamplerState::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_state_that_lies_about_its_size_is_refused() {
        let mut lie = vec![60u8];
        lie.extend_from_slice(&48_000u32.to_le_bytes());
        lie.extend_from_slice(&u32::MAX.to_le_bytes());
        lie.extend_from_slice(&0u32.to_le_bytes());
        assert!(
            SamplerState::from_bytes(&lie).is_none(),
            "channels past any sample"
        );
        let mut wrap = vec![60u8];
        wrap.extend_from_slice(&48_000u32.to_le_bytes());
        wrap.extend_from_slice(&2u32.to_le_bytes());
        wrap.extend_from_slice(&(1u32 << 31).to_le_bytes());
        assert!(
            SamplerState::from_bytes(&wrap).is_none(),
            "frames the bytes do not hold"
        );
        // Bytes behind the sample that are not a settings block.
        let mut junk = dc_sample();
        junk.extend_from_slice(b"what");
        assert!(SamplerState::from_bytes(&junk).is_none());
    }

    fn dc_sample() -> Vec<u8> {
        // 100 frames of DC = 0.5, stereo, base note 60.
        let ch = vec![0.5_f32; 100];
        encode_sample_state(48_000, 60, vec![ch.clone(), ch])
    }

    /// A ramp 0, 1, 2, ... so the output says where the voice is reading.
    fn ramp_sample(frames: usize) -> Vec<u8> {
        let ch: Vec<f32> = (0..frames).map(|i| i as f32).collect();
        encode_sample_state_from(48_000, 60, vec![ch], "C:\\packs\\ramp.wav")
    }

    fn sampler_with(bytes: &[u8]) -> NativeSampler {
        let mut s = NativeSampler::new();
        s.activate(48_000.0, 512).unwrap();
        s.set_state(bytes).unwrap();
        s
    }

    fn note_on(note: u8) -> MidiEvent {
        MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note,
            velocity: 1.0,
        }
    }
    fn note_off(note: u8) -> MidiEvent {
        MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note,
            velocity: 0.0,
        }
    }

    /// One block of `n` samples; the left channel comes back.
    fn run(s: &mut NativeSampler, events: &[MidiEvent], n: usize) -> Vec<f32> {
        let mut out = vec![vec![0.0; n], vec![0.0; n]];
        s.process(&[], &mut out, events, &mut Vec::new(), n);
        out.swap_remove(0)
    }

    fn set(s: &mut NativeSampler, id: u32, v: f64) {
        s.set_parameter_value(id, v);
    }

    #[test]
    fn state_round_trips() {
        let bytes = dc_sample();
        let s = SamplerState::from_bytes(&bytes).unwrap();
        assert_eq!(s.sample.sample_rate, 48_000);
        assert_eq!(s.sample.base_note, 60);
        assert_eq!(s.sample.channels.len(), 2);
        assert_eq!(s.sample.frames(), 100);
        assert!((s.sample.channels[0][0] - 0.5).abs() < 1e-6);
        assert!(s.params.is_empty(), "a bare sample carries no settings");
        assert_eq!(s.path, "");
    }

    #[test]
    fn the_settings_and_the_file_travel_with_the_sample() {
        let mut a = sampler_with(&ramp_sample(100));
        set(&mut a, PARAM_ENVELOPE, 1.0);
        set(&mut a, PARAM_SUSTAIN, 0.25);
        set(&mut a, PARAM_LOOP, 1.0);
        set(&mut a, PARAM_LOOP_START, 0.2);
        set(&mut a, PARAM_ROOT, 72.0 / 127.0);
        let bytes = a.get_state();

        let mut b = NativeSampler::new();
        b.set_state(&bytes).unwrap();
        for id in 0..PARAM_COUNT {
            assert!(
                (a.get_parameter_value(id) - b.get_parameter_value(id)).abs() < 1e-6,
                "parameter {id} did not survive the state"
            );
        }
        assert_eq!(b.root(), 72);
        assert_eq!(b.path, "C:\\packs\\ramp.wav");
        let decoded = decode_sample_state(&bytes).unwrap();
        assert_eq!(decoded.sample.frames(), 100);
        assert_eq!(
            decoded.sample.base_note, 72,
            "the header names the root too"
        );
    }

    #[test]
    fn a_knob_can_be_written_into_saved_bytes_without_the_audio() {
        let bare = ramp_sample(50);
        let once = with_parameter(&bare, PARAM_SUSTAIN, 0.3).unwrap();
        let twice = with_parameter(&once, PARAM_ROOT, 72.0 / 127.0).unwrap();
        let again = with_parameter(&twice, PARAM_SUSTAIN, 0.6).unwrap();
        let s = SamplerState::from_bytes(&again).unwrap();
        assert_eq!(s.sample.frames(), 50);
        assert_eq!(s.path, "C:\\packs\\ramp.wav");
        assert_eq!(s.sample.base_note, 72);
        assert_eq!(s.params.len(), 2, "one entry per setting: {:?}", s.params);
        assert!(s.params.contains(&(PARAM_SUSTAIN, 0.6)));
        let mut p = NativeSampler::new();
        p.set_state(&again).unwrap();
        assert!((p.get_parameter_value(PARAM_SUSTAIN) - 0.6).abs() < 1e-6);
        assert_eq!(p.root(), 72);
        assert!(with_parameter(&bare, PARAM_COUNT, 0.5).is_none());
        assert!(with_parameter(b"junk", PARAM_GAIN, 0.5).is_none());
    }

    #[test]
    fn an_fl_sampler_channel_starts_with_its_own_settings() {
        use hardwave_project::fl_import::{FlEnvelope, FlSamplerSetup};
        let setup = FlSamplerSetup {
            envelope: Some(FlEnvelope {
                enabled: true,
                delay: 0.0,
                attack: 0.5,
                hold: 0.0,
                decay: 1.0,
                sustain: 0.5,
                release: 0.25,
            }),
            use_loop_points: true,
            ping_pong: true,
            mono: true,
            porta: false,
            cut_itself: false,
            declick: 4,
            crossfade: 0.25,
            trim: 0.5,
            start: Some(0.25),
            length: None,
            fade_stereo: true,
            ..Default::default()
        };
        let ch = vec![0.5_f32; 100];
        let bytes = encode_sample_state_with(
            48_000,
            62,
            vec![ch],
            "C:\\packs\\kick.wav",
            settings_from_fl(&setup, 62),
        );
        let mut s = NativeSampler::new();
        s.set_state(&bytes).unwrap();
        assert_eq!(s.root(), 62, "the root comes along");
        assert!(s.on(PARAM_ENVELOPE) && s.on(PARAM_LOOP) && s.on(PARAM_PING_PONG));
        assert!(s.on(PARAM_MONO) && !s.on(PARAM_PORTA));
        assert!(!s.on(PARAM_CUT_ITSELF), "FL's channel did not cut itself");
        assert_eq!(s.declick(), DeclickMode::Smooth);
        assert!(s.on(PARAM_FADE_STEREO));
        let v = |id: u32| s.get_parameter_value(id) as f32;
        assert_eq!(
            (v(PARAM_ATTACK), v(PARAM_SUSTAIN), v(PARAM_RELEASE)),
            (0.5, 0.5, 0.25)
        );
        assert_eq!((v(PARAM_XFADE), v(PARAM_TRIM)), (0.25, 0.5));
        assert_eq!(v(PARAM_SAMPLE_START), 0.25);
        assert_eq!(
            v(PARAM_SAMPLE_LENGTH),
            1.0,
            "no length saved: the rest of the sample"
        );
        assert_eq!(s.path, "C:\\packs\\kick.wav");

        // A channel FL saved without an envelope plays its sample as it is.
        let plain = settings_from_fl(&FlSamplerSetup::default(), 60);
        assert!(plain.contains(&(PARAM_ENVELOPE, 0.0)));
    }

    #[test]
    fn a_new_sample_keeps_the_channel_s_settings() {
        let mut s = sampler_with(&ramp_sample(100));
        set(&mut s, PARAM_ENVELOPE, 1.0);
        set(&mut s, PARAM_MONO, 1.0);
        // load_sampler ships a bare sample: the envelope and mono stay.
        s.set_state(&dc_sample()).unwrap();
        assert!(s.on(PARAM_ENVELOPE));
        assert!(s.on(PARAM_MONO));
        assert_eq!(s.root(), 60, "a bare sample names its own root");
        assert_eq!(s.sample.as_ref().unwrap().frames(), 100);
    }

    #[test]
    fn plays_loaded_sample_on_note() {
        let mut s = sampler_with(&dc_sample());
        let out = run(&mut s, &[note_on(60)], 64);
        let peak = out.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(
            peak > 0.0,
            "loaded sample must sound on note-on, got {peak}"
        );
    }

    #[test]
    fn silent_without_sample() {
        let mut s = NativeSampler::new();
        s.activate(48_000.0, 512).unwrap();
        let out = run(&mut s, &[note_on(60)], 32);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn without_the_envelope_the_sample_plays_to_its_end() {
        // FL: a drum plays in full however short its note is.
        let mut s = sampler_with(&ramp_sample(200));
        let a = run(&mut s, &[note_on(60)], 50);
        assert!((a[10] - 10.0).abs() < 1e-3, "plays straight, got {}", a[10]);
        let b = run(&mut s, &[note_off(60)], 50);
        assert!(
            (b[10] - 60.0).abs() < 1e-3,
            "the key up changed nothing, got {}",
            b[10]
        );
        let c = run(&mut s, &[], 200);
        assert!(
            (c[99] - 199.0).abs() < 1e-3,
            "reads to the last frame, got {}",
            c[99]
        );
        assert_eq!(c[100], 0.0, "and stops there");
        assert!(s.voices.is_empty());
    }

    #[test]
    fn with_the_envelope_the_key_up_lets_go() {
        let mut s = sampler_with(&dc_sample());
        set(&mut s, PARAM_ENVELOPE, 1.0);
        set(&mut s, PARAM_ATTACK, 0.0); // 0.1 ms
        set(
            &mut s,
            PARAM_RELEASE,
            (0.001_f32 / RELEASE_MAX_S).sqrt() as f64,
        ); // 1 ms = 48 samples
        let _ = run(&mut s, &[note_on(60)], 20);
        let out = run(&mut s, &[note_off(60)], 60);
        assert!(
            out[0] > 0.4,
            "still sounding as the release starts, got {}",
            out[0]
        );
        assert!(
            out[24] > 0.1 && out[24] < 0.4,
            "half way down, got {}",
            out[24]
        );
        assert_eq!(out[55], 0.0, "gone after the release");
    }

    #[test]
    fn the_envelope_delays_attacks_holds_decays_and_sustains() {
        let mut s = sampler_with(&dc_sample());
        // The sample is 100 frames; loop it so the envelope has room.
        set(&mut s, PARAM_LOOP, 1.0);
        set(&mut s, PARAM_ENVELOPE, 1.0);
        let k = |seconds: f32, max: f32| (seconds / max).sqrt() as f64;
        set(&mut s, PARAM_DELAY, k(0.001, DELAY_MAX_S)); // 48 samples of nothing
        set(&mut s, PARAM_ATTACK, k(0.001, ATTACK_MAX_S)); // 48 samples up
        set(&mut s, PARAM_HOLD, k(0.001, HOLD_MAX_S)); // 48 at the top
        set(&mut s, PARAM_DECAY, k(0.001, DECAY_MAX_S)); // 48 down to
        set(&mut s, PARAM_SUSTAIN, 0.5); // half
        let out = run(&mut s, &[note_on(60)], 300);
        assert!(
            out[..48].iter().all(|v| *v == 0.0),
            "silent through the delay"
        );
        assert!(
            out[48 + 24] > 0.2 && out[48 + 24] < 0.3,
            "half way up the attack: {}",
            out[72]
        );
        assert!(
            (out[48 + 48 + 20] - 0.5).abs() < 0.02,
            "held at the top: {}",
            out[116]
        );
        assert!(
            out[48 + 48 + 48 + 24] > 0.3 && out[168] < 0.45,
            "half way down the decay: {}",
            out[168]
        );
        assert!(
            (out[280] - 0.25).abs() < 0.02,
            "sustaining at half: {}",
            out[280]
        );
    }

    #[test]
    fn a_loop_keeps_the_voice_alive_past_the_end() {
        let mut s = sampler_with(&ramp_sample(100));
        set(&mut s, PARAM_LOOP, 1.0);
        set(&mut s, PARAM_LOOP_START, 0.5);
        set(&mut s, PARAM_LOOP_END, 0.8);
        let out = run(&mut s, &[note_on(60)], 400);
        // Up to 80, back to 50, and so on: never past 80, never below 50
        // once it has got there, and still going after 400 samples.
        assert!(out[79] > 78.0);
        assert!(
            out[85] >= 50.0 && out[85] < 60.0,
            "wrapped to the loop start: {}",
            out[85]
        );
        assert!(out[200..].iter().all(|v| *v >= 49.0 && *v <= 80.0));
        assert_eq!(s.voices.len(), 1);
    }

    #[test]
    fn a_ping_pong_loop_turns_round() {
        let mut s = sampler_with(&ramp_sample(100));
        set(&mut s, PARAM_LOOP, 1.0);
        set(&mut s, PARAM_PING_PONG, 1.0);
        set(&mut s, PARAM_LOOP_START, 0.5);
        set(&mut s, PARAM_LOOP_END, 0.8);
        let out = run(&mut s, &[note_on(60)], 400);
        assert!(
            out[85] < out[80],
            "reads backwards after the loop end: {} then {}",
            out[80],
            out[85]
        );
        assert!(
            out[115] > out[112],
            "and forwards again after the loop start: {} then {}",
            out[112],
            out[115]
        );
        assert!(out[200..].iter().all(|v| *v >= 49.0 && *v <= 80.0));
    }

    #[test]
    fn a_key_up_stops_a_loop_even_without_the_envelope() {
        let mut s = sampler_with(&dc_sample());
        set(&mut s, PARAM_LOOP, 1.0);
        let _ = run(&mut s, &[note_on(60)], 50);
        let out = run(&mut s, &[note_off(60)], 400);
        assert!(out[0] > 0.0);
        assert!(out[399] == 0.0 && s.voices.is_empty(), "let go within 5 ms");
    }

    #[test]
    fn the_offset_starts_the_sample_later() {
        let mut s = sampler_with(&ramp_sample(100));
        set(&mut s, PARAM_OFFSET, 0.25);
        let out = run(&mut s, &[note_on(60)], 4);
        assert!(
            (out[0] - 25.0).abs() < 1e-3,
            "starts a quarter in: {}",
            out[0]
        );
    }

    #[test]
    fn mono_plays_one_voice_and_cut_itself_restarts() {
        let mut s = sampler_with(&ramp_sample(1000));
        set(&mut s, PARAM_CUT_ITSELF, 0.0);
        let _ = run(&mut s, &[note_on(60), note_on(64)], 10);
        assert_eq!(s.voices.len(), 2, "polyphonic when neither is on");

        let mut s = sampler_with(&ramp_sample(1000));
        set(&mut s, PARAM_MONO, 1.0);
        let _ = run(&mut s, &[note_on(60)], 10);
        let _ = run(&mut s, &[note_on(64)], 200);
        assert_eq!(s.voices.len(), 1, "the first voice was cut away");
        assert_eq!(s.voices[0].note, 64);

        let mut s = sampler_with(&ramp_sample(1000));
        let _ = run(&mut s, &[note_on(60)], 500);
        let out = run(&mut s, &[note_on(60)], 200);
        assert!(
            out[150] < 200.0,
            "the retrigger reads from the start again: {}",
            out[150]
        );
        assert_eq!(s.voices.len(), 1);
    }

    #[test]
    fn porta_slides_the_held_note_to_the_new_one() {
        let mut s = sampler_with(&ramp_sample(100_000));
        set(&mut s, PARAM_MONO, 1.0);
        set(&mut s, PARAM_PORTA, 1.0);
        let _ = run(&mut s, &[note_on(60)], 10);
        let _ = run(&mut s, &[note_on(72)], 10);
        assert_eq!(s.voices.len(), 1, "no new voice: the held one slides");
        let early = s.voices[0].step;
        assert!(early > 1.0 && early < 2.0, "on its way up: {early}");
        let _ = run(&mut s, &[], 48_000 / 4);
        assert!(
            (s.voices[0].step - 2.0).abs() < 1e-9,
            "arrived an octave up"
        );
    }

    #[test]
    fn the_root_note_and_the_pitch_knob_tune_the_channel() {
        let mut s = sampler_with(&dc_sample());
        s.start_voice(72, 1.0); // +12 semitones from the root (60)
        assert!((s.voices[0].step - 2.0).abs() < 1e-6, "octave up → 2x step");
        set(&mut s, PARAM_ROOT, 72.0 / 127.0);
        s.start_voice(72, 1.0);
        assert!(
            (s.voices[1].step - 1.0).abs() < 1e-6,
            "on its root the sample plays as recorded"
        );
        set(&mut s, PARAM_PITCH, 0.75); // +12 st of the 24 each way
        s.start_voice(72, 1.0);
        assert!(
            (s.voices[2].step - 2.0).abs() < 1e-6,
            "the pitch knob shifts the whole channel"
        );
    }

    #[test]
    fn the_sample_is_its_region_of_the_file() {
        let mut s = sampler_with(&ramp_sample(1000));
        set(&mut s, PARAM_SAMPLE_START, 0.25);
        set(&mut s, PARAM_SAMPLE_LENGTH, 0.5);
        let out = run(&mut s, &[note_on(60)], 500);
        assert!(
            (out[0] - 250.0).abs() < 1e-3,
            "starts a quarter in: {}",
            out[0]
        );
        assert!(
            (out[374] - 624.0).abs() < 1e-3,
            "reads to the region's end: {}",
            out[374]
        );
        assert_eq!(out[375], 0.0, "and stops there");
        // The offset counts within the region.
        set(&mut s, PARAM_OFFSET, 0.5);
        let out = run(&mut s, &[note_on(60)], 4);
        assert!(
            (out[0] - 437.5).abs() < 1e-3,
            "half way into the region: {}",
            out[0]
        );
    }

    #[test]
    fn the_trim_cuts_the_quiet_tail() {
        // Loud for 400 frames, a whisper for the rest.
        let ch: Vec<f32> = (0..1000)
            .map(|i| if i < 400 { 0.8 } else { 0.001 })
            .collect();
        let bytes = encode_sample_state(48_000, 60, vec![ch]);
        let mut s = sampler_with(&bytes);
        let out = run(&mut s, &[note_on(60)], 1000);
        assert!(out[900].abs() > 0.0, "untrimmed, the whisper plays");
        set(&mut s, PARAM_TRIM, 0.5); // -30 dB under the peak
        let out = run(&mut s, &[note_on(60)], 1000);
        assert!(out[300].abs() > 0.5, "the loud part plays");
        assert_eq!(out[500], 0.0, "the whisper is cut: {}", out[500]);
        assert_eq!(s.parameter_text(PARAM_TRIM, 0.5).as_deref(), Some("-30 dB"));
        assert_eq!(s.parameter_text(PARAM_TRIM, 0.0).as_deref(), Some("Off"));
    }

    #[test]
    fn the_loop_crossfade_blends_the_seam() {
        let mut s = sampler_with(&ramp_sample(1000));
        set(&mut s, PARAM_LOOP, 1.0);
        set(&mut s, PARAM_LOOP_START, 0.5);
        set(&mut s, PARAM_LOOP_END, 0.9);
        set(&mut s, PARAM_XFADE, 0.25); // the last 100 frames of the 400-frame loop
        let out = run(&mut s, &[note_on(60)], 900);
        // At frame 850, 50 into the crossfade: half of 850 and half of 450.
        assert!((out[850] - 650.0).abs() < 2.0, "blended: {}", out[850]);
        assert!(
            (out[799] - 799.0).abs() < 1.0,
            "untouched before the crossfade: {}",
            out[799]
        );
    }

    #[test]
    fn fade_stereo_and_the_declicking_attack() {
        let ch: Vec<f32> = vec![1.0; 1000];
        let bytes = encode_sample_state(48_000, 60, vec![ch.clone(), ch]);
        let mut s = sampler_with(&bytes);
        set(&mut s, PARAM_FADE_STEREO, 1.0);
        let mut out = vec![vec![0.0; 1000], vec![0.0; 1000]];
        s.process(&[], &mut out, &[note_on(60)], &mut Vec::new(), 1000);
        assert!(
            out[0][500] > 0.45 && out[0][500] < 0.55,
            "left half way down: {}",
            out[0][500]
        );
        assert!(
            out[1][500] > 0.45 && out[1][500] < 0.55,
            "right half way up: {}",
            out[1][500]
        );
        assert!(out[0][10] > 0.95 && out[1][10] < 0.05);

        let mut s = sampler_with(&bytes);
        set(&mut s, PARAM_DECLICK, 4.0 / 5.0); // Smooth: 10 ms in
        assert_eq!(
            s.parameter_text(PARAM_DECLICK, 4.0 / 5.0).as_deref(),
            Some("Smooth")
        );
        let out = run(&mut s, &[note_on(60)], 600);
        assert!(out[0] < 0.01, "starts from nothing: {}", out[0]);
        assert!(
            out[240] > 0.45 && out[240] < 0.55,
            "half way up at 5 ms: {}",
            out[240]
        );
        assert!(
            (out[500] - 1.0).abs() < 1e-3,
            "full after 10 ms: {}",
            out[500]
        );
        let mut s = sampler_with(&bytes);
        let out = run(&mut s, &[note_on(60)], 4);
        assert!(
            (out[0] - 1.0).abs() < 1e-3,
            "out only: the attack is left alone"
        );
    }

    #[test]
    fn values_read_as_text() {
        let p = NativeSampler::new();
        let shown = |id: u32| p.parameter_text(id, p.get_parameter_value(id));
        assert_eq!(shown(PARAM_GAIN).as_deref(), Some("100 %"));
        assert_eq!(shown(PARAM_ATTACK).as_deref(), Some("2.00 ms"));
        assert_eq!(shown(PARAM_RELEASE).as_deref(), Some("60.0 ms"));
        assert_eq!(shown(PARAM_SUSTAIN).as_deref(), Some("100 %"));
        assert_eq!(shown(PARAM_ENVELOPE).as_deref(), Some("Off"));
        assert_eq!(shown(PARAM_CUT_ITSELF).as_deref(), Some("On"));
        assert_eq!(shown(PARAM_ROOT).as_deref(), Some("C4"));
        assert_eq!(shown(PARAM_PITCH).as_deref(), Some("0.00 st"));
        assert_eq!(
            p.parameter_text(PARAM_PITCH, 1.0).as_deref(),
            Some("+24.00 st")
        );
        assert_eq!(p.parameter_text(PARAM_ROOT, 0.0).as_deref(), Some("C-1"));
        // The floors the setter keeps show up in the text too.
        assert_eq!(
            p.parameter_text(PARAM_ATTACK, 0.0).as_deref(),
            Some("0.10 ms")
        );
        assert_eq!(
            p.parameter_text(PARAM_RELEASE, 1.0).as_deref(),
            Some("10.00 s")
        );
        for id in 0..PARAM_COUNT {
            assert!(
                p.parameter_text(id, 0.5).is_some(),
                "parameter {id} has no text"
            );
            assert!(p.get_parameter_info(id).is_some());
        }
        assert_eq!(p.parameter_options(PARAM_ROOT).map(|o| o.len()), Some(128));
        assert_eq!(p.parameter_options(PARAM_DECLICK).map(|o| o.len()), Some(6));
        assert_eq!(shown(PARAM_DECLICK).as_deref(), Some("Out only"));
        assert_eq!(shown(PARAM_SAMPLE_LENGTH).as_deref(), Some("100 %"));
    }
}
