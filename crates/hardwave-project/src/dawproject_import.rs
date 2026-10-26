//! Opening a DAWproject (see `dawproject`): a song from Bitwig Studio,
//! Studio One or Cubase as a song here.
//!
//! The tracks take the first rows, as an imported FL project's do, with
//! their mix, routing and sends; MIDI clips come with their notes, audio
//! clips with where they sit in their files, automation with its points
//! and the tempo with its changes. What needs the computer is handed back:
//! where each sample is (its file may be in the zip), and which plug-in on
//! this computer each device is. What the song uses that cannot come is
//! listed in `left_out`.

use crate::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use crate::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef};
use crate::mixer::Send;
use crate::project::Project;
use crate::tempo::{TempoEntry, TempoRamp};
use crate::track::{PluginSlot, Track, TrackKind};
use std::collections::{BTreeSet, HashMap};

const PPQ: f64 = 960.0;

/// An audio clip whose file and place in it are finished once its file is
/// loaded (the clip's sample offsets count in the rate it loads at).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedAudio {
    pub track_id: String,
    pub clip_id: String,
    /// The file's path in the zip, or on disk when `external`.
    pub file: String,
    pub external: bool,
    /// Where the clip starts in its file, in seconds.
    pub start_secs: f64,
    /// Points through the clip: (beats into the clip, seconds into the
    /// file). Empty when it plays at the file's own speed.
    pub warps: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFormat {
    Vst3,
    Clap,
    Vst2,
    Au,
    Builtin,
}

/// A device on a track, to be found among this computer's plug-ins. Its
/// slot is in the song; one that is not found is taken out again.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedPlugin {
    pub track_id: String,
    pub slot_id: String,
    pub format: DeviceFormat,
    /// The VST3 class id, CLAP id, VST2 id or AU id the other DAW gave.
    pub device_id: String,
    pub name: String,
    pub vendor: String,
    pub instrument: bool,
    /// Its settings file, a path in the zip (or on disk when `external`).
    pub state_file: Option<String>,
    pub external_state: bool,
}

/// The song, and what is left to do and to say.
#[derive(Debug)]
pub struct Import {
    pub project: Project,
    pub audio: Vec<ImportedAudio>,
    pub plugins: Vec<ImportedPlugin>,
    /// The application that wrote it, for the report.
    pub application: String,
    pub left_out: Vec<String>,
}

/// Read `project_xml` into a song called `name`.
pub fn import(project_xml: &str, name: &str) -> Result<Import, String> {
    let root = parse(project_xml)?;
    if root.name != "Project" {
        return Err("this is not a DAWproject (no Project at the top)".into());
    }
    let mut im = Importer {
        project: Project::default(),
        audio: Vec::new(),
        plugins: Vec::new(),
        left_out: BTreeSet::new(),
        row: 0,
        track_of_xml: HashMap::new(),
        track_of_channel: HashMap::new(),
        params: HashMap::new(),
    };
    im.project.metadata.name = name.to_string();
    let application = root
        .child("Application")
        .map(|a| {
            format!(
                "{} {}",
                a.attr("name").unwrap_or("another DAW"),
                a.attr("version").unwrap_or("")
            )
            .trim()
            .to_string()
        })
        .unwrap_or_else(|| "another DAW".into());

    im.tempo(&root);
    if let Some(structure) = root.child("Structure") {
        im.structure(structure);
    }
    if let Some(arrangement) = root.child("Arrangement") {
        if let Some(lanes) = arrangement.child("Lanes") {
            let unit = lanes.attr("timeUnit").unwrap_or("beats").to_string();
            im.lanes(lanes, None, &unit);
        }
        if arrangement.child("Markers").is_some() {
            im.left_out.insert("markers".into());
        }
    }
    if root.child("Scenes").is_some_and(|s| !s.children.is_empty()) {
        im.left_out.insert("the clip launcher's scenes".into());
    }
    Ok(Import {
        project: im.project,
        audio: im.audio,
        plugins: im.plugins,
        application,
        left_out: im.left_out.into_iter().collect(),
    })
}

/// What an automation point moves.
#[derive(Debug, Clone)]
enum ParamTarget {
    Volume,
    Pan,
    Mute,
    Send(usize),
    Plugin {
        slot_id: String,
        param_id: u32,
        min: f64,
        max: f64,
    },
}

struct Importer {
    project: Project,
    audio: Vec<ImportedAudio>,
    plugins: Vec<ImportedPlugin>,
    left_out: BTreeSet<String>,
    row: usize,
    /// The other DAW's track and channel ids, to ours.
    track_of_xml: HashMap<String, String>,
    track_of_channel: HashMap<String, String>,
    /// A parameter's id, to the track it is on and what it moves.
    params: HashMap<String, (String, ParamTarget)>,
}

impl Importer {
    fn tempo(&mut self, root: &El) {
        let transport = root.child("Transport");
        let bpm = transport
            .and_then(|t| t.child("Tempo"))
            .and_then(|t| t.f64("value"))
            .filter(|b| *b > 1.0)
            .unwrap_or(120.0);
        let (num, den) = transport
            .and_then(|t| t.child("TimeSignature"))
            .map(|t| {
                (
                    t.f64("numerator").unwrap_or(4.0) as u32,
                    t.f64("denominator").unwrap_or(4.0) as u32,
                )
            })
            .unwrap_or((4, 4));
        let mut entries = vec![TempoEntry {
            tick: 0,
            bpm,
            time_sig_num: num.max(1),
            time_sig_den: den.max(1),
            ramp: TempoRamp::Instant,
        }];
        let arrangement = root.child("Arrangement");
        if let Some(points) = arrangement.and_then(|a| a.child("TempoAutomation")) {
            let mut tempo: Vec<(u64, f64, bool)> = points
                .children
                .iter()
                .filter(|p| p.name == "RealPoint")
                .filter_map(|p| {
                    Some((
                        beats_to_ticks(p.f64("time")?),
                        p.f64("value")?,
                        p.attr("interpolation") == Some("linear"),
                    ))
                })
                .collect();
            tempo.sort_by_key(|t| t.0);
            for (tick, value, ramp) in tempo {
                let ramp = if ramp {
                    TempoRamp::Linear
                } else {
                    TempoRamp::Instant
                };
                match entries.iter_mut().find(|e| e.tick == tick) {
                    Some(e) => {
                        e.bpm = value.max(1.0);
                        e.ramp = ramp;
                    }
                    None => entries.push(TempoEntry {
                        tick,
                        bpm: value.max(1.0),
                        time_sig_num: num.max(1),
                        time_sig_den: den.max(1),
                        ramp,
                    }),
                }
            }
        }
        if let Some(points) = arrangement.and_then(|a| a.child("TimeSignatureAutomation")) {
            for p in points
                .children
                .iter()
                .filter(|p| p.name == "TimeSignaturePoint")
            {
                let (Some(time), Some(n), Some(d)) =
                    (p.f64("time"), p.f64("numerator"), p.f64("denominator"))
                else {
                    continue;
                };
                let tick = beats_to_ticks(time);
                entries.sort_by_key(|e| e.tick);
                let bpm_here = entries
                    .iter()
                    .rev()
                    .find(|e| e.tick <= tick)
                    .map(|e| e.bpm)
                    .unwrap_or(bpm);
                match entries.iter_mut().find(|e| e.tick == tick) {
                    Some(e) => {
                        e.time_sig_num = n as u32;
                        e.time_sig_den = d as u32;
                    }
                    None => entries.push(TempoEntry {
                        tick,
                        bpm: bpm_here,
                        time_sig_num: (n as u32).max(1),
                        time_sig_den: (d as u32).max(1),
                        ramp: TempoRamp::Instant,
                    }),
                }
            }
        }
        entries.sort_by_key(|e| e.tick);
        // A signature carries on from where it was set.
        for i in 1..entries.len() {
            let (prev_num, prev_den) = (entries[i - 1].time_sig_num, entries[i - 1].time_sig_den);
            let set_here = arrangement
                .and_then(|a| a.child("TimeSignatureAutomation"))
                .is_some_and(|p| {
                    p.children
                        .iter()
                        .any(|pt| pt.f64("time").map(beats_to_ticks) == Some(entries[i].tick))
                });
            if !set_here {
                entries[i].time_sig_num = prev_num;
                entries[i].time_sig_den = prev_den;
            }
        }
        self.project.tempo_map.entries = entries;
    }

    fn structure(&mut self, structure: &El) {
        // Tracks first, in order, groups before what they hold; then the
        // routing, once every channel has a track.
        let mut defs: Vec<(&El, Option<String>)> = Vec::new();
        fn walk<'a>(el: &'a El, parent: Option<String>, out: &mut Vec<(&'a El, Option<String>)>) {
            for child in &el.children {
                match child.name.as_str() {
                    "Track" => {
                        out.push((child, parent.clone()));
                        let channel_id = child
                            .child("Channel")
                            .and_then(|c| c.attr("id"))
                            .map(str::to_string);
                        walk(child, channel_id.or_else(|| parent.clone()), out);
                    }
                    "Channel" if el.name == "Structure" => out.push((child, parent.clone())),
                    _ => {}
                }
            }
        }
        walk(structure, None, &mut defs);
        for (el, _) in &defs {
            self.make_track(el);
        }
        for (el, parent) in &defs {
            self.route(el, parent.as_deref());
        }
    }

    fn make_track(&mut self, el: &El) {
        let channel = if el.name == "Channel" {
            Some(el)
        } else {
            el.child("Channel")
        };
        let role = channel.and_then(|c| c.attr("role")).unwrap_or("regular");
        let name = el
            .attr("name")
            .or_else(|| channel.and_then(|c| c.attr("name")))
            .unwrap_or("Track")
            .to_string();
        let id = if role == "master" {
            match self
                .project
                .tracks
                .iter()
                .find(|t| matches!(t.kind, TrackKind::Master))
            {
                Some(m) => m.id.clone(),
                None => return,
            }
        } else {
            let content = el.attr("contentType").unwrap_or("");
            let group = el.children.iter().any(|c| c.name == "Track");
            let notes = content.split_whitespace().any(|c| c == "notes")
                && !content.split_whitespace().any(|c| c == "audio")
                && !group;
            self.row += 1;
            let id = self.project.claim_row(self.row, &name, notes);
            if group || role == "submix" || role == "effect" {
                if let Some(t) = self.project.track_mut(&id) {
                    let mut bus = Track::new_bus(id.clone(), name.clone());
                    if role == "effect" {
                        bus.kind = TrackKind::Return;
                    }
                    *t = bus;
                }
            }
            id
        };
        if let Some(xml_id) = el.attr("id") {
            self.track_of_xml.insert(xml_id.to_string(), id.clone());
        }
        if let Some(color) = el.attr("color").and_then(css_color) {
            if let Some(t) = self.project.track_mut(&id) {
                t.color = color;
            }
        }
        let Some(channel) = channel else {
            return;
        };
        if let Some(cid) = channel.attr("id") {
            self.track_of_channel.insert(cid.to_string(), id.clone());
        }
        let mut params: Vec<(String, ParamTarget)> = Vec::new();
        let volume = channel.child("Volume");
        let pan = channel.child("Pan");
        let mute = channel.child("Mute");
        if let Some(t) = self.project.track_mut(&id) {
            if let Some(v) = volume {
                t.volume_db = level_db(v);
                if let Some(pid) = v.attr("id") {
                    params.push((pid.into(), ParamTarget::Volume));
                }
            }
            if let Some(p) = pan {
                t.pan = pan_value(p);
                if let Some(pid) = p.attr("id") {
                    params.push((pid.into(), ParamTarget::Pan));
                }
            }
            if let Some(m) = mute {
                t.muted = m.attr("value") == Some("true");
                if let Some(pid) = m.attr("id") {
                    params.push((pid.into(), ParamTarget::Mute));
                }
            }
            t.soloed = channel.attr("solo") == Some("true");
        }
        if let Some(devices) = channel.child("Devices") {
            for device in &devices.children {
                self.device(&id, device, &mut params);
            }
        }
        for (pid, target) in params {
            self.params.insert(pid, (id.clone(), target));
        }
    }

    fn device(&mut self, track_id: &str, device: &El, params: &mut Vec<(String, ParamTarget)>) {
        let format = match device.name.as_str() {
            "Vst3Plugin" => DeviceFormat::Vst3,
            "ClapPlugin" => DeviceFormat::Clap,
            "Vst2Plugin" => DeviceFormat::Vst2,
            "AuPlugin" => DeviceFormat::Au,
            _ => DeviceFormat::Builtin,
        };
        let name = device
            .attr("deviceName")
            .or_else(|| device.attr("name"))
            .unwrap_or("Device")
            .to_string();
        if format == DeviceFormat::Builtin {
            self.left_out.insert(format!(
                "the other DAW's own {} (no match here)",
                name.to_lowercase()
            ));
            return;
        }
        let slot_id = uuid::Uuid::new_v4().to_string();
        let enabled = device
            .child("Enabled")
            .and_then(|e| e.attr("value"))
            .map(|v| v != "false")
            .unwrap_or(true);
        if let Some(t) = self.project.track_mut(track_id) {
            t.inserts.push(PluginSlot {
                id: slot_id.clone(),
                plugin_id: String::new(),
                enabled,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
        let state = device.child("State");
        self.plugins.push(ImportedPlugin {
            track_id: track_id.to_string(),
            slot_id: slot_id.clone(),
            format,
            device_id: device.attr("deviceID").unwrap_or("").to_string(),
            name,
            vendor: device.attr("deviceVendor").unwrap_or("").to_string(),
            instrument: device.attr("deviceRole") == Some("instrument"),
            state_file: state.and_then(|s| s.attr("path")).map(str::to_string),
            external_state: state.and_then(|s| s.attr("external")) == Some("true"),
        });
        if let Some(list) = device.child("Parameters") {
            for p in &list.children {
                let (Some(pid), Some(param_id)) = (p.attr("id"), p.f64("parameterID")) else {
                    continue;
                };
                params.push((
                    pid.to_string(),
                    ParamTarget::Plugin {
                        slot_id: slot_id.clone(),
                        param_id: param_id as u32,
                        min: p.f64("min").unwrap_or(0.0),
                        max: p.f64("max").unwrap_or(1.0),
                    },
                ));
            }
        }
    }

    fn route(&mut self, el: &El, parent: Option<&str>) {
        let channel = if el.name == "Channel" {
            Some(el)
        } else {
            el.child("Channel")
        };
        let Some(channel) = channel else {
            return;
        };
        let Some(id) = channel
            .attr("id")
            .and_then(|c| self.track_of_channel.get(c))
            .cloned()
        else {
            return;
        };
        let is_master = channel.attr("role") == Some("master");
        // Where it goes: its destination, or the group it sits in.
        let to = channel
            .attr("destination")
            .or(parent)
            .and_then(|d| self.track_of_channel.get(d))
            .cloned();
        let master = self
            .project
            .tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .map(|t| t.id.clone());
        let mut sends = Vec::new();
        let mut send_params = Vec::new();
        if let Some(list) = channel.child("Sends") {
            for (i, send) in list
                .children
                .iter()
                .filter(|s| s.name == "Send")
                .enumerate()
            {
                let Some(target) = send
                    .attr("destination")
                    .and_then(|d| self.track_of_channel.get(d))
                    .cloned()
                else {
                    continue;
                };
                let volume = send.child("Volume");
                sends.push(Send {
                    target,
                    gain_db: volume.map(level_db).unwrap_or(0.0),
                    pre_fader: send.attr("type") == Some("pre"),
                    enabled: send
                        .child("Enable")
                        .and_then(|e| e.attr("value"))
                        .map(|v| v != "false")
                        .unwrap_or(true),
                });
                if let Some(pid) = volume.and_then(|v| v.attr("id")) {
                    send_params.push((pid.to_string(), ParamTarget::Send(i)));
                }
            }
        }
        if let Some(t) = self.project.track_mut(&id) {
            if !is_master && to.is_some() && to != master {
                t.output_bus = to;
            }
            t.sends = sends;
        }
        for (pid, target) in send_params {
            self.params.insert(pid, (id.clone(), target));
        }
    }

    /// A timeline and what it holds, for `track` (None above any track).
    fn lanes(&mut self, el: &El, track: Option<&str>, unit: &str) {
        let track_here = el
            .attr("track")
            .and_then(|t| self.track_of_xml.get(t))
            .cloned()
            .or_else(|| track.map(str::to_string));
        let unit = el.attr("timeUnit").unwrap_or(unit).to_string();
        for child in &el.children {
            match child.name.as_str() {
                "Lanes" => self.lanes(child, track_here.as_deref(), &unit),
                "Clips" => {
                    if let Some(t) = &track_here {
                        for clip in child.children.iter().filter(|c| c.name == "Clip") {
                            self.clip(t, clip, 0.0, None, &unit);
                        }
                    }
                }
                "Points" => {
                    if let Some(t) = &track_here {
                        self.automation(t, child, &unit);
                    }
                }
                "Notes" | "Audio" | "Warps" => {
                    if let Some(t) = &track_here {
                        // Content straight on a lane: one clip around it.
                        let wrapper = El {
                            name: "Clip".into(),
                            attrs: vec![("time".into(), "0".into())],
                            children: vec![child.clone()],
                        };
                        self.clip(t, &wrapper, 0.0, None, &unit);
                    }
                }
                "ClipSlot" => {
                    self.left_out.insert("the clip launcher's slots".into());
                }
                "Video" => {
                    self.left_out.insert("video".into());
                }
                _ => {}
            }
        }
    }

    /// Seconds to beats at the song's tempo (its first tempo; the clip
    /// times that use seconds are rare and short).
    fn to_beats(&self, value: f64, unit: &str) -> f64 {
        if unit == "seconds" {
            let bpm = self
                .project
                .tempo_map
                .entries
                .first()
                .map(|e| e.bpm)
                .unwrap_or(120.0);
            value * bpm / 60.0
        } else {
            value
        }
    }

    /// One clip at `offset` beats (its parent clip's place), cut to
    /// `window` (start, end in beats) when it sits inside another.
    fn clip(
        &mut self,
        track_id: &str,
        clip: &El,
        offset: f64,
        window: Option<(f64, f64)>,
        unit: &str,
    ) {
        let time = offset + self.to_beats(clip.f64("time").unwrap_or(0.0), unit);
        let content_unit = clip.attr("contentTimeUnit").unwrap_or(unit).to_string();
        let play_start = self.to_beats(clip.f64("playStart").unwrap_or(0.0), &content_unit);
        let content = clip.children.iter().find(|c| {
            matches!(
                c.name.as_str(),
                "Notes" | "Clips" | "Warps" | "Audio" | "Lanes" | "Points"
            )
        });
        let duration = clip
            .f64("duration")
            .map(|d| self.to_beats(d, unit))
            .unwrap_or_else(|| {
                content
                    .map(|c| content_length(c, &content_unit, self))
                    .unwrap_or(4.0)
            });
        let (mut start, mut end) = (time, time + duration);
        if let Some((w0, w1)) = window {
            start = start.max(w0);
            end = end.min(w1);
        }
        if end <= start {
            return;
        }
        let muted = clip.attr("enable") == Some("false");
        let name = clip.attr("name").unwrap_or("").to_string();
        let Some(content) = content else {
            return;
        };
        match content.name.as_str() {
            "Notes" => {
                let loop_region = match (clip.f64("loopStart"), clip.f64("loopEnd")) {
                    (Some(a), Some(b)) if b > a => Some((
                        self.to_beats(a, &content_unit),
                        self.to_beats(b, &content_unit),
                    )),
                    _ => None,
                };
                self.notes_clip(
                    track_id,
                    content,
                    NotesClip {
                        time,
                        start,
                        end,
                        play_start,
                        loop_region,
                        muted,
                        name,
                    },
                );
            }
            "Clips" => {
                // A clip of clips (how Bitwig keeps audio events): each
                // inner clip where it sits, cut to this one.
                for inner in content.children.iter().filter(|c| c.name == "Clip") {
                    // The outer clip's name, fades and switch are the ones
                    // shown (Bitwig and our export keep them there); an
                    // inner clip's own win.
                    let mut inner = inner.clone();
                    for key in [
                        "name",
                        "enable",
                        "fadeTimeUnit",
                        "fadeInTime",
                        "fadeOutTime",
                    ] {
                        let inner_has = inner.attr(key).is_some_and(|v| {
                            !(key.starts_with("fade") && v.parse::<f64>().is_ok_and(|f| f == 0.0))
                        });
                        if !inner_has {
                            if let Some(v) = clip.attr(key) {
                                inner.attrs.retain(|(k, _)| k != key);
                                inner.attrs.push((key.to_string(), v.to_string()));
                            }
                        }
                    }
                    self.clip(
                        track_id,
                        &inner,
                        time - play_start,
                        Some((start, end)),
                        &content_unit,
                    );
                }
            }
            "Warps" | "Audio" => {
                self.audio_clip(
                    track_id,
                    clip,
                    content,
                    time,
                    start,
                    end,
                    play_start,
                    &content_unit,
                    muted,
                    name,
                );
            }
            _ => {
                self.left_out
                    .insert("clips holding lanes or automation".into());
            }
        }
    }

    fn notes_clip(&mut self, track_id: &str, notes: &El, c: NotesClip) {
        let length = c.end - c.start;
        // Where the clip's own content starts on the timeline, in beats.
        let content_zero = c.time - c.play_start;
        let mut out = Vec::new();
        let raw: Vec<(f64, f64, u8, f32, f32, u8)> = notes
            .children
            .iter()
            .filter(|n| n.name == "Note")
            .filter_map(|n| {
                Some((
                    n.f64("time")?,
                    n.f64("duration")?,
                    n.f64("key")? as u8,
                    n.f64("vel").unwrap_or(0.8) as f32,
                    n.f64("rel").unwrap_or(0.5) as f32,
                    n.f64("channel").unwrap_or(0.0) as u8,
                ))
            })
            .collect();
        let mut place = |at: f64, n: &(f64, f64, u8, f32, f32, u8)| {
            let on = at - c.start;
            if on < 0.0 || on >= length {
                return;
            }
            out.push(hardwave_midi::MidiNote {
                start_tick: beats_to_ticks(on),
                duration_ticks: beats_to_ticks(n.1.min(length - on)).max(1),
                pitch: n.2.min(127),
                velocity: n.3.clamp(0.0, 1.0),
                channel: n.5 & 0x0F,
                muted: false,
                release_velocity: n.4.clamp(0.0, 1.0),
                ..Default::default()
            });
        };
        match c.loop_region {
            // A looped clip plays its loop over and over to its end.
            Some((l0, l1)) => {
                let len = l1 - l0;
                for n in &raw {
                    // Before the loop, once.
                    if n.0 < l0 && n.0 >= c.play_start {
                        place(content_zero + n.0, n);
                    }
                }
                let mut pass = 0.0;
                // Each pass of the loop, from where the clip reaches it.
                let first = content_zero + l0.max(c.play_start);
                while first + pass * len < c.end && pass < 10_000.0 {
                    for n in raw.iter().filter(|n| n.0 >= l0 && n.0 < l1) {
                        let at = content_zero + n.0 + pass * len;
                        if pass == 0.0 && n.0 < c.play_start {
                            continue;
                        }
                        place(at, n);
                    }
                    pass += 1.0;
                }
            }
            None => {
                for n in &raw {
                    place(content_zero + n.0, n);
                }
            }
        }
        out.sort_by_key(|n| (n.start_tick, n.pitch));
        let clip_id = uuid::Uuid::new_v4().to_string();
        let mut midi = hardwave_midi::MidiClip::new(
            clip_id.clone(),
            if c.name.is_empty() {
                "Clip".into()
            } else {
                c.name
            },
            beats_to_ticks(length),
        );
        midi.notes = out;
        midi.muted = c.muted;
        if let Some(t) = self.project.track_mut(track_id) {
            t.clips.push(ClipPlacement {
                content: ClipContent::Midi(MidiClipRef {
                    id: clip_id,
                    clip: midi,
                }),
                track_id: track_id.to_string(),
                position_ticks: beats_to_ticks(c.start),
                length_ticks: beats_to_ticks(length).max(1),
                lane: 0,
                color: None,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn audio_clip(
        &mut self,
        track_id: &str,
        clip: &El,
        content: &El,
        time: f64,
        start: f64,
        end: f64,
        play_start: f64,
        content_unit: &str,
        muted: bool,
        name: String,
    ) {
        let (audio, warps) = if content.name == "Warps" {
            let audio = content.child("Audio");
            let warp_time_unit = content.attr("timeUnit").unwrap_or(content_unit);
            let warp_content_unit = content.attr("contentTimeUnit").unwrap_or("seconds");
            let points: Vec<(f64, f64)> = content
                .children
                .iter()
                .filter(|w| w.name == "Warp")
                .filter_map(|w| {
                    let beats = self.to_beats(w.f64("time")?, warp_time_unit);
                    let secs = w.f64("contentTime")?;
                    let secs = if warp_content_unit == "beats" {
                        secs * 60.0 / self.first_bpm()
                    } else {
                        secs
                    };
                    Some((beats, secs))
                })
                .collect();
            (audio, points)
        } else {
            (Some(content), Vec::new())
        };
        let Some(audio) = audio else {
            return;
        };
        let Some(file) = audio.child("File") else {
            return;
        };
        let Some(path) = file.attr("path") else {
            return;
        };
        // Where the clip starts on the timeline against where its content
        // starts: `start` may be later than `time` when an outer clip cut
        // it, and the content runs from `play_start`.
        let cut = (start - time).max(0.0) + play_start;
        let at = |beats: f64, pts: &[(f64, f64)]| -> f64 { interpolate(pts, beats) };
        let (start_secs, clip_warps) = if warps.len() >= 2 {
            let start_secs = at(cut, &warps);
            // The points from the clip's start on, in beats into it.
            let mut pts: Vec<(f64, f64)> = vec![(0.0, start_secs)];
            pts.extend(
                warps
                    .iter()
                    .filter(|(b, _)| *b > cut && *b - cut < end - start + 1e-9)
                    .map(|(b, s)| (b - cut, *s)),
            );
            (start_secs, pts)
        } else {
            let offset = warps.first().map(|w| w.1).unwrap_or(0.0);
            // `cut` is in beats whatever the content counts in.
            let secs = cut * 60.0 / self.first_bpm();
            (offset + secs, Vec::new())
        };
        let clip_id = uuid::Uuid::new_v4().to_string();
        let fade_unit = clip.attr("fadeTimeUnit").unwrap_or("beats");
        let fade = |k: &str| -> u64 {
            clip.f64(k)
                .map(|f| beats_to_ticks(self.to_beats(f, fade_unit)))
                .unwrap_or(0)
        };
        let audio_clip = AudioClip {
            id: clip_id.clone(),
            name: if name.is_empty() {
                path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
            } else {
                name
            },
            source_path: String::new(),
            source_hash: String::new(),
            source_start: 0,
            source_end: 0,
            gain_db: 0.0,
            fade_in_ticks: fade("fadeInTime"),
            fade_out_ticks: fade("fadeOutTime"),
            muted,
            reversed: false,
            pitch_semitones: 0.0,
            stretch_ratio: 1.0,
            warp_markers: Vec::new(),
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            source_file: String::new(),
            treatment: crate::clip::SampleTreatment {
                resample: audio.attr("algorithm") == Some("repitch"),
                ..Default::default()
            },
        };
        if let Some(t) = self.project.track_mut(track_id) {
            t.clips.push(ClipPlacement {
                content: ClipContent::Audio(audio_clip),
                track_id: track_id.to_string(),
                position_ticks: beats_to_ticks(start),
                length_ticks: beats_to_ticks(end - start).max(1),
                lane: 0,
                color: None,
            });
        }
        self.audio.push(ImportedAudio {
            track_id: track_id.to_string(),
            clip_id,
            file: path.to_string(),
            external: file.attr("external") == Some("true"),
            start_secs,
            warps: clip_warps,
        });
    }

    fn first_bpm(&self) -> f64 {
        self.project
            .tempo_map
            .entries
            .first()
            .map(|e| e.bpm)
            .unwrap_or(120.0)
    }

    fn automation(&mut self, track_id: &str, points: &El, unit: &str) {
        let Some(target) = points.child("Target") else {
            return;
        };
        if target.attr("expression").is_some() {
            self.left_out
                .insert("controller and note expression automation".into());
            return;
        }
        let Some((on_track, what)) = target
            .attr("parameter")
            .and_then(|p| self.params.get(p))
            .cloned()
        else {
            return;
        };
        let unit_attr = points.attr("unit").unwrap_or("");
        let mut out: Vec<AutomationPoint> = Vec::new();
        for p in &points.children {
            let Some(time) = p.f64("time") else {
                continue;
            };
            let tick = beats_to_ticks(self.to_beats(time, unit));
            let curve = if p.attr("interpolation") == Some("hold") {
                CurveMode::Step
            } else {
                CurveMode::Linear
            };
            let value = match (p.name.as_str(), &what) {
                ("BoolPoint", _) => {
                    if p.attr("value") == Some("true") {
                        1.0
                    } else {
                        0.0
                    }
                }
                ("RealPoint", ParamTarget::Volume | ParamTarget::Send(_)) => {
                    let v = p.f64("value").unwrap_or(0.0);
                    let db = if unit_attr == "decibel" {
                        v
                    } else if v <= 0.0 {
                        -100.0
                    } else {
                        20.0 * v.log10()
                    };
                    ((db + 60.0) / 66.0).clamp(0.0, 1.0)
                }
                ("RealPoint", ParamTarget::Pan) => p.f64("value").unwrap_or(0.5).clamp(0.0, 1.0),
                ("RealPoint", ParamTarget::Plugin { min, max, .. }) => {
                    let v = p.f64("value").unwrap_or(0.0);
                    if (max - min).abs() > 1e-12 {
                        ((v - min) / (max - min)).clamp(0.0, 1.0)
                    } else {
                        v.clamp(0.0, 1.0)
                    }
                }
                _ => continue,
            };
            out.push(AutomationPoint {
                tick,
                value,
                curve: if p.name == "BoolPoint" {
                    CurveMode::Step
                } else {
                    curve
                },
                tension: 0.0,
            });
        }
        if out.is_empty() {
            return;
        }
        out.sort_by_key(|p| p.tick);
        let target = match what {
            ParamTarget::Volume => AutomationTarget::TrackVolume,
            ParamTarget::Pan => AutomationTarget::TrackPan,
            ParamTarget::Mute => AutomationTarget::TrackMute,
            ParamTarget::Send(i) => AutomationTarget::SendLevel { send_index: i },
            ParamTarget::Plugin {
                slot_id, param_id, ..
            } => AutomationTarget::PluginParam { slot_id, param_id },
        };
        let _ = track_id;
        if let Some(t) = self.project.track_mut(&on_track) {
            t.automation_lanes.push(AutomationLane {
                id: uuid::Uuid::new_v4().to_string(),
                target,
                points: out,
                visible: true,
            });
        }
    }
}

struct NotesClip {
    /// Where the clip is on the timeline, before any cut by a parent.
    time: f64,
    start: f64,
    end: f64,
    play_start: f64,
    loop_region: Option<(f64, f64)>,
    muted: bool,
    name: String,
}

/// How long a clip's content runs, for a clip that does not say.
fn content_length(content: &El, unit: &str, im: &Importer) -> f64 {
    let mut end: f64 = 0.0;
    for c in &content.children {
        let t = c.f64("time").unwrap_or(0.0) + c.f64("duration").unwrap_or(0.0);
        end = end.max(im.to_beats(t, unit));
    }
    if end > 0.0 {
        end
    } else {
        4.0
    }
}

/// The seconds at `beats` along warp points, carried on past the ends.
fn interpolate(points: &[(f64, f64)], beats: f64) -> f64 {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let seg = pts
        .windows(2)
        .find(|w| beats <= w[1].0)
        .or_else(|| pts.windows(2).last());
    match seg {
        Some(w) if (w[1].0 - w[0].0).abs() > 1e-12 => {
            w[0].1 + (beats - w[0].0) * (w[1].1 - w[0].1) / (w[1].0 - w[0].0)
        }
        _ => pts.first().map(|p| p.1).unwrap_or(0.0),
    }
}

fn beats_to_ticks(beats: f64) -> u64 {
    (beats.max(0.0) * PPQ).round() as u64
}

/// A level parameter in dB: linear gain unless it says decibels.
fn level_db(p: &El) -> f64 {
    let v = p.f64("value").unwrap_or(1.0);
    if p.attr("unit") == Some("decibel") {
        v
    } else if v <= 1e-5 {
        -100.0
    } else {
        20.0 * v.log10()
    }
}

/// A pan parameter as -1 (left) to 1 (right).
fn pan_value(p: &El) -> f64 {
    let v = p.f64("value").unwrap_or(0.5);
    let (min, max) = (p.f64("min").unwrap_or(0.0), p.f64("max").unwrap_or(1.0));
    let t = if (max - min).abs() > 1e-12 {
        (v - min) / (max - min)
    } else {
        0.5
    };
    (t * 2.0 - 1.0).clamp(-1.0, 1.0)
}

/// `#rrggbb` from the format's colour, which is the same.
fn css_color(c: &str) -> Option<String> {
    let hex = c.trim().strip_prefix('#')?;
    (hex.len() >= 6 && hex[..6].chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex[..6].to_lowercase()))
}

/// A parsed element: its name, attributes and children.
#[derive(Debug, Clone, Default)]
struct El {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<El>,
}

impl El {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn f64(&self, key: &str) -> Option<f64> {
        self.attr(key)?.trim().parse().ok()
    }

    fn child(&self, name: &str) -> Option<&El> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// The document as a tree of elements. A project file is at most a few
/// megabytes, so holding it is fine.
fn parse(xml: &str) -> Result<El, String> {
    use quick_xml::events::{BytesStart, Event};
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let element = |e: &BytesStart| -> Result<El, String> {
        let mut attrs = Vec::new();
        for a in e.attributes() {
            let a = a.map_err(|e| format!("the project file is damaged: {e}"))?;
            let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
            let value = a
                .normalized_value(quick_xml::XmlVersion::default())
                .map_err(|e| format!("the project file is damaged: {e}"))?
                .into_owned();
            attrs.push((key, value));
        }
        Ok(El {
            name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
            attrs,
            children: Vec::new(),
        })
    };
    let mut stack: Vec<El> = Vec::new();
    let mut root: Option<El> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => stack.push(element(&e)?),
            Ok(Event::Empty(e)) => {
                let el = element(&e)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Ok(Event::End(_)) => {
                let Some(el) = stack.pop() else {
                    return Err("the project file is damaged: an element closes twice".into());
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("the project file is damaged: {e}")),
        }
    }
    root.ok_or_else(|| "the project file is empty".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track<'a>(p: &'a Project, name: &str) -> &'a Track {
        p.tracks
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no track {name}"))
    }

    /// The example from the format's own README, as Bitwig Studio 5 wrote
    /// it: an instrument track with a CLAP synth and notes, an audio track
    /// with a warped loop, and the master.
    #[test]
    fn bitwig_s_example_opens() {
        let xml = include_str!("testdata/dawproject_bitwig_example.xml");
        let im = import(xml, "Example").expect("opens");
        assert_eq!(im.application, "Bitwig Studio 5.0");
        let p = &im.project;
        assert_eq!(p.tempo_map.entries[0].bpm, 149.0);
        let bass = track(p, "Bass");
        assert!(matches!(bass.kind, TrackKind::Midi));
        assert_eq!(bass.color, "#a2eabf");
        // A linear gain of 0.659 is -3.6 dB.
        assert!((bass.volume_db - 20.0 * 0.659140f64.log10()).abs() < 1e-6);
        assert_eq!(bass.pan, 0.0);
        let ClipContent::Midi(m) = &bass.clips[0].content else {
            panic!("a MIDI clip")
        };
        assert_eq!(m.clip.notes.len(), 9);
        assert_eq!(bass.clips[0].length_ticks, 960 * 8);
        let first = m.clip.notes.iter().find(|n| n.start_tick == 0).unwrap();
        assert_eq!((first.pitch, first.duration_ticks), (65, 240));
        // The synth, to be found among this computer's plug-ins.
        let surge = &im.plugins[0];
        assert_eq!(surge.format, DeviceFormat::Clap);
        assert_eq!(surge.device_id, "org.surge-synth-team.surge-xt");
        assert!(surge.instrument);
        assert_eq!(
            surge.state_file.as_deref(),
            Some("plugins/d19b1f6e-bbb6-42fe-a6c9-54b41d97a05d.clap-preset")
        );
        // The loop: its file, from its start, warped 8 beats to 2.82 s.
        let drums = track(p, "Drumloop");
        assert!(matches!(drums.kind, TrackKind::Audio));
        assert_eq!(drums.clips.len(), 1);
        let audio = &im.audio[0];
        assert_eq!(audio.file, "audio/Drumfunk3 170bpm.wav");
        assert_eq!(audio.start_secs, 0.0);
        assert_eq!(
            audio.warps.last().map(|w| (w.0 * 1000.0).round()),
            Some(8000.0)
        );
        // Both go to the master, which is not routed to itself.
        assert_eq!(bass.output_bus, None);
        assert!(im.left_out.is_empty(), "{:?}", im.left_out);
    }

    /// A song exported and opened again is the same song.
    #[test]
    fn an_exported_song_opens_as_itself() {
        use crate::automation::{AutomationLane, AutomationPoint};
        use crate::clip::WarpMarker;
        let mut p = Project::default();
        let mut bus = Track::new_bus("bus".into(), "FX bus".into());
        bus.volume_db = -3.0;
        let mut lead = Track::new_midi("lead".into(), "Lead".into());
        lead.pan = 0.5;
        lead.output_bus = Some("bus".into());
        lead.muted = true;
        let mut clip = hardwave_midi::MidiClip::new("m".into(), "Riff".into(), 960 * 4);
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick: 480,
            duration_ticks: 240,
            pitch: 62,
            velocity: 0.75,
            ..Default::default()
        });
        lead.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "m".into(),
                clip,
            }),
            track_id: "lead".into(),
            position_ticks: 960 * 4,
            length_ticks: 960 * 4,
            lane: 0,
            color: None,
        });
        lead.automation_lanes.push(AutomationLane {
            id: "a".into(),
            target: AutomationTarget::TrackPan,
            points: vec![
                AutomationPoint {
                    tick: 0,
                    value: 0.25,
                    curve: CurveMode::Linear,
                    tension: 0.0,
                },
                AutomationPoint {
                    tick: 960,
                    value: 1.0,
                    curve: CurveMode::Step,
                    tension: 0.0,
                },
            ],
            visible: true,
        });
        let mut drums = Track::new_audio("drums".into(), "Drums".into());
        drums.sends.push(Send {
            target: "bus".into(),
            gain_db: -6.0,
            pre_fader: true,
            enabled: true,
        });
        drums.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "a".into(),
                name: "Loop".into(),
                source_path: "pool".into(),
                source_hash: String::new(),
                source_start: 48_000,
                source_end: 0,
                gain_db: 0.0,
                fade_in_ticks: 96,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: vec![
                    WarpMarker {
                        clip_tick: 0,
                        source_sample: 48_000,
                    },
                    WarpMarker {
                        clip_tick: 960,
                        source_sample: 72_000,
                    },
                ],
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: "/s/loop.wav".into(),
                treatment: Default::default(),
            }),
            track_id: "drums".into(),
            position_ticks: 960 * 2,
            length_ticks: 960 * 2,
            lane: 0,
            color: None,
        });
        p.tracks.insert(1, drums);
        p.tracks.insert(1, lead);
        p.tracks.insert(1, bus);
        p.tempo_map.entries[0].bpm = 174.0;
        let out = crate::dawproject::export(
            &p,
            "test",
            &|a: &AudioClip| {
                Some(crate::dawproject::MediaInfo {
                    file: std::path::PathBuf::from(&a.source_file),
                    sample_rate: 48_000,
                    channels: 2,
                    seconds: 10.0,
                    frames_per_second: 48_000.0,
                })
            },
            &|_| None,
        );
        let im = import(&out.project_xml, "Back").expect("opens");
        let q = &im.project;
        assert_eq!(q.tempo_map.entries[0].bpm, 174.0);
        let bus = track(q, "FX bus");
        assert!(matches!(bus.kind, TrackKind::Bus));
        assert!((bus.volume_db + 3.0).abs() < 1e-3);
        let lead = track(q, "Lead");
        assert!(matches!(lead.kind, TrackKind::Midi));
        assert!((lead.pan - 0.5).abs() < 1e-6);
        assert!(lead.muted);
        assert_eq!(lead.output_bus.as_deref(), Some(bus.id.as_str()));
        let ClipContent::Midi(m) = &lead.clips[0].content else {
            panic!("a MIDI clip")
        };
        assert_eq!(lead.clips[0].position_ticks, 960 * 4);
        assert_eq!(
            (m.clip.notes[0].start_tick, m.clip.notes[0].pitch),
            (480, 62)
        );
        assert!((m.clip.notes[0].velocity - 0.75).abs() < 1e-6);
        let lane = &lead.automation_lanes[0];
        assert!(matches!(lane.target, AutomationTarget::TrackPan));
        assert_eq!(lane.points.len(), 2);
        assert!(matches!(lane.points[1].curve, CurveMode::Step));
        let drums = track(q, "Drums");
        assert_eq!(drums.sends.len(), 1);
        assert_eq!(drums.sends[0].target, bus.id);
        assert!(drums.sends[0].pre_fader);
        assert!((drums.sends[0].gain_db + 6.0).abs() < 1e-3);
        assert_eq!(drums.clips[0].position_ticks, 960 * 2);
        let ClipContent::Audio(a) = &drums.clips[0].content else {
            panic!("an audio clip")
        };
        assert_eq!(a.fade_in_ticks, 96);
        let audio = &im.audio[0];
        assert_eq!(audio.file, "audio/loop.wav");
        assert!((audio.start_secs - 1.0).abs() < 1e-6);
        // The marker: a beat in, half a second further into the file.
        assert!(audio
            .warps
            .iter()
            .any(|(b, s)| (b - 1.0).abs() < 1e-6 && (s - 1.5).abs() < 1e-6));
    }
}
