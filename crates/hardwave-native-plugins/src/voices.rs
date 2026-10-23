//! Polyphony, mono and legato for the built-in synths.
//!
//! 3xOSC, FM and Wavetable played every note on one of eight voices, always:
//! a bass line could not be played mono, a lead could not slide from note to
//! note, and a pad could not have more than eight notes. Each synth now has
//! three more parameters, shared through this module:
//!
//! * Voice mode: Poly, Mono (one voice, every note starts the envelope again)
//!   or Legato (one voice, a note played over a held one only moves the pitch).
//! * Voices: how many notes Poly plays at once, 1 to 16.
//! * Glide: how long Mono and Legato take to slide to a new note.
//!
//! The voices are made when the synth starts, all sixteen, so nothing is
//! allocated on the audio thread when the count changes.

use crate::format;

/// Voices each synth makes up front; the Voices parameter picks how many of
/// them Poly uses.
pub const MAX_VOICES: usize = 16;

pub const MODE_NAMES: [&str; 3] = ["Poly", "Mono", "Legato"];

/// Glide time at the top of its knob, in seconds.
const GLIDE_MAX_S: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceMode {
    Poly,
    Mono,
    Legato,
}

/// The three shared settings, with the conversions to and from the 0..1
/// values the host stores.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceSettings {
    pub mode: VoiceMode,
    /// Notes Poly plays at once, 1..=MAX_VOICES.
    pub voices: usize,
    /// Seconds Mono and Legato take to reach a new note.
    pub glide_s: f32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            mode: VoiceMode::Poly,
            voices: 8,
            glide_s: 0.05,
        }
    }
}

impl VoiceSettings {
    pub fn mode_from_norm(v: f64) -> VoiceMode {
        match format::option_index(v, MODE_NAMES.len()) {
            0 => VoiceMode::Poly,
            1 => VoiceMode::Mono,
            _ => VoiceMode::Legato,
        }
    }

    pub fn mode_to_norm(mode: VoiceMode) -> f64 {
        match mode {
            VoiceMode::Poly => 0.0,
            VoiceMode::Mono => 0.5,
            VoiceMode::Legato => 1.0,
        }
    }

    pub fn voices_from_norm(v: f64) -> usize {
        (1.0 + v.clamp(0.0, 1.0) * (MAX_VOICES - 1) as f64).round() as usize
    }

    pub fn voices_to_norm(n: usize) -> f64 {
        (n.clamp(1, MAX_VOICES) - 1) as f64 / (MAX_VOICES - 1) as f64
    }

    pub fn glide_from_norm(v: f64) -> f32 {
        (v.clamp(0.0, 1.0) * GLIDE_MAX_S) as f32
    }

    pub fn glide_to_norm(s: f32) -> f64 {
        (s as f64 / GLIDE_MAX_S).clamp(0.0, 1.0)
    }

    /// Text for each of the three, by which one it is: 0 mode, 1 voices,
    /// 2 glide.
    pub fn text(which: u32, v: f64) -> Option<String> {
        match which {
            0 => format::option_text(v, &MODE_NAMES),
            1 => Some(Self::voices_from_norm(v).to_string()),
            2 => Some(format::secs(Self::glide_from_norm(v) as f64)),
            _ => None,
        }
    }

    /// The three as `"vm":..,"vn":..,"vg":..` for a synth's saved state.
    pub fn state_fields(&self) -> String {
        format!(
            "\"vm\":{},\"vn\":{},\"vg\":{}",
            Self::mode_to_norm(self.mode),
            self.voices,
            self.glide_s
        )
    }

    /// Read them back from a saved state; a state from before they existed
    /// keeps the defaults.
    pub fn read_state(&mut self, read: impl Fn(&str) -> Option<f32>) {
        if let Some(v) = read("vm") {
            self.mode = Self::mode_from_norm(v as f64);
        }
        if let Some(v) = read("vn") {
            self.voices = (v.round() as usize).clamp(1, MAX_VOICES);
        }
        if let Some(v) = read("vg") {
            self.glide_s = v.clamp(0.0, GLIDE_MAX_S as f32);
        }
    }
}

/// What a mono or legato synth does with a key going down or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonoStep {
    /// Play this note on the voice and start its envelope again.
    Start(u8),
    /// Move the voice to this note and keep its envelope going.
    Glide(u8),
    /// Let the voice go.
    Release,
    /// Nothing changes for the voice.
    Nothing,
}

/// Keys held down, oldest first, so letting go of the newest goes back to
/// the one held before it, as a mono synth does.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeldNotes {
    notes: [u8; MAX_VOICES],
    len: usize,
}

impl HeldNotes {
    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn last(&self) -> Option<u8> {
        self.len.checked_sub(1).map(|i| self.notes[i])
    }

    fn remove(&mut self, note: u8) {
        if let Some(i) = self.notes[..self.len].iter().position(|&n| n == note) {
            self.notes.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }

    /// A key goes down.
    pub fn press(&mut self, mode: VoiceMode, note: u8) -> MonoStep {
        let sounding = !self.is_empty();
        self.remove(note);
        if self.len == MAX_VOICES {
            // Sixteen keys held: the oldest is forgotten.
            self.notes.copy_within(1.., 0);
            self.len -= 1;
        }
        self.notes[self.len] = note;
        self.len += 1;
        if mode == VoiceMode::Legato && sounding {
            MonoStep::Glide(note)
        } else {
            MonoStep::Start(note)
        }
    }

    /// A key comes up.
    pub fn lift(&mut self, mode: VoiceMode, note: u8) -> MonoStep {
        let was_newest = self.last() == Some(note);
        self.remove(note);
        if !was_newest {
            return MonoStep::Nothing;
        }
        match self.last() {
            Some(back) if mode == VoiceMode::Legato => MonoStep::Glide(back),
            Some(back) => MonoStep::Start(back),
            None => MonoStep::Release,
        }
    }
}

/// A pitch, as a MIDI note number with fractions, that slides to where it
/// is sent over a set time.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glide {
    now: f32,
    target: f32,
    step: f32,
    /// Samples until it arrives; counted, so float steps cannot miss.
    left: u32,
}

impl Glide {
    pub fn jump(&mut self, note: f32) {
        self.now = note;
        self.target = note;
        self.step = 0.0;
        self.left = 0;
    }

    /// Slide to `note` in `seconds`; no time is a jump.
    pub fn slide(&mut self, note: f32, seconds: f32, sample_rate: f32) {
        let samples = seconds * sample_rate;
        if samples < 1.0 {
            self.jump(note);
            return;
        }
        self.target = note;
        self.left = samples.round() as u32;
        self.step = (note - self.now) / self.left as f32;
    }

    pub fn gliding(&self) -> bool {
        self.left > 0
    }

    pub fn pitch(&self) -> f32 {
        self.now
    }

    /// One sample on; returns the pitch.
    pub fn advance(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.now = if self.left == 0 {
                self.target
            } else {
                self.now + self.step
            };
        }
        self.now
    }
}

/// A voice to steal or use among the first `limit`: an idle one, else the
/// one `rank` puts lowest (furthest into its release, say).
pub fn pick_voice<V>(
    voices: &[V],
    limit: usize,
    idle: impl Fn(&V) -> bool,
    rank: impl Fn(&V) -> u8,
) -> usize {
    let pool = &voices[..limit.clamp(1, MAX_VOICES).min(voices.len())];
    if let Some(i) = pool.iter().position(&idle) {
        return i;
    }
    pool.iter()
        .enumerate()
        .min_by_key(|(_, v)| rank(v))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// MIDI note (with fractions) to Hz.
pub fn note_hz(note: f32) -> f32 {
    440.0 * 2.0_f32.powf((note - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_goes_back_to_the_key_still_held() {
        let mut held = HeldNotes::default();
        assert_eq!(held.press(VoiceMode::Mono, 60), MonoStep::Start(60));
        assert_eq!(held.press(VoiceMode::Mono, 64), MonoStep::Start(64));
        assert_eq!(held.lift(VoiceMode::Mono, 64), MonoStep::Start(60));
        assert_eq!(held.lift(VoiceMode::Mono, 60), MonoStep::Release);
    }

    #[test]
    fn legato_slides_over_a_held_key_and_starts_fresh_from_silence() {
        let mut held = HeldNotes::default();
        assert_eq!(held.press(VoiceMode::Legato, 60), MonoStep::Start(60));
        assert_eq!(held.press(VoiceMode::Legato, 67), MonoStep::Glide(67));
        assert_eq!(
            held.lift(VoiceMode::Legato, 60),
            MonoStep::Nothing,
            "not the newest"
        );
        assert_eq!(held.lift(VoiceMode::Legato, 67), MonoStep::Release);
        assert_eq!(held.press(VoiceMode::Legato, 62), MonoStep::Start(62));
    }

    #[test]
    fn a_glide_reaches_its_note_in_its_time() {
        let mut g = Glide::default();
        g.jump(60.0);
        g.slide(72.0, 0.01, 1000.0);
        for _ in 0..9 {
            g.advance();
        }
        assert!(g.gliding() && g.pitch() < 72.0);
        assert_eq!(g.advance(), 72.0);
        assert!(!g.gliding());
    }

    #[test]
    fn poly_uses_only_the_voices_it_is_given() {
        let busy = [true, true, false, false];
        assert_eq!(
            pick_voice(&busy, 2, |b| !*b, |_| 0),
            0,
            "both taken: steal within two"
        );
        assert_eq!(pick_voice(&busy, 4, |b| !*b, |_| 0), 2);
    }

    #[test]
    fn settings_round_trip_and_read_as_text() {
        let s = VoiceSettings::default();
        assert_eq!(
            VoiceSettings::voices_from_norm(VoiceSettings::voices_to_norm(s.voices)),
            8
        );
        assert_eq!(VoiceSettings::text(0, 0.5).as_deref(), Some("Mono"));
        assert_eq!(VoiceSettings::text(1, 1.0).as_deref(), Some("16"));
        let mut back = VoiceSettings::default();
        let state = format!(
            "{{{}}}",
            VoiceSettings {
                mode: VoiceMode::Legato,
                voices: 3,
                glide_s: 0.2
            }
            .state_fields()
        );
        back.read_state(|k| {
            let needle = format!("\"{k}\":");
            let i = state.find(&needle)?;
            let rest = &state[i + needle.len()..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            rest[..end].parse().ok()
        });
        assert_eq!(back.mode, VoiceMode::Legato);
        assert_eq!(back.voices, 3);
        assert!((back.glide_s - 0.2).abs() < 1e-6);
    }
}
