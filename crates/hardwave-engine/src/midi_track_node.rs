//! MIDI track audio node — turns scheduled MIDI notes into audio.
//!
//! Minimal monophonic synthesizer that consumes pre-resolved [`MidiNoteRegion`]s
//! (notes already mapped to absolute sample positions) and writes a mono-summed
//! sine wave with a linear ADSR envelope into both stereo output channels.
//!
//! Scope: this is the first audible MIDI path. It is intentionally a built-in
//! sine-osc monosynth so that a freshly-recorded or freshly-drawn piano-roll
//! clip becomes audible immediately, without requiring a third-party VST.
//! Polyphony, alternative oscillator shapes, filters, and instrument plug-ins
//! all layer on top of this scaffold once the wiring proves out.

use std::sync::{atomic::Ordering, Arc};

use crate::graph::{AudioNode, ProcessContext};
use crate::track_node::TrackMeterState;

/// A single MIDI note pre-resolved to sample positions on the timeline.
///
/// Equivalent in spirit to [`crate::track_node::ClipRegion`] for audio:
/// the heavy lifting (tempo-map lookups, tick → sample conversion, clip
/// position offset) happens once on the UI thread inside
/// `engine.rebuild_graph()`, so the audio thread sees a flat, sorted list of
/// pre-baked sample indices and never touches the tempo map.
#[derive(Debug, Clone)]
pub struct MidiNoteRegion {
    /// Absolute timeline sample at which note-on fires.
    pub note_on_sample: u64,
    /// Absolute timeline sample at which note-off fires (release begins).
    pub note_off_sample: u64,
    /// MIDI pitch 0..=127. Mapped to frequency via the standard 440 Hz A4
    /// equal-temperament formula.
    pub pitch: u8,
    /// Normalised velocity 0..=1. Drives the per-voice gain.
    pub velocity: f32,
    /// When true, the note is skipped entirely (used by the "mute" tool).
    pub muted: bool,
}

/// Phase of the ADSR envelope. See [`Voice::sample_envelope`] for the
/// per-stage gain calculation.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EnvStage {
    Attack,
    Decay,
    Sustain,
    Release,
    Idle,
}

/// One playing note. The synth currently only keeps a single Voice (mono),
/// so a new note-on while one is held replaces the active voice rather than
/// stacking. That mirrors the behaviour of FL Studio's default "mono" voice
/// mode and keeps CPU bounded for the first audible release.
#[derive(Debug, Clone)]
struct Voice {
    /// Hz, derived once from MIDI pitch.
    freq: f32,
    /// 0..=1, used as a multiplier on the envelope output.
    velocity: f32,
    /// Oscillator phase in radians, advances by `2*PI*freq/sr` per sample.
    phase: f32,
    /// Current envelope stage. Note-off transitions Attack/Decay/Sustain → Release.
    stage: EnvStage,
    /// Linear gain produced by the envelope this sample (smoothed across stages).
    env_value: f32,
}

const TWO_PI: f32 = std::f32::consts::TAU;

const ATTACK_SECS: f32 = 0.005;
const DECAY_SECS: f32 = 0.080;
const SUSTAIN_LEVEL: f32 = 0.70;
const RELEASE_SECS: f32 = 0.150;

/// Convert MIDI pitch number → frequency in Hz using equal-temperament.
#[inline]
fn pitch_to_freq(pitch: u8) -> f32 {
    440.0 * 2.0_f32.powf((pitch as f32 - 69.0) / 12.0)
}

pub struct MidiTrackNode {
    track_id: String,
    name: String,
    notes: Vec<MidiNoteRegion>,
    /// Index of the next note to consider for note-on. We advance through
    /// `notes` linearly each block; because `notes` is kept sorted by
    /// `note_on_sample`, the audio thread does no per-sample scanning.
    next_note_idx: usize,
    /// Single playing voice. None when nothing is held.
    voice: Option<Voice>,
    /// Linear post-fader gain. Mirrors TrackNode's volume/pan model so
    /// the mixer panel can drive the same atomics.
    volume: f32,
    pan: f32,
    muted: bool,
    soloed: bool,
    /// Shared meter state (post-fader peak + RMS). Same struct that the UI
    /// reads for audio tracks, so MIDI tracks light up the meter strip too.
    meter: Arc<TrackMeterState>,
    rms_smooth: f32,
}

impl MidiTrackNode {
    pub fn new(track_id: String, name: String, meter: Arc<TrackMeterState>) -> Self {
        Self {
            track_id,
            name,
            notes: Vec::new(),
            next_note_idx: 0,
            voice: None,
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            meter,
            rms_smooth: 0.0,
        }
    }

    /// Replace this node's note schedule. Caller must pre-sort by
    /// `note_on_sample`; the audio thread relies on monotonic ordering for
    /// its linear scan.
    pub fn set_notes(&mut self, mut notes: Vec<MidiNoteRegion>) {
        notes.sort_by_key(|n| n.note_on_sample);
        self.notes = notes;
        self.next_note_idx = 0;
        // Don't kill an in-flight voice — it'll release naturally when its
        // schedule cuts off. Forcing it to Idle here would click on graph
        // rebuilds during playback.
    }

    pub fn set_volume_db(&mut self, db: f64) {
        self.volume = 10.0_f64.powf(db / 20.0) as f32;
    }
    pub fn set_pan(&mut self, pan: f64) {
        self.pan = pan.clamp(-1.0, 1.0) as f32;
    }
    pub fn set_muted(&mut self, m: bool) {
        self.muted = m;
    }
    pub fn set_soloed(&mut self, s: bool) {
        self.soloed = s;
    }

    /// Advance the envelope by one sample and return the current gain.
    fn step_envelope(voice: &mut Voice, sr: f32) -> f32 {
        match voice.stage {
            EnvStage::Idle => 0.0,
            EnvStage::Attack => {
                let step = if ATTACK_SECS > 0.0 { 1.0 / (ATTACK_SECS * sr) } else { 1.0 };
                voice.env_value += step;
                if voice.env_value >= 1.0 {
                    voice.env_value = 1.0;
                    voice.stage = EnvStage::Decay;
                }
                voice.env_value
            }
            EnvStage::Decay => {
                let step = if DECAY_SECS > 0.0 {
                    (1.0 - SUSTAIN_LEVEL) / (DECAY_SECS * sr)
                } else {
                    1.0 - SUSTAIN_LEVEL
                };
                voice.env_value -= step;
                if voice.env_value <= SUSTAIN_LEVEL {
                    voice.env_value = SUSTAIN_LEVEL;
                    voice.stage = EnvStage::Sustain;
                }
                voice.env_value
            }
            EnvStage::Sustain => SUSTAIN_LEVEL,
            EnvStage::Release => {
                let step = if RELEASE_SECS > 0.0 { voice.env_value / (RELEASE_SECS * sr) } else { voice.env_value };
                voice.env_value -= step;
                if voice.env_value <= 0.0 {
                    voice.env_value = 0.0;
                    voice.stage = EnvStage::Idle;
                }
                voice.env_value
            }
        }
    }
}

impl AudioNode for MidiTrackNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn track_id(&self) -> Option<&str> {
        Some(&self.track_id)
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        // Defensive: zero outputs first so we never leak undefined data.
        for buf in outputs.iter_mut() {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
        }
        if self.muted || !ctx.playing {
            return;
        }

        let sr = ctx.sample_rate as f32;
        let block_size = outputs.first().map(|b| b.len()).unwrap_or(0);
        if block_size == 0 {
            return;
        }
        let block_start = ctx.position_samples;

        // Constant-power pan curve. At pan=0 both channels get
        // cos(π/4) = sin(π/4) ≈ 0.707; at pan=±1 the far channel hits
        // unity while the near channel is silent. Matches the standard
        // pan law every audio TrackNode in the engine uses.
        let theta = (self.pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let pan_l = theta.cos();
        let pan_r = theta.sin();

        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;
        let mut energy_acc = 0.0_f32;

        // Skip notes that ended before the block starts. This fast-forwards
        // `next_note_idx` after a seek so we don't fire stale note-ons.
        while self.next_note_idx < self.notes.len()
            && self.notes[self.next_note_idx].note_off_sample < block_start
        {
            self.next_note_idx += 1;
        }

        for i in 0..block_size {
            let global_sample = block_start.saturating_add(i as u64);

            // Fire any pending note-ons that land on this exact sample.
            while self.next_note_idx < self.notes.len()
                && self.notes[self.next_note_idx].note_on_sample <= global_sample
            {
                let note = &self.notes[self.next_note_idx];
                if !note.muted {
                    // Carry the previous voice's envelope value forward
                    // into the new attack so a quick legato note-on
                    // doesn't restart from silence and click. With pure
                    // monophony, the new note takes over from wherever
                    // the previous voice was in its envelope.
                    let carry_gain = self.voice.as_ref().map(|v| v.env_value).unwrap_or(0.0);
                    self.voice = Some(Voice {
                        freq: pitch_to_freq(note.pitch),
                        velocity: note.velocity.clamp(0.0, 1.0),
                        phase: 0.0,
                        stage: EnvStage::Attack,
                        env_value: carry_gain,
                    });
                }
                self.next_note_idx += 1;
            }

            // Trigger release when the active voice's note-off sample is reached.
            if let Some(v) = self.voice.as_mut() {
                if v.stage != EnvStage::Release && v.stage != EnvStage::Idle {
                    // Find the note that owns this voice — cheap because
                    // MidiTrackNode is monophonic and we just consumed it.
                    let off_sample = self
                        .notes
                        .get(self.next_note_idx.saturating_sub(1))
                        .map(|n| n.note_off_sample)
                        .unwrap_or(global_sample);
                    if off_sample <= global_sample {
                        v.stage = EnvStage::Release;
                    }
                }
            }

            // Sample the synth.
            let sample = if let Some(v) = self.voice.as_mut() {
                let env = Self::step_envelope(v, sr);
                let osc = v.phase.sin();
                v.phase += TWO_PI * v.freq / sr;
                if v.phase >= TWO_PI {
                    v.phase -= TWO_PI;
                }
                if matches!(v.stage, EnvStage::Idle) {
                    self.voice = None;
                    0.0
                } else {
                    osc * env * v.velocity
                }
            } else {
                0.0
            };

            let mixed = sample * self.volume;
            let l = mixed * pan_l;
            let r = mixed * pan_r;

            if let Some(buf) = outputs.get_mut(0) {
                buf[i] = l;
            }
            if let Some(buf) = outputs.get_mut(1) {
                buf[i] = r;
            }

            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
            energy_acc += (l * l + r * r) * 0.5;
        }

        // Update meters — same atomics the audio TrackNode writes to, so
        // the mixer strip lights up identically for MIDI tracks.
        let to_db = |v: f32| if v > 0.0 { 20.0 * v.log10() } else { -120.0 };
        let peak_db_l = to_db(peak_l);
        let peak_db_r = to_db(peak_r);
        let pre_peak_db = peak_db_l.max(peak_db_r);
        let rms_inst = (energy_acc / block_size as f32).sqrt();
        // Smoothing: ~100 ms time-constant at typical block sizes.
        self.rms_smooth = self.rms_smooth * 0.9 + rms_inst * 0.1;
        let rms_db = to_db(self.rms_smooth);
        self.meter.peak_db_l.store(peak_db_l, Ordering::Relaxed);
        self.meter.peak_db_r.store(peak_db_r, Ordering::Relaxed);
        self.meter.rms_db.store(rms_db, Ordering::Relaxed);
        self.meter.pre_fader_peak_db.store(pre_peak_db, Ordering::Relaxed);

        // Soloed flag: the engine's mix step honours TrackKind solo across
        // the whole project, so we just expose our own solo bit unchanged.
        let _ = self.soloed;
    }

    fn reset(&mut self) {
        self.voice = None;
        self.next_note_idx = 0;
        self.rms_smooth = 0.0;
    }
}
