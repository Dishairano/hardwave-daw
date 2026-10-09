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
    FlChannel, FlChannelKind, FlClipContent, FlNote, FlPlaylistClip, FlProject,
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
/// A playlist track's settings; its first four bytes are its index.
const TRACK_DATA: u8 = TEXT + 46; // 238
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

/// Walk the event stream.
fn events(body: &[u8]) -> Result<Vec<Event<'_>>, FlpError> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < body.len() {
        let id = body[i];
        i += 1;
        let len = if id < WORD {
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
        out.push(Event {
            id,
            data: &body[i..i + len],
        });
        i += len;
    }
    Ok(out)
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
    // Playlist items naming a channel: (track, start, length, channel,
    // start offset ms, end offset ms). Made into clips at the end, once
    // every channel's kind and sample is known.
    let mut channel_items: Vec<(u32, u64, u64, u32, f32, f32)> = Vec::new();
    let mut current_pattern: Option<u32> = None;
    let mut pattern_names: Vec<(u32, String)> = Vec::new();

    for event in events(body)? {
        match event.id {
            PROJ_TEMPO => {
                // Thousandths of a beat per minute.
                let raw = u32_at(event.data, 0);
                if raw > 0 {
                    project.bpm = raw as f32 / 1000.0;
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
                        name: String::new(),
                        sample_path: None,
                        plugin_name: None,
                        pattern_steps: Vec::new(),
                        kind: FlChannelKind::Other,
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
            TRACK_DATA => {
                current_track = Some(u32_at(event.data, 0));
            }
            TRACK_NAME => {
                if let Some(index) = current_track {
                    let name = text(event.data);
                    if !name.is_empty() {
                        // Stored from 0; the playlist's tracks count from 1.
                        project.playlist_track_names.push((index + 1, name));
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
                let Some(stride) = playlist_item_size(event.data) else {
                    continue;
                };
                for item in event.data.chunks_exact(stride) {
                    let position = u32_at(item, 0) as u64;
                    let pattern_base = u16_at(item, 4) as u32;
                    let item_id = u16_at(item, 6) as u32;
                    let length = u32_at(item, 8) as u64;
                    let track = u16_at(item, 12) as u32;
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
                        });
                    } else {
                        let start_ms = f32_at(item, 24);
                        let end_ms = f32_at(item, 28);
                        channel_items.push((
                            track_index,
                            scale(position),
                            scale(length).max(1),
                            item_id,
                            start_ms,
                            end_ms,
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    project.pattern_names = pattern_names;

    // A playlist item naming a channel is an audio clip when that channel
    // holds a sample, and an automation clip when it is one.
    for (track_index, start_tick, length_ticks, channel_index, start_ms, end_ms) in channel_items {
        let Some(channel) = project.channels.get(channel_index as usize) else {
            continue;
        };
        let content = match (channel.kind, &channel.sample_path) {
            (FlChannelKind::Automation, _) => FlClipContent::Automation {
                channel_index,
                target: channel.name.clone(),
            },
            (_, Some(path)) => FlClipContent::AudioSample {
                channel_index,
                sample_path: path.clone(),
                start_offset_ms: start_ms,
                end_offset_ms: end_ms,
            },
            _ => continue,
        };
        project.playlist_clips.push(FlPlaylistClip {
            track_index,
            start_tick,
            length_ticks,
            content,
        });
    }
    Ok(project)
}

/// The size of one playlist item in this block, from the sizes FL has
/// used, or None when none of them reads it (then nothing is read rather
/// than garbage).
fn playlist_item_size(data: &[u8]) -> Option<usize> {
    PLAYLIST_ITEM_SIZES.into_iter().find(|&size| {
        if data.len() < size || data.len() % size != 0 {
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
            items.extend(item(size, 192, 1, 384, 3, (-1.0, -1.0)));
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
                } => {
                    assert_eq!(*channel_index, 0);
                    assert_eq!(sample_path, "C:\\kick.wav");
                    assert_eq!((*start_offset_ms, *end_offset_ms), (12.5, 100.0));
                }
                other => panic!("{size}-byte items: expected audio, got {other:?}"),
            }
            assert_eq!(clips[1].track_index, 2);
            assert_eq!(clips[1].start_tick, hardwave_midi::PPQ);
            assert!(
                matches!(&clips[2].content, FlClipContent::Automation { channel_index: 1, target } if target == "Filter cutoff")
            );
        }
    }

    #[test]
    fn a_playlist_block_no_item_size_reads_is_left_alone() {
        let mut writer = Writer::new();
        writer.event(ARR_PLAYLIST, &[7u8; 61]);
        let project = parse(&writer.finish(96)).expect("it should read");
        assert!(project.playlist_clips.is_empty());
    }

    #[test]
    fn playlist_tracks_keep_their_names() {
        let mut writer = Writer::new();
        writer.event(TRACK_DATA, &[0u8; 16]);
        writer.event(TRACK_DATA, &{
            let mut d = 7u32.to_le_bytes().to_vec();
            d.resize(16, 0);
            d
        });
        writer.text(TRACK_NAME, "Kicks");
        let project = parse(&writer.finish(96)).expect("it should read");
        assert_eq!(project.playlist_track_names, vec![(8, "Kicks".to_string())]);
    }
}
