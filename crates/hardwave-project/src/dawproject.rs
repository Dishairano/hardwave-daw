//! DAWproject: the open project format Bitwig Studio, Studio One and
//! Cubase read and write (https://github.com/bitwig/dawproject).
//!
//! A song went to another DAW as stems, one file per track, and everything
//! that made it a song (the notes, where each clip sat, the mix, the
//! plug-ins and their settings) stayed behind. A DAWproject file is a zip
//! with `project.xml` describing all of that and the audio and plug-in
//! states beside it.
//!
//! This builds the two XML documents and says which files go in the zip;
//! reading the files and writing the zip is the app's job, which knows
//! where the samples are and what the plug-ins hold. Time is in beats (a
//! quarter note), volume a linear gain and pan 0 (left) to 1 (right), as
//! the format has them. What the format cannot carry is listed in
//! `left_out` so the person is told rather than finding out in the other
//! DAW.

use crate::automation::{AutomationLane, AutomationTarget, CurveMode};
use crate::clip::{AudioClip, ClipContent, ClipPlacement, FREEZE_LANE};
use crate::project::Project;
use crate::track::{PluginSlot, Track, TrackKind};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

const PPQ: f64 = 960.0;

/// An audio clip's file, as the app found it.
#[derive(Debug, Clone)]
pub struct MediaInfo {
    /// The file to put in the zip.
    pub file: PathBuf,
    /// The file's own rate, channels and length.
    pub sample_rate: u32,
    pub channels: u16,
    pub seconds: f64,
    /// The rate a clip's sample offsets count in: the rate the app loaded
    /// the file at, which may not be the file's own.
    pub frames_per_second: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    Vst3,
    Clap,
}

/// A plug-in slot, as the app knows it.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub format: PluginFormat,
    /// The VST3 class id (32 hex digits) or the CLAP plug-in id.
    pub device_id: String,
    pub name: String,
    pub vendor: String,
    pub instrument: bool,
    /// Its settings as a file: a .vstpreset for a VST3, the plug-in's own
    /// saved state for a CLAP.
    pub state: Option<Vec<u8>>,
}

/// Where a file in the zip comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum FileSource {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

/// One file in the zip besides the two documents.
#[derive(Debug, Clone, PartialEq)]
pub struct PackedFile {
    pub path: String,
    pub source: FileSource,
}

/// The documents, the files to pack with them, and what could not go.
#[derive(Debug, Clone)]
pub struct Export {
    pub project_xml: String,
    pub metadata_xml: String,
    pub files: Vec<PackedFile>,
    pub left_out: Vec<String>,
}

/// Build a DAWproject from `project`. `media` finds an audio clip's file;
/// `plugin` describes a plug-in slot (None for one that cannot go, such as
/// a built-in).
pub fn export(
    project: &Project,
    app_version: &str,
    media: &dyn Fn(&AudioClip) -> Option<MediaInfo>,
    plugin: &dyn Fn(&PluginSlot) -> Option<PluginInfo>,
) -> Export {
    Exporter {
        project,
        media,
        plugin,
        ids: 0,
        xml: Xml::default(),
        files: Vec::new(),
        file_names: HashMap::new(),
        left_out: Vec::new(),
    }
    .run(app_version)
}

/// The tracks worth taking: anything with clips, plug-ins, sends or a
/// bus's job, and the tracks those feed. The 500 empty insert rows a new
/// song starts with stay behind.
fn tracks_to_take(project: &Project) -> Vec<&Track> {
    let mut wanted: HashSet<&str> = HashSet::new();
    for t in &project.tracks {
        let has_content = t.clips.iter().any(|c| c.lane != FREEZE_LANE)
            || !t.inserts.is_empty()
            || !t.sends.is_empty()
            || !t.automation_lanes.is_empty()
            || matches!(t.kind, TrackKind::Bus | TrackKind::Return);
        if has_content && !matches!(t.kind, TrackKind::Master | TrackKind::Automation) {
            wanted.insert(&t.id);
        }
    }
    // What a taken track feeds is taken too.
    loop {
        let before = wanted.len();
        for t in &project.tracks {
            if !wanted.contains(t.id.as_str()) {
                continue;
            }
            let feeds = t
                .output_bus
                .iter()
                .chain(t.sends.iter().map(|s| &s.target))
                .cloned()
                .collect::<Vec<_>>();
            for id in feeds {
                if let Some(target) = project.tracks.iter().find(|x| x.id == id) {
                    if !matches!(target.kind, TrackKind::Master) {
                        wanted.insert(&target.id);
                    }
                }
            }
        }
        if wanted.len() == before {
            break;
        }
    }
    let mut out: Vec<&Track> = project
        .tracks
        .iter()
        .filter(|t| wanted.contains(t.id.as_str()))
        .collect();
    if let Some(master) = project
        .tracks
        .iter()
        .find(|t| matches!(t.kind, TrackKind::Master))
    {
        out.push(master);
    }
    out
}

/// The ids a track's parts were given, for routing and automation.
#[derive(Default, Clone)]
struct TrackIds {
    track: String,
    channel: String,
    volume: String,
    pan: String,
    mute: String,
    sends: Vec<String>,
    /// (slot id, parameter id) of an automated plug-in knob.
    params: HashMap<(String, u32), String>,
}

struct Exporter<'a> {
    project: &'a Project,
    media: &'a dyn Fn(&AudioClip) -> Option<MediaInfo>,
    plugin: &'a dyn Fn(&PluginSlot) -> Option<PluginInfo>,
    ids: u32,
    xml: Xml,
    files: Vec<PackedFile>,
    /// A source file's name in the zip, once given.
    file_names: HashMap<PathBuf, String>,
    left_out: Vec<String>,
}

impl Exporter<'_> {
    fn id(&mut self) -> String {
        let id = format!("id{}", self.ids);
        self.ids += 1;
        id
    }

    fn run(mut self, app_version: &str) -> Export {
        let project = self.project;
        let tracks = tracks_to_take(project);
        let mut ids: BTreeMap<String, TrackIds> = BTreeMap::new();
        for t in &tracks {
            let mut tid = TrackIds {
                track: self.id(),
                channel: self.id(),
                volume: self.id(),
                pan: self.id(),
                mute: self.id(),
                ..Default::default()
            };
            tid.sends = t.sends.iter().map(|_| self.id()).collect();
            for lane in &t.automation_lanes {
                if let AutomationTarget::PluginParam { slot_id, param_id } = &lane.target {
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        tid.params.entry((slot_id.clone(), *param_id))
                    {
                        slot.insert(format!("id{}", self.ids));
                        self.ids += 1;
                    }
                }
            }
            ids.insert(t.id.clone(), tid);
        }
        let master_channel = tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .and_then(|t| ids.get(&t.id))
            .map(|i| i.channel.clone());

        self.xml.decl();
        self.xml.open("Project", &[("version", "1.0".into())]);
        self.xml.empty(
            "Application",
            &[
                ("name", "Hardwave DAW".into()),
                ("version", app_version.into()),
            ],
        );
        let tempo_id = self.id();
        let first = project.tempo_map.entries.first();
        let bpm = first.map(|e| e.bpm).unwrap_or(140.0);
        let (num, den) = first
            .map(|e| (e.time_sig_num, e.time_sig_den))
            .unwrap_or((4, 4));
        self.xml.open("Transport", &[]);
        self.xml.empty(
            "Tempo",
            &[
                ("max", "999.000000".into()),
                ("min", "20.000000".into()),
                ("unit", "bpm".into()),
                ("value", num6(bpm)),
                ("id", tempo_id.clone()),
                ("name", "Tempo".into()),
            ],
        );
        let ts_id = self.id();
        self.xml.empty(
            "TimeSignature",
            &[
                ("denominator", den.to_string()),
                ("numerator", num.to_string()),
                ("id", ts_id.clone()),
            ],
        );
        self.xml.close("Transport");

        self.xml.open("Structure", &[]);
        for t in &tracks {
            self.track(t, &ids, master_channel.as_deref());
        }
        self.xml.close("Structure");

        let arrangement_id = self.id();
        self.xml.open("Arrangement", &[("id", arrangement_id)]);
        let lanes_id = self.id();
        self.xml
            .open("Lanes", &[("timeUnit", "beats".into()), ("id", lanes_id)]);
        for t in &tracks {
            let tid = ids[&t.id].clone();
            let lane_id = self.id();
            self.xml
                .open("Lanes", &[("track", tid.track.clone()), ("id", lane_id)]);
            let clips_id = self.id();
            self.xml.open("Clips", &[("id", clips_id)]);
            let mut placed: Vec<&ClipPlacement> =
                t.clips.iter().filter(|c| c.lane != FREEZE_LANE).collect();
            placed.sort_by_key(|c| c.position_ticks);
            for clip in placed {
                self.clip(t, clip);
            }
            self.xml.close("Clips");
            for lane in &t.automation_lanes {
                self.automation(t, lane, &tid);
            }
            self.xml.close("Lanes");
        }
        self.xml.close("Lanes");
        if project.tempo_map.entries.len() > 1 {
            self.tempo_automation(&tempo_id, &ts_id);
        }
        self.xml.close("Arrangement");
        self.xml.empty("Scenes", &[]);
        self.xml.close("Project");

        Export {
            project_xml: self.xml.out,
            metadata_xml: metadata(project),
            files: self.files,
            left_out: self.left_out,
        }
    }

    fn track(&mut self, t: &Track, ids: &BTreeMap<String, TrackIds>, master: Option<&str>) {
        let tid = &ids[&t.id];
        let (content, role) = match t.kind {
            TrackKind::Master => ("audio notes", "master"),
            TrackKind::Midi => ("notes", "regular"),
            TrackKind::Return => ("audio", "effect"),
            TrackKind::Bus => ("audio", "submix"),
            _ => ("audio", "regular"),
        };
        let mut attrs = vec![
            ("contentType", content.to_string()),
            ("loaded", "true".into()),
            ("id", tid.track.clone()),
            ("name", t.name.clone()),
        ];
        if let Some(color) = hex_color(&t.color) {
            attrs.push(("color", color));
        }
        self.xml.open("Track", &attrs);
        // Where it goes: its bus, or the master.
        let destination = t
            .output_bus
            .as_ref()
            .and_then(|b| ids.get(b))
            .map(|i| i.channel.clone())
            .or_else(|| {
                (!matches!(t.kind, TrackKind::Master))
                    .then(|| master.map(str::to_string))
                    .flatten()
            });
        let mut ch = vec![("audioChannels", "2".to_string())];
        if let Some(d) = destination {
            ch.push(("destination", d));
        }
        ch.push(("role", role.into()));
        ch.push(("solo", t.soloed.to_string()));
        ch.push(("id", tid.channel.clone()));
        self.xml.open("Channel", &ch);

        // Devices, in chain order.
        let mut wrote_devices = false;
        for slot in &t.inserts {
            let Some(info) = (self.plugin)(slot) else {
                self.left_out.push(format!(
                    "{}: {} (a built-in plug-in, which other DAWs do not have)",
                    t.name,
                    slot.plugin_id.trim_start_matches("hardwave.native.")
                ));
                continue;
            };
            if !wrote_devices {
                self.xml.open("Devices", &[]);
                wrote_devices = true;
            }
            self.device(t, slot, &info, tid);
        }
        if wrote_devices {
            self.xml.close("Devices");
        }
        self.xml.empty(
            "Mute",
            &[
                ("value", t.muted.to_string()),
                ("id", tid.mute.clone()),
                ("name", "Mute".into()),
            ],
        );
        self.xml.empty(
            "Pan",
            &[
                ("max", "1.000000".into()),
                ("min", "0.000000".into()),
                ("unit", "normalized".into()),
                ("value", num6(((t.pan + 1.0) * 0.5).clamp(0.0, 1.0))),
                ("id", tid.pan.clone()),
                ("name", "Pan".into()),
            ],
        );
        if !t.sends.is_empty() {
            self.xml.open("Sends", &[]);
            for (i, send) in t.sends.iter().enumerate() {
                let Some(target) = ids.get(&send.target) else {
                    continue;
                };
                self.xml.open(
                    "Send",
                    &[
                        ("destination", target.channel.clone()),
                        ("type", if send.pre_fader { "pre" } else { "post" }.into()),
                        ("id", self.next_id_peek()),
                    ],
                );
                self.ids += 1;
                let enable_id = self.id();
                self.xml.empty(
                    "Enable",
                    &[
                        ("value", send.enabled.to_string()),
                        ("id", enable_id),
                        ("name", "Enable".into()),
                    ],
                );
                self.xml.empty(
                    "Volume",
                    &[
                        ("max", "2.000000".into()),
                        ("min", "0.000000".into()),
                        ("unit", "linear".into()),
                        ("value", num6(db_to_gain(send.gain_db))),
                        ("id", tid.sends[i].clone()),
                        ("name", "Volume".into()),
                    ],
                );
                self.xml.close("Send");
            }
            self.xml.close("Sends");
        }
        self.xml.empty(
            "Volume",
            &[
                ("max", "2.000000".into()),
                ("min", "0.000000".into()),
                ("unit", "linear".into()),
                ("value", num6(db_to_gain(t.volume_db))),
                ("id", tid.volume.clone()),
                ("name", "Volume".into()),
            ],
        );
        self.xml.close("Channel");
        self.xml.close("Track");
    }

    fn next_id_peek(&self) -> String {
        format!("id{}", self.ids)
    }

    fn device(&mut self, t: &Track, slot: &PluginSlot, info: &PluginInfo, tid: &TrackIds) {
        let element = match info.format {
            PluginFormat::Vst3 => "Vst3Plugin",
            PluginFormat::Clap => "ClapPlugin",
        };
        let device_id = self.id();
        let mut attrs = vec![
            ("deviceID", info.device_id.clone()),
            ("deviceName", info.name.clone()),
            (
                "deviceRole",
                if info.instrument {
                    "instrument"
                } else {
                    "audioFX"
                }
                .into(),
            ),
        ];
        if !info.vendor.is_empty() {
            attrs.push(("deviceVendor", info.vendor.clone()));
        }
        attrs.push(("loaded", "true".into()));
        attrs.push(("id", device_id));
        attrs.push(("name", info.name.clone()));
        self.xml.open(element, &attrs);
        // The knobs that are automated, so the automation can point at them.
        let mut automated: Vec<(u32, String)> = tid
            .params
            .iter()
            .filter(|((s, _), _)| *s == slot.id)
            .map(|((_, p), id)| (*p, id.clone()))
            .collect();
        automated.sort();
        self.xml.open("Parameters", &[]);
        for (param_id, id) in automated {
            self.xml.empty(
                "RealParameter",
                &[
                    ("max", "1.000000".into()),
                    ("min", "0.000000".into()),
                    ("unit", "normalized".into()),
                    ("parameterID", param_id.to_string()),
                    ("id", id),
                ],
            );
        }
        self.xml.close("Parameters");
        let enabled_id = self.id();
        self.xml.empty(
            "Enabled",
            &[
                ("value", slot.enabled.to_string()),
                ("id", enabled_id),
                ("name", "On/Off".into()),
            ],
        );
        if let Some(state) = &info.state {
            let ext = match info.format {
                PluginFormat::Vst3 => "vstpreset",
                PluginFormat::Clap => "clap-preset",
            };
            let path = format!("plugins/{}-{}.{ext}", safe_name(&t.name), slot.id);
            self.files.push(PackedFile {
                path: path.clone(),
                source: FileSource::Bytes(state.clone()),
            });
            self.xml.empty("State", &[("path", path)]);
        }
        self.xml.close(element);
    }

    fn clip(&mut self, t: &Track, clip: &ClipPlacement) {
        let time = clip.position_ticks as f64 / PPQ;
        let duration = clip.length_ticks as f64 / PPQ;
        match &clip.content {
            ClipContent::Midi(m) => {
                let mut attrs = vec![
                    ("time", num6(time)),
                    ("duration", num6(duration)),
                    ("playStart", "0.0".to_string()),
                ];
                if m.clip.muted {
                    attrs.push(("enable", "false".into()));
                }
                attrs.push(("name", m.clip.name.clone()));
                self.xml.open("Clip", &attrs);
                let notes_id = self.id();
                self.xml.open("Notes", &[("id", notes_id)]);
                let mut notes: Vec<_> = m
                    .clip
                    .notes
                    .iter()
                    .filter(|n| !n.muted && n.start_tick < clip.length_ticks)
                    .collect();
                notes.sort_by_key(|n| (n.start_tick, n.pitch));
                for n in notes {
                    let len = n
                        .duration_ticks
                        .max(1)
                        .min(clip.length_ticks - n.start_tick);
                    self.xml.empty(
                        "Note",
                        &[
                            ("time", num6(n.start_tick as f64 / PPQ)),
                            ("duration", num6(len as f64 / PPQ)),
                            ("channel", (n.channel & 0x0F).to_string()),
                            ("key", n.pitch.to_string()),
                            ("vel", num6(n.velocity.clamp(0.0, 1.0) as f64)),
                            ("rel", num6(n.release_velocity.clamp(0.0, 1.0) as f64)),
                        ],
                    );
                }
                self.xml.close("Notes");
                self.xml.close("Clip");
            }
            ClipContent::Audio(a) => self.audio_clip(t, clip, a, time, duration),
        }
    }

    fn audio_clip(
        &mut self,
        t: &Track,
        clip: &ClipPlacement,
        a: &AudioClip,
        time: f64,
        duration: f64,
    ) {
        let Some(info) = (self.media)(a) else {
            self.left_out
                .push(format!("{}: {} (its file was not found)", t.name, a.name));
            return;
        };
        if a.reversed {
            self.left_out.push(format!(
                "{}: {} plays forwards (reversing is not in the format)",
                t.name, a.name
            ));
        }
        if a.pitch_semitones.abs() > 1e-6 {
            self.left_out.push(format!(
                "{}: {} is not transposed (transposing an audio clip is not in the format)",
                t.name, a.name
            ));
        }
        if a.gain_db.abs() > 1e-6 {
            self.left_out.push(format!(
                "{}: {} loses its clip gain (not in the format)",
                t.name, a.name
            ));
        }
        let zip_path = self.media_file(&info.file);
        let fps = info.frames_per_second.max(1.0);
        let fade_in = a.fade_in_ticks as f64 / PPQ;
        let fade_out = a.fade_out_ticks as f64 / PPQ;
        let mut outer = vec![
            ("time", num6(time)),
            ("duration", num6(duration)),
            ("playStart", "0.0".to_string()),
            ("fadeTimeUnit", "beats".into()),
            ("fadeInTime", num6(fade_in)),
            ("fadeOutTime", num6(fade_out)),
        ];
        if a.muted {
            outer.push(("enable", "false".into()));
        }
        outer.push(("name", a.name.clone()));
        self.xml.open("Clip", &outer);
        let clips_id = self.id();
        self.xml.open("Clips", &[("id", clips_id)]);
        self.xml.open(
            "Clip",
            &[
                ("time", "0.0".into()),
                ("duration", num6(duration)),
                ("contentTimeUnit", "beats".into()),
                ("playStart", "0.0".into()),
            ],
        );
        let warps_id = self.id();
        self.xml.open(
            "Warps",
            &[
                ("contentTimeUnit", "seconds".into()),
                ("timeUnit", "beats".into()),
                ("id", warps_id),
            ],
        );
        let audio_id = self.id();
        self.xml.open(
            "Audio",
            &[
                (
                    "algorithm",
                    if a.treatment.resample {
                        "repitch"
                    } else {
                        "stretch"
                    }
                    .into(),
                ),
                ("channels", info.channels.to_string()),
                ("duration", num6(info.seconds)),
                ("sampleRate", info.sample_rate.to_string()),
                ("id", audio_id),
            ],
        );
        self.xml.empty("File", &[("path", zip_path)]);
        self.xml.close("Audio");
        for (beat, secs) in self.warp_points(clip, a, fps) {
            self.xml
                .empty("Warp", &[("time", num6(beat)), ("contentTime", num6(secs))]);
        }
        self.xml.close("Warps");
        self.xml.close("Clip");
        self.xml.close("Clips");
        self.xml.close("Clip");
    }

    /// Where in its file the clip is, at points through it: (beats into
    /// the clip, seconds into the file).
    fn warp_points(&self, clip: &ClipPlacement, a: &AudioClip, fps: f64) -> Vec<(f64, f64)> {
        let start_secs = a.source_start as f64 / fps;
        let length = clip.length_ticks;
        if !a.warp_markers.is_empty() {
            let mut pts: Vec<(f64, f64)> = Vec::new();
            if a.warp_markers.first().map(|m| m.clip_tick) != Some(0) {
                pts.push((0.0, start_secs));
            }
            for m in &a.warp_markers {
                pts.push((m.clip_tick as f64 / PPQ, m.source_sample as f64 / fps));
            }
            // On past the last marker at the pace of the last stretch.
            let end = length as f64 / PPQ;
            if let (Some(&(b1, s1)), Some(&(b0, s0))) = (pts.last(), pts.iter().rev().nth(1)) {
                if end > b1 && b1 > b0 {
                    pts.push((end, s1 + (end - b1) * (s1 - s0) / (b1 - b0)));
                }
            }
            return pts;
        }
        // Unwarped: the file plays at its own speed (or the clip's stretch),
        // through whatever tempo the song has, so a point goes at each tempo
        // change inside the clip.
        let stretch = if a.stretch_ratio <= 0.01 {
            1.0
        } else {
            a.stretch_ratio
        };
        let map = &self.project.tempo_map;
        let secs = |tick: u64| map.tick_to_samples(tick, 1_000_000.0) as f64 / 1_000_000.0;
        let t0 = secs(clip.position_ticks);
        let mut ticks: Vec<u64> = vec![0, length];
        for e in &map.entries {
            if e.tick > clip.position_ticks && e.tick < clip.position_ticks + length {
                ticks.push(e.tick - clip.position_ticks);
            }
        }
        ticks.sort_unstable();
        ticks.dedup();
        ticks
            .into_iter()
            .map(|k| {
                let elapsed = secs(clip.position_ticks + k) - t0;
                (k as f64 / PPQ, start_secs + elapsed / stretch)
            })
            .collect()
    }

    /// A file's place in the zip, given once per file.
    fn media_file(&mut self, file: &PathBuf) -> String {
        if let Some(name) = self.file_names.get(file) {
            return name.clone();
        }
        let base = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.wav")
            .to_string();
        let (stem, ext) = match base.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (base.clone(), String::new()),
        };
        let taken: HashSet<&String> = self.file_names.values().collect();
        let mut name = format!("audio/{base}");
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("audio/{stem} ({n}){ext}");
            n += 1;
        }
        self.file_names.insert(file.clone(), name.clone());
        self.files.push(PackedFile {
            path: name.clone(),
            source: FileSource::Path(file.clone()),
        });
        name
    }

    fn automation(&mut self, t: &Track, lane: &AutomationLane, tid: &TrackIds) {
        if lane.points.is_empty() {
            return;
        }
        // Lane values are 0..1 across the control's range.
        let (target, unit, convert): (String, &str, fn(f64) -> f64) = match &lane.target {
            AutomationTarget::TrackVolume => (tid.volume.clone(), "linear", lane_db_to_gain),
            AutomationTarget::TrackPan => (tid.pan.clone(), "normalized", |v| v),
            AutomationTarget::TrackMute => (tid.mute.clone(), "", |v| v),
            AutomationTarget::SendLevel { send_index } => match tid.sends.get(*send_index) {
                Some(id) => (id.clone(), "linear", lane_db_to_gain),
                None => return,
            },
            AutomationTarget::PluginParam { slot_id, param_id } => {
                match tid.params.get(&(slot_id.clone(), *param_id)) {
                    // Only a plug-in that went (not a built-in) has its knob.
                    Some(id)
                        if t.inserts
                            .iter()
                            .any(|s| s.id == *slot_id && (self.plugin)(s).is_some()) =>
                    {
                        (id.clone(), "normalized", |v| v)
                    }
                    _ => return,
                }
            }
        };
        let points_id = self.id();
        let mut attrs = vec![("timeUnit", "beats".to_string()), ("id", points_id)];
        if !unit.is_empty() {
            attrs.insert(0, ("unit", unit.to_string()));
        }
        self.xml.open("Points", &attrs);
        self.xml.empty("Target", &[("parameter", target)]);
        let mute = matches!(lane.target, AutomationTarget::TrackMute);
        let mut points = lane.points.clone();
        points.sort_by_key(|p| p.tick);
        for p in points {
            let time = num6(p.tick as f64 / PPQ);
            if mute {
                self.xml.empty(
                    "BoolPoint",
                    &[("time", time), ("value", (p.value > 0.5).to_string())],
                );
            } else {
                let interpolation = match p.curve {
                    CurveMode::Step => "hold",
                    _ => "linear",
                };
                self.xml.empty(
                    "RealPoint",
                    &[
                        ("time", time),
                        ("value", num6(convert(p.value.clamp(0.0, 1.0)))),
                        ("interpolation", interpolation.into()),
                    ],
                );
            }
        }
        self.xml.close("Points");
    }

    fn tempo_automation(&mut self, tempo_id: &str, ts_id: &str) {
        let entries = self.project.tempo_map.entries.clone();
        let id = self.id();
        self.xml.open(
            "TempoAutomation",
            &[
                ("unit", "bpm".into()),
                ("timeUnit", "beats".into()),
                ("id", id),
            ],
        );
        self.xml.empty("Target", &[("parameter", tempo_id.into())]);
        for (i, e) in entries.iter().enumerate() {
            // A ramp runs from this tempo to the next one.
            let ramps = matches!(e.ramp, crate::tempo::TempoRamp::Linear) && i + 1 < entries.len();
            self.xml.empty(
                "RealPoint",
                &[
                    ("time", num6(e.tick as f64 / PPQ)),
                    ("value", num6(e.bpm)),
                    (
                        "interpolation",
                        if ramps { "linear" } else { "hold" }.into(),
                    ),
                ],
            );
        }
        self.xml.close("TempoAutomation");
        let id = self.id();
        self.xml.open(
            "TimeSignatureAutomation",
            &[("timeUnit", "beats".into()), ("id", id)],
        );
        self.xml.empty("Target", &[("parameter", ts_id.into())]);
        let mut last = (0u32, 0u32);
        for e in &entries {
            if (e.time_sig_num, e.time_sig_den) != last {
                last = (e.time_sig_num, e.time_sig_den);
                self.xml.empty(
                    "TimeSignaturePoint",
                    &[
                        ("time", num6(e.tick as f64 / PPQ)),
                        ("numerator", e.time_sig_num.to_string()),
                        ("denominator", e.time_sig_den.to_string()),
                    ],
                );
            }
        }
        self.xml.close("TimeSignatureAutomation");
    }
}

fn metadata(project: &Project) -> String {
    let m = &project.metadata;
    let mut xml = Xml::default();
    xml.decl();
    xml.open("MetaData", &[]);
    let title = if m.title.is_empty() {
        &m.name
    } else {
        &m.title
    };
    for (tag, value) in [
        ("Title", title),
        ("Artist", &m.author),
        ("Genre", &m.genre),
        ("Website", &m.url),
        ("Comment", &m.info),
    ] {
        if !value.trim().is_empty() {
            xml.text(tag, value);
        }
    }
    xml.close("MetaData");
    xml.out
}

/// A level in dB to the format's linear gain, 0 to 2 (+6 dB).
fn db_to_gain(db: f64) -> f64 {
    if db <= -100.0 {
        0.0
    } else {
        10f64.powf(db / 20.0).min(2.0)
    }
}

/// A volume or send lane's 0..1 (across -60..+6 dB) to linear gain.
fn lane_db_to_gain(v: f64) -> f64 {
    if v <= 0.0 {
        0.0
    } else {
        db_to_gain(-60.0 + v * 66.0)
    }
}

fn num6(v: f64) -> String {
    format!("{v:.6}")
}

/// `#rrggbb`, or None for a colour that is not one.
fn hex_color(c: &str) -> Option<String> {
    let c = c.trim();
    let hex = c.strip_prefix('#').unwrap_or(c);
    (hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_lowercase()))
}

/// A name fit for a file in the zip.
fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "track".into()
    } else {
        cleaned
    }
}

/// A small XML writer: elements, attributes escaped, two-space indent.
#[derive(Default)]
struct Xml {
    out: String,
    depth: usize,
}

impl Xml {
    fn decl(&mut self) {
        self.out
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    }

    fn start(&mut self, tag: &str, attrs: &[(&str, String)]) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push('<');
        self.out.push_str(tag);
        for (k, v) in attrs {
            self.out.push(' ');
            self.out.push_str(k);
            self.out.push_str("=\"");
            self.out.push_str(&escape(v));
            self.out.push('"');
        }
    }

    fn open(&mut self, tag: &str, attrs: &[(&str, String)]) {
        self.start(tag, attrs);
        self.out.push_str(">\n");
        self.depth += 1;
    }

    fn empty(&mut self, tag: &str, attrs: &[(&str, String)]) {
        self.start(tag, attrs);
        self.out.push_str("/>\n");
    }

    fn text(&mut self, tag: &str, text: &str) {
        self.start(tag, &[]);
        self.out.push('>');
        self.out.push_str(&escape(text));
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push_str(">\n");
    }

    fn close(&mut self, tag: &str) {
        self.depth = self.depth.saturating_sub(1);
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push_str(">\n");
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::{AutomationLane, AutomationPoint};
    use crate::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef, WarpMarker};
    use crate::mixer::Send;
    use crate::tempo::{TempoEntry, TempoRamp};
    use crate::track::Track;

    /// A song with a bit of everything the format carries.
    fn song() -> Project {
        let mut p = Project::default();
        p.metadata.title = "Rawstyle anthem".into();
        p.metadata.author = "Hardwave".into();
        p.tempo_map.entries = vec![
            TempoEntry {
                tick: 0,
                bpm: 150.0,
                time_sig_num: 4,
                time_sig_den: 4,
                ramp: TempoRamp::Instant,
            },
            TempoEntry {
                tick: 960 * 16,
                bpm: 75.0,
                time_sig_num: 3,
                time_sig_den: 4,
                ramp: TempoRamp::Instant,
            },
        ];
        let mut bus = Track::new_bus("bus".into(), "Drums & FX".into());
        bus.color = "#22C55E".into();
        let mut lead = Track::new_midi("lead".into(), "Lead <screech>".into());
        lead.volume_db = -6.0;
        lead.pan = -0.5;
        lead.output_bus = Some("bus".into());
        lead.inserts.push(PluginSlot {
            id: "s1".into(),
            plugin_id: "vst3:serum".into(),
            enabled: true,
            state: Some(vec![1, 2, 3]),
            sidechain_source: None,
            wet: 1.0,
        });
        lead.inserts.push(PluginSlot {
            id: "s2".into(),
            plugin_id: "hardwave.native.eq".into(),
            enabled: true,
            state: None,
            sidechain_source: None,
            wet: 1.0,
        });
        let mut clip = hardwave_midi::MidiClip::new("m".into(), "Riff".into(), 960 * 4);
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick: 480,
            duration_ticks: 240,
            pitch: 62,
            velocity: 0.75,
            ..Default::default()
        });
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick: 960 * 5,
            duration_ticks: 240,
            pitch: 64,
            velocity: 0.5,
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
            id: "a1".into(),
            target: AutomationTarget::PluginParam {
                slot_id: "s1".into(),
                param_id: 7,
            },
            points: vec![
                AutomationPoint {
                    tick: 0,
                    value: 0.2,
                    curve: CurveMode::Linear,
                    tension: 0.0,
                },
                AutomationPoint {
                    tick: 960 * 8,
                    value: 0.9,
                    curve: CurveMode::Step,
                    tension: 0.0,
                },
            ],
            visible: true,
        });
        lead.automation_lanes.push(AutomationLane {
            id: "a2".into(),
            target: AutomationTarget::TrackVolume,
            points: vec![AutomationPoint {
                tick: 0,
                value: 60.0 / 66.0,
                curve: CurveMode::Linear,
                tension: 0.0,
            }],
            visible: true,
        });
        let mut drums = Track::new_audio("drums".into(), "Drum loop".into());
        drums.sends.push(Send {
            target: "bus".into(),
            gain_db: -6.0,
            pre_fader: true,
            enabled: true,
        });
        drums.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "a".into(),
                name: "Kick loop".into(),
                source_path: "pool-1".into(),
                source_hash: String::new(),
                source_start: 48_000,
                source_end: 480_000,
                gain_db: 0.0,
                fade_in_ticks: 96,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: "C:/Samples/kick loop.wav".into(),
                treatment: Default::default(),
            }),
            track_id: "drums".into(),
            position_ticks: 0,
            length_ticks: 960 * 8,
            lane: 0,
            color: None,
        });
        let mut warped = drums.clips[0].clone();
        if let ClipContent::Audio(a) = &mut warped.content {
            a.id = "w".into();
            a.warp_markers = vec![
                WarpMarker {
                    clip_tick: 0,
                    source_sample: 0,
                },
                WarpMarker {
                    clip_tick: 960,
                    source_sample: 24_000,
                },
            ];
        }
        warped.position_ticks = 960 * 8;
        drums.clips.push(warped);
        // Before the inserts, which are empty and stay behind.
        p.tracks.insert(1, drums);
        p.tracks.insert(1, lead);
        p.tracks.insert(1, bus);
        p
    }

    fn run(p: &Project) -> Export {
        export(
            p,
            "0.266.27",
            &|a: &AudioClip| {
                Some(MediaInfo {
                    file: PathBuf::from(&a.source_file),
                    sample_rate: 44_100,
                    channels: 2,
                    seconds: 10.0,
                    frames_per_second: 48_000.0,
                })
            },
            &|slot: &PluginSlot| {
                slot.plugin_id.starts_with("vst3:").then(|| PluginInfo {
                    format: PluginFormat::Vst3,
                    device_id: "ABCDEF0123456789ABCDEF0123456789".into(),
                    name: "Serum".into(),
                    vendor: "Xfer Records".into(),
                    instrument: true,
                    state: Some(b"preset".to_vec()),
                })
            },
        )
    }

    fn attr_values<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
        let key = format!(" {name}=\"");
        xml.match_indices(&key)
            .map(|(i, _)| {
                let rest = &xml[i + key.len()..];
                &rest[..rest.find('"').unwrap()]
            })
            .collect()
    }

    #[test]
    fn every_reference_points_at_something_that_is_there() {
        let out = run(&song());
        let ids: HashSet<&str> = attr_values(&out.project_xml, "id").into_iter().collect();
        assert_eq!(
            ids.len(),
            attr_values(&out.project_xml, "id").len(),
            "ids are unique"
        );
        for attr in ["destination", "track", "parameter"] {
            for r in attr_values(&out.project_xml, attr) {
                assert!(ids.contains(r), "{attr}=\"{r}\" points at nothing");
            }
        }
    }

    #[test]
    fn the_song_s_tracks_mix_notes_and_audio_go_across() {
        let out = run(&song());
        // For checking against the format's schema by hand.
        if let Ok(dir) = std::env::var("HW_DAWPROJECT_DUMP") {
            let _ = std::fs::write(format!("{dir}/project.xml"), &out.project_xml);
            let _ = std::fs::write(format!("{dir}/metadata.xml"), &out.metadata_xml);
        }
        let x = &out.project_xml;
        // The three tracks with something on them, and the master; the 500
        // empty inserts stay behind.
        assert_eq!(x.matches("<Track ").count(), 4, "{x}");
        assert!(
            x.contains("name=\"Lead &lt;screech&gt;\""),
            "names are escaped"
        );
        assert!(x.contains("role=\"submix\""), "the bus is a submix");
        assert!(x.contains("role=\"master\""));
        assert!(x.contains("color=\"#22c55e\""));
        // -6 dB is a gain of about 0.501, pan -0.5 is 0.25.
        assert!(x.contains("value=\"0.501187\""), "volume as linear gain");
        assert!(
            x.contains("unit=\"normalized\" value=\"0.250000\""),
            "pan 0 to 1"
        );
        // A note half a beat into a clip four beats in; the note past the
        // clip's end stays behind.
        assert!(x.contains("<Clip time=\"4.000000\" duration=\"4.000000\""));
        assert!(x.contains("<Note time=\"0.500000\" duration=\"0.250000\" channel=\"0\" key=\"62\" vel=\"0.750000\""));
        assert!(!x.contains("key=\"64\""));
        // The plug-in, with its state file, and the built-in left out.
        assert!(x.contains("<Vst3Plugin deviceID=\"ABCDEF0123456789ABCDEF0123456789\" deviceName=\"Serum\" deviceRole=\"instrument\""));
        assert!(x.contains("<State path=\"plugins/Lead__screech_-s1.vstpreset\"/>"));
        assert!(
            out.left_out.iter().any(|l| l.contains("eq")),
            "{:?}",
            out.left_out
        );
        // The automated knob is a parameter the automation points at.
        assert!(x.contains("parameterID=\"7\""));
        assert!(x.contains("interpolation=\"hold\""));
        // The send is pre-fader, at -6 dB.
        assert!(x.contains("type=\"pre\""));
        // The tempo change and the bar change.
        assert!(x.contains("<TempoAutomation"));
        assert!(x.contains("numerator=\"3\""));
        // The audio file goes in once, however many clips use it.
        let audio: Vec<_> = out
            .files
            .iter()
            .filter(|f| f.path.starts_with("audio/"))
            .collect();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].path, "audio/kick loop.wav");
        assert!(x.contains("<File path=\"audio/kick loop.wav\"/>"));
        // The plain clip starts a second into its file (48,000 at 48 kHz)
        // and plays 8 beats at 150 bpm, 3.2 s; the warped one follows its
        // markers: a beat to half a second.
        assert!(x.contains("<Warp time=\"0.000000\" contentTime=\"1.000000\"/>"));
        assert!(x.contains("<Warp time=\"8.000000\" contentTime=\"4.200000\"/>"));
        assert!(x.contains("<Warp time=\"1.000000\" contentTime=\"0.500000\"/>"));
        assert!(out.metadata_xml.contains("<Title>Rawstyle anthem</Title>"));
    }
}
