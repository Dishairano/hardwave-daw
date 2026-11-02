//! A plain sine voice for checking a controller.
//!
//! The first run's Controller step asks the producer to play a few keys. A
//! new song has no instrument, so the keys played nothing and the step could
//! only light a light. While the step is open this voice plays every note
//! that comes in, straight to the sound card: the keys are heard, so a
//! controller that works sounds like one.
//!
//! Mixed in after the master and the capture path like the test tone, so it
//! never reaches a recording, and an export's callback never has it on.

use hardwave_midi::MidiEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How many notes can sound at once. A chord or two; the next note takes
/// the voice that has sounded longest.
const VOICES: usize = 8;
/// The level of a note played at full velocity, per voice.
const VOICE_DBFS: f32 = -18.0;
const ATTACK_SECONDS: f32 = 0.005;
const RELEASE_SECONDS: f32 = 0.15;

/// Whether the voice is on. The window turns it on while the Controller
/// step shows and off when it closes.
#[derive(Clone, Default)]
pub struct CheckVoiceSwitch {
    on: Arc<AtomicBool>,
}

impl CheckVoiceSwitch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    sounding: bool,
    held: bool,
    channel: u8,
    note: u8,
    level: f32,
    envelope: f32,
    phase: f32,
    step: f32,
    /// When it started, for taking the oldest voice when all are busy.
    started: u64,
}

/// The voice on the audio thread: a fixed set of sines, nothing allocated.
pub struct CheckVoice {
    switch: CheckVoiceSwitch,
    voices: [Voice; VOICES],
    notes_started: u64,
}

impl CheckVoice {
    pub fn new(switch: CheckVoiceSwitch) -> Self {
        Self {
            switch,
            voices: [Voice::default(); VOICES],
            notes_started: 0,
        }
    }

    /// A voice nobody can turn on: what an export's engine callback gets.
    pub fn unwired() -> Self {
        Self::new(CheckVoiceSwitch::new())
    }

    /// Take the notes that came in this block.
    pub fn take_events(&mut self, events: &[MidiEvent], sample_rate: f64) {
        if !self.switch.is_on() {
            return;
        }
        for event in events {
            match *event {
                MidiEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                    ..
                } if velocity > 0.0 => self.start(channel, note, velocity, sample_rate),
                MidiEvent::NoteOn { channel, note, .. }
                | MidiEvent::NoteOff { channel, note, .. } => {
                    for voice in self.voices.iter_mut() {
                        if voice.sounding && voice.channel == channel && voice.note == note {
                            voice.held = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn start(&mut self, channel: u8, note: u8, velocity: f32, sample_rate: f64) {
        let slot = self
            .voices
            .iter()
            .position(|v| !v.sounding)
            .unwrap_or_else(|| {
                // All busy: the one that started first.
                let mut oldest = 0;
                for (i, v) in self.voices.iter().enumerate() {
                    if v.started < self.voices[oldest].started {
                        oldest = i;
                    }
                }
                oldest
            });
        self.notes_started += 1;
        let hz = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
        self.voices[slot] = Voice {
            sounding: true,
            held: true,
            channel,
            note,
            level: 10f32.powf(VOICE_DBFS / 20.0) * velocity.clamp(0.0, 1.0),
            envelope: 0.0,
            phase: 0.0,
            step: (std::f64::consts::TAU * hz / sample_rate.max(1.0)) as f32,
            started: self.notes_started,
        };
    }

    /// Whether a note is sounding, held or dying away.
    pub fn is_sounding(&self) -> bool {
        self.voices.iter().any(|v| v.sounding)
    }

    /// Add the sounding notes to an interleaved stereo block, in the middle.
    pub fn render(&mut self, output: &mut [f32], num_frames: usize, sample_rate: f64) {
        if !self.switch.is_on() {
            // Turned off: a note still sounding stops here rather than
            // hanging on until the step is opened again.
            for voice in self.voices.iter_mut() {
                voice.sounding = false;
            }
            return;
        }
        if !self.is_sounding() {
            return;
        }
        let rate = sample_rate.max(1.0) as f32;
        let rise = 1.0 / (ATTACK_SECONDS * rate);
        let fall = 1.0 / (RELEASE_SECONDS * rate);
        let frames = num_frames.min(output.len() / 2);
        for voice in self.voices.iter_mut().filter(|v| v.sounding) {
            for frame in 0..frames {
                if voice.held {
                    voice.envelope = (voice.envelope + rise).min(1.0);
                } else {
                    voice.envelope -= fall;
                    if voice.envelope <= 0.0 {
                        voice.sounding = false;
                        break;
                    }
                }
                let sample = voice.level * voice.envelope * voice.phase.sin();
                output[frame * 2] += sample;
                output[frame * 2 + 1] += sample;
                voice.phase = (voice.phase + voice.step) % std::f32::consts::TAU;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn on(note: u8, velocity: f32) -> MidiEvent {
        MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note,
            velocity,
        }
    }

    fn off(note: u8) -> MidiEvent {
        MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note,
            velocity: 0.0,
        }
    }

    fn block(voice: &mut CheckVoice, events: &[MidiEvent]) -> Vec<f32> {
        voice.take_events(events, RATE);
        let mut out = vec![0.0f32; 512 * 2];
        voice.render(&mut out, 512, RATE);
        out
    }

    fn peak(out: &[f32]) -> f32 {
        out.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn a_key_played_is_heard_while_the_step_is_open() {
        let switch = CheckVoiceSwitch::new();
        let mut voice = CheckVoice::new(switch.clone());
        assert_eq!(peak(&block(&mut voice, &[on(60, 1.0)])), 0.0, "off: silent");

        switch.set(true);
        let out = block(&mut voice, &[on(60, 1.0)]);
        assert!(peak(&out) > 0.05, "on: the key is heard");
        assert_eq!(out[0], out[1], "in the middle");
        assert!(
            peak(&out) <= 10f32.powf(-18.0 / 20.0) + 1e-4,
            "no louder than -18 dBFS"
        );
    }

    #[test]
    fn a_soft_key_is_quieter_than_a_hard_one() {
        let switch = CheckVoiceSwitch::new();
        switch.set(true);
        let mut hard = CheckVoice::new(switch.clone());
        let mut soft = CheckVoice::new(switch);
        let hard = peak(&block(&mut hard, &[on(60, 1.0)]));
        let soft = peak(&block(&mut soft, &[on(60, 0.25)]));
        assert!(soft < hard * 0.5, "soft {soft}, hard {hard}");
    }

    #[test]
    fn a_released_key_dies_away_and_stops() {
        let switch = CheckVoiceSwitch::new();
        switch.set(true);
        let mut voice = CheckVoice::new(switch);
        block(&mut voice, &[on(64, 1.0)]);
        block(&mut voice, &[off(64)]);
        assert!(voice.is_sounding(), "a short tail, not a cut");
        for _ in 0..30 {
            block(&mut voice, &[]);
        }
        assert!(!voice.is_sounding(), "then silence");
        assert_eq!(peak(&block(&mut voice, &[])), 0.0);
    }

    #[test]
    fn turning_it_off_stops_a_held_note() {
        let switch = CheckVoiceSwitch::new();
        switch.set(true);
        let mut voice = CheckVoice::new(switch.clone());
        block(&mut voice, &[on(60, 1.0)]);
        switch.set(false);
        assert_eq!(peak(&block(&mut voice, &[])), 0.0);
        switch.set(true);
        assert_eq!(peak(&block(&mut voice, &[])), 0.0, "nothing hangs on");
    }

    #[test]
    fn more_notes_than_voices_take_the_oldest() {
        let switch = CheckVoiceSwitch::new();
        switch.set(true);
        let mut voice = CheckVoice::new(switch);
        let chord: Vec<MidiEvent> = (60..60 + VOICES as u8 + 2).map(|n| on(n, 1.0)).collect();
        block(&mut voice, &chord);
        let notes: Vec<u8> = voice.voices.iter().map(|v| v.note).collect();
        assert!(
            notes.contains(&(60 + VOICES as u8 + 1)),
            "the last note plays"
        );
        assert!(!notes.contains(&60), "the first one made way: {notes:?}");
    }

    #[test]
    fn an_unwired_voice_never_plays() {
        let mut voice = CheckVoice::unwired();
        assert_eq!(peak(&block(&mut voice, &[on(60, 1.0)])), 0.0);
    }
}
