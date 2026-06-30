//! MIDI track audio node — turns scheduled MIDI notes into audio.
//!
//! Polyphonic built-in synthesizer that consumes pre-resolved [`MidiNoteRegion`]s
//! (notes already mapped to absolute sample positions) and writes a mono-summed
//! sine wave with a linear ADSR envelope per voice into both stereo channels.
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

/// One playing note. The synth keeps a pool of up to [`MAX_VOICES`] of
/// these, so chords and overlapping notes sound together; the pool prunes
/// finished voices each block and steals the oldest when full.
#[derive(Debug, Clone)]
struct Voice {
    /// MIDI pitch this voice is playing — used to match note-offs (poly).
    pitch: u8,
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
    /// For clip-scheduled notes, the absolute sample at which to release.
    /// `None` for live/held notes (released by an explicit NoteOff).
    off_sample: Option<u64>,
}

/// Max simultaneous built-in-synth voices. Beyond this, the oldest voice
/// is stolen — bounds CPU while comfortably covering chords.
const MAX_VOICES: usize = 16;

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
    /// Active built-in-synth voices (polyphonic — a chord plays as many
    /// voices). Live notes carry `off_sample = None` (released by an
    /// explicit NoteOff); clip notes carry their scheduled release sample.
    voices: Vec<Voice>,
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
    /// Oscillator shape for the built-in synth (ignored by KickSynth).
    waveform: Waveform,
    kick: hardwave_dsp::kick_synth::KickSynth,
    /// Pre-fader insert chain so effects (filter, reverb, distortion,
    /// the vocoder, VST3/CLAP…) can be placed directly on an instrument
    /// track — synth → inserts → fader → pan, the standard channel strip.
    chain: crate::insert_chain::InsertChain,
    chain_scratch: crate::insert_chain::Scratch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instrument {
    /// Default — built-in synth (waveform chosen via [`Waveform`]) + ADSR.
    BuiltinSine,
    /// Native KickSynth — every note-on retriggers the kick voice
    /// regardless of MIDI pitch. Velocity scales the layer mix.
    KickSynth,
}

/// Oscillator shape for the built-in synth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Waveform {
    #[default]
    Sine,
    Saw,
    Square,
    Triangle,
}

impl Waveform {
    /// One sample for an oscillator `phase` in `[0, TAU)`.
    fn sample(self, phase: f32) -> f32 {
        let p = (phase / TWO_PI).rem_euclid(1.0); // 0..1
        match self {
            Waveform::Sine => phase.sin(),
            Waveform::Saw => 2.0 * p - 1.0,
            Waveform::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
        }
    }
}

impl MidiTrackNode {
    pub fn new(track_id: String, name: String, meter: Arc<TrackMeterState>) -> Self {
        Self {
            track_id,
            name,
            notes: Vec::new(),
            next_note_idx: 0,
            voices: Vec::new(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            meter,
            rms_smooth: 0.0,
            instrument: Instrument::BuiltinSine,
            waveform: Waveform::Sine,
            kick: hardwave_dsp::kick_synth::KickSynth::new(48_000.0),
            chain: crate::insert_chain::InsertChain::new(),
            chain_scratch: crate::insert_chain::Scratch::default(),
        }
    }

    /// Switch which native instrument voices this track. Cheap — flag
    /// flip + reset of the relevant voice state. Audio thread sees the
    /// change on the next block.
    pub fn set_instrument(&mut self, kind: Instrument, sample_rate: f32) {
        self.instrument = kind;
        match kind {
            Instrument::BuiltinSine => {
                self.voices.clear();
            }
            Instrument::KickSynth => {
                self.kick = hardwave_dsp::kick_synth::KickSynth::new(sample_rate.max(1.0));
            }
        }
    }

    /// Set the built-in synth's oscillator shape. Audio thread picks it
    /// up on the next block (existing voices switch shape immediately).
    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    /// Start a built-in-synth voice. Steals a voice (preferring an
    /// already-releasing/idle one, else the oldest) when at the polyphony
    /// cap so a fresh chord never silently drops notes.
    fn start_voice(&mut self, pitch: u8, velocity: f32, off_sample: Option<u64>) {
        if self.voices.len() >= MAX_VOICES {
            let steal = self
                .voices
                .iter()
                .position(|v| matches!(v.stage, EnvStage::Idle | EnvStage::Release))
                .unwrap_or(0);
            self.voices.remove(steal);
        }
        self.voices.push(Voice {
            pitch,
            freq: pitch_to_freq(pitch),
            velocity,
            phase: 0.0,
            stage: EnvStage::Attack,
            env_value: 0.0,
            off_sample,
        });
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
                        // Live note → a held voice (off_sample None).
                        self.start_voice(note, velocity.clamp(0.0, 1.0), None);
                    }
                    Instrument::KickSynth => {
                        self.kick.note_on(note, velocity);
                    }
                },
                hardwave_midi::MidiEvent::NoteOff { note, .. } => {
                    // Release held (live) voices of this pitch. Clip-driven
                    // voices (off_sample Some) release on their own schedule
                    // so a live release can't cut a playing clip note.
                    for v in self.voices.iter_mut() {
                        if v.pitch == note
                            && v.off_sample.is_none()
                            && v.stage != EnvStage::Idle
                            && v.stage != EnvStage::Release
                        {
                            v.stage = EnvStage::Release;
                        }
                    }
                }
                _ => {}
            }
        }

        // When transport is stopped, render the live-MIDI voice tail
        // (envelope release) but skip the clip-schedule path so we
        // don't fire stale clip notes from a paused playhead.
        if !ctx.playing && self.voices.is_empty() {
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
                    // Clone the note's fields so the borrow on `self.notes`
                    // ends before `start_voice` borrows `self` mutably.
                    let n = self.notes[self.next_note_idx].clone();
                    if !n.muted {
                        match self.instrument {
                            Instrument::BuiltinSine => {
                                // Polyphonic: each clip note adds a voice
                                // that releases on its own note-off sample.
                                self.start_voice(
                                    n.pitch,
                                    n.velocity.clamp(0.0, 1.0),
                                    Some(n.note_off_sample),
                                );
                            }
                            Instrument::KickSynth => {
                                self.kick.note_on(n.pitch, n.velocity);
                            }
                        }
                    }
                    self.next_note_idx += 1;
                }

                // Release clip-scheduled voices whose note-off sample has
                // arrived. Each voice tracks its own off_sample, so
                // overlapping/chord notes release independently.
                for v in self.voices.iter_mut() {
                    if let Some(off) = v.off_sample {
                        if off <= global_sample
                            && v.stage != EnvStage::Release
                            && v.stage != EnvStage::Idle
                        {
                            v.stage = EnvStage::Release;
                        }
                    }
                }
            }

            // Sample the synth.
            let (sample_l_raw, sample_r_raw) = match self.instrument {
                Instrument::BuiltinSine => {
                    // Sum every active voice (polyphony). Idle voices are
                    // pruned after the block.
                    let mut s = 0.0_f32;
                    for v in self.voices.iter_mut() {
                        if matches!(v.stage, EnvStage::Idle) {
                            continue;
                        }
                        let env = Self::step_envelope(v, sr);
                        let osc = self.waveform.sample(v.phase);
                        v.phase += TWO_PI * v.freq / sr;
                        if v.phase >= TWO_PI {
                            v.phase -= TWO_PI;
                        }
                        s += osc * env * v.velocity;
                    }
                    (s, s)
                }
                Instrument::KickSynth => {
                    // Pre-rendered block — pull the i-th sample. Kick
                    // is mono internally so L == R before pan.
                    (kick_l[i], kick_r[i])
                }
            };

            // Write the DRY synth signal (pre-fader, pre-pan). Inserts
            // run on this below; the fader + pan are applied afterward.
            if let Some(buf) = outputs.get_mut(0) {
                buf[i] = sample_l_raw;
            }
            if let Some(buf) = outputs.get_mut(1) {
                buf[i] = sample_r_raw;
            }
        }

        // Prune voices whose envelope finished this block.
        self.voices.retain(|v| !matches!(v.stage, EnvStage::Idle));

        // Route this block's notes (clip schedule + live input) to the
        // insert chain so a hosted instrument plug-in (VST3 / CLAP synth
        // or sampler) can generate audio from them. Timing is block-local.
        let mut block_midi: Vec<hardwave_midi::MidiEvent> = midi_in.to_vec();
        if ctx.playing {
            let block_end = block_start.saturating_add(block_size as u64);
            for n in &self.notes {
                if n.muted {
                    continue;
                }
                if n.note_on_sample >= block_start && n.note_on_sample < block_end {
                    block_midi.push(hardwave_midi::MidiEvent::NoteOn {
                        timing: (n.note_on_sample - block_start) as u32,
                        channel: 0,
                        note: n.pitch,
                        velocity: n.velocity,
                    });
                }
                if n.note_off_sample >= block_start && n.note_off_sample < block_end {
                    block_midi.push(hardwave_midi::MidiEvent::NoteOff {
                        timing: (n.note_off_sample - block_start) as u32,
                        channel: 0,
                        note: n.pitch,
                        velocity: 0.0,
                    });
                }
            }
        }

        // If the chain hosts an instrument plug-in, it is the sound source:
        // discard the built-in synth's dry signal so the two don't double.
        let has_instrument = self.chain.slots.iter().any(|s| {
            s.plugin.descriptor().category
                == hardwave_plugin_host::types::PluginCategory::Instrument
        });
        if has_instrument {
            for buf in outputs.iter_mut() {
                for s in buf.iter_mut() {
                    *s = 0.0;
                }
            }
        }

        // Pre-fader insert FX: synth → inserts → fader → pan. Empty chain
        // is a no-op, so tracks without FX pay nothing.
        if !self.chain.slots.is_empty() {
            if let [left, right, ..] = outputs {
                self.chain.process(
                    left,
                    right,
                    block_size,
                    &mut self.chain_scratch,
                    &block_midi,
                );
            }
        }

        // Apply fader + pan and compute meters on the post-FX signal.
        for i in 0..block_size {
            let dry_l = outputs.first().map(|b| b[i]).unwrap_or(0.0);
            let dry_r = outputs.get(1).map(|b| b[i]).unwrap_or(dry_l);
            let l = dry_l * self.volume * pan_l;
            let r = dry_r * self.volume * pan_r;
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
        self.voices.clear();
        self.next_note_idx = 0;
        self.rms_smooth = 0.0;
    }

    fn snapshot_plugin_states(&self) -> Vec<(String, Vec<u8>)> {
        self.chain
            .slots
            .iter()
            .map(|slot| (slot.slot_id.clone(), slot.plugin.get_state()))
            .collect()
    }

    fn apply_insert_command(
        &mut self,
        cmd: crate::insert_chain::InsertCommand,
        graveyard: &mut crate::insert_chain::PluginGraveyardSender,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        use crate::insert_chain::InsertCommand;
        match cmd {
            InsertCommand::Add { slot, .. } => {
                if let Err(e) = self.chain.push_slot(slot, sample_rate, max_block_size) {
                    log::warn!("midi track {} insert add failed: {e}", self.track_id);
                }
            }
            InsertCommand::Remove { slot_id, .. } => {
                if let Some(slot) = self.chain.take_slot(&slot_id) {
                    let _ = graveyard.try_bury(slot);
                }
            }
            InsertCommand::Reorder { from, to, .. } => self.chain.reorder(from, to),
            InsertCommand::SetEnabled {
                slot_id, enabled, ..
            } => {
                self.chain.set_enabled(&slot_id, enabled);
            }
            InsertCommand::SetWet { slot_id, wet, .. } => {
                self.chain.set_wet(&slot_id, wet);
            }
            InsertCommand::SetParameter {
                slot_id,
                param_id,
                value,
                ..
            } => {
                self.chain.set_parameter(&slot_id, param_id, value);
            }
            InsertCommand::SetState { slot_id, bytes, .. } => {
                self.chain.set_state(&slot_id, &bytes);
            }
        }
    }

    fn take_chain(&mut self) -> Option<crate::insert_chain::InsertChain> {
        let taken = std::mem::take(&mut self.chain);
        if taken.slots.is_empty() {
            None
        } else {
            Some(taken)
        }
    }

    fn restore_chain(&mut self, chain: crate::insert_chain::InsertChain) {
        self.chain = chain;
    }

    fn push_offline_slot(
        &mut self,
        slot: crate::insert_chain::LiveSlot,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        if let Err(e) = self.chain.push_slot(slot, sample_rate, max_block_size) {
            log::warn!(
                "midi track {} offline insert add failed: {e}",
                self.track_id
            );
        }
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

    /// An insert effect placed on an instrument track must process its
    /// audio (pre-fix, MidiTrackNode had no chain so FX were dropped).
    /// A "silencer" insert should zero the synth output.
    #[test]
    fn insert_chain_applies_to_instrument_output() {
        use hardwave_plugin_host::types::{
            HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
        };
        struct Silencer(PluginDescriptor);
        impl HostedPlugin for Silencer {
            fn descriptor(&self) -> &PluginDescriptor {
                &self.0
            }
            fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
                Ok(())
            }
            fn deactivate(&mut self) {}
            fn process(
                &mut self,
                _i: &[&[f32]],
                outputs: &mut [Vec<f32>],
                _mi: &[MidiEvent],
                _mo: &mut Vec<MidiEvent>,
                n: usize,
            ) {
                for o in outputs.iter_mut() {
                    o.clear();
                    o.resize(n, 0.0);
                }
            }
            fn get_parameter_count(&self) -> u32 {
                0
            }
            fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
                None
            }
            fn get_parameter_value(&self, _i: u32) -> f64 {
                0.0
            }
            fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
            fn get_state(&self) -> Vec<u8> {
                Vec::new()
            }
            fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
                Ok(())
            }
            fn latency_samples(&self) -> u32 {
                0
            }
            fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
                false
            }
            fn close_editor(&mut self) {}
            fn has_editor(&self) -> bool {
                false
            }
        }
        let desc = PluginDescriptor {
            id: "test.silencer".into(),
            name: "Silencer".into(),
            vendor: "t".into(),
            version: "1".into(),
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from("<native>"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        };

        let mut node = make_node();
        node.push_offline_slot(
            crate::insert_chain::LiveSlot {
                slot_id: "s".into(),
                plugin: Box::new(Silencer(desc)),
                enabled: true,
                wet: 1.0,
            },
            48_000.0,
            256,
        );
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, false);
        let inputs: [&[f32]; 0] = [];
        let events = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 69,
            velocity: 0.9,
        }];
        node.process(&inputs, &mut out, &events, &mut Vec::new(), &ctx);
        let peak = out[0]
            .iter()
            .chain(out[1].iter())
            .fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(
            peak < 1e-6,
            "silencer insert must zero the instrument output, got peak={peak}"
        );
    }

    /// A hosted instrument plug-in in the chain must generate audio from
    /// the track's MIDI, and the built-in synth must be bypassed (so the
    /// VST is the sound source). The instrument outputs a DC level while a
    /// note is held — distinguishable from the built-in sine's oscillation.
    #[test]
    fn instrument_plugin_in_chain_is_the_sound_source() {
        use hardwave_plugin_host::types::{
            HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
        };
        struct ToneGen {
            desc: PluginDescriptor,
            active: bool,
        }
        impl HostedPlugin for ToneGen {
            fn descriptor(&self) -> &PluginDescriptor {
                &self.desc
            }
            fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
                Ok(())
            }
            fn deactivate(&mut self) {}
            fn process(
                &mut self,
                _i: &[&[f32]],
                outputs: &mut [Vec<f32>],
                midi_in: &[MidiEvent],
                _mo: &mut Vec<MidiEvent>,
                n: usize,
            ) {
                for ev in midi_in {
                    match ev {
                        MidiEvent::NoteOn { .. } => self.active = true,
                        MidiEvent::NoteOff { .. } => self.active = false,
                        _ => {}
                    }
                }
                let v = if self.active { 0.5 } else { 0.0 };
                for o in outputs.iter_mut() {
                    o.clear();
                    o.resize(n, v);
                }
            }
            fn get_parameter_count(&self) -> u32 {
                0
            }
            fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
                None
            }
            fn get_parameter_value(&self, _i: u32) -> f64 {
                0.0
            }
            fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
            fn get_state(&self) -> Vec<u8> {
                Vec::new()
            }
            fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
                Ok(())
            }
            fn latency_samples(&self) -> u32 {
                0
            }
            fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
                false
            }
            fn close_editor(&mut self) {}
            fn has_editor(&self) -> bool {
                false
            }
        }
        let desc = PluginDescriptor {
            id: "test.tonegen".into(),
            name: "ToneGen".into(),
            vendor: "t".into(),
            version: "1".into(),
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from("<native>"),
            category: PluginCategory::Instrument,
            num_inputs: 0,
            num_outputs: 2,
            has_midi_input: true,
            has_editor: false,
        };

        let mut node = make_node();
        node.push_offline_slot(
            crate::insert_chain::LiveSlot {
                slot_id: "inst".into(),
                plugin: Box::new(ToneGen {
                    desc,
                    active: false,
                }),
                enabled: true,
                wet: 1.0,
            },
            48_000.0,
            256,
        );
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, true);
        let inputs: [&[f32]; 0] = [];
        let events = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 1.0,
        }];
        node.process(&inputs, &mut out, &events, &mut Vec::new(), &ctx);

        // Output should be the ToneGen's DC level through fader+pan
        // (0.5 * unity * cos(pi/4) ≈ 0.3536), not an oscillating sine.
        let peak = out[0].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(
            peak > 0.3,
            "instrument plug-in must produce audio from MIDI, got peak={peak}"
        );
        let flat = (out[0][32] - out[0][200]).abs();
        assert!(
            flat < 1e-4,
            "instrument (DC) should replace the built-in sine, got delta={flat}"
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

        let stage = node.voices.first().map(|v| v.stage);
        assert_eq!(
            stage,
            Some(EnvStage::Release),
            "matching NoteOff should trigger release"
        );
    }

    /// The built-in synth must be polyphonic — a chord plays every note,
    /// not just the last one (pre-fix it kept a single mono voice).
    #[test]
    fn builtin_synth_is_polyphonic() {
        let mut node = make_node();
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, false);
        let inputs: [&[f32]; 0] = [];
        let chord = vec![
            MidiEvent::NoteOn {
                timing: 0,
                channel: 0,
                note: 60,
                velocity: 0.8,
            },
            MidiEvent::NoteOn {
                timing: 0,
                channel: 0,
                note: 64,
                velocity: 0.8,
            },
            MidiEvent::NoteOn {
                timing: 0,
                channel: 0,
                note: 67,
                velocity: 0.8,
            },
        ];
        node.process(&inputs, &mut out, &chord, &mut Vec::new(), &ctx);
        assert_eq!(node.voices.len(), 3, "a 3-note chord must hold 3 voices");
        let peak = out[0]
            .iter()
            .chain(out[1].iter())
            .fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(peak > 0.0, "chord should be audible");
    }

    #[test]
    fn waveform_shapes_are_correct() {
        use std::f32::consts::PI;
        // Sine ≈ 0 at phase 0; saw = -1 at phase 0; square = +1 in first
        // half; triangle peaks at the midpoint (phase = PI).
        assert!(Waveform::Sine.sample(0.0).abs() < 1e-6);
        assert!((Waveform::Saw.sample(0.0) + 1.0).abs() < 1e-6);
        assert!((Waveform::Square.sample(0.1) - 1.0).abs() < 1e-6);
        assert!(Waveform::Square.sample(PI + 0.1) < 0.0);
        assert!((Waveform::Triangle.sample(PI) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn non_sine_waveform_produces_output() {
        let mut node = make_node();
        node.set_waveform(Waveform::Saw);
        let mut out = block_outputs(256);
        let ctx = ctx_at(48_000.0, 256, 0, false);
        let inputs: [&[f32]; 0] = [];
        let on = vec![MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 57,
            velocity: 0.9,
        }];
        node.process(&inputs, &mut out, &on, &mut Vec::new(), &ctx);
        let peak = out[0].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.0, "saw waveform should be audible");
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
