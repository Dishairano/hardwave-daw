//! FL Studio `.flp` import — parser contract + import report.
//!
//! This module defines the data shape the importer emits on a
//! best-effort parse of FL Studio project files. The binary parser
//! lives in a sibling module / worker crate; here we define the
//! types the UI consumes and the plugin-mapping table it uses to
//! translate FL natives into Hardwave equivalents.

use serde::{Deserialize, Serialize};

/// Parsed channel-rack channel from a `.flp` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlChannel {
    pub name: String,
    /// Its colour in the rack, as `#rrggbb`.
    #[serde(default)]
    pub color: Option<String>,
    /// A layer's channels, by their place in the rack: the layer plays its
    /// notes on each of them and has no sound of its own.
    #[serde(default)]
    pub layer_children: Vec<u32>,
    pub sample_path: Option<String>,
    pub plugin_name: Option<String>,
    pub pattern_steps: Vec<bool>,
    /// What the channel is in FL, which decides what it becomes here.
    #[serde(default)]
    pub kind: FlChannelKind,
    /// Its level, pan, pitch and sample settings.
    #[serde(default)]
    pub settings: FlChannelSettings,
    /// Its plug-in's own data: for "Fruity Wrapper", the plug-in it wraps
    /// and that plug-in's settings (`read_wrapper`).
    #[serde(skip)]
    pub plugin_data: Vec<u8>,
    /// For an automation clip's channel, its points.
    #[serde(default)]
    pub automation: Vec<FlAutomationPoint>,
}

/// A point of an automation clip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlAutomationPoint {
    /// From the start of the clip's data, in our ticks.
    pub tick: u64,
    /// Where the knob is, 0 to 1 across its range.
    pub value: f64,
    /// The bend of the curve into this point.
    pub tension: f32,
}

/// What an automation clip moves. FL names a knob by where it is: on a
/// channel, on a mixer insert, or on a plug-in in either; anything else
/// (the song's tempo, a send level) is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlAutomationTarget {
    ChannelVolume(u32),
    ChannelPan(u32),
    ChannelPitch(u32),
    /// A parameter of the channel's plug-in, by FL's index.
    ChannelPlugin {
        channel: u32,
        param: u32,
    },
    InsertVolume(u32),
    InsertPan(u32),
    InsertSeparation(u32),
    SlotEnabled {
        insert: u32,
        slot: u32,
    },
    SlotMix {
        insert: u32,
        slot: u32,
    },
    /// A parameter of the plug-in in an insert's slot, by FL's index.
    SlotPlugin {
        insert: u32,
        slot: u32,
        param: u32,
    },
    Other,
}

/// An automation clip's channel and the knob it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlAutomationLink {
    pub channel: u32,
    pub target: FlAutomationTarget,
}

/// A channel's settings as FL stores them, in FL's own units. What they
/// mean here is worked out where the song is built (fl_build.rs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlChannelSettings {
    /// The channel's on/off switch in the rack.
    pub enabled: bool,
    /// The volume knob, 0 to 12800; 10000 is FL's 78 % default.
    pub volume: u32,
    /// The pan knob, 0 (left) to 12800 (right); 6400 is the centre.
    pub pan: i32,
    /// The pitch knob, in cents.
    pub pitch_cents: i32,
    /// The mixer insert it plays into: 0 is the master, -1 "current".
    pub mixer_insert: i32,
    /// Sample settings: played backwards.
    pub reversed: bool,
    /// Sample settings: left and right swapped.
    pub swap_stereo: bool,
    /// Sample settings: polarity inverted.
    pub polarity_inverted: bool,
    /// The root note, as a MIDI key; FL's default is 60 (its C5).
    pub root_note: u8,
    /// FL's quick fade knobs on the sample, 0 to 1024 (0 is off).
    pub fade_in: u16,
    pub fade_out: u16,
    pub stretch: FlStretch,
    /// What a sampler channel does with its sample beyond the above: its
    /// volume envelope, loop, voices and the sample's precomputed effects.
    #[serde(default)]
    pub sampler: FlSamplerSetup,
}

/// A sampler channel's envelope, loop, voices and sample effects, as
/// fractions of FL's own knob ranges (0 to 1) where FL has a knob. The
/// layouts follow PyFLP 2.2 (MIT), checked field by field against the
/// events already read from real files (polarity at 81, stretch at 96).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlSamplerSetup {
    /// The volume envelope (FL's second envelope event; the first is pan).
    pub envelope: Option<FlEnvelope>,
    /// "Use loop points": loop the sample's own loop region (its WAV
    /// loop), or the whole sample when it has none.
    pub use_loop_points: bool,
    pub ping_pong: bool,
    pub mono: bool,
    pub porta: bool,
    /// The channel cuts itself: FL's cut group and cut-by group are the
    /// same, non-zero group.
    pub cut_itself: bool,
    /// FL's declicking mode, 0 to 5 in FL's order.
    pub declick: u8,
    /// The loop crossfade, 0 to 1.
    pub crossfade: f32,
    /// The trim knob, 0 (off) to 1.
    pub trim: f32,
    /// Sample start and length, 0 to 1 of the file; None when FL kept
    /// none (then the whole sample).
    pub start: Option<f32>,
    pub length: Option<f32>,
    pub fade_stereo: bool,
    pub normalize: bool,
    pub remove_dc: bool,
    /// How many envelope events this channel has had; the second is the
    /// volume envelope. Reading state only.
    #[serde(skip)]
    pub envelope_events: u8,
}

impl Default for FlSamplerSetup {
    fn default() -> Self {
        Self {
            envelope: None,
            use_loop_points: false,
            ping_pong: false,
            mono: false,
            porta: false,
            cut_itself: false,
            declick: 0,
            crossfade: 0.0,
            trim: 0.0,
            start: None,
            length: None,
            fade_stereo: false,
            normalize: false,
            remove_dc: false,
            envelope_events: 0,
        }
    }
}

/// FL's envelope knobs, each 0 to 1 of the knob's travel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlEnvelope {
    pub enabled: bool,
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for FlChannelSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 10_000,
            pan: 6_400,
            pitch_cents: 0,
            mixer_insert: -1,
            reversed: false,
            swap_stereo: false,
            polarity_inverted: false,
            root_note: 60,
            fade_in: 0,
            fade_out: 0,
            stretch: FlStretch::default(),
            sampler: FlSamplerSetup::default(),
        }
    }
}

/// The time stretching section of FL's sample settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlStretch {
    /// The TIME knob: how long the whole sample should last, in our ticks.
    /// 0 is "(none)", the sample at its own length.
    pub time_ticks: f64,
    /// The PITCH knob, in cents.
    pub pitch_cents: i32,
    /// The MUL knob, as a factor on the length (0.25 to 4).
    pub multiplier: f64,
    /// The mode: 0 is Resample, where length and pitch change together like
    /// a tape; every other mode keeps them apart.
    pub mode: i32,
}

impl Default for FlStretch {
    fn default() -> Self {
        Self {
            time_ticks: 0.0,
            pitch_cents: 0,
            multiplier: 1.0,
            mode: 0,
        }
    }
}

impl FlStretch {
    /// Resample: speed and pitch move together.
    pub fn is_resample(&self) -> bool {
        self.mode == 0
    }
}

/// The kinds of channel in FL's channel rack, as the file numbers them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlChannelKind {
    /// Plays a sample from its notes.
    Sampler,
    /// A plug-in instrument (FL's own, or a VST in FL's wrapper).
    Instrument,
    /// Several channels played as one.
    Layer,
    /// An audio clip: a file placed on the playlist.
    AudioClip,
    /// An automation clip.
    Automation,
    /// Anything else, or not said.
    #[default]
    Other,
}

impl FlChannelKind {
    /// From the byte FL writes.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Sampler,
            2 => Self::Instrument,
            3 => Self::Layer,
            4 => Self::AudioClip,
            5 => Self::Automation,
            _ => Self::Other,
        }
    }
}

/// Parsed piano-roll note.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FlNote {
    pub tick: u64,
    pub length_ticks: u64,
    pub pitch: u8,
    pub velocity: u8,
}

/// Parsed arrangement / playlist clip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlPlaylistClip {
    pub track_index: u32,
    pub start_tick: u64,
    pub length_ticks: u64,
    pub content: FlClipContent,
    /// Muted on the playlist (FL greys it out).
    #[serde(default)]
    pub muted: bool,
    /// Which of FL's arrangements it is in, an index into
    /// [`FlProject::arrangements`] (0 for a file from before them).
    #[serde(default)]
    pub arrangement: usize,
}

/// One of FL's arrangements (12.9 and later): its own playlist, its own
/// track names and switches. The clips say which one they are in.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlArrangement {
    pub name: String,
    #[serde(default)]
    pub track_names: Vec<(u32, String)>,
    /// Each playlist track's colour, as `#rrggbb`.
    #[serde(default)]
    pub track_colors: Vec<(u32, String)>,
    #[serde(default)]
    pub tracks_off: Vec<u32>,
    #[serde(default)]
    pub tracks_solo: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlClipContent {
    Pattern {
        pattern_index: u32,
        /// How far into the pattern the clip starts: a clip trimmed at its
        /// left edge plays from there, not from the pattern's start.
        #[serde(default)]
        offset_ticks: u64,
    },
    /// A sample on the playlist: the channel it belongs to, its file, and
    /// where in the file the clip starts and ends, in milliseconds (FL
    /// writes -1 for "not trimmed").
    AudioSample {
        channel_index: u32,
        sample_path: String,
        start_offset_ms: f32,
        end_offset_ms: f32,
        /// The clip's own fades, in milliseconds (FL 20.99 and later).
        #[serde(default)]
        fade_in_ms: f32,
        #[serde(default)]
        fade_out_ms: f32,
        /// The clip's own volume, as a factor (FL 20.99 and later; 1 before).
        #[serde(default = "unit_volume")]
        volume: f32,
    },
    Automation {
        channel_index: u32,
        target: String,
        /// Where the placement starts in the clip's data, in our ticks.
        #[serde(default)]
        offset_ticks: u64,
    },
}

fn unit_volume() -> f32 {
    1.0
}

/// An insert in FL's mixer: where channels play into, where the effects
/// sit, and where it sends its sound on. Values are FL's own; the import
/// turns them into ours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlMixerTrack {
    /// Its number in FL's mixer; 0 is the master.
    pub index: u32,
    /// Its colour, as `#rrggbb`, when it was given one.
    #[serde(default)]
    pub color: Option<String>,
    /// Empty when it has FL's default name ("Insert 3").
    pub name: String,
    /// Off is FL's mute switch on the insert.
    pub enabled: bool,
    /// All its effect slots on or bypassed at once.
    pub effects_enabled: bool,
    pub polarity_inverted: bool,
    pub swap_stereo: bool,
    pub solo: bool,
    /// The fader, 12800 at 100 % (0 dB), 16000 at the top.
    pub volume: i32,
    /// The pan knob, -6400 (left) to 6400 (right).
    pub pan: i32,
    /// Stereo separation: 64 is mono, 0 as recorded, -64 fully separated.
    pub stereo_separation: i32,
    /// The gains of the insert's own three-band EQ, in hundredths of a dB.
    pub eq_gains: [i32; 3],
    /// Where it sends its sound, with the level of each.
    pub routes: Vec<FlRoute>,
    /// The effect slots that hold a plug-in.
    pub slots: Vec<FlMixerSlot>,
}

impl Default for FlMixerTrack {
    fn default() -> Self {
        Self {
            index: 0,
            color: None,
            name: String::new(),
            enabled: true,
            effects_enabled: true,
            polarity_inverted: false,
            swap_stereo: false,
            solo: false,
            volume: 12_800,
            pan: 0,
            stereo_separation: 0,
            eq_gains: [0; 3],
            routes: Vec::new(),
            slots: Vec::new(),
        }
    }
}

/// A route from one insert to another.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlRoute {
    /// The insert it goes to; 0 is the master.
    pub to: u32,
    /// The send level, on the fader's scale (12800 is 0 dB). 0 carries no
    /// sound: FL makes those for sidechains.
    pub level: i32,
}

/// An effect slot in an insert, with the plug-in in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlMixerSlot {
    /// Its place in the insert, 0 to 9.
    pub index: u32,
    /// What FL calls the plug-in ("Fruity Parametric EQ 2"; "Fruity
    /// Wrapper" for anyone else's).
    pub plugin: String,
    /// The name it was given, when it has one.
    pub name: Option<String>,
    pub enabled: bool,
    /// Dry/wet, 0 to 12800 (fully wet).
    pub mix: i32,
    /// The plug-in's own data: its settings, in its own format.
    #[serde(skip)]
    pub data: Vec<u8>,
}

impl FlMixerSlot {
    /// The name to show: the one it was given, the name of the plug-in in
    /// FL's wrapper, or FL's own name for it.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| (self.plugin == WRAPPER).then(|| wrapped_plugin_name(&self.data))?)
            .unwrap_or_else(|| self.plugin.clone())
    }
}

/// What FL calls the wrapper it puts anyone else's plug-in in.
pub const WRAPPER: &str = "Fruity Wrapper";

/// The kinds of plug-in FL's wrapper holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlPluginFormat {
    Vst2,
    Vst3,
    Clap,
}

/// A plug-in in FL's wrapper: what it is, and its settings.
#[derive(Debug, Clone, PartialEq)]
pub struct FlWrappedPlugin {
    pub name: String,
    pub vendor: String,
    pub format: FlPluginFormat,
    /// An instrument rather than an effect.
    pub instrument: bool,
    /// Where it was on the computer that saved the song.
    pub file: String,
    /// A CLAP's own id.
    pub clap_id: Option<String>,
    /// For a VST3, its component's state: what a host gets from getState and
    /// gives back to setState. Empty when FL kept none, and for VST2 and
    /// CLAP, whose state is not read.
    pub state: Vec<u8>,
    /// For a VST3, its parameters' ids in FL's order: FL automates a
    /// plug-in's parameter by its place in this list.
    pub param_ids: Vec<u32>,
}

/// The chunks FL's wrapper data is made of, from `at`: a kind (four bytes),
/// a length (eight) and that many bytes. Stops at the first chunk that does
/// not fit.
fn wrapper_chunks(data: &[u8], mut at: usize) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    while let Some(head) = data.get(at..at.saturating_add(12)) {
        let kind = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let len = u64::from_le_bytes([
            head[4], head[5], head[6], head[7], head[8], head[9], head[10], head[11],
        ]);
        at += 12;
        let Some(body) = usize::try_from(len)
            .ok()
            .and_then(|len| data.get(at..at.checked_add(len)?))
        else {
            break;
        };
        out.push((kind, body));
        at += body.len();
    }
    out
}

fn chunk_text(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

/// FL's wrapper data with these chunks after its four leading bytes.
#[cfg(test)]
pub(crate) fn test_wrapper(chunks: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = vec![12, 0, 0, 0];
    for (kind, body) in chunks {
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(body);
    }
    out
}

/// FL's wrapper around a VST3 called `name`, its component's state `state`
/// (none when empty), as FL 24 to 26 write it.
#[cfg(test)]
pub(crate) fn test_vst3_wrapper(name: &str, instrument: bool, state: &[u8]) -> Vec<u8> {
    let kind: u32 = if instrument { 8 } else { 7 };
    let mut info = kind.to_le_bytes().to_vec();
    info.extend_from_slice(&[0; 8]);
    // The state: four bytes, then a 64-byte chunk FL keeps for itself, the
    // component's state, the controller's, and the parameter ids.
    let ids: Vec<u8> = [3u32, 0, 7, 0x7072_6F67]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    let mut vst3 = test_wrapper(&[(1, &[0; 64]), (3, state), (2, b"controller"), (4, &ids)]);
    vst3[..4].copy_from_slice(&1u32.to_le_bytes());
    let file = format!("C:\\Program Files\\Common Files\\VST3\\{name}.vst3");
    let mut chunks: Vec<(u32, &[u8])> =
        vec![(50, &info), (54, name.as_bytes()), (55, file.as_bytes())];
    if !state.is_empty() {
        chunks.push((53, &vst3));
    }
    chunks.push((56, b"Vendor"));
    test_wrapper(&chunks)
}

/// The name of the plug-in in FL's wrapper.
pub fn wrapped_plugin_name(data: &[u8]) -> Option<String> {
    const NAME: u32 = 54;
    wrapper_chunks(data, 4)
        .into_iter()
        .find(|(kind, _)| *kind == NAME)
        .map(|(_, body)| chunk_text(body))
        .filter(|name| !name.is_empty())
}

/// What FL's wrapper holds. Its data is four bytes, then chunks: 50 says
/// what kind of plug-in it is (0 and 4 a VST2 effect and instrument, 7 and
/// 8 a VST3, 11 and 12 a CLAP), 53 is its state, 54 its name, 55 its file,
/// 56 its vendor, 58 a CLAP's id. A VST3's state is four bytes and chunks of
/// the same shape again, the component's state in chunk 3.
pub fn read_wrapper(data: &[u8]) -> Option<FlWrappedPlugin> {
    const INFO: u32 = 50;
    const STATE: u32 = 53;
    const NAME: u32 = 54;
    const FILE: u32 = 55;
    const VENDOR: u32 = 56;
    const CLAP_ID: u32 = 58;
    const VST3_COMPONENT: u32 = 3;
    const VST3_PARAM_IDS: u32 = 4;

    let chunks = wrapper_chunks(data, 4);
    let get = |want: u32| {
        chunks
            .iter()
            .find(|(kind, _)| *kind == want)
            .map(|(_, b)| *b)
    };
    let info = get(INFO)?;
    let (format, instrument) = match u32::from_le_bytes(info.get(..4)?.try_into().ok()?) {
        0 => (FlPluginFormat::Vst2, false),
        4 => (FlPluginFormat::Vst2, true),
        7 => (FlPluginFormat::Vst3, false),
        8 => (FlPluginFormat::Vst3, true),
        11 => (FlPluginFormat::Clap, false),
        12 => (FlPluginFormat::Clap, true),
        _ => return None,
    };
    let name = get(NAME).map(chunk_text).filter(|n| !n.is_empty())?;
    let vst3 = match (format, get(STATE)) {
        (FlPluginFormat::Vst3, Some(state)) => wrapper_chunks(state, 4),
        _ => Vec::new(),
    };
    let vst3_chunk = |want: u32| vst3.iter().find(|(kind, _)| *kind == want).map(|(_, b)| *b);
    let state = vst3_chunk(VST3_COMPONENT)
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    // A count, then that many ids.
    let param_ids = vst3_chunk(VST3_PARAM_IDS)
        .map(|body| {
            let words: Vec<u32> = body
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| u32::from_le_bytes(*w))
                .collect();
            let count = words.first().copied().unwrap_or(0) as usize;
            words.into_iter().skip(1).take(count).collect()
        })
        .unwrap_or_default();
    Some(FlWrappedPlugin {
        name,
        vendor: get(VENDOR).map(chunk_text).unwrap_or_default(),
        format,
        instrument,
        file: get(FILE).map(chunk_text).unwrap_or_default(),
        clap_id: get(CLAP_ID).map(chunk_text).filter(|id| !id.is_empty()),
        state,
        param_ids,
    })
}

/// Parsed `.flp` project — the raw import result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlProject {
    pub bpm: f32,
    pub time_sig_numerator: u8,
    pub time_sig_denominator: u8,
    pub channels: Vec<FlChannel>,
    pub notes: Vec<(u32, Vec<FlNote>)>, // (channel_index, notes)
    /// The same notes, but kept per pattern as well, which is what
    /// the playlist needs: a pattern is placed many times, and each
    /// placement brings its channels' notes with it.
    #[serde(default)]
    pub pattern_notes: Vec<(u32, u32, Vec<FlNote>)>, // (pattern, channel, notes)
    /// Pattern names, for naming the clips they become.
    #[serde(default)]
    pub pattern_names: Vec<(u32, String)>,
    /// Pattern colours, as `#rrggbb`, for the clips they become.
    #[serde(default)]
    pub pattern_colors: Vec<(u32, String)>,
    /// Playlist track colours, the open arrangement's, by track number.
    #[serde(default)]
    pub playlist_track_colors: Vec<(u32, String)>,
    pub playlist_clips: Vec<FlPlaylistClip>,
    pub mixer: Vec<FlMixerTrack>,
    /// Names given to playlist tracks, by track number (1 is the top).
    #[serde(default)]
    pub playlist_track_names: Vec<(u32, String)>,
    /// Playlist tracks switched off in FL, by track number.
    #[serde(default)]
    pub playlist_tracks_off: Vec<u32>,
    /// Playlist tracks soloed in FL. A solo switches every other track off,
    /// so while one is soloed the switches say nothing else.
    #[serde(default)]
    pub playlist_tracks_solo: Vec<u32>,
    /// Which knob each automation clip moves.
    #[serde(default)]
    pub automation_links: Vec<FlAutomationLink>,
    /// FL's arrangements in the file's order; empty for a file from before
    /// them. The playlist track names and switches above are the open one's.
    #[serde(default)]
    pub arrangements: Vec<FlArrangement>,
    /// The arrangement open in FL, an index into `arrangements`.
    #[serde(default)]
    pub current_arrangement: usize,
}

/// Plugin mapping — translate an FL native plugin name into a
/// Hardwave equivalent. `None` on the right means "no native
/// equivalent; flag for user to resolve".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMap {
    pub from_fl: String,
    pub to_hardwave: Option<String>,
}

/// Built-in mapping table — maps the well-known FL natives to the
/// closest Hardwave plugin.
pub fn default_plugin_mappings() -> Vec<PluginMap> {
    vec![
        PluginMap {
            from_fl: "Sytrus".into(),
            to_hardwave: Some("hardwave-fm".into()),
        },
        PluginMap {
            from_fl: "3xOsc".into(),
            to_hardwave: Some("hardwave-subtractive".into()),
        },
        PluginMap {
            from_fl: "FPC".into(),
            to_hardwave: Some("hardwave-drum-machine".into()),
        },
        PluginMap {
            from_fl: "Fruity Kick".into(),
            to_hardwave: Some("hardwave-drum-synth".into()),
        },
        PluginMap {
            from_fl: "Fruity Limiter".into(),
            to_hardwave: Some("hardwave-limiter".into()),
        },
        PluginMap {
            from_fl: "Fruity Parametric EQ 2".into(),
            to_hardwave: Some("hardwave-parametric-eq".into()),
        },
        PluginMap {
            from_fl: "Fruity Delay 3".into(),
            to_hardwave: Some("hardwave-delay".into()),
        },
        PluginMap {
            from_fl: "Fruity Reverb 2".into(),
            to_hardwave: Some("hardwave-reverb".into()),
        },
        PluginMap {
            from_fl: "Harmor".into(),
            to_hardwave: None, // no equivalent
        },
    ]
}

/// One row in the user-facing import report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFinding {
    pub severity: FindingSeverity,
    pub category: FindingCategory,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingCategory {
    ChannelRack,
    PianoRoll,
    Playlist,
    Mixer,
    Automation,
    Plugin,
    Tempo,
    Unsupported,
}

/// Human-readable summary of what was imported and what wasn't —
/// shown to the user after the import finishes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub findings: Vec<ImportFinding>,
    pub imported_channels: u32,
    pub imported_notes: u32,
    pub imported_playlist_clips: u32,
    pub imported_mixer_tracks: u32,
    pub imported_automation_clips: u32,
    pub skipped_plugins: Vec<String>,
}

impl ImportReport {
    pub fn info(&mut self, category: FindingCategory, message: impl Into<String>) {
        self.findings.push(ImportFinding {
            severity: FindingSeverity::Info,
            category,
            message: message.into(),
        });
    }

    pub fn warn(&mut self, category: FindingCategory, message: impl Into<String>) {
        self.findings.push(ImportFinding {
            severity: FindingSeverity::Warning,
            category,
            message: message.into(),
        });
    }

    pub fn error(&mut self, category: FindingCategory, message: impl Into<String>) {
        self.findings.push(ImportFinding {
            severity: FindingSeverity::Error,
            category,
            message: message.into(),
        });
    }

    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == FindingSeverity::Error)
    }

    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Warning)
            .count()
    }
}

/// Simulated end-to-end summary pass — given a parsed `FlProject`
/// and the plugin mapping table, produce an `ImportReport` for the
/// UI. The actual arrangement + channel-rack mutation happens
/// elsewhere; this helper keeps the reporting logic testable in
/// isolation.
pub fn summarize_import(project: &FlProject, mappings: &[PluginMap]) -> ImportReport {
    let mut report = ImportReport {
        imported_channels: project.channels.len() as u32,
        imported_notes: project.notes.iter().map(|(_, n)| n.len() as u32).sum(),
        imported_playlist_clips: project.playlist_clips.len() as u32,
        imported_mixer_tracks: project.mixer.len() as u32,
        imported_automation_clips: project
            .playlist_clips
            .iter()
            .filter(|c| matches!(c.content, FlClipContent::Automation { .. }))
            .count() as u32,
        ..Default::default()
    };
    report.info(
        FindingCategory::Tempo,
        format!(
            "Tempo {} BPM, {}/{}",
            project.bpm, project.time_sig_numerator, project.time_sig_denominator
        ),
    );
    for channel in &project.channels {
        if let Some(plugin) = &channel.plugin_name {
            match map_plugin(plugin, mappings) {
                PluginMapResult::Equivalent(target) => {
                    report.info(
                        FindingCategory::Plugin,
                        format!("Mapped {} → {}", plugin, target),
                    );
                }
                PluginMapResult::NoEquivalent => {
                    report.warn(
                        FindingCategory::Plugin,
                        format!("{} has no Hardwave equivalent — channel muted until you assign a plugin", plugin),
                    );
                    report.skipped_plugins.push(plugin.clone());
                }
                PluginMapResult::Unknown => {
                    report.warn(
                        FindingCategory::Unsupported,
                        format!("Unknown plugin reference: {} (skipped)", plugin),
                    );
                    report.skipped_plugins.push(plugin.clone());
                }
            }
        }
    }
    report
}

#[derive(Debug, Clone, PartialEq)]
enum PluginMapResult {
    Equivalent(String),
    NoEquivalent,
    Unknown,
}

fn map_plugin(fl_name: &str, mappings: &[PluginMap]) -> PluginMapResult {
    if let Some(m) = mappings.iter().find(|m| m.from_fl == fl_name) {
        match &m.to_hardwave {
            Some(target) => PluginMapResult::Equivalent(target.clone()),
            None => PluginMapResult::NoEquivalent,
        }
    } else {
        PluginMapResult::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_project() -> FlProject {
        FlProject {
            bpm: 140.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            channels: vec![
                FlChannel {
                    color: None,
                    layer_children: Vec::new(),
                    plugin_data: Vec::new(),
                    automation: Vec::new(),
                    name: "Kick".into(),
                    sample_path: Some("/samples/kick.wav".into()),
                    plugin_name: None,
                    pattern_steps: [true, false, false, false].repeat(4),
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
                FlChannel {
                    color: None,
                    layer_children: Vec::new(),
                    plugin_data: Vec::new(),
                    automation: Vec::new(),
                    name: "Sytrus Lead".into(),
                    sample_path: None,
                    plugin_name: Some("Sytrus".into()),
                    pattern_steps: vec![true; 16],
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
                FlChannel {
                    color: None,
                    layer_children: Vec::new(),
                    plugin_data: Vec::new(),
                    automation: Vec::new(),
                    name: "Harmor Pad".into(),
                    sample_path: None,
                    plugin_name: Some("Harmor".into()),
                    pattern_steps: vec![false; 16],
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
                FlChannel {
                    color: None,
                    layer_children: Vec::new(),
                    plugin_data: Vec::new(),
                    automation: Vec::new(),
                    name: "Mystery Plugin".into(),
                    sample_path: None,
                    plugin_name: Some("WeirdOne".into()),
                    pattern_steps: vec![false; 16],
                    kind: FlChannelKind::Other,
                    settings: FlChannelSettings::default(),
                },
            ],
            notes: vec![(
                1,
                vec![FlNote {
                    tick: 0,
                    length_ticks: 480,
                    pitch: 60,
                    velocity: 100,
                }],
            )],
            playlist_clips: vec![FlPlaylistClip {
                arrangement: 0,
                track_index: 0,
                start_tick: 0,
                length_ticks: 1920,
                content: FlClipContent::Pattern {
                    pattern_index: 0,
                    offset_ticks: 0,
                },
                muted: false,
            }],
            mixer: vec![FlMixerTrack {
                name: "Master".into(),
                ..Default::default()
            }],
            pattern_notes: Vec::new(),
            pattern_names: Vec::new(),
            pattern_colors: Vec::new(),
            playlist_track_colors: Vec::new(),
            playlist_track_names: Vec::new(),
            playlist_tracks_off: Vec::new(),
            automation_links: Vec::new(),
            playlist_tracks_solo: Vec::new(),
            arrangements: Vec::new(),
            current_arrangement: 0,
        }
    }

    #[test]
    fn summary_counts_aggregate_from_project() {
        let project = sample_project();
        let mappings = default_plugin_mappings();
        let report = summarize_import(&project, &mappings);
        assert_eq!(report.imported_channels, 4);
        assert_eq!(report.imported_notes, 1);
        assert_eq!(report.imported_playlist_clips, 1);
        assert_eq!(report.imported_mixer_tracks, 1);
    }

    #[test]
    fn summary_maps_sytrus_to_hardwave_fm() {
        let project = sample_project();
        let mappings = default_plugin_mappings();
        let report = summarize_import(&project, &mappings);
        assert!(report
            .findings
            .iter()
            .any(|f| f.message.contains("Sytrus") && f.message.contains("hardwave-fm")));
    }

    #[test]
    fn summary_warns_on_unknown_plugin_and_adds_to_skipped() {
        let project = sample_project();
        let mappings = default_plugin_mappings();
        let report = summarize_import(&project, &mappings);
        assert!(report.skipped_plugins.iter().any(|p| p == "WeirdOne"));
        assert!(report.skipped_plugins.iter().any(|p| p == "Harmor"));
        assert!(report.warning_count() >= 2);
        assert!(!report.has_errors());
    }

    #[test]
    fn default_mappings_contain_core_fl_natives() {
        let m = default_plugin_mappings();
        assert!(m.iter().any(|p| p.from_fl == "Sytrus"));
        assert!(m.iter().any(|p| p.from_fl == "3xOsc"));
        assert!(m.iter().any(|p| p.from_fl == "Fruity Limiter"));
    }

    #[test]
    fn report_severity_accessors() {
        let mut r = ImportReport::default();
        r.info(FindingCategory::Tempo, "ok");
        r.warn(FindingCategory::Plugin, "meh");
        r.error(FindingCategory::ChannelRack, "bad");
        assert!(r.has_errors());
        assert_eq!(r.warning_count(), 1);
    }

    #[test]
    fn automation_clip_count_derived_from_content_kind() {
        let mut project = sample_project();
        project.playlist_clips.push(FlPlaylistClip {
            arrangement: 0,
            track_index: 1,
            start_tick: 0,
            length_ticks: 960,
            content: FlClipContent::Automation {
                channel_index: 0,
                target: "master-volume".into(),
                offset_ticks: 0,
            },
            muted: false,
        });
        let report = summarize_import(&project, &default_plugin_mappings());
        assert_eq!(report.imported_automation_clips, 1);
    }

    use super::test_wrapper as wrapper;

    #[test]
    fn a_wrapped_plug_in_goes_by_its_own_name() {
        let slot = |name: Option<&str>, data| FlMixerSlot {
            index: 0,
            plugin: WRAPPER.into(),
            name: name.map(String::from),
            enabled: true,
            mix: 12_800,
            data,
        };
        let data = wrapper(&[(1, &[0; 4]), (54, b"Serum2"), (56, b"Xfer Records")]);
        assert_eq!(slot(None, data.clone()).label(), "Serum2");
        assert_eq!(slot(Some("Lead synth"), data).label(), "Lead synth");
        // Data that says it is longer than it is gives no name, not a crash.
        let mut lying = wrapper(&[(54, b"Serum2")]);
        lying[8] = 0xFF;
        assert_eq!(slot(None, lying).label(), WRAPPER);
        assert_eq!(slot(None, Vec::new()).label(), WRAPPER);
    }

    #[test]
    fn a_wrapped_vst3_comes_with_its_component_state() {
        let p = read_wrapper(&super::test_vst3_wrapper("Serum2", true, b"component")).unwrap();
        assert_eq!(p.name, "Serum2");
        assert_eq!(p.vendor, "Vendor");
        assert_eq!(p.format, FlPluginFormat::Vst3);
        assert!(p.instrument);
        assert!(p.file.ends_with("Serum2.vst3"));
        assert_eq!(
            p.state, b"component",
            "the component's, not the controller's"
        );
        assert_eq!(p.param_ids, vec![0, 7, 0x7072_6F67]);

        let effect = read_wrapper(&super::test_vst3_wrapper("OTT", false, b"")).unwrap();
        assert!(!effect.instrument);
        assert!(effect.state.is_empty(), "FL kept none");
    }

    #[test]
    fn vst2_and_clap_say_what_they_are_without_their_state() {
        let info = |kind: u32| kind.to_le_bytes().to_vec();
        let vst2 = wrapper(&[
            (50, &info(4)),
            (53, &[0xF7, 0xFF, 0xFF, 0xFF]),
            (54, b"Serum"),
        ]);
        let p = read_wrapper(&vst2).unwrap();
        assert_eq!((p.format, p.instrument), (FlPluginFormat::Vst2, true));
        assert!(p.state.is_empty());
        let clap = wrapper(&[
            (50, &info(11)),
            (54, b"Surge XT Effects"),
            (58, b"org.surge-synth-team.surge-xt-fx"),
        ]);
        let p = read_wrapper(&clap).unwrap();
        assert_eq!(p.format, FlPluginFormat::Clap);
        assert_eq!(
            p.clap_id.as_deref(),
            Some("org.surge-synth-team.surge-xt-fx")
        );
        // A kind FL has not written, or no name: nothing to go on.
        assert!(read_wrapper(&wrapper(&[(50, &info(99)), (54, b"X")])).is_none());
        assert!(read_wrapper(&wrapper(&[(50, &info(7))])).is_none());
        assert!(read_wrapper(&[]).is_none());
    }
}
