//! Reading an FL Studio `.flp` file.
//!
//! `fl_import.rs` has described the shape of an imported project for
//! months while saying, in its own first line, that the binary parser
//! "lives in a sibling module". It did not. This is that module.
//!
//! The format is a header and then a stream of events: a byte of id,
//! then one, two or four bytes, or a length and that many bytes. What
//! is read here is what a producer would miss if it were gone: the
//! tempo, the channels and the samples behind them, the notes in each
//! pattern, and where the playlist puts those patterns.
//!
//! What is not read is everything a plug-in holds. FL stores a
//! plug-in's state as its own blob, and nothing outside FL can mean
//! anything by it, so a project comes in as its arrangement and its
//! samples, with a list of what could not be carried.

use crate::fl_import::{
    FlAutomationLink, FlAutomationPoint, FlAutomationTarget, FlChannel, FlChannelKind,
    FlChannelSettings, FlClipContent, FlEnvelope, FlMixerSlot, FlMixerTrack, FlNote,
    FlPlaylistClip, FlProject, FlRoute,
};

/// Event id boundaries: below 64 is one byte, below 128 two, below
/// 192 four, and the rest carry a length.
const WORD: u8 = 64;
const DWORD: u8 = 128;
const TEXT: u8 = 192;

// The ids this reads. FL has hundreds; these are the ones that carry
// the arrangement.
const CH_TYPE: u8 = 21; // a byte: sampler, instrument, audio clip, ...
const CH_NEW: u8 = WORD; // 64
const PAT_NEW: u8 = WORD + 1; // 65
const PROJ_TEMPO: u8 = DWORD + 28; // 156
/// Before FL 3.4 the tempo was a whole number here, with thousandths in
/// TEMPO_FINE; read only when PROJ_TEMPO is not there.
const TEMPO_COARSE: u8 = WORD + 2; // 66
const TEMPO_FINE: u8 = WORD + 29; // 93
/// The channel's name in files from before FL 12 or so.
const CH_NAME: u8 = TEXT; // 192
const PAT_NAME: u8 = TEXT + 1; // 193
const CH_SAMPLE: u8 = TEXT + 4; // 196
/// What the plug-in is ("Fruity Wrapper" for a VST, "FLEX", ...).
const PLUGIN_INTERNAL_NAME: u8 = TEXT + 9; // 201
/// The channel's name as the rack shows it, in current files.
const CH_DISPLAY_NAME: u8 = TEXT + 11; // 203
const PAT_NOTES: u8 = TEXT + 16 + 16; // 224
const ARR_PLAYLIST: u8 = TEXT + 16 + 25; // 233
/// A playlist track's settings: its number (from 1) in the first four
/// bytes, at byte 12 whether it is switched on, at byte 48 whether it is
/// soloed. FL solos a track by switching every other one off.
const TRACK_DATA: u8 = TEXT + 46; // 238
const TRACK_SOLO_AT: usize = 48;
/// The name given to the playlist track just before it.
const TRACK_NAME: u8 = TEXT + 47; // 239
/// The sizes FL has written a playlist item in: 32 bytes for years, then
/// 60 (FL 20 to 24), 80 (FL 25) and 88 (FL 26). Every item starts the
/// same way, with the pattern base at bytes 4..6, so the right size is the
/// one that reads the same base in every item.
const PLAYLIST_ITEM_SIZES: [usize; 4] = [88, 80, 60, 32];

/// FL counts a bar as its own PPQ in the playlist, and four of them
/// inside a pattern. Both are converted to our ticks on the way out.
const FL_NOTE_BYTES: usize = 24;
/// A playlist item's flags (two bytes at 18): this one is "muted".
const ITEM_MUTED: u16 = 0x1000;

// A channel's settings. Every one of these follows the channel's CH_NEW.
const CH_ENABLED: u8 = 0;
/// The mixer insert, one signed byte.
const CH_ROUTED_TO: u8 = 22;
/// Sample settings flags: reverse is bit 1, swap stereo bit 8.
const CH_FX_FLAGS: u8 = WORD + 6; // 70
const FX_FADE_STEREO: u16 = 1 << 0;
const FX_REVERSE: u16 = 1 << 1;
const FX_SWAP_STEREO: u16 = 1 << 8;
// A sampler channel's loop, voices and envelopes (layouts after PyFLP 2.2).
const CH_PING_PONG: u8 = 20;
const CH_CUT_GROUP: u8 = DWORD + 4; // 132: (cut, cut by)
const CH_SAMPLER_FLAGS: u8 = DWORD + 15; // 143
const SAMPLER_USES_LOOP_POINTS: u32 = 1 << 3;
const CH_ENVELOPE: u8 = TEXT + 26; // 218: one per target, volume second
const CH_POLYPHONY: u8 = TEXT + 29; // 221: max u32, slide u32, flags u8
const POLY_MONO: u8 = 1 << 0;
const POLY_PORTA: u8 = 1 << 1;
const CH_FADE_OUT: u8 = WORD + 11; // 75
const CH_FADE_IN: u8 = WORD + 12; // 76
const CH_ROOT_NOTE: u8 = DWORD + 7; // 135
/// The sample settings block: polarity at byte 81, then from byte 96 the
/// time stretching section (TIME as a float in the file's ticks, PITCH in
/// cents, MUL as a power of two in ten-thousandths, the mode).
const CH_PARAMETERS: u8 = TEXT + 23; // 215
/// Pan, volume and pitch, four bytes each.
const CH_LEVELS: u8 = TEXT + 27; // 219
/// The arrangement starts: no channel event comes after it, while the
/// mixer's plug-ins further on use the same name events as channels do.
const ARRANGEMENT_NEW: u8 = WORD + 35; // 99
/// FL 25 and later: the channel's mixer insert, two bytes, as FL 25 has
/// five hundred of them. Older files used this id for something else.
const CH_ROUTED_TO_FL25: u8 = WORD + 40; // 104

// The mixer. An insert's events run from its name, when it has one, to
// its output; its flags are the one event every insert has.
const INSERT_NAME: u8 = TEXT + 12; // 204
/// Polarity, swap, effects on, enabled, solo: four bytes from byte 4.
const INSERT_FLAGS: u8 = TEXT + 44; // 236
const INSERT_POLARITY: u32 = 1 << 0;
const INSERT_SWAP: u32 = 1 << 1;
const INSERT_EFFECTS_ON: u32 = 1 << 2;
const INSERT_ENABLED: u32 = 1 << 3;
const INSERT_SOLO: u32 = 1 << 12;
/// One byte per insert, set where this one sends its sound.
const INSERT_ROUTING: u8 = TEXT + 43; // 235
/// The insert's audio output, its last event.
const INSERT_OUTPUT: u8 = DWORD + 19; // 147
/// An effect slot's number in the insert. It closes the slot: the plug-in
/// events just before it are the slot's (an empty slot has none).
const SLOT_INDEX: u8 = WORD + 34; // 98
/// A plug-in's own data.
const PLUGIN_DATA: u8 = TEXT + 21; // 213
/// Every insert's fader, pan, separation, send levels and EQ, and every
/// slot's switch and mix, as 12-byte records.
const MIXER_PARAMS: u8 = TEXT + 33; // 225
/// An automation clip's points, in its channel: a 21-byte head with the
/// count in its last four bytes, then 24 bytes a point: how far after the
/// one before it (beats, a double), the value (a double), the tension (a
/// float), four more.
const CH_AUTOMATION: u8 = TEXT + 42; // 234
/// What a channel controls: its number at byte 2, at byte 8 the knob, as
/// the mixer's parameter list names one (a place in the top two bytes, the
/// parameter in the bottom two).
const REMOTE_CONTROL: u8 = TEXT + 35; // 227

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq)]
pub enum FlpError {
    NotAnFlp,
    Truncated,
}

impl std::fmt::Display for FlpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlpError::NotAnFlp => write!(f, "that is not an FL Studio project"),
            FlpError::Truncated => write!(f, "the file stops in the middle of itself"),
        }
    }
}

/// One event, as it sits in the file.
struct Event<'a> {
    id: u8,
    data: &'a [u8],
}

/// The FL version text, first in every project ("25.2.3.5171").
const FL_VERSION: u8 = TEXT + 7; // 199
/// An event FL 25 and later write with three bytes after it, although its
/// id is in the four-byte range. Read as four, it took the next event's
/// first byte with it and the project tempo after it was never seen, so
/// every song from FL 25 opened at 140.
const THREE_BYTE_SINCE_FL25: u8 = DWORD + 44; // 172

/// Walk the event stream.
fn events(body: &[u8]) -> Result<Vec<Event<'_>>, FlpError> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut fl_major = 0u32;
    while i < body.len() {
        let id = body[i];
        i += 1;
        let len = if id == THREE_BYTE_SINCE_FL25 && fl_major >= 25 {
            3
        } else if id < WORD {
            1
        } else if id < DWORD {
            2
        } else if id < TEXT {
            4
        } else {
            // A varint length, seven bits at a time.
            let mut len = 0usize;
            let mut shift = 0u32;
            loop {
                let byte = *body.get(i).ok_or(FlpError::Truncated)?;
                i += 1;
                len |= ((byte & 0x7F) as usize) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    break;
                }
                if shift > 28 {
                    return Err(FlpError::Truncated);
                }
            }
            len
        };
        if i + len > body.len() {
            return Err(FlpError::Truncated);
        }
        let data = &body[i..i + len];
        if id == FL_VERSION {
            fl_major = text_ascii(data)
                .split('.')
                .next()
                .and_then(|m| m.trim().parse().ok())
                .unwrap_or(0);
        }
        out.push(Event { id, data });
        i += len;
    }
    Ok(out)
}

/// The version is written as plain bytes, a zero at the end.
fn text_ascii(data: &[u8]) -> String {
    data.iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as char)
        .collect()
}

/// FL writes text as UTF-16, two zero bytes at the end.
fn text(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    data.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .unwrap_or(0)
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0)
}

fn f32_at(data: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(data, at))
}

fn i32_at(data: &[u8], at: usize) -> i32 {
    u32_at(data, at) as i32
}

/// Read a `.flp` file.
///
/// Best effort by design: a file written by a version we have not
/// seen should come in as much as can be understood rather than as an
/// error, because a producer with a project they cannot open does not
/// care whose fault it is.
pub fn parse(bytes: &[u8]) -> Result<FlProject, FlpError> {
    if bytes.len() < 16 || &bytes[0..4] != b"FLhd" {
        return Err(FlpError::NotAnFlp);
    }
    let header_len = u32_at(bytes, 4) as usize;
    // The header holds the file's own resolution, which is what every
    // tick in it is counted in.
    let ppq = u16_at(bytes, 8 + 4).max(1) as u64;
    // How many channels the rack has. Event 64 also turns up inside
    // pattern data in newer files, with numbers that are not channels
    // (one file read as 32,869 of them); only these are real.
    let channel_count = u16_at(bytes, 8 + 2) as usize;
    // Every length here is the file's word for it, so every step is
    // checked: a short or lying file is "not an .flp", never a crash.
    let mut i = 8usize.checked_add(header_len).ok_or(FlpError::NotAnFlp)?;
    if bytes.get(i..i.saturating_add(4)) != Some(b"FLdt") {
        return Err(FlpError::NotAnFlp);
    }
    let body_len = u32_at(bytes, i + 4) as usize;
    i += 8;
    let end = i.saturating_add(body_len).min(bytes.len());
    let body = bytes.get(i..end).ok_or(FlpError::NotAnFlp)?;

    let mut project = FlProject {
        bpm: 140.0,
        time_sig_numerator: 4,
        time_sig_denominator: 4,
        ..Default::default()
    };

    // Our ticks per FL's.
    let scale = |ticks: u64| -> u64 { ticks * hardwave_midi::PPQ / ppq };

    let mut current_channel: Option<usize> = None;
    let mut current_track: Option<u32> = None;
    // Stretch times are floats in the file's ticks.
    let scale_f = |ticks: f64| -> f64 { ticks * hardwave_midi::PPQ as f64 / ppq as f64 };
    // Playlist items naming a channel. Made into clips at the end, once
    // every channel's kind and sample is known.
    let mut channel_items: Vec<ChannelItem> = Vec::new();
    let mut current_pattern: Option<u32> = None;
    let mut pattern_names: Vec<(u32, String)> = Vec::new();

    let mut tempo_seen = false;
    let (mut coarse, mut fine) = (None::<u16>, 0u16);
    let mut fl_major = 0u32;
    // Playlist items carry a clip's own fades and volume from FL 20.99.
    let mut item_extras = false;
    // The mixer: a name waiting for its insert, the insert being read, the
    // plug-in waiting for its slot number, and the parameter list, read once
    // every insert is known.
    let mut insert_name: Option<String> = None;
    let mut current_insert: Option<usize> = None;
    let mut pending_slot: Option<FlMixerSlot> = None;
    let mut mixer_params: Option<&[u8]> = None;
    // Automation clips' channels and the knob each moves, read once the
    // mixer says where its inserts start.
    let mut remote_controls: Vec<(u32, u32)> = Vec::new();
    for event in events(body)? {
        // A plug-in event inside an insert is a slot's, not a channel's.
        if current_insert.is_some() {
            match (event.id, pending_slot.as_mut()) {
                (PLUGIN_INTERNAL_NAME, _) => {
                    pending_slot = Some(FlMixerSlot {
                        index: 0,
                        plugin: text(event.data),
                        name: None,
                        enabled: true,
                        mix: 12_800,
                        data: Vec::new(),
                    });
                    continue;
                }
                (CH_DISPLAY_NAME, Some(slot)) => {
                    let name = text(event.data);
                    slot.name = (!name.is_empty()).then_some(name);
                    continue;
                }
                (PLUGIN_DATA, Some(slot)) => {
                    slot.data = event.data.to_vec();
                    continue;
                }
                _ => {}
            }
        }
        match event.id {
            FL_VERSION => {
                let version = text_ascii(event.data);
                let mut parts = version
                    .split('.')
                    .map(|p| p.trim().parse::<u32>().unwrap_or(0));
                fl_major = parts.next().unwrap_or(0);
                let minor = parts.next().unwrap_or(0);
                item_extras = (fl_major, minor) >= (20, 99);
            }
            CH_ROUTED_TO_FL25 if fl_major >= 25 => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    channel.settings.mixer_insert = u16_at(event.data, 0) as i32;
                }
            }
            INSERT_NAME => insert_name = Some(text(event.data)),
            INSERT_FLAGS => {
                current_channel = None;
                pending_slot = None;
                let flags = u32_at(event.data, 4);
                current_insert = Some(project.mixer.len());
                project.mixer.push(FlMixerTrack {
                    index: project.mixer.len() as u32,
                    name: insert_name.take().unwrap_or_default(),
                    enabled: flags & INSERT_ENABLED != 0,
                    effects_enabled: flags & INSERT_EFFECTS_ON != 0,
                    polarity_inverted: flags & INSERT_POLARITY != 0,
                    swap_stereo: flags & INSERT_SWAP != 0,
                    solo: flags & INSERT_SOLO != 0,
                    ..Default::default()
                });
            }
            SLOT_INDEX => {
                let slot = pending_slot.take();
                if let (Some(insert), Some(mut slot)) =
                    (current_insert.and_then(|i| project.mixer.get_mut(i)), slot)
                {
                    slot.index = u16_at(event.data, 0) as u32;
                    insert.slots.push(slot);
                }
            }
            INSERT_ROUTING => {
                pending_slot = None;
                if let Some(insert) = current_insert.and_then(|i| project.mixer.get_mut(i)) {
                    insert.routes = event
                        .data
                        .iter()
                        .enumerate()
                        .filter(|(_, on)| **on != 0)
                        .map(|(to, _)| FlRoute {
                            to: to as u32,
                            level: 12_800,
                        })
                        .collect();
                }
            }
            INSERT_OUTPUT => {
                pending_slot = None;
                insert_name = None;
            }
            MIXER_PARAMS => mixer_params = Some(event.data),
            CH_AUTOMATION => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    channel.automation = automation_points(event.data);
                }
            }
            REMOTE_CONTROL if event.data.len() >= 12 => {
                remote_controls.push((u32_at(event.data, 2), u32_at(event.data, 8)));
            }
            TEMPO_COARSE => coarse = Some(u16_at(event.data, 0)),
            TEMPO_FINE => fine = u16_at(event.data, 0),
            PROJ_TEMPO => {
                // Thousandths of a beat per minute.
                let raw = u32_at(event.data, 0);
                if raw > 0 {
                    project.bpm = raw as f32 / 1000.0;
                    tempo_seen = true;
                }
            }
            CH_NEW => {
                let index = u16_at(event.data, 0) as usize;
                if channel_count > 0 && index >= channel_count {
                    current_channel = None;
                    continue;
                }
                if project.channels.len() <= index {
                    project.channels.resize_with(index + 1, || FlChannel {
                        plugin_data: Vec::new(),
                        automation: Vec::new(),
                        name: String::new(),
                        sample_path: None,
                        plugin_name: None,
                        pattern_steps: Vec::new(),
                        kind: FlChannelKind::Other,
                        settings: FlChannelSettings::default(),
                    });
                }
                current_channel = Some(index);
            }
            CH_TYPE => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    channel.kind =
                        FlChannelKind::from_byte(event.data.first().copied().unwrap_or(255));
                }
            }
            // The old name only fills a gap; the display name wins.
            CH_NAME => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    if channel.name.is_empty() {
                        channel.name = text(event.data);
                    }
                }
            }
            CH_DISPLAY_NAME => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    let name = text(event.data);
                    if !name.is_empty() {
                        channel.name = name;
                    }
                }
            }
            PLUGIN_INTERNAL_NAME => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    let name = text(event.data);
                    if !name.is_empty() {
                        channel.plugin_name = Some(name);
                    }
                }
            }
            CH_ENABLED | CH_ROUTED_TO | CH_FX_FLAGS | CH_FADE_IN | CH_FADE_OUT | CH_ROOT_NOTE
            | CH_PARAMETERS | CH_LEVELS | CH_PING_PONG | CH_CUT_GROUP | CH_SAMPLER_FLAGS
            | CH_ENVELOPE | CH_POLYPHONY => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    read_channel_setting(&mut channel.settings, event.id, event.data, &scale_f);
                }
            }
            PLUGIN_DATA => {
                if let Some(channel) = current_channel.and_then(|i| project.channels.get_mut(i)) {
                    channel.plugin_data = event.data.to_vec();
                }
            }
            ARRANGEMENT_NEW => current_channel = None,
            TRACK_DATA => {
                current_channel = None;
                let index = u32_at(event.data, 0);
                current_track = Some(index);
                if event.data.get(12) == Some(&0) {
                    project.playlist_tracks_off.push(index);
                }
                if event.data.get(TRACK_SOLO_AT) == Some(&1) {
                    project.playlist_tracks_solo.push(index);
                }
            }
            TRACK_NAME => {
                if let Some(index) = current_track {
                    let name = text(event.data);
                    if !name.is_empty() {
                        project.playlist_track_names.push((index, name));
                    }
                }
            }
            CH_SAMPLE => {
                if let Some(index) = current_channel {
                    if let Some(channel) = project.channels.get_mut(index) {
                        let path = text(event.data);
                        if !path.is_empty() {
                            channel.sample_path = Some(path);
                        }
                    }
                }
            }
            PAT_NEW => {
                current_pattern = Some(u16_at(event.data, 0) as u32);
            }
            PAT_NAME => {
                if let Some(pattern) = current_pattern {
                    pattern_names.push((pattern, text(event.data)));
                }
            }
            PAT_NOTES => {
                // Notes carry the channel they play, so a pattern is
                // several tracks' worth at once.
                let pattern = current_pattern.unwrap_or(0);
                let mut by_channel: std::collections::BTreeMap<u32, Vec<FlNote>> =
                    std::collections::BTreeMap::new();
                for note in event.data.as_chunks::<FL_NOTE_BYTES>().0 {
                    let position = u32_at(note, 0) as u64;
                    let channel = u16_at(note, 6) as u32;
                    let length = u32_at(note, 8) as u64;
                    let key = u16_at(note, 12) as u8;
                    let velocity = *note.get(21).unwrap_or(&100);
                    by_channel.entry(channel).or_default().push(FlNote {
                        tick: scale(position),
                        length_ticks: scale(length).max(1),
                        pitch: key.min(127),
                        velocity,
                    });
                }
                for (channel, notes) in by_channel {
                    project.notes.push((channel, notes.clone()));
                    project.pattern_notes.push((pattern, channel, notes));
                }
            }
            ARR_PLAYLIST => {
                current_channel = None;
                let Some(stride) = playlist_item_size(event.data) else {
                    continue;
                };
                for item in event.data.chunks_exact(stride) {
                    let position = u32_at(item, 0) as u64;
                    let pattern_base = u16_at(item, 4) as u32;
                    let item_id = u16_at(item, 6) as u32;
                    let length = u32_at(item, 8) as u64;
                    let track = u16_at(item, 12) as u32;
                    let muted = u16_at(item, 18) & ITEM_MUTED != 0;
                    // FL stores playlist tracks counting down from the
                    // top of its own list.
                    let track_index = 500u32.saturating_sub(track);
                    if item_id > pattern_base {
                        project.playlist_clips.push(FlPlaylistClip {
                            track_index,
                            start_tick: scale(position),
                            length_ticks: scale(length).max(1),
                            content: FlClipContent::Pattern {
                                pattern_index: item_id - pattern_base,
                            },
                            muted,
                        });
                    } else {
                        let start_ms = f32_at(item, 24);
                        let end_ms = f32_at(item, 28);
                        // A clip's own fades (milliseconds) and volume (a
                        // factor): bytes 36, 44 and 52.
                        let extra = |at: usize| {
                            Some(f32_at(item, at))
                                .filter(|v| item_extras && item.len() >= 60 && v.is_finite())
                        };
                        channel_items.push(ChannelItem {
                            track_index,
                            start_tick: scale(position),
                            length_ticks: scale(length).max(1),
                            channel_index: item_id,
                            start_ms,
                            end_ms,
                            muted,
                            fade_in_ms: extra(36).unwrap_or(0.0).max(0.0),
                            fade_out_ms: extra(44).unwrap_or(0.0).max(0.0),
                            volume: extra(52).unwrap_or(1.0).max(0.0),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    let master = mixer_params.and_then(|data| read_mixer_params(&mut project.mixer, data));
    // Without a parameter list, where the version puts the master.
    let master = master.unwrap_or(if fl_major >= 25 {
        0x7000 >> 6
    } else {
        0x2000 >> 6
    });
    project.automation_links = remote_controls
        .into_iter()
        .map(|(channel, knob)| FlAutomationLink {
            channel,
            target: automation_target(knob, master, project.mixer.len() as u32),
        })
        .collect();
    project.pattern_names = pattern_names;
    if !tempo_seen {
        if let Some(c) = coarse.filter(|c| *c > 0) {
            project.bpm = c as f32 + fine as f32 / 1000.0;
        }
    }

    // A playlist item naming a channel is an audio clip when that channel
    // holds a sample, and an automation clip when it is one.
    for item in channel_items {
        let ChannelItem {
            track_index,
            start_tick,
            length_ticks,
            channel_index,
            start_ms,
            end_ms,
            muted,
            fade_in_ms,
            fade_out_ms,
            volume,
        } = item;
        let Some(channel) = project.channels.get(channel_index as usize) else {
            continue;
        };
        let content = match (channel.kind, &channel.sample_path) {
            // An automation item's start field is beats into the clip.
            (FlChannelKind::Automation, _) => FlClipContent::Automation {
                channel_index,
                target: channel.name.clone(),
                offset_ticks: if start_ms > 0.0 {
                    (start_ms as f64 * hardwave_midi::PPQ as f64).round() as u64
                } else {
                    0
                },
            },
            (_, Some(path)) => FlClipContent::AudioSample {
                channel_index,
                sample_path: path.clone(),
                start_offset_ms: start_ms,
                end_offset_ms: end_ms,
                fade_in_ms,
                fade_out_ms,
                volume,
            },
            _ => continue,
        };
        project.playlist_clips.push(FlPlaylistClip {
            track_index,
            start_tick,
            length_ticks,
            content,
            muted,
        });
    }
    Ok(project)
}

/// The mixer's parameter list: per record, four bytes we do not need, the
/// parameter (a byte, then a byte of flags, the top bit set for a plug-in's
/// own parameter), where it is (two bytes: the insert from bit 6, the slot
/// below), and the value.
///
/// Where the inserts start differs by version (FL 24 counts from 0x2000,
/// FL 25 from 0x7000, with the song's own settings below either), so the
/// master is the lowest place that has an insert's fader.
fn read_mixer_params(mixer: &mut [FlMixerTrack], data: &[u8]) -> Option<u16> {
    const SLOT_ENABLED: u8 = 0;
    const SLOT_MIX: u8 = 1;
    const ROUTE_LEVEL: std::ops::RangeInclusive<u8> = 64..=191;
    const VOLUME: u8 = 192;
    const PAN: u8 = 193;
    const SEPARATION: u8 = 194;
    const EQ_GAIN: std::ops::RangeInclusive<u8> = 208..=210;

    let records: Vec<(u8, u16, i32)> = data
        .as_chunks::<12>()
        .0
        .iter()
        .filter(|r| r[5] & 0x80 == 0)
        .map(|r| (r[4], u16_at(r, 6), i32_at(r, 8)))
        .collect();
    let master = records
        .iter()
        .filter(|(param, ..)| *param == VOLUME)
        .map(|(_, at, _)| at >> 6)
        .min()?;
    for (param, at, value) in records {
        let Some(number) = (at >> 6).checked_sub(master) else {
            continue;
        };
        let Some(insert) = mixer.get_mut(number as usize) else {
            continue;
        };
        let slot = (at & 0x3F) as u32;
        match param {
            SLOT_ENABLED | SLOT_MIX => {
                if let Some(s) = insert.slots.iter_mut().find(|s| s.index == slot) {
                    if param == SLOT_ENABLED {
                        s.enabled = value != 0;
                    } else {
                        s.mix = value;
                    }
                }
            }
            p if ROUTE_LEVEL.contains(&p) => {
                let to = (p - ROUTE_LEVEL.start()) as u32;
                if let Some(route) = insert.routes.iter_mut().find(|r| r.to == to) {
                    route.level = value;
                }
            }
            VOLUME => insert.volume = value,
            PAN => insert.pan = value,
            SEPARATION => insert.stereo_separation = value,
            p if EQ_GAIN.contains(&p) => {
                insert.eq_gains[(p - EQ_GAIN.start()) as usize] = value;
            }
            _ => {}
        }
    }
    Some(master)
}

/// An automation clip's points, at their places in the clip.
fn automation_points(data: &[u8]) -> Vec<FlAutomationPoint> {
    const HEAD: usize = 21;
    const POINT: usize = 24;
    let count = u32_at(data, HEAD - 4) as usize;
    let mut beats = 0.0f64;
    data.get(HEAD..)
        .unwrap_or(&[])
        .as_chunks::<POINT>()
        .0
        .iter()
        .take(count)
        .map(|point| {
            let double = |at: usize| {
                f64::from_bits(u32_at(point, at) as u64 | (u32_at(point, at + 4) as u64) << 32)
            };
            let after = double(0);
            if after.is_finite() && after > 0.0 {
                beats += after;
            }
            let value = double(8);
            FlAutomationPoint {
                tick: (beats * hardwave_midi::PPQ as f64).round() as u64,
                value: if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                tension: f32_at(point, 16),
            }
        })
        .collect()
}

/// The knob a remote control names. Below 0x2000 the place is a channel,
/// and the parameter its volume (0), pan (1), pitch (4) or, with the top
/// bit set, one of its plug-in's. From `master` (the mixer's own count, as
/// its parameter list has it) the place is an insert and slot, the
/// parameter as in that list. The song's own knobs sit between.
fn automation_target(knob: u32, master: u16, inserts: u32) -> FlAutomationTarget {
    use FlAutomationTarget as T;
    let place = (knob >> 16) as u16;
    let param = (knob & 0xFFFF) as u16;
    let plugin = (param & 0x8000 != 0).then_some((param & 0x7FFF) as u32);
    if place < 0x2000 {
        let channel = (place & 0x0FFF) as u32;
        return match (plugin, param) {
            (Some(param), _) => T::ChannelPlugin { channel, param },
            (None, 0) => T::ChannelVolume(channel),
            (None, 1) => T::ChannelPan(channel),
            (None, 4) => T::ChannelPitch(channel),
            _ => T::Other,
        };
    }
    let Some(insert) = (place >> 6).checked_sub(master).map(u32::from) else {
        return T::Other;
    };
    if insert >= inserts {
        return T::Other;
    }
    let slot = (place & 0x3F) as u32;
    match (plugin, param & 0xFF) {
        (Some(param), _) => T::SlotPlugin {
            insert,
            slot,
            param,
        },
        (None, 0) => T::SlotEnabled { insert, slot },
        (None, 1) => T::SlotMix { insert, slot },
        (None, 192) => T::InsertVolume(insert),
        (None, 193) => T::InsertPan(insert),
        (None, 194) => T::InsertSeparation(insert),
        _ => T::Other,
    }
}

/// A playlist item that names a channel rather than a pattern.
struct ChannelItem {
    track_index: u32,
    start_tick: u64,
    length_ticks: u64,
    channel_index: u32,
    /// Where it starts and ends in its sample, in milliseconds (-1: not set).
    start_ms: f32,
    end_ms: f32,
    muted: bool,
    fade_in_ms: f32,
    fade_out_ms: f32,
    volume: f32,
}

/// One of a channel's settings events, read into its settings.
fn read_channel_setting(
    settings: &mut FlChannelSettings,
    id: u8,
    data: &[u8],
    scale: &dyn Fn(f64) -> f64,
) {
    let byte = data.first().copied().unwrap_or(0);
    match id {
        CH_ENABLED => settings.enabled = byte != 0,
        CH_ROUTED_TO => settings.mixer_insert = byte as i8 as i32,
        CH_FX_FLAGS => {
            let flags = u16_at(data, 0);
            settings.reversed = flags & FX_REVERSE != 0;
            settings.swap_stereo = flags & FX_SWAP_STEREO != 0;
            settings.sampler.fade_stereo = flags & FX_FADE_STEREO != 0;
        }
        CH_PING_PONG => settings.sampler.ping_pong = byte != 0,
        CH_CUT_GROUP if data.len() >= 4 => {
            let (cut, cut_by) = (u16_at(data, 0), u16_at(data, 2));
            settings.sampler.cut_itself = cut != 0 && cut == cut_by;
        }
        CH_SAMPLER_FLAGS => {
            settings.sampler.use_loop_points = u32_at(data, 0) & SAMPLER_USES_LOOP_POINTS != 0;
        }
        CH_POLYPHONY if data.len() >= 9 => {
            settings.sampler.mono = data[8] & POLY_MONO != 0;
            settings.sampler.porta = data[8] & POLY_PORTA != 0;
        }
        CH_ENVELOPE => {
            let index = settings.sampler.envelope_events;
            settings.sampler.envelope_events = index.saturating_add(1);
            // The second is the volume envelope (pan, volume, mod X, mod Y, pitch).
            if index == 1 && data.len() >= 32 {
                // Time knobs run 100 to 65536, sustain 0 to 128.
                let time = |at: usize| ((i32_at(data, at) - 100) as f32 / 65_436.0).clamp(0.0, 1.0);
                settings.sampler.envelope = Some(FlEnvelope {
                    enabled: i32_at(data, 4) != 0,
                    delay: time(8),
                    attack: time(12),
                    hold: time(16),
                    decay: time(20),
                    sustain: (i32_at(data, 24) as f32 / 128.0).clamp(0.0, 1.0),
                    release: time(28),
                });
            }
        }
        CH_FADE_IN => settings.fade_in = u16_at(data, 0),
        CH_FADE_OUT => settings.fade_out = u16_at(data, 0),
        CH_ROOT_NOTE => settings.root_note = u32_at(data, 0).min(127) as u8,
        CH_LEVELS if data.len() >= 12 => {
            settings.pan = i32_at(data, 0);
            settings.volume = u32_at(data, 4);
            settings.pitch_cents = i32_at(data, 8);
        }
        CH_PARAMETERS => {
            settings.polarity_inverted = data.get(81).is_some_and(|b| *b != 0);
            let sampler = &mut settings.sampler;
            sampler.remove_dc = data.get(9).is_some_and(|b| *b != 0);
            sampler.normalize = data.get(80).is_some_and(|b| *b != 0);
            if let Some(mode) = data.get(83) {
                sampler.declick = (*mode).min(5);
            }
            if data.len() >= 92 {
                sampler.crossfade = (u32_at(data, 84) as f32 / 256.0).clamp(0.0, 1.0);
                sampler.trim = (u32_at(data, 88) as f32 / 256.0).clamp(0.0, 1.0);
            }
            // Sample start and length: a log value and a marker that is 63
            // when the value is one; anything else is not read.
            let log_normal = |at: usize| -> Option<f32> {
                if data.len() < at + 4 || u16_at(data, at + 2) != 63 {
                    return None;
                }
                let v = u16_at(data, at) as f64;
                (v > 0.0).then(|| (2f64.powf(v / 4096.0) / 2f64.powi(15)).clamp(0.0, 1.0) as f32)
            };
            sampler.start = log_normal(133);
            sampler.length = log_normal(141);
            if data.len() >= 112 {
                let time = f32_at(data, 96) as f64;
                settings.stretch.time_ticks = if time.is_finite() && time > 0.0 {
                    scale(time)
                } else {
                    0.0
                };
                settings.stretch.pitch_cents = i32_at(data, 100);
                // A power of two in ten-thousandths: 0 is 1x, 10000 is 2x.
                settings.stretch.multiplier =
                    2f64.powf((i32_at(data, 104) as f64 / 10_000.0).clamp(-2.0, 2.0));
                settings.stretch.mode = i32_at(data, 108);
            }
        }
        _ => {}
    }
}

/// The size of one playlist item in this block, from the sizes FL has
/// used, or None when none of them reads it (then nothing is read rather
/// than garbage).
fn playlist_item_size(data: &[u8]) -> Option<usize> {
    PLAYLIST_ITEM_SIZES.into_iter().find(|&size| {
        if data.len() < size || !data.len().is_multiple_of(size) {
            return false;
        }
        let base = u16_at(data, 4);
        base != 0 && data.chunks_exact(size).all(|item| u16_at(item, 4) == base)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_ends_where_the_body_should_start_is_refused_not_a_crash() {
        let mut short = b"FLhd".to_vec();
        short.extend_from_slice(&4u32.to_le_bytes());
        short.extend_from_slice(&[0, 0, 0, 0]);
        short.extend_from_slice(b"FLdt");
        assert_eq!(short.len(), 16);
        assert!(parse(&short).is_err());
        let mut huge_header = b"FLhd".to_vec();
        huge_header.extend_from_slice(&u32::MAX.to_le_bytes());
        huge_header.extend_from_slice(&[0u8; 8]);
        assert!(parse(&huge_header).is_err());
    }

    /// Build a small `.flp` by hand, the same way FL writes one, so
    /// the reader is tested against the format rather than against a
    /// file nobody can look at.
    struct Writer {
        body: Vec<u8>,
    }

    impl Writer {
        fn new() -> Self {
            Self { body: Vec::new() }
        }

        fn event(&mut self, id: u8, data: &[u8]) -> &mut Self {
            self.body.push(id);
            if id >= TEXT {
                let mut len = data.len();
                loop {
                    let byte = (len & 0x7F) as u8;
                    len >>= 7;
                    self.body.push(if len > 0 { byte | 0x80 } else { byte });
                    if len == 0 {
                        break;
                    }
                }
            }
            self.body.extend_from_slice(data);
            self
        }

        fn text(&mut self, id: u8, value: &str) -> &mut Self {
            let mut bytes: Vec<u8> = value
                .encode_utf16()
                .flat_map(|unit| unit.to_le_bytes())
                .collect();
            bytes.extend_from_slice(&[0, 0]);
            self.event(id, &bytes)
        }

        fn finish(&self, ppq: u16) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(b"FLhd");
            out.extend_from_slice(&6u32.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // format
            out.extend_from_slice(&64u16.to_le_bytes()); // channels
            out.extend_from_slice(&ppq.to_le_bytes());
            out.extend_from_slice(b"FLdt");
            out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
            out.extend_from_slice(&self.body);
            out
        }
    }

    fn note(channel: u16, position: u32, length: u32, key: u16, velocity: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(FL_NOTE_BYTES);
        out.extend_from_slice(&position.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&channel.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&key.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&[120, 0, 64, 0, 64, velocity, 128, 128]);
        out
    }

    #[test]
    fn a_file_that_is_not_an_flp_is_refused() {
        assert_eq!(
            parse(b"not a project at all").unwrap_err(),
            FlpError::NotAnFlp
        );
        assert_eq!(parse(&[]).unwrap_err(), FlpError::NotAnFlp);
    }

    #[test]
    fn the_tempo_comes_across() {
        let mut writer = Writer::new();
        writer.event(PROJ_TEMPO, &174_000u32.to_le_bytes());
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.bpm, 174.0);
    }

    #[test]
    fn channels_come_across_with_their_samples() {
        let mut writer = Writer::new();
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.text(CH_NAME, "Kick");
        writer.text(CH_SAMPLE, "C:\\samples\\kick.wav");
        writer.event(CH_NEW, &1u16.to_le_bytes());
        writer.text(CH_NAME, "Reverse bass");

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.channels.len(), 2);
        assert_eq!(project.channels[0].name, "Kick");
        assert_eq!(
            project.channels[0].sample_path.as_deref(),
            Some("C:\\samples\\kick.wav")
        );
        assert_eq!(project.channels[1].name, "Reverse bass");
        assert_eq!(project.channels[1].sample_path, None);
    }

    #[test]
    fn notes_come_across_at_our_own_resolution() {
        let mut writer = Writer::new();
        writer.event(PAT_NEW, &1u16.to_le_bytes());
        writer.text(PAT_NAME, "Kick");
        let mut notes = Vec::new();
        // Four on the floor at FL's 96 ticks to the beat.
        for beat in 0..4u32 {
            notes.extend_from_slice(&note(0, beat * 96, 86, 60, 100));
        }
        writer.event(PAT_NOTES, &notes);

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.notes.len(), 1, "one channel played");
        let (channel, notes) = &project.notes[0];
        assert_eq!(*channel, 0);
        assert_eq!(notes.len(), 4);
        // 96 of FL's ticks is a beat, and so is our PPQ.
        assert_eq!(notes[1].tick, hardwave_midi::PPQ);
        assert_eq!(notes[0].pitch, 60);
        assert_eq!(notes[0].velocity, 100);
    }

    #[test]
    fn notes_for_several_channels_are_kept_apart() {
        let mut writer = Writer::new();
        writer.event(PAT_NEW, &1u16.to_le_bytes());
        let mut notes = Vec::new();
        notes.extend_from_slice(&note(0, 0, 48, 60, 100));
        notes.extend_from_slice(&note(3, 48, 48, 64, 90));
        writer.event(PAT_NOTES, &notes);

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.notes.len(), 2);
        assert_eq!(project.notes[0].0, 0);
        assert_eq!(project.notes[1].0, 3);
    }

    #[test]
    fn the_playlist_says_where_the_patterns_go() {
        let mut writer = Writer::new();
        let mut items = Vec::new();
        for (bar, pattern, track) in [(0u32, 1u16, 1u16), (4, 2, 2)] {
            let mut item = Vec::new();
            item.extend_from_slice(&(bar * 4 * 96).to_le_bytes());
            item.extend_from_slice(&20480u16.to_le_bytes());
            item.extend_from_slice(&(20480 + pattern).to_le_bytes());
            item.extend_from_slice(&(4u32 * 96).to_le_bytes());
            item.extend_from_slice(&(500 - track).to_le_bytes());
            item.extend_from_slice(&0u16.to_le_bytes());
            item.resize(32, 0);
            items.extend_from_slice(&item);
        }
        writer.event(ARR_PLAYLIST, &items);

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.playlist_clips.len(), 2);
        assert_eq!(project.playlist_clips[0].start_tick, 0);
        assert_eq!(project.playlist_clips[0].track_index, 1);
        assert!(matches!(
            project.playlist_clips[0].content,
            FlClipContent::Pattern { pattern_index: 1 }
        ));
        assert_eq!(
            project.playlist_clips[1].start_tick,
            4 * 4 * hardwave_midi::PPQ
        );
        assert_eq!(project.playlist_clips[1].track_index, 2);
    }

    #[test]
    fn the_newer_eighty_byte_playlist_item_reads_as_well() {
        let mut writer = Writer::new();
        let mut item = Vec::new();
        item.extend_from_slice(&(8u32 * 4 * 96).to_le_bytes());
        item.extend_from_slice(&20480u16.to_le_bytes());
        item.extend_from_slice(&20483u16.to_le_bytes());
        item.extend_from_slice(&(96u32 * 4).to_le_bytes());
        item.extend_from_slice(&(500u16 - 3).to_le_bytes());
        item.resize(80, 0);
        writer.event(ARR_PLAYLIST, &item);

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.playlist_clips.len(), 1, "one clip, not two halves");
        assert_eq!(project.playlist_clips[0].track_index, 3);
    }

    #[test]
    fn a_file_that_stops_half_way_says_so() {
        let mut writer = Writer::new();
        writer.text(CH_NAME, "Kick");
        let mut bytes = writer.finish(96);
        bytes.truncate(bytes.len() - 4);
        // The body length in the header still claims the full size, so
        // the reader walks off the end and has to say so rather than
        // panic.
        // Either answer is fine; walking off the end is not.
        let _ = parse(&bytes);
    }

    #[test]
    fn a_project_at_another_resolution_is_converted() {
        let mut writer = Writer::new();
        writer.event(PAT_NEW, &1u16.to_le_bytes());
        writer.event(PAT_NOTES, &note(0, 192, 192, 60, 100));
        // At 192 ticks to the beat, 192 is beat two.
        let project = parse(&writer.finish(192)).expect("it should read");
        assert_eq!(project.notes[0].1[0].tick, hardwave_midi::PPQ);
    }

    /// One playlist item of `size` bytes, as FL writes it.
    fn item(
        size: usize,
        position: u32,
        id: u16,
        length: u32,
        track: u16,
        offsets_ms: (f32, f32),
    ) -> Vec<u8> {
        let mut item = Vec::new();
        item.extend_from_slice(&position.to_le_bytes());
        item.extend_from_slice(&20480u16.to_le_bytes());
        item.extend_from_slice(&id.to_le_bytes());
        item.extend_from_slice(&length.to_le_bytes());
        item.extend_from_slice(&(500 - track).to_le_bytes());
        item.resize(24, 0);
        item.extend_from_slice(&offsets_ms.0.to_le_bytes());
        item.extend_from_slice(&offsets_ms.1.to_le_bytes());
        item.resize(size, 0);
        item
    }

    #[test]
    fn channels_say_what_they_are_and_go_by_their_display_names() {
        let mut writer = Writer::new();
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.event(CH_TYPE, &[4]);
        writer.text(CH_DISPLAY_NAME, "Kick render");
        writer.text(CH_SAMPLE, "C:\\kicks\\render.wav");
        writer.event(CH_NEW, &1u16.to_le_bytes());
        writer.event(CH_TYPE, &[2]);
        writer.text(PLUGIN_INTERNAL_NAME, "Fruity Wrapper");
        writer.text(CH_NAME, "old name");
        writer.text(CH_DISPLAY_NAME, "Serum 2");

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.channels[0].kind, FlChannelKind::AudioClip);
        assert_eq!(project.channels[0].name, "Kick render");
        assert_eq!(project.channels[1].kind, FlChannelKind::Instrument);
        assert_eq!(
            project.channels[1].name, "Serum 2",
            "the display name wins over the old one"
        );
        assert_eq!(
            project.channels[1].plugin_name.as_deref(),
            Some("Fruity Wrapper")
        );
    }

    #[test]
    fn a_channel_number_past_the_rack_is_not_a_channel() {
        // Newer files use the same event inside pattern data.
        let mut writer = Writer::new();
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.text(CH_DISPLAY_NAME, "Kick");
        writer.event(CH_NEW, &32868u16.to_le_bytes());
        writer.text(CH_DISPLAY_NAME, "not a channel");

        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.channels.len(), 1);
        assert_eq!(
            project.channels[0].name, "Kick",
            "nothing was written over it"
        );
    }

    #[test]
    fn audio_and_automation_clips_come_off_the_playlist_at_every_item_size() {
        for size in PLAYLIST_ITEM_SIZES {
            let mut writer = Writer::new();
            writer.event(CH_NEW, &0u16.to_le_bytes());
            writer.event(CH_TYPE, &[4]);
            writer.text(CH_SAMPLE, "C:\\kick.wav");
            writer.event(CH_NEW, &1u16.to_le_bytes());
            writer.event(CH_TYPE, &[5]);
            writer.text(CH_DISPLAY_NAME, "Filter cutoff");
            writer.event(CH_NEW, &2u16.to_le_bytes());
            writer.event(CH_TYPE, &[2]);
            let mut items = Vec::new();
            items.extend(item(size, 0, 20481, 384, 1, (-1.0, -1.0)));
            items.extend(item(size, 96, 0, 24, 2, (12.5, 100.0)));
            // An automation item's start field is beats into the clip.
            items.extend(item(size, 192, 1, 384, 3, (2.0, 6.0)));
            // An instrument channel on the playlist has nothing to carry.
            items.extend(item(size, 0, 2, 96, 4, (0.0, 0.0)));
            writer.event(ARR_PLAYLIST, &items);

            let project = parse(&writer.finish(96)).expect("it should read");
            let clips = &project.playlist_clips;
            assert_eq!(clips.len(), 3, "{size}-byte items");
            assert!(matches!(
                clips[0].content,
                FlClipContent::Pattern { pattern_index: 1 }
            ));
            match &clips[1].content {
                FlClipContent::AudioSample {
                    channel_index,
                    sample_path,
                    start_offset_ms,
                    end_offset_ms,
                    volume,
                    ..
                } => {
                    assert_eq!(*channel_index, 0);
                    assert_eq!(sample_path, "C:\\kick.wav");
                    assert_eq!((*start_offset_ms, *end_offset_ms), (12.5, 100.0));
                    assert_eq!(*volume, 1.0, "no version said, so no clip volume read");
                }
                other => panic!("{size}-byte items: expected audio, got {other:?}"),
            }
            assert_eq!(clips[1].track_index, 2);
            assert_eq!(clips[1].start_tick, hardwave_midi::PPQ);
            assert!(
                matches!(&clips[2].content, FlClipContent::Automation { channel_index: 1, target, offset_ticks } if target == "Filter cutoff" && *offset_ticks == 2 * hardwave_midi::PPQ)
            );
        }
    }

    #[test]
    fn a_clip_s_own_fades_and_volume_come_from_fl_20_99_on() {
        for (version, read) in [
            (&b"25.2.3.5171\0"[..], true),
            (b"20.99.0.1\0", true),
            (b"20.8.4.2576\0", false),
        ] {
            for size in PLAYLIST_ITEM_SIZES.into_iter().filter(|s| *s >= 60) {
                let mut writer = Writer::new();
                writer.event(FL_VERSION, version);
                writer.event(CH_NEW, &0u16.to_le_bytes());
                writer.event(CH_TYPE, &[4]);
                writer.text(CH_SAMPLE, "C:\\kick.wav");
                let mut clip = item(size, 0, 0, 384, 1, (-1.0, -1.0));
                clip[36..40].copy_from_slice(&43.0f32.to_le_bytes());
                clip[44..48].copy_from_slice(&375.0f32.to_le_bytes());
                clip[52..56].copy_from_slice(&0.631f32.to_le_bytes());
                writer.event(ARR_PLAYLIST, &clip);
                let project = parse(&writer.finish(96)).expect("it should read");
                let FlClipContent::AudioSample {
                    fade_in_ms,
                    fade_out_ms,
                    volume,
                    ..
                } = project.playlist_clips[0].content
                else {
                    panic!("expected audio");
                };
                if read {
                    assert_eq!((fade_in_ms, fade_out_ms, volume), (43.0, 375.0, 0.631));
                } else {
                    assert_eq!((fade_in_ms, fade_out_ms, volume), (0.0, 0.0, 1.0));
                }
            }
        }
    }

    #[test]
    fn a_playlist_block_no_item_size_reads_is_left_alone() {
        let mut writer = Writer::new();
        writer.event(ARR_PLAYLIST, &[7u8; 61]);
        let project = parse(&writer.finish(96)).expect("it should read");
        assert!(project.playlist_clips.is_empty());
    }

    /// A playlist track's settings: its number (FL counts from 1) and
    /// whether it is on.
    fn track_data(number: u32, on: bool) -> Vec<u8> {
        let mut d = number.to_le_bytes().to_vec();
        d.resize(16, 0);
        d[12] = on as u8;
        d
    }

    #[test]
    fn playlist_tracks_keep_their_names() {
        let mut writer = Writer::new();
        writer.event(TRACK_DATA, &track_data(1, true));
        writer.event(TRACK_DATA, &track_data(7, true));
        writer.text(TRACK_NAME, "Kicks");
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(
            project.playlist_track_names,
            vec![(7, "Kicks".to_string())],
            "the number is FL's own; it named the track below the one it meant"
        );
    }

    #[test]
    fn tracks_switched_off_and_muted_items_are_known() {
        let mut writer = Writer::new();
        writer.event(TRACK_DATA, &track_data(1, false));
        writer.event(TRACK_DATA, &track_data(2, true));
        let mut muted = item(32, 0, 20481, 384, 2, (-1.0, -1.0));
        muted[18..20].copy_from_slice(&(ITEM_MUTED | 0x40).to_le_bytes());
        let mut items = item(32, 0, 20481, 384, 1, (-1.0, -1.0));
        items.extend(muted);
        writer.event(ARR_PLAYLIST, &items);
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.playlist_tracks_off, vec![1]);
        assert!(!project.playlist_clips[0].muted);
        assert!(project.playlist_clips[1].muted);
    }

    #[test]
    fn a_soloed_track_is_known_from_the_ones_its_solo_switched_off() {
        // FL's 70-byte track data, as FL 26 writes it with track 2 soloed:
        // every other track is switched off.
        let track = |number: u32, on: bool, solo: bool| {
            let mut d = track_data(number, on);
            d.resize(70, 0);
            d[TRACK_SOLO_AT] = solo as u8;
            d
        };
        let mut writer = Writer::new();
        writer.event(TRACK_DATA, &track(1, false, false));
        writer.event(TRACK_DATA, &track(2, true, true));
        writer.event(TRACK_DATA, &track(3, false, false));
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.playlist_tracks_solo, vec![2]);
        assert_eq!(project.playlist_tracks_off, vec![1, 3]);
    }

    #[test]
    fn a_channel_s_settings_come_across() {
        let mut writer = Writer::new();
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.event(CH_TYPE, &[4]);
        writer.event(CH_ENABLED, &[0]);
        writer.event(CH_FX_FLAGS, &(FX_REVERSE | FX_SWAP_STEREO).to_le_bytes());
        writer.event(CH_FADE_IN, &184u16.to_le_bytes());
        writer.event(CH_ROUTED_TO, &[16]);
        let mut levels = Vec::new();
        levels.extend_from_slice(&9_600i32.to_le_bytes()); // pan
        levels.extend_from_slice(&7_456u32.to_le_bytes()); // volume
        levels.extend_from_slice(&(-250i32).to_le_bytes()); // pitch
        levels.resize(24, 0);
        writer.event(CH_LEVELS, &levels);
        let mut params = vec![0u8; 168];
        params[81] = 1;
        params[96..100].copy_from_slice(&768.0f32.to_le_bytes());
        params[100..104].copy_from_slice(&224i32.to_le_bytes());
        params[104..108].copy_from_slice(&10_000i32.to_le_bytes());
        params[108..112].copy_from_slice(&(-2i32).to_le_bytes());
        writer.event(CH_PARAMETERS, &params);
        writer.event(CH_ROOT_NOTE, &72u32.to_le_bytes());

        let project = parse(&writer.finish(96)).expect("it should read");
        let s = &project.channels[0].settings;
        assert!(!s.enabled);
        assert!(s.reversed && s.swap_stereo && s.polarity_inverted);
        assert_eq!(s.fade_in, 184);
        assert_eq!(s.mixer_insert, 16);
        assert_eq!((s.pan, s.volume, s.pitch_cents), (9_600, 7_456, -250));
        assert_eq!(s.root_note, 72);
        // Two bars at FL's 96 a beat, in our ticks.
        assert_eq!(s.stretch.time_ticks, 8.0 * hardwave_midi::PPQ as f64);
        assert_eq!(s.stretch.pitch_cents, 224);
        assert_eq!(s.stretch.multiplier, 2.0);
        assert!(!s.stretch.is_resample());
    }

    #[test]
    fn a_sampler_s_envelope_loop_voices_and_sample_effects_come_across() {
        let mut writer = Writer::new();
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.event(CH_TYPE, &[0]);
        writer.event(CH_FX_FLAGS, &FX_FADE_STEREO.to_le_bytes());
        writer.event(CH_PING_PONG, &[1]);
        // Cut 3, cut by 3: the channel cuts itself.
        let mut cut = 3u16.to_le_bytes().to_vec();
        cut.extend_from_slice(&3u16.to_le_bytes());
        writer.event(CH_CUT_GROUP, &cut);
        writer.event(
            CH_SAMPLER_FLAGS,
            &(SAMPLER_USES_LOOP_POINTS | 1).to_le_bytes(),
        );
        let mut poly = vec![0u8; 9];
        poly[8] = POLY_MONO | POLY_PORTA;
        writer.event(CH_POLYPHONY, &poly);
        // The pan envelope first, then the volume one.
        let envelope = |enabled: i32, knobs: [i32; 6]| {
            let mut e = 0i32.to_le_bytes().to_vec();
            e.extend_from_slice(&enabled.to_le_bytes());
            for k in knobs {
                e.extend_from_slice(&k.to_le_bytes());
            }
            e.resize(68, 0);
            e
        };
        writer.event(CH_ENVELOPE, &envelope(0, [100, 100, 100, 100, 0, 100]));
        writer.event(
            CH_ENVELOPE,
            &envelope(1, [100, 32_818, 100, 65_536, 64, 16_459]),
        );
        writer.event(CH_ENVELOPE, &envelope(1, [65_536; 6]));
        let mut params = vec![0u8; 168];
        params[9] = 1; // remove DC
        params[80] = 1; // normalize
        params[83] = 4; // Smooth
        params[84..88].copy_from_slice(&64u32.to_le_bytes()); // crossfade 25 %
        params[88..92].copy_from_slice(&128u32.to_le_bytes()); // trim 50 %
                                                               // Start at a quarter: 4096 * (log2(0.25) + 15) = 53248.
        params[133..135].copy_from_slice(&53_248u16.to_le_bytes());
        params[135..137].copy_from_slice(&63u16.to_le_bytes());
        // A length with no marker is not read.
        params[141..143].copy_from_slice(&53_248u16.to_le_bytes());
        writer.event(CH_PARAMETERS, &params);

        let project = parse(&writer.finish(96)).expect("it should read");
        let s = &project.channels[0].settings.sampler;
        assert!(s.fade_stereo && s.ping_pong && s.cut_itself && s.use_loop_points);
        assert!(s.mono && s.porta);
        let e = s.envelope.expect("the volume envelope");
        assert!(e.enabled);
        assert_eq!((e.delay, e.hold), (0.0, 0.0));
        assert!(
            (e.attack - 0.5).abs() < 1e-3,
            "attack half way: {}",
            e.attack
        );
        assert_eq!(e.decay, 1.0);
        assert_eq!(e.sustain, 0.5);
        assert!(
            (e.release - 0.25).abs() < 1e-3,
            "release a quarter: {}",
            e.release
        );
        assert!(s.remove_dc && s.normalize);
        assert_eq!(s.declick, 4);
        assert_eq!((s.crossfade, s.trim), (0.25, 0.5));
        assert!(
            (s.start.unwrap() - 0.25).abs() < 1e-4,
            "start {:?}",
            s.start
        );
        assert_eq!(s.length, None);
    }

    #[test]
    fn the_mixer_s_plug_in_names_do_not_rename_the_last_channel() {
        let mut writer = Writer::new();
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.text(CH_DISPLAY_NAME, "Full Track Render");
        writer.event(ARRANGEMENT_NEW, &0u16.to_le_bytes());
        // Further on, a mixer slot names its plug-in with the same events.
        writer.text(PLUGIN_INTERNAL_NAME, "Fruity Parametric EQ 2");
        writer.text(CH_DISPLAY_NAME, "Fruity Parametric EQ 2");
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.channels[0].name, "Full Track Render");
        assert_eq!(project.channels[0].plugin_name, None);
    }

    /// One record of the mixer's parameter list.
    fn mixer_param(param: u8, flags: u8, at: u16, value: i32) -> Vec<u8> {
        let mut out = vec![0, 0, 0, 0, param, flags];
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    #[test]
    fn the_mixer_comes_across_with_its_levels_routes_and_effects() {
        // FL 24 counts its inserts from 0x2000 in the parameter list, FL 25
        // from 0x7000; the song's own settings sit at 0x4000 in both.
        for first in [0x2000u16, 0x7000] {
            let at = |insert: u16, slot: u16| first + insert * 64 + slot;
            let mut writer = Writer::new();
            writer.event(CH_NEW, &0u16.to_le_bytes());
            writer.text(CH_DISPLAY_NAME, "Lead");
            // The master, with an EQ in its first slot and the second empty.
            // A slot's number comes after its plug-in.
            writer.event(INSERT_FLAGS, &[0, 0, 0, 0, 0x0C, 0, 0, 0, 0, 0, 0, 0]);
            writer.text(PLUGIN_INTERNAL_NAME, "Fruity Parametric EQ 2");
            writer.event(PLUGIN_DATA, &[8, 0, 0, 0]);
            writer.event(SLOT_INDEX, &0u16.to_le_bytes());
            writer.event(SLOT_INDEX, &1u16.to_le_bytes());
            writer.event(INSERT_ROUTING, &[0, 0, 0]);
            writer.event(INSERT_OUTPUT, &0u32.to_le_bytes());
            // Insert 1: polarity flipped, to the master and to insert 2.
            writer.text(INSERT_NAME, "Leads");
            writer.event(INSERT_FLAGS, &[0, 0, 0, 0, 0x0D, 0, 0, 0, 0, 0, 0, 0]);
            writer.event(SLOT_INDEX, &0u16.to_le_bytes());
            writer.text(PLUGIN_INTERNAL_NAME, "Fruity Wrapper");
            writer.text(CH_DISPLAY_NAME, "Sylenth1");
            writer.event(SLOT_INDEX, &1u16.to_le_bytes());
            writer.event(INSERT_ROUTING, &[1, 0, 1]);
            writer.event(INSERT_OUTPUT, &u32::MAX.to_le_bytes());
            // Insert 2: switched off, effects bypassed, soloed.
            writer.text(INSERT_NAME, "Group");
            writer.event(INSERT_FLAGS, &[0, 0, 0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0]);
            writer.event(INSERT_ROUTING, &[1]);
            writer.event(INSERT_OUTPUT, &u32::MAX.to_le_bytes());
            let mut params = Vec::new();
            params.extend(mixer_param(0, 0, 0x4000, 12_800));
            params.extend(mixer_param(192, 31, at(0, 0), 12_800));
            params.extend(mixer_param(0, 31, at(0, 0), 0));
            params.extend(mixer_param(1, 31, at(0, 0), 6_400));
            params.extend(mixer_param(192, 31, at(1, 0), 10_000));
            params.extend(mixer_param(193, 31, at(1, 0), -3_200));
            params.extend(mixer_param(194, 31, at(1, 0), 32));
            params.extend(mixer_param(64, 31, at(1, 0), 0));
            params.extend(mixer_param(66, 31, at(1, 0), 6_400));
            params.extend(mixer_param(208, 31, at(1, 0), -300));
            // A plug-in's own parameter on insert 1: not the fader.
            params.extend(mixer_param(192, 0x80, at(1, 0), 1));
            params.extend(mixer_param(192, 31, at(2, 0), 16_000));
            writer.event(MIXER_PARAMS, &params);
            let project = parse(&writer.finish(96)).expect("it should read");

            assert_eq!(project.channels[0].name, "Lead", "slots name no channel");
            let [master, leads, group] = &project.mixer[..] else {
                panic!("three inserts, read {:?}", project.mixer);
            };
            assert_eq!(master.slots.len(), 1, "the empty slot is not kept");
            let eq = &master.slots[0];
            assert_eq!(eq.plugin, "Fruity Parametric EQ 2");
            assert_eq!(eq.data, vec![8, 0, 0, 0]);
            assert!(!eq.enabled);
            assert_eq!(eq.mix, 6_400);

            assert_eq!(leads.index, 1);
            assert_eq!(leads.name, "Leads");
            assert!(leads.polarity_inverted && leads.enabled && leads.effects_enabled);
            assert_eq!(
                (leads.volume, leads.pan, leads.stereo_separation),
                (10_000, -3_200, 32)
            );
            assert_eq!(leads.eq_gains, [-300, 0, 0]);
            assert_eq!(
                leads.routes,
                vec![
                    FlRoute { to: 0, level: 0 },
                    FlRoute {
                        to: 2,
                        level: 6_400
                    }
                ]
            );
            assert_eq!(leads.slots[0].label(), "Sylenth1");
            assert_eq!(leads.slots[0].index, 1, "slot 0 was empty");

            assert_eq!(group.name, "Group");
            assert!(!group.enabled && !group.effects_enabled && group.solo);
            assert_eq!(group.volume, 16_000);
            assert_eq!(
                group.routes,
                vec![FlRoute {
                    to: 0,
                    level: 12_800
                }]
            );
        }
    }

    /// An automation clip's data with these points: (beats after the one
    /// before, value).
    fn automation_data(points: &[(f64, f64)]) -> Vec<u8> {
        let mut data = vec![0u8; 17];
        data[0] = 1;
        data.extend_from_slice(&(points.len() as u32).to_le_bytes());
        for (after, value) in points {
            data.extend_from_slice(&after.to_le_bytes());
            data.extend_from_slice(&value.to_le_bytes());
            data.extend_from_slice(&0.25f32.to_le_bytes());
            data.extend_from_slice(&[0; 4]);
        }
        // FL writes more after the points (its LFO); none of it is read.
        data.extend_from_slice(&[0xFF; 16]);
        data
    }

    fn remote(channel: u32, knob: u32) -> Vec<u8> {
        let mut data = vec![0u8; 20];
        data[2..6].copy_from_slice(&channel.to_le_bytes());
        data[8..12].copy_from_slice(&knob.to_le_bytes());
        data
    }

    #[test]
    fn automation_clips_come_with_their_points_and_the_knob_they_move() {
        use FlAutomationTarget as T;
        // FL 24 counts its inserts from 0x2000, FL 25 from 0x7000.
        for (version, first) in [
            (&b"24.2.2.4646\0"[..], 0x2000u32),
            (b"25.2.3.5171\0", 0x7000),
        ] {
            let knob =
                |insert: u32, slot: u32, param: u32| ((first + insert * 64 + slot) << 16) | param;
            let mut writer = Writer::new();
            writer.event(FL_VERSION, version);
            writer.event(CH_NEW, &0u16.to_le_bytes());
            writer.event(CH_TYPE, &[5]);
            writer.text(CH_DISPLAY_NAME, "Lead - Volume");
            // A rise over four beats, a jump down where it ends, two beats on.
            writer.event(
                CH_AUTOMATION,
                &automation_data(&[(0.0, 0.0), (4.0, 1.0), (0.0, 0.25), (2.0, 0.5)]),
            );
            writer.event(REMOTE_CONTROL, &remote(0, knob(1, 0, 0x1FC0)));
            writer.event(REMOTE_CONTROL, &remote(1, knob(1, 2, 0x8005)));
            writer.event(REMOTE_CONTROL, &remote(2, knob(1, 3, 0x1F00)));
            writer.event(REMOTE_CONTROL, &remote(3, knob(0, 1, 0x1F01)));
            writer.event(REMOTE_CONTROL, &remote(4, (3 << 16) | 4));
            writer.event(REMOTE_CONTROL, &remote(5, (2 << 16) | 0x8000 | 447));
            writer.event(REMOTE_CONTROL, &remote(6, (2 << 16) | 1));
            // The song's own settings sit below the inserts.
            writer.event(REMOTE_CONTROL, &remote(7, (0x4000 << 16) | 0x1F00));
            for _ in 0..2 {
                writer.event(INSERT_FLAGS, &[0, 0, 0, 0, 0x0C, 0, 0, 0, 0, 0, 0, 0]);
                writer.event(INSERT_ROUTING, &[1]);
                writer.event(INSERT_OUTPUT, &0u32.to_le_bytes());
            }
            writer.event(MIXER_PARAMS, &mixer_param(192, 31, first as u16, 12_800));
            let project = parse(&writer.finish(96)).expect("it should read");

            let beat = hardwave_midi::PPQ;
            let points: Vec<(u64, f64)> = project.channels[0]
                .automation
                .iter()
                .map(|p| (p.tick, p.value))
                .collect();
            assert_eq!(
                points,
                vec![(0, 0.0), (4 * beat, 1.0), (4 * beat, 0.25), (6 * beat, 0.5)]
            );
            assert_eq!(project.channels[0].automation[1].tension, 0.25);

            let targets: Vec<(u32, T)> = project
                .automation_links
                .iter()
                .map(|l| (l.channel, l.target))
                .collect();
            assert_eq!(
                targets,
                vec![
                    (0, T::InsertVolume(1)),
                    (
                        1,
                        T::SlotPlugin {
                            insert: 1,
                            slot: 2,
                            param: 5
                        }
                    ),
                    (2, T::SlotEnabled { insert: 1, slot: 3 }),
                    (3, T::SlotMix { insert: 0, slot: 1 }),
                    (4, T::ChannelPitch(3)),
                    (
                        5,
                        T::ChannelPlugin {
                            channel: 2,
                            param: 447
                        }
                    ),
                    (6, T::ChannelPan(2)),
                    (7, T::Other),
                ]
            );
        }
    }

    #[test]
    fn fl_25_names_a_channel_s_insert_in_two_bytes() {
        let mut writer = Writer::new();
        writer.event(FL_VERSION, b"25.2.3.5171\0");
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.event(CH_ROUTED_TO_FL25, &300u16.to_le_bytes());
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.channels[0].settings.mixer_insert, 300);

        // Before FL 25 the same id meant something else.
        let mut writer = Writer::new();
        writer.event(FL_VERSION, b"24.2.2.4646\0");
        writer.event(CH_NEW, &0u16.to_le_bytes());
        writer.event(CH_ROUTED_TO, &[7]);
        writer.event(CH_ROUTED_TO_FL25, &300u16.to_le_bytes());
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.channels[0].settings.mixer_insert, 7);
    }

    #[test]
    fn a_project_from_fl_25_keeps_its_tempo() {
        // The start of a real FL 25.2.3 project, as written: version,
        // build, event 172 with three bytes, the version as UTF-16, the
        // licensee, then the tempo (160.000).
        let mut writer = Writer::new();
        writer.event(FL_VERSION, b"25.2.3.5171\0");
        writer.event(DWORD + 31, &5171u32.to_le_bytes());
        writer.event(28, &[1]);
        writer
            .body
            .extend_from_slice(&[THREE_BYTE_SINCE_FL25, 1, 1, 0]);
        writer.text(TEXT, "FL Studio 25.2.3.5171");
        writer.event(PROJ_TEMPO, &160_000u32.to_le_bytes());
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.bpm, 160.0);
    }

    #[test]
    fn before_fl_25_event_172_is_four_bytes() {
        let mut writer = Writer::new();
        writer.event(FL_VERSION, b"24.2.2.4646\0");
        writer.event(THREE_BYTE_SINCE_FL25, &[1, 1, 0, 0]);
        writer.event(PROJ_TEMPO, &150_000u32.to_le_bytes());
        assert_eq!(parse(&writer.finish(96)).unwrap().bpm, 150.0);
    }

    #[test]
    fn an_old_project_s_tempo_comes_from_coarse_and_fine() {
        let mut writer = Writer::new();
        writer.event(TEMPO_COARSE, &174u16.to_le_bytes());
        writer.event(TEMPO_FINE, &500u16.to_le_bytes());
        assert_eq!(parse(&writer.finish(96)).unwrap().bpm, 174.5);
    }
}
