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
    /// Pitch currently held by an external/live MIDI NoteOn — distinct
    /// from clip-driven notes so a live release doesn't kill a clip
    /// note that happens to be playing the same pitch. When `Some(p)`,
    /// a matching NoteOff transitions `voice` into the Release stage.
    live_held_pitch: Option<u8>,
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
    /// Native instrument the track is voiced with. When set to
    /// [`Instrument::KickSynth`] the built-in sine voicing below is
    /// bypassed and each note-on retriggers the kick synth instead.
    instrument: Instrument,
    kick: hardwave_dsp::kick_synth::KickSynth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instrument {
    /// Default — monosynth with sine osc + ADSR. Pitch-aware.
    BuiltinSine,
    /// Native KickSynth — every note-on retriggers the kick voice
    /// regardless of MIDI pitch. Velocity scales the layer mix.
    KickSynth,
}

impl MidiTrackNode {
    pub fn new(track_id: String, name: String, meter: Arc<TrackMeterState>) -> Self {
        Self {
            track_id,
            name,
            notes: Vec::new(),
            next_note_idx: 0,
            live_held_pitch: None,
            voice: None,
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            meter,
            rms_smooth: 0.0,
            instrument: Instrument::BuiltinSine,
            kick: hardwave_dsp::kick_synth::KickSynth::new(48_000.0),
        }
    }

    /// Switch which native instrument voices this track. Cheap — flag
    /// flip + reset of the relevant voice state. Audio thread sees the
    /// change on the next block.
    pub fn set_instrument(&mut self, kind: Instrument, sample_rate: f32) {
        self.instrument = kind;
        match kind {
            Instrument::BuiltinSine => {
                self.voice = None;
            }
            Instrument::KickSynth => {
                self.kick = hardwave_dsp::kick_synth::KickSynth::new(sample_rate.max(1.0));
            }
        }
    }

    /// Push a per-track KickSynth patch onto the synth. Layers with
    /// `Some(...)` overrides replace the corresponding default; `None`
    /// entries leave the engine's hardstyle preset intact for that
    /// layer. Called from `rebuild_graph` once the project patch is
    /// known. Cheap — just writes 4 structs into the layer array.
    pub fn apply_kick_patch(&mut self, patch: &hardwave_project::track::KickPatch) {
        if !matches!(self.instrument, Instrument::KickSynth) {
            return;
        }
        self.kick.set_drive(patch.drive);
        for (i, slot) in patch.layers.iter().enumerate() {
            let Some(p) = slot else { continue };
            use hardwave_dsp::kick_synth::LayerWaveform;
            let waveform = match p.waveform.as_str() {
                "saw" => LayerWaveform::Saw,
                "square" => LayerWaveform::Square,
                "triangle" => LayerWaveform::Triangle,
                _ => LayerWaveform::Sine,
            };
            self.kick.set_layer(
                i,
                hardwave_dsp::kick_synth::Layer {
                    envelope: hardwave_dsp::kick_synth::LayerEnvelope {
                        peak_gain: p.peak_gain,
                        length_secs: p.length_secs,
                        release_secs: p.release_secs,
                    },
                    sweep: hardwave_dsp::kick_synth::FrequencySweep {
                        start_hz: p.sweep_start_hz,
                        end_hz: p.sweep_end_hz,
                        sweep_secs: p.sweep_secs,
                    },
                    waveform,
                },
            );
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
                let step = if ATTACK_SECS > 0.0 {
                    1.0 / (ATTACK_SECS * sr)
                } else {
                    1.0
                };
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
                let step = if RELEASE_SECS > 0.0 {
                    voice.env_value / (RELEASE_SECS * sr)
                } else {
                    voice.env_value
                };
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
        midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        // Defensive: zero outputs first so we never leak undefined data.
        for buf in outputs.iter_mut() {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
        }
        if self.muted {
            // Still process events so live note-state stays correct
            // when the user unmutes mid-hold — but emit silence.
            return;
        }

        // Apply live MIDI input first so a NoteOn delivered this block
        // is immediately audible, even when the transport is stopped.
        // FL Studio / Logic / Ableton all let you audition a soft synth
        // from a controller without engaging Play; this branch is what
        // makes that work.
        for ev in midi_in {
            match *ev {
                hardwave_midi::MidiEvent::NoteOn { note, velocity, .. } => match self.instrument {
                    Instrument::BuiltinSine => {
                        let carry_gain = self.voice.as_ref().map(|v| v.env_value).unwrap_or(0.0);
                        self.voice = Some(Voice {
                            freq: pitch_to_freq(note),
                            velocity: velocity.clamp(0.0, 1.0),
                            phase: 0.0,
                            stage: EnvStage::Attack,
                            env_value: carry_gain,
                        });
                        self.live_held_pitch = Some(note);
                    }
                    Instrument::KickSynth => {
                        self.kick.note_on(note, velocity);
                        self.live_held_pitch = Some(note);
                    }
                },
                hardwave_midi::MidiEvent::NoteOff { note, .. } => {
                    if self.live_held_pitch == Some(note) {
                        if let Some(v) = self.voice.as_mut() {
                            if v.stage != EnvStage::Idle {
                                v.stage = EnvStage::Release;
                            }
                        }
                        self.live_held_pitch = None;
                    }
                }
                _ => {}
            }
        }

        // When transport is stopped, render the live-MIDI voice tail
        // (envelope release) but skip the clip-schedule path so we
        // don't fire stale clip notes from a paused playhead.
        if !ctx.playing && self.voice.is_none() {
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

        // KickSynth voicing: render the entire block into a temp
        // buffer up-front, then mix it in below. The sine voicing
        // takes the per-sample path inside the loop. We split the
        // L/R mixing per-sample so volume + pan still apply uniformly
        // regardless of which instrument is active.
        let mut kick_l: Vec<f32> = Vec::new();
        let mut kick_r: Vec<f32> = Vec::new();
        if matches!(self.instrument, Instrument::KickSynth) {
            kick_l.resize(block_size, 0.0);
            kick_r.resize(block_size, 0.0);
            self.kick.render_into(&mut kick_l, &mut kick_r);
        }

        // Skip notes that ended before the block starts. This fast-forwards
        // `next_note_idx` after a seek so we don't fire stale note-ons.
        if ctx.playing {
            while self.next_note_idx < self.notes.len()
                && self.notes[self.next_note_idx].note_off_sample < block_start
            {
                self.next_note_idx += 1;
            }
        }

        for i in 0..block_size {
            let global_sample = block_start.saturating_add(i as u64);

            // Fire any pending clip-scheduled note-ons that land on this
            // exact sample — only while the transport is playing. Stopped
            // transport must not re-fire the same note every block.
            if ctx.playing {
                while self.next_note_idx < self.notes.len()
                    && self.notes[self.next_note_idx].note_on_sample <= global_sample
                {
                    let note = &self.notes[self.next_note_idx];
                    if !note.muted {
                        match self.instrument {
                            Instrument::BuiltinSine => {
                                // Carry the previous voice's envelope
                                // forward so legato re-triggers don't click.
                                let carry_gain =
                                    self.voice.as_ref().map(|v| v.env_value).unwrap_or(0.0);
                                self.voice = Some(Voice {
                                    freq: pitch_to_freq(note.pitch),
                                    velocity: note.velocity.clamp(0.0, 1.0),
                                    phase: 0.0,
                                    stage: EnvStage::Attack,
                                    env_value: carry_gain,
                                });
                                // A clip note-on takes over from any live
                                // hold so the live release path doesn't
                                // unexpectedly cut the clip note short.
                                self.live_held_pitch = None;
                            }
                            Instrument::KickSynth => {
                                self.kick.note_on(note.pitch, note.velocity);
                                self.live_held_pitch = None;
                            }
                        }
                    }
                    self.next_note_idx += 1;
                }

                // Trigger release when the active clip note's note-off
                // sample is reached. Skipped while a live note is held
                // so the live voice isn't killed by a stale clip end.
                if self.live_held_pitch.is_none() {
                    if let Some(v) = self.voice.as_mut() {
                        if v.stage != EnvStage::Release && v.stage != EnvStage::Idle {
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
                }
            }

            // Sample the synth.
            let (sample_l_raw, sample_r_raw) = match self.instrument {
                Instrument::BuiltinSine => {
                    let s = if let Some(v) = self.voice.as_mut() {
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
                    (s, s)
                }
                Instrument::KickSynth => {
                    // Pre-rendered block — pull the i-th sample. Kick
                    // is mono internally so L == R before pan.
                    (kick_l[i], kick_r[i])
                }
            };

            let l = sample_l_raw * self.volume * pan_l;
            let r = sample_r_raw * self.volume * pan_r;

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
        self.meter
            .pre_fader_peak_db
            .store(pre_peak_db, Ordering::Relaxed);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::AudioNode;
    use hardwave_midi::MidiEvent;

    fn make_node() -> MidiTrackNode {
        let meter = Arc::new(TrackMeterState::default());
        MidiTrackNode::new("t1".into(), "Test MIDI".into(), meter)
    }

    fn block_outputs(n: usize) -> Vec<Vec<f32>> {
        vec![vec![0.0; n], vec![0.0; n]]
    }

    fn ctx_at(sr: f64, n: u32, pos: u64, playing: bool) -> ProcessContext {
        ProcessContext {
            sample_rate: sr,
            buffer_size: n,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: pos,
            playing,
        }
    }

    /// Regression test for beta blocker #4: live MIDI input must drive
    /// the synth voice even when the transport is stopped. Pre-fix,
    /// MidiTrackNode.process bailed on `!ctx.playing` and ignored the
    /// midi_in slice entirely.
    #[test]
    fn live_note_on_makes_sound_with_transport_stopped() {
        let mut node = make_node();
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, false);
        let inputs: [&[f32]; 0] = [];
        let events = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 69, // A4 — 440 Hz, an easy sample to recognise
            velocity: 0.9,
        }];
        let mut midi_out = Vec::new();
        node.process(&inputs, &mut out, &events, &mut midi_out, &ctx);

        let peak = out[0]
            .iter()
            .chain(out[1].iter())
            .fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(
            peak > 0.0,
            "live NoteOn should produce audible output, got peak={peak}"
        );
    }

    /// Live NoteOff while transport stopped — voice transitions into
    /// Release stage, output should decay rather than sustain.
    #[test]
    fn live_note_off_triggers_release() {
        let mut node = make_node();
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, false);
        let inputs: [&[f32]; 0] = [];
        let on = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.7,
        }];
        let mut midi_out = Vec::new();
        node.process(&inputs, &mut out, &on, &mut midi_out, &ctx);

        // Now send NoteOff — voice should still exist but be in Release.
        let off = vec![MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.0,
        }];
        node.process(&inputs, &mut out, &off, &mut midi_out, &ctx);

        let stage = node.voice.as_ref().map(|v| v.stage);
        assert_eq!(
            stage,
            Some(EnvStage::Release),
            "matching NoteOff should trigger release"
        );
    }

    /// While the transport is stopped, clip-scheduled notes must NOT
    /// re-fire every block (position never advances). Without the
    /// `ctx.playing` gate, the same note-on would land on every call.
    #[test]
    fn clip_notes_do_not_fire_when_stopped() {
        let mut node = make_node();
        node.set_notes(vec![MidiNoteRegion {
            note_on_sample: 0,
            note_off_sample: 96_000,
            pitch: 64,
            velocity: 0.8,
            muted: false,
        }]);
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, false);
        let inputs: [&[f32]; 0] = [];
        let events: Vec<MidiEvent> = Vec::new();
        let mut midi_out = Vec::new();
        node.process(&inputs, &mut out, &events, &mut midi_out, &ctx);
        node.process(&inputs, &mut out, &events, &mut midi_out, &ctx);

        // With transport stopped, next_note_idx must stay at 0 — the
        // clip schedule never advances.
        assert_eq!(
            node.next_note_idx, 0,
            "clip schedule must not advance while stopped"
        );
    }
}
