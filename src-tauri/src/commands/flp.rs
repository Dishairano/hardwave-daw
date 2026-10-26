//! Opening an FL Studio project.
//!
//! Most of the people this DAW is for work in FL. "Can I open my
//! projects" is the first question any of them ask, and until now the
//! answer was no, with a stub module describing what an answer would
//! look like.
//!
//! What comes across is the song: the tempo, an instrument track per
//! channel that plays notes (a sampler channel with its sample in the
//! built-in sampler), every sample on the playlist where FL had it at its
//! channel's level, pan, pitch, direction and stretch, the playlist's
//! track names and switches, and the mixer: every insert the sound passes
//! through as a track of its own, with its fader, pan, switches and
//! routes, FL's Parametric EQ 2 as the built-in EQ, other people's VST3
//! instruments and effects with their settings, and the automation clips
//! moving any of those. It opens as a song of its own, like a .hwp. What
//! cannot come across is what FL's own instruments and most of its
//! effects hold, a VST2's or CLAP's settings, and automation of knobs with
//! no counterpart here. The report says what was left behind rather than
//! letting it be found later.

use hardwave_project::clip::ClipContent;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlpImportReport {
    pub bpm: f32,
    pub tracks: usize,
    pub clips: usize,
    pub notes: usize,
    /// Samples the project points at that are not on this computer.
    pub samples: Vec<String>,
    /// What could not be carried, in plain words.
    pub left_behind: Vec<String>,
}

/// Open a `.flp` as a new song.
#[tauri::command]
pub async fn import_flp(app: AppHandle, path: String) -> Result<FlpImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_flp_blocking(&app, &path))
        .await
        .map_err(|e| format!("opening stopped: {e}"))?
}

fn import_flp_blocking(app: &AppHandle, path: &str) -> Result<FlpImportReport, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read that file: {e}"))?;
    let fl = hardwave_project::flp_parser::parse(&bytes).map_err(|e| e.to_string())?;

    // A playlist copies a pattern's notes into every placement, so a
    // small file can ask for billions of notes. Counted before anything
    // is made: past what any song holds, nothing is imported.
    let mut notes_per_pattern: HashMap<u32, usize> = HashMap::new();
    for (pattern, _, notes) in &fl.pattern_notes {
        *notes_per_pattern.entry(*pattern).or_default() += notes.len();
    }
    let mut would_make = 0usize;
    for clip in &fl.playlist_clips {
        if let hardwave_project::fl_import::FlClipContent::Pattern { pattern_index, .. } = &clip.content
        {
            would_make = would_make
                .saturating_add(notes_per_pattern.get(pattern_index).copied().unwrap_or(0));
        }
    }
    if would_make > hardwave_project::sanitize::MAX_NOTES
        || fl.playlist_clips.len() > hardwave_project::sanitize::MAX_CLIPS
        || fl.channels.len() > hardwave_project::sanitize::MAX_TRACKS
    {
        return Err(
            "that FL project expands to more than any song holds; it was not imported".into(),
        );
    }

    let file = Path::new(path);
    let name = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "FL project".into());
    let finder = SampleFinder::new(file.parent());
    let built = hardwave_project::fl_build::build_project(&fl, &name, &|p: &str| finder.find(p));
    let mut project = built.project;

    // Sampler channels play their sample in the built-in sampler, which
    // keeps it as its state; the song's plug-ins are made from that when
    // it opens. The channel's envelope, loop, voices and the sample's
    // effects (reverse, swap, polarity, DC, normalize) become the
    // sampler's settings.
    let mut samplers_failed = Vec::new();
    for sampler in &built.sampler_samples {
        match hardwave_dsp::audio_file::AudioFileReader::read(Path::new(&sampler.file)) {
            Ok((info, channels)) => {
                // The sample stays as it is on disk; reverse, swap, polarity,
                // DC removal and normalize are the sampler's settings, done as
                // it plays, so the sample window shows them and they can be
                // turned off again.
                use hardwave_native_plugins::sampler as hs;
                let on = |b: bool| if b { 1.0 } else { 0.0 };
                let mut settings = hs::settings_from_fl(&sampler.setup, sampler.root_note);
                settings.extend([
                    (hs::PARAM_REVERSE, on(sampler.reversed)),
                    (hs::PARAM_SWAP_STEREO, on(sampler.swap_stereo)),
                    (hs::PARAM_POLARITY, on(sampler.polarity_inverted)),
                ]);
                let state = hs::encode_sample_state_with(
                    info.sample_rate,
                    sampler.root_note,
                    channels,
                    &sampler.file,
                    settings,
                );
                let slot_id = uuid::Uuid::new_v4().to_string();
                if let Some(track) = project.track_mut(&sampler.track_id) {
                    track.inserts.insert(
                        0,
                        hardwave_project::track::PluginSlot {
                            id: slot_id.clone(),
                            plugin_id: hardwave_native_plugins::sampler::NativeSampler::ID
                                .to_string(),
                            enabled: true,
                            state: None,
                            sidechain_source: None,
                            wet: 1.0,
                        },
                    );
                    project.set_plugin_state(slot_id, "native-sampler", state);
                }
            }
            Err(e) => {
                log::warn!("import_flp: sampler sample {}: {e:?}", sampler.file);
                samplers_failed.push(sampler.file.clone());
            }
        }
    }

    // FL effects with a built-in counterpart: a fresh one, set as FL had
    // it, saved as the slot's state.
    let mut effects_failed = 0usize;
    for effect in &built.native_effects {
        match native_state(effect) {
            Some(state) => project.set_plugin_state(effect.slot_id.clone(), "native", state),
            None => effects_failed += 1,
        }
    }

    // Other people's plug-ins: found among the ones on this computer by the
    // ids the build expects, and given FL's settings. One that is not here
    // keeps its slot and settings, so it loads once it is installed (the
    // missing plug-ins banner says so); a track it played stays muted.
    let available: std::collections::HashSet<String> = {
        use tauri::Manager;
        let state = app.state::<crate::AppState>();
        let engine = state.engine.lock();
        let scanner = engine.plugin_scanner.lock();
        scanner.plugins().iter().map(|p| p.id.clone()).collect()
    };
    let not_here = carry_plugins(&mut project, &built.external_plugins, &available);

    let fixups = built.audio_fixups;
    crate::commands::project::open_project_blocking(app, project, None, move |song, pool| {
        apply_fixups(song, pool, &fixups)
    })?;
    log::info!(
        "import_flp: {name}: {} instrument and {} audio tracks, {} audio and {} pattern clips, {} samples missing",
        built.report.instrument_tracks,
        built.report.audio_tracks,
        built.report.audio_clips,
        built.report.pattern_clips,
        built.report.missing_samples.len()
    );

    let r = built.report;
    let mut left_behind = Vec::new();
    if !r.missing_samples.is_empty() {
        let names: Vec<String> = r
            .missing_samples
            .iter()
            .take(5)
            .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p).to_string())
            .collect();
        left_behind.push(format!(
            "{} sample{} not found on this computer ({}{}); their clips are in place and can be relinked",
            r.missing_samples.len(),
            if r.missing_samples.len() == 1 { " was" } else { "s were" },
            names.join(", "),
            if r.missing_samples.len() > 5 { ", ..." } else { "" }
        ));
    }
    if !samplers_failed.is_empty() {
        left_behind.push(format!(
            "{} sampler sample{} could not be read",
            samplers_failed.len(),
            if samplers_failed.len() == 1 { "" } else { "s" }
        ));
    }
    if !not_here.is_empty() {
        let names: Vec<String> = not_here
            .iter()
            .map(|(name, n)| {
                if *n == 1 {
                    name.clone()
                } else {
                    format!("{name} ({n}x)")
                }
            })
            .collect();
        left_behind.push(format!(
            "plug-ins that are not on this computer: {}. Their slots and settings are in the song and load once they are installed",
            list_of(&names)
        ));
    }
    if !r.plugins_without_settings.is_empty() {
        left_behind.push(format!(
            "the settings of {}: switched off until you set them",
            list_of(&r.plugins_without_settings)
        ));
    }
    if !r.plugins_left_behind.is_empty() {
        left_behind.push(format!(
            "plug-ins on {}: their notes are here on muted tracks; put your own plug-in on one and unmute it",
            r.plugins_left_behind.join(", ")
        ));
    }
    if !r.sample_fades_left_behind.is_empty() {
        left_behind.push(format!(
            "the quick fades on {}",
            list_of(&r.sample_fades_left_behind)
        ));
    }
    if !r.mixed_lanes.is_empty() {
        left_behind.push(format!(
            "the routing of {}, whose samples play into different mixer inserts; it plays into the one most of them use",
            list_of(&r.mixed_lanes)
        ));
    }
    if !r.effects_left_behind.is_empty() {
        left_behind.push(format!(
            "{} mixer effect{} with nothing built in to stand in for {} ({})",
            r.effects_left_behind.len(),
            if r.effects_left_behind.len() == 1 {
                ""
            } else {
                "s"
            },
            if r.effects_left_behind.len() == 1 {
                "it"
            } else {
                "them"
            },
            list_of(&r.effects_left_behind)
        ));
    }
    if effects_failed > 0 {
        left_behind.push(format!(
            "{effects_failed} EQ setting{} that could not be made",
            if effects_failed == 1 { "" } else { "s" }
        ));
    }
    if !r.silent_inserts.is_empty() {
        left_behind.push(format!(
            "{}, which sent nothing on in FL (a sidechain only, or nowhere): muted here",
            list_of(&r.silent_inserts)
        ));
    }
    if !r.route_levels_left_behind.is_empty() {
        left_behind.push(format!(
            "the send level of the main route on {}, which plays at full level here",
            list_of(&r.route_levels_left_behind)
        ));
    }
    if !r.automation_left_behind.is_empty() {
        left_behind.push(format!(
            "automation of {}, which moves something that has no counterpart here",
            list_of(&r.automation_left_behind)
        ));
    }
    if r.automation_curves_straightened {
        left_behind
            .push("the curves in automation clips: they run straight between their points".into());
    }

    Ok(FlpImportReport {
        bpm: r.bpm,
        tracks: r.instrument_tracks + r.audio_tracks,
        clips: r.audio_clips + r.pattern_clips,
        notes: r.notes,
        samples: r.missing_samples,
        left_behind,
    })
}

/// Put other people's plug-ins from FL in their slots: the id they have on
/// this computer, and FL's settings. Returns the ones not here, by name, with
/// how many slots each. A track one of those plays stays muted.
fn carry_plugins(
    project: &mut hardwave_project::Project,
    plugins: &[hardwave_project::fl_build::ExternalPlugin],
    available: &std::collections::HashSet<String>,
) -> std::collections::BTreeMap<String, usize> {
    let mut not_here: std::collections::BTreeMap<String, usize> = Default::default();
    for external in plugins {
        let found = external
            .candidates
            .iter()
            .find(|id| available.contains(*id))
            .cloned();
        if let Some(track) = project.track_mut(&external.track_id) {
            match &found {
                Some(id) => {
                    if let Some(slot) = track.inserts.iter_mut().find(|s| s.id == external.slot_id)
                    {
                        slot.plugin_id = id.clone();
                    }
                }
                None if external.plays_track => track.muted = true,
                None => {}
            }
        }
        if found.is_none() {
            *not_here.entry(external.plugin.name.clone()).or_default() += 1;
        }
        if !external.plugin.state.is_empty() {
            project.set_plugin_state(
                external.slot_id.clone(),
                "vst3",
                external.plugin.state.clone(),
            );
        }
    }
    not_here
}

/// The state of a built-in effect set up as an FL one was.
fn native_state(effect: &hardwave_project::fl_build::NativeEffect) -> Option<Vec<u8>> {
    let descriptor = hardwave_native_plugins::native_plugin_descriptors()
        .into_iter()
        .find(|d| d.id == effect.plugin_id)?;
    let mut plugin = crate::commands::plugins::instantiate_plugin(&descriptor).ok()?;
    for (id, value) in &effect.params {
        plugin.set_parameter_value(*id, *value);
    }
    Some(plugin.get_state())
}

/// "a, b, c" for up to five names, then "and n more".
fn list_of(names: &[String]) -> String {
    let shown = names.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 5 {
        format!("{shown} and {} more", names.len() - 5)
    } else {
        shown
    }
}

/// Finish each audio clip now that its file is loaded: where it starts in
/// the file, how far it is stretched and where a reversed one reads from
/// all depend on the file's length (fl_build.rs, `AudioFixup`).
fn apply_fixups(
    project: &mut hardwave_project::Project,
    pool: &hardwave_engine::audio_pool::AudioPool,
    fixups: &[hardwave_project::fl_build::AudioFixup],
) {
    for fixup in fixups {
        let Some(track) = project.track_mut(&fixup.track_id) else {
            continue;
        };
        for placement in &mut track.clips {
            let ClipContent::Audio(clip) = &mut placement.content else {
                continue;
            };
            if clip.id != fixup.clip_id {
                continue;
            }
            let Some(buffer) = pool.get(&clip.source_path) else {
                continue;
            };
            fixup.apply(clip, buffer.num_frames as u64, buffer.sample_rate);
        }
    }
}

/// Finds the samples an FL project names on this computer. FL writes
/// paths as they were on the machine that saved it, often through its own
/// variables (%FLStudioFactoryData% for the samples it ships with). A
/// project handed to someone else usually has its samples beside it, so
/// a file of the same name in the project's folder counts too.
struct SampleFinder {
    by_name: HashMap<String, PathBuf>,
}

impl SampleFinder {
    /// Index the audio files in `dir`, three folders deep at most.
    fn new(dir: Option<&Path>) -> Self {
        let mut by_name = HashMap::new();
        if let Some(dir) = dir {
            let mut stack = vec![(dir.to_path_buf(), 0usize)];
            let mut seen = 0usize;
            while let Some((folder, depth)) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&folder) else {
                    continue;
                };
                for entry in entries.flatten() {
                    seen += 1;
                    if seen > 20_000 {
                        return Self { by_name };
                    }
                    let path = entry.path();
                    if path.is_dir() {
                        if depth < 3 {
                            stack.push((path, depth + 1));
                        }
                    } else if is_audio(&path) {
                        if let Some(n) = path.file_name() {
                            by_name
                                .entry(n.to_string_lossy().to_lowercase())
                                .or_insert(path);
                        }
                    }
                }
            }
        }
        Self { by_name }
    }

    fn find(&self, fl_path: &str) -> Option<String> {
        for candidate in expand_fl_path(fl_path) {
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        let name = fl_path.rsplit(['\\', '/']).next()?.to_lowercase();
        self.by_name
            .get(&name)
            .map(|p| p.to_string_lossy().into_owned())
    }
}

fn is_audio(path: &Path) -> bool {
    matches!(
        path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .as_deref(),
        Some("wav" | "mp3" | "flac" | "ogg" | "aif" | "aiff")
    )
}

/// The places a path from an FL project could mean here: FL's own
/// variables for its install and user folders, and any %VARIABLE% from
/// the environment.
fn expand_fl_path(fl_path: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let lower = fl_path.to_lowercase();
    let tail_after = |var: &str| {
        fl_path[var.len()..]
            .trim_start_matches(['\\', '/'])
            .to_string()
    };
    if lower.starts_with("%flstudiofactorydata%") {
        let tail = tail_after("%FLStudioFactoryData%");
        for root in fl_install_dirs() {
            out.push(root.join(&tail));
        }
    } else if lower.starts_with("%flstudiouserdata%") {
        let tail = tail_after("%FLStudioUserData%");
        for root in fl_user_data_dirs() {
            out.push(root.join(&tail));
        }
    } else {
        out.push(PathBuf::from(expand_env(fl_path)));
    }
    out
}

/// FL's install folders, newest first.
fn fl_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        let Ok(base) = std::env::var(var) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(Path::new(&base).join("Image-Line")) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("FL Studio") {
                dirs.push(entry.path());
            }
        }
    }
    dirs.sort();
    dirs.reverse();
    dirs
}

/// Where FL keeps the user's own data.
fn fl_user_data_dirs() -> Vec<PathBuf> {
    let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    ["Documents", "OneDrive\\Documents", "OneDrive\\Documenten"]
        .iter()
        .map(|docs| home.join(docs).join("Image-Line").join("FL Studio"))
        .collect()
}

/// Replace each %NAME% with that environment variable, when it is set.
fn expand_env(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(len) = after.find('%') else { break };
        let var = &after[..len];
        out.push_str(&rest[..start]);
        match std::env::var(var) {
            Ok(value) if !var.is_empty() => out.push_str(&value),
            _ => {
                out.push('%');
                out.push_str(var);
                out.push('%');
            }
        }
        rest = &after[len + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_beside_the_project_is_found_by_its_name() {
        let dir = std::env::temp_dir().join(format!("hw-flp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("Samples")).unwrap();
        std::fs::write(dir.join("Samples").join("Kick 16.wav"), b"RIFF").unwrap();
        let finder = SampleFinder::new(Some(&dir));
        let found = finder
            .find("C:\\Users\\someone else\\Packs\\kick 16.WAV")
            .expect("found by name");
        assert!(found.ends_with("Kick 16.wav"));
        assert!(finder.find("C:\\nowhere\\other.wav").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_fl_eq_s_settings_reach_the_built_in_eq() {
        use hardwave_native_plugins::NativeEq;
        use hardwave_plugin_host::types::HostedPlugin;
        use hardwave_project::fl_import::{FlMixerSlot, FlMixerTrack, FlProject};

        assert_eq!(hardwave_project::fl_build::NATIVE_EQ, NativeEq::ID);
        // A Parametric EQ 2: version 8, band 4 up 6 dB, every band a bell
        // at 632 Hz, the main level down 3 dB.
        let mut words = vec![0i32; 88];
        words[0] = 8;
        for band in 0..7 {
            words[8 + band] = 32_768;
            words[15 + band] = 25_412;
            words[22 + band] = 6;
        }
        words[4] = 600;
        words[36] = -300;
        let fl = FlProject {
            mixer: vec![FlMixerTrack {
                slots: vec![FlMixerSlot {
                    index: 0,
                    plugin: "Fruity Parametric EQ 2".into(),
                    name: None,
                    enabled: true,
                    mix: 12_800,
                    data: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let built = hardwave_project::fl_build::build_project(&fl, "eq", &|_| None);
        let state = native_state(&built.native_effects[0]).expect("an EQ is made");
        let mut eq = NativeEq::new();
        eq.set_state(&state).expect("its own state reads back");
        assert_eq!(eq.get_parameter_value(12), 1.0, "band 4 on");
        assert_eq!(eq.get_parameter_value(14), 6.0);
        assert!((eq.get_parameter_value(13) - 632.46).abs() < 0.01);
        assert_eq!(eq.get_parameter_value(4), 0.0, "band 2 at 0 dB stays off");
        assert_eq!(eq.get_parameter_value(28), -3.0);
    }

    #[test]
    fn plug_ins_on_this_computer_get_their_settings_and_the_rest_are_named() {
        use hardwave_project::fl_build::ExternalPlugin;
        use hardwave_project::fl_import::{FlPluginFormat, FlWrappedPlugin};
        use hardwave_project::track::PluginSlot;

        let mut project = hardwave_project::Project::default();
        let plugin = |name: &str, state: &[u8]| FlWrappedPlugin {
            name: name.into(),
            vendor: String::new(),
            format: FlPluginFormat::Vst3,
            instrument: false,
            file: String::new(),
            clap_id: None,
            state: state.to_vec(),
            param_ids: Vec::new(),
        };
        let mut plugins = Vec::new();
        for (track, slot, name, candidates, plays) in [
            ("insert-001", "s1", "Serum2", vec!["vst3:serum2"], true),
            ("insert-002", "s2", "Sylenth1", vec!["vst3:sylenth1"], true),
            (
                "insert-003",
                "s3",
                "FabFilter Pro-Q 4",
                vec!["vst3:x", "vst3:pro-q-4"],
                false,
            ),
            ("insert-003", "s4", "OTT", vec!["vst3:ott"], false),
        ] {
            project.track_mut(track).unwrap().inserts.push(PluginSlot {
                id: slot.into(),
                plugin_id: candidates[0].into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
            plugins.push(ExternalPlugin {
                track_id: track.into(),
                slot_id: slot.into(),
                place: name.into(),
                plugin: plugin(name, name.as_bytes()),
                candidates: candidates.into_iter().map(String::from).collect(),
                plays_track: plays,
            });
        }
        let available: std::collections::HashSet<String> = ["vst3:serum2", "vst3:pro-q-4"]
            .into_iter()
            .map(String::from)
            .collect();
        let not_here = carry_plugins(&mut project, &plugins, &available);

        assert!(!project.track("insert-001").unwrap().muted);
        assert!(
            project.track("insert-002").unwrap().muted,
            "its synth is not here"
        );
        let effects = &project.track("insert-003").unwrap().inserts;
        assert_eq!(
            effects[0].plugin_id, "vst3:pro-q-4",
            "found by its second id"
        );
        assert_eq!(
            effects[1].plugin_id, "vst3:ott",
            "kept, to load once installed"
        );
        assert_eq!(project.plugin_state("s1").unwrap().chunk, b"Serum2");
        assert_eq!(project.plugin_state("s2").unwrap().chunk, b"Sylenth1");
        assert_eq!(
            not_here.into_iter().collect::<Vec<_>>(),
            vec![("OTT".to_string(), 1), ("Sylenth1".to_string(), 1)]
        );
    }

    #[test]
    fn environment_variables_in_a_path_are_filled_in() {
        std::env::set_var("HW_FLP_TEST_ROOT", "/tmp/root");
        assert_eq!(
            expand_env("%HW_FLP_TEST_ROOT%/kick.wav"),
            "/tmp/root/kick.wav"
        );
        assert_eq!(
            expand_env("%HW_NOT_SET_ANYWHERE%/kick.wav"),
            "%HW_NOT_SET_ANYWHERE%/kick.wav"
        );
        assert_eq!(expand_env("no variables"), "no variables");
    }
}
