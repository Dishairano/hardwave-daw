use serde::{Deserialize, Serialize};

use crate::track::TrackId;

pub type ClipId = String;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
    SCurve,
    Logarithmic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: ClipId,
    pub name: String,
    /// Key into the engine's audio pool. Despite the name this is an
    /// opaque id, not a location: `import_audio_file` stores what
    /// `load_audio_file` returned, and playback looks the buffer up by it.
    /// Keeping it stable is what lets a moved file be relinked without
    /// touching any clip. The file it came from is `source_file`.
    pub source_path: String,
    /// SHA-256 of the source file at import, or empty for clips written
    /// before it was recorded. Identifies a file that moved or was renamed.
    pub source_hash: String,
    /// Offset into source file in samples.
    pub source_start: u64,
    /// End position in source file in samples.
    pub source_end: u64,
    /// Clip gain in dB.
    pub gain_db: f64,
    /// Fade in length in ticks.
    pub fade_in_ticks: u64,
    /// Fade out length in ticks.
    pub fade_out_ticks: u64,
    pub muted: bool,
    /// Play this clip with the source read backwards.
    #[serde(default)]
    pub reversed: bool,
    /// Pitch shift in semitones. Combined with stretch via resampling.
    #[serde(default)]
    pub pitch_semitones: f64,
    /// Time-stretch ratio. 1.0 = realtime, 2.0 = half speed (longer), 0.5 = double speed.
    #[serde(default = "default_stretch_ratio")]
    pub stretch_ratio: f64,
    #[serde(default)]
    pub fade_in_curve: FadeCurve,
    #[serde(default)]
    pub fade_out_curve: FadeCurve,
    /// Transient-anchored warp markers: a piecewise-linear timeline→source
    /// map. Each marker pins `clip_tick` (relative to the clip start, 960
    /// PPQ) to `source_sample` in the audio file; playback interpolates
    /// linearly between neighbouring markers. Kept sorted by `clip_tick`,
    /// unique per tick. Empty = classic single-ratio stretch via
    /// `stretch_ratio` (legacy projects deserialize to empty).
    #[serde(default)]
    pub warp_markers: Vec<WarpMarker>,
    /// Where the audio actually lives on disk.
    ///
    /// Without this a saved project had no record of its samples at all:
    /// `source_path` holds a pool id, so reopening a project tried to open a
    /// file named after a hash, every source failed to load, and every audio
    /// clip came back silent. Empty on projects written before this field
    /// existed, which the engine falls back to handling by treating
    /// `source_path` as a path.
    ///
    /// Fields added later come after this one, like `Project::timeline_state`:
    /// .hwp is MessagePack written by `rmp_serde::to_vec`, which encodes a
    /// struct positionally, so a field inserted before an older one shifts
    /// every field after it.
    #[serde(default)]
    pub source_file: String,
    /// What is done to the sample as it plays: its pan, polarity, stereo
    /// swap, normalising, DC offset and whether it plays like a tape. FL keeps
    /// these per channel; here they belong to the clip.
    #[serde(default)]
    pub treatment: SampleTreatment,
    /// The take's notes as tuned by hand in the sample window's NOTES view:
    /// each where it was sung and where it plays now. Empty when the take
    /// has not been opened there; the file itself is never changed.
    #[serde(default)]
    pub tuned_notes: Vec<TunedNote>,
}

/// One sung note of a clip's take, as found and as moved by hand.
///
/// Positions are seconds from the start of the file, so they hold at
/// whatever rate the file is loaded at. Pitches are MIDI notes with the
/// cents in the fraction: 63.38 is D#4, 38 cents sharp.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TunedNote {
    /// Where it was sung.
    pub start: f64,
    pub end: f64,
    /// Its pitch as sung, the average over the note.
    pub sung: f64,
    /// Where it plays now.
    pub pitch: f64,
    /// How much of the wobble around its pitch (vibrato, the slide into
    /// it) is kept: 1 as sung, 0 flat, the hard-tuned sound.
    pub drift: f64,
    /// Where it now starts and ends. As sung, `start` and `end`.
    pub new_start: f64,
    pub new_end: f64,
    /// The voice keeps its character as the pitch moves. On unless turned
    /// off for the note.
    pub formants: bool,
}

/// The most notes a take may have: far beyond any song, small enough
/// that a damaged file cannot ask for a huge plan.
pub const MAX_TUNED_NOTES: usize = 20_000;
/// How far a note can be moved, in semitones, and how much longer or
/// shorter it can be made: past these the result is no longer a voice.
pub const MAX_TUNE_SEMITONES: f64 = 24.0;
pub const MAX_NOTE_STRETCH: f64 = 4.0;

impl TunedNote {
    /// Whether playing it differs from the take: moved, flattened or
    /// retimed.
    pub fn changes_sound(&self) -> bool {
        (self.pitch - self.sung).abs() > 0.005
            || self.drift < 0.999
            || (self.new_start - self.start).abs() > 1e-5
            || (self.new_end - self.end).abs() > 1e-5
    }
}

/// Put a take's notes in order and in range, as anything the NOTES view
/// itself could have made: in time order, not overlapping, each where it
/// was sung inside the file, moved no further than the limits, and each
/// new start and end between its neighbours'. `seconds` is the file's
/// length when known.
pub fn sanitize_tuned_notes(notes: &mut Vec<TunedNote>, seconds: Option<f64>) {
    let limit = seconds
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(f64::MAX);
    notes.truncate(MAX_TUNED_NOTES);
    notes.retain(|n| {
        [
            n.start,
            n.end,
            n.sung,
            n.pitch,
            n.drift,
            n.new_start,
            n.new_end,
        ]
        .iter()
        .all(|v| v.is_finite())
            && n.start >= 0.0
            && n.end > n.start
            && n.start < limit
    });
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut kept: Vec<TunedNote> = Vec::with_capacity(notes.len());
    for mut n in notes.drain(..) {
        if kept.last().is_some_and(|p| n.start < p.end) {
            continue;
        }
        n.end = n.end.min(limit);
        n.sung = n.sung.clamp(0.0, 127.0);
        n.pitch = n
            .pitch
            .clamp(n.sung - MAX_TUNE_SEMITONES, n.sung + MAX_TUNE_SEMITONES)
            .clamp(0.0, 127.0);
        n.drift = n.drift.clamp(0.0, 1.0);
        kept.push(n);
    }
    // The new edges: inside the file, after the previous note's new end,
    // before the next one's new start, and not stretched past the limit.
    for i in 0..kept.len() {
        let floor = if i > 0 { kept[i - 1].new_end } else { 0.0 };
        let ceiling = kept
            .get(i + 1)
            .map(|next| next.new_start.max(floor))
            .unwrap_or(limit);
        let n = &mut kept[i];
        let sung = n.end - n.start;
        n.new_start = n.new_start.clamp(floor, ceiling);
        n.new_end = n.new_end.clamp(n.new_start, ceiling);
        let length = n.new_end - n.new_start;
        if length < sung / MAX_NOTE_STRETCH || length > sung * MAX_NOTE_STRETCH || length <= 0.0 {
            // Back to as sung where that fits, else the most it can be.
            n.new_start = n.start.clamp(floor, ceiling);
            n.new_end = n.end.clamp(n.new_start, ceiling);
        }
        if n.new_end <= n.new_start {
            n.new_start = n.start;
            n.new_end = n.end;
        }
    }
    *notes = kept;
}

/// A short fingerprint of a take's notes, the same for the same notes:
/// what the engine keys the tuned audio by, and what tells the NOTES view
/// that an undo changed them.
pub fn tuned_notes_hash(notes: &[TunedNote]) -> u64 {
    // FNV-1a over the numbers' bits: the same on every machine and run.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bits: u64| {
        for byte in bits.to_le_bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for n in notes {
        for v in [
            n.start,
            n.end,
            n.sung,
            n.pitch,
            n.drift,
            n.new_start,
            n.new_end,
        ] {
            eat(v.to_bits());
        }
        eat(n.formants as u64);
    }
    h
}

/// What is done to a clip's sample as it plays, on top of its gain, pitch,
/// stretch and fades. All off is the sample as recorded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampleTreatment {
    /// -1 is left, 1 right.
    pub pan: f64,
    pub polarity_inverted: bool,
    pub swap_stereo: bool,
    /// Brought up so its loudest peak reaches full scale.
    pub normalize: bool,
    /// Each channel's average taken off, so it sits on zero.
    pub remove_dc: bool,
    /// Speed and pitch move together, like a tape: a longer clip plays
    /// lower. Off, length and pitch are set apart.
    pub resample: bool,
    /// The left channel fades out and the right fades in over the sample.
    #[serde(default)]
    pub fade_stereo: bool,
    /// Where the sample begins, as a share of the file (0 = its first
    /// frame): what comes before is cut away, as FL's SMP START does.
    #[serde(default)]
    pub start: f64,
    /// How much of what remains after `start` is the sample, 0..1.
    #[serde(default = "one")]
    pub length: f64,
    /// Cuts the sample's quiet tail. 0 is off; from there the threshold
    /// rises from -60 dB under the peak (0.01) to the peak itself (1).
    #[serde(default)]
    pub trim: f64,
    /// The tiny fades at a clip's edges when it has none of its own.
    #[serde(default)]
    pub declick: DeclickMode,
    /// Show the regions and the slice markers the file carries (FL's
    /// "load regions" and "load slice markers").
    #[serde(default)]
    pub load_regions: bool,
    #[serde(default)]
    pub load_slices: bool,
}

fn one() -> f64 {
    1.0
}

impl Default for SampleTreatment {
    fn default() -> Self {
        Self {
            pan: 0.0,
            polarity_inverted: false,
            swap_stereo: false,
            normalize: false,
            remove_dc: false,
            resample: false,
            fade_stereo: false,
            start: 0.0,
            length: 1.0,
            trim: 0.0,
            declick: DeclickMode::OutOnly,
            load_regions: false,
            load_slices: false,
        }
    }
}

impl SampleTreatment {
    /// The amplitude under which the tail is cut, as a share of the
    /// sample's peak, or `None` when trim is off: -60 dB at the first
    /// step, the peak itself at the top.
    pub fn trim_threshold(&self) -> Option<f64> {
        let trim = self.trim.clamp(0.0, 1.0);
        (trim > 0.0).then(|| 10f64.powf(-60.0 * (1.0 - trim) / 20.0))
    }
}

/// FL's declicking modes, in FL's order (the order of the byte FL saves,
/// 0 to 5): how a sample's edges are smoothed when the clip has no fades
/// of its own. Each is a pair of tiny fades, in and out. "No bleeding"
/// means a note does not ring on past its end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclickMode {
    /// Only the end is smoothed; the attack is left as it is.
    #[default]
    // A name a test build wrote before the modes followed FL's list.
    #[serde(alias = "Bleeding")]
    OutOnly,
    /// A hair at the start, so a transient keeps its edge; a short end.
    TransientNoBleeding,
    Transient,
    Generic,
    Smooth,
    /// Longer at both ends, for clips that overlap.
    Crossfade,
}

impl DeclickMode {
    pub const ALL: [DeclickMode; 6] = [
        DeclickMode::OutOnly,
        DeclickMode::TransientNoBleeding,
        DeclickMode::Transient,
        DeclickMode::Generic,
        DeclickMode::Smooth,
        DeclickMode::Crossfade,
    ];

    /// The mode FL saved as `byte`, or None when FL has no such mode.
    pub fn from_fl(byte: u8) -> Option<Self> {
        Self::ALL.get(byte as usize).copied()
    }

    /// The fades it stands for, in milliseconds: in, out.
    pub fn fades_ms(self) -> (f64, f64) {
        match self {
            DeclickMode::OutOnly => (0.0, 1.0),
            DeclickMode::TransientNoBleeding => (0.3, 1.0),
            DeclickMode::Transient => (0.3, 4.0),
            DeclickMode::Generic => (2.0, 5.0),
            DeclickMode::Smooth => (10.0, 20.0),
            DeclickMode::Crossfade => (5.0, 5.0),
        }
    }

    /// As FL names it.
    pub fn name(self) -> &'static str {
        match self {
            DeclickMode::OutOnly => "Out only",
            DeclickMode::TransientNoBleeding => "Transient (no bleeding)",
            DeclickMode::Transient => "Transient",
            DeclickMode::Generic => "Generic",
            DeclickMode::Smooth => "Smooth",
            DeclickMode::Crossfade => "Crossfade",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|m| m.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// One warp anchor: timeline tick ↔ source sample.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    pub clip_tick: u64,
    pub source_sample: u64,
}

fn default_stretch_ratio() -> f64 {
    1.0
}

impl AudioClip {
    /// Whether the take plays with notes tuned by hand.
    pub fn has_tuned_notes(&self) -> bool {
        self.tuned_notes.iter().any(TunedNote::changes_sound)
    }

    /// How much longer than its file the sample lasts as it plays: the
    /// stretch, and in resample also the pitch, as a tape played faster is
    /// higher and shorter.
    pub fn time_factor(&self) -> f64 {
        let stretch = if self.stretch_ratio <= 0.01 {
            1.0
        } else {
            self.stretch_ratio
        };
        if self.treatment.resample {
            stretch / 2f64.powf(self.pitch_semitones / 12.0)
        } else {
            stretch
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiClipRef {
    pub id: ClipId,
    pub clip: hardwave_midi::MidiClip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClipContent {
    Audio(AudioClip),
    Midi(MidiClipRef),
}

/// A clip placed on a track at a specific position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipPlacement {
    pub content: ClipContent,
    pub track_id: TrackId,
    /// Position on the timeline in ticks (960 PPQ).
    pub position_ticks: u64,
    /// Length on the timeline in ticks (may differ from source length for time-stretched clips).
    pub length_ticks: u64,
    /// Lane index for comping (0 = main lane).
    pub lane: u32,
    /// The colour picked for this clip, as `#rrggbb`; none keeps the
    /// track's colour. Copies and splits keep it, since they clone the
    /// placement. It used to live only in the interface, so every clip
    /// was back to its track's colour after a reload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The playlist row it is drawn on, counting from 0 (`Project::rows`).
    /// Where it sits, never how it sounds: it plays through the track it
    /// is on. None for a clip made by something not yet taught about
    /// rows; the playlist puts it on its track's row, and the engine never
    /// silences it.
    #[serde(default)]
    pub row: Option<u32>,
}

/// The lane a freeze render sits on.
///
/// Out of the way of the take lanes, which count up from zero, so a
/// frozen track's render never shows up as one more take to comp.
pub const FREEZE_LANE: u32 = u32::MAX;

#[cfg(test)]
mod color_tests {
    use super::*;

    fn placement(color: Option<&str>) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "c1".into(),
                clip: hardwave_midi::MidiClip::new("m1".into(), "Lead".into(), 3840),
            }),
            track_id: "t1".into(),
            position_ticks: 0,
            length_ticks: 3840,
            lane: 0,
            color: color.map(str::to_string),
            row: None,
        }
    }

    #[test]
    fn a_clip_colour_is_kept_in_the_song() {
        let json = serde_json::to_string(&placement(Some("#ff8800"))).unwrap();
        let back: ClipPlacement = serde_json::from_str(&json).unwrap();
        assert_eq!(back.color.as_deref(), Some("#ff8800"));
    }

    #[test]
    fn a_song_saved_before_clip_colours_still_opens() {
        let mut value = serde_json::to_value(placement(None)).unwrap();
        assert!(value.get("color").is_none(), "no colour, no field");
        value.as_object_mut().unwrap().remove("color");
        let back: ClipPlacement = serde_json::from_value(value).unwrap();
        assert_eq!(back.color, None);
    }
}

#[cfg(test)]
mod declick_tests {
    use super::*;

    #[test]
    fn the_modes_are_fl_s_in_fl_s_order() {
        let names: Vec<&str> = DeclickMode::ALL.iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "Out only",
                "Transient (no bleeding)",
                "Transient",
                "Generic",
                "Smooth",
                "Crossfade"
            ]
        );
        assert_eq!(DeclickMode::from_fl(4), Some(DeclickMode::Smooth));
        assert_eq!(DeclickMode::from_fl(6), None);
        assert_eq!(
            DeclickMode::from_name("transient (no bleeding)"),
            Some(DeclickMode::TransientNoBleeding)
        );
        // A project a test build saved with the old made-up name still opens.
        let old: DeclickMode = serde_json::from_str("\"Bleeding\"").unwrap();
        assert_eq!(old, DeclickMode::OutOnly);
    }
}

#[cfg(test)]
mod tuned_note_tests {
    use super::*;

    fn note(start: f64, end: f64, sung: f64) -> TunedNote {
        TunedNote {
            start,
            end,
            sung,
            pitch: sung,
            drift: 1.0,
            new_start: start,
            new_end: end,
            formants: true,
        }
    }

    fn clip(notes: Vec<TunedNote>) -> AudioClip {
        AudioClip {
            id: "c1".into(),
            name: "Vocal".into(),
            source_path: "src".into(),
            source_hash: String::new(),
            source_start: 0,
            source_end: 48_000,
            gain_db: 0.0,
            fade_in_ticks: 0,
            fade_out_ticks: 0,
            muted: false,
            reversed: false,
            pitch_semitones: 0.0,
            stretch_ratio: 1.0,
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            warp_markers: Vec::new(),
            source_file: String::new(),
            treatment: SampleTreatment::default(),
            tuned_notes: notes,
        }
    }

    #[test]
    fn tuned_notes_are_kept_in_the_song() {
        let mut moved = note(0.5, 1.2, 63.38);
        moved.pitch = 65.0;
        moved.drift = 0.4;
        moved.new_end = 1.3;
        moved.formants = false;
        let original = clip(vec![note(0.0, 0.4, 65.08), moved]);
        // JSON, as a room partner and the undo history see it.
        let json = serde_json::to_string(&original).unwrap();
        let back: AudioClip = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tuned_notes, original.tuned_notes);
        // MessagePack with field names, as a .hwp is written, and as a
        // positional array, as one was before format 2.
        for bytes in [
            rmp_serde::to_vec_named(&original).unwrap(),
            rmp_serde::to_vec(&original).unwrap(),
        ] {
            let back: AudioClip = rmp_serde::from_slice(&bytes).unwrap();
            assert_eq!(back.tuned_notes, original.tuned_notes);
        }
        let back: AudioClip =
            rmp_serde::from_slice(&rmp_serde::to_vec_named(&original).unwrap()).unwrap();
        assert!(back.has_tuned_notes());
    }

    #[test]
    fn a_song_saved_before_tuned_notes_still_opens() {
        let old = clip(Vec::new());
        // Written by a build without the field: the same struct one
        // field shorter, in both formats.
        let mut value = serde_json::to_value(&old).unwrap();
        value.as_object_mut().unwrap().remove("tuned_notes");
        let back: AudioClip = serde_json::from_value(value).unwrap();
        assert!(back.tuned_notes.is_empty());
        // A .hwp written before format 2 holds a clip as a positional
        // array: 19 fields, the last the empty list. An older build wrote
        // the same array without it.
        let mut bytes = rmp_serde::to_vec(&old).unwrap();
        assert_eq!(&bytes[..3], &[0xdc, 0x00, 19], "a clip is an array of 19");
        assert_eq!(bytes.last(), Some(&0x90), "ending in the empty list");
        bytes.pop();
        bytes[2] = 18;
        let back: AudioClip = rmp_serde::from_slice(&bytes).unwrap();
        assert!(back.tuned_notes.is_empty());
        assert!(!back.has_tuned_notes());
    }

    #[test]
    fn notes_as_sung_change_nothing() {
        let c = clip(vec![note(0.0, 0.4, 65.08), note(0.5, 1.0, 67.3)]);
        assert!(!c.has_tuned_notes());
        let mut flat = note(0.0, 0.4, 65.08);
        flat.drift = 0.0;
        assert!(flat.changes_sound());
    }

    #[test]
    fn damaged_notes_are_put_back_in_range() {
        let mut wild = note(0.5, 1.0, 60.0);
        wild.pitch = 200.0;
        wild.drift = 7.0;
        let mut long = note(1.2, 1.4, 62.0);
        long.new_end = 9.0; // 39 times as long, past the next note
        let mut notes = vec![
            note(2.0, 2.5, 64.0),
            wild,
            note(0.7, 0.9, 61.0), // overlaps the one before
            note(f64::NAN, 1.0, 60.0),
            long,
            note(3.0, 2.0, 60.0), // ends before it starts
        ];
        sanitize_tuned_notes(&mut notes, Some(10.0));
        let starts: Vec<f64> = notes.iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![0.5, 1.2, 2.0]);
        assert_eq!(notes[0].pitch, 84.0, "two octaves at most");
        assert_eq!(notes[0].drift, 1.0);
        assert!(notes[1].new_end <= notes[2].new_start);
        assert_eq!(
            (notes[1].new_start, notes[1].new_end),
            (1.2, 1.4),
            "back to as sung"
        );
    }

    #[test]
    fn the_fingerprint_follows_the_notes() {
        let a = vec![note(0.0, 0.4, 65.08)];
        let mut b = a.clone();
        assert_eq!(tuned_notes_hash(&a), tuned_notes_hash(&b));
        b[0].pitch = 66.0;
        assert_ne!(tuned_notes_hash(&a), tuned_notes_hash(&b));
    }
}
