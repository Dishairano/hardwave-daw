//! Opening an FL Studio project.
//!
//! Most of the people this DAW is for work in FL. "Can I open my
//! projects" is the first question any of them ask, and until now the
//! answer was no, with a stub module describing what an answer would
//! look like.
//!
//! What comes across is the song, in FL's shape: the tempo; every channel
//! of FL's channel rack as a channel, in FL's order and filter groups,
//! with its level, pan and the insert it plays into (a sampler channel
//! with its sample in the built-in sampler, other people's VST3
//! instruments with their settings); every sample on the playlist on its
//! channel and on its FL row, at its pitch, direction and stretch; FL's
//! rows with their names, colours and switches, per arrangement; the
//! mixer's inserts the song uses, numbered and named as in FL, with their
//! fader, pan, switches, routes and effects (FL's Parametric EQ 2 as the
//! built-in EQ, other people's VST3 effects with their settings); and the
//! automation clips on their own channels, moving any of those. It opens
//! as a song of its own, like a .hwp. What cannot come across is what FL's
//! own instruments and most of its effects hold, a VST2's or CLAP's
//! settings, and automation of knobs with no counterpart here. Pattern
//! clips come as a clip per channel on the pattern clip's row. The report
//! says what was left behind rather than letting it be found later.
//!
//! FL's zipped loop package, the .zip it makes to hand a project to
//! someone else, opens the same way once it is unpacked (flp_package.rs).

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
    /// The song's name: the .flp's, also when it came in a zip.
    pub name: String,
    /// Where a zipped package was unpacked; its samples stay there.
    pub package_folder: Option<String>,
}

/// Open a `.flp`, or FL's zipped loop package (a .zip holding the .flp
/// and its samples), as a new song.
#[tauri::command]
pub async fn import_flp(app: AppHandle, path: String) -> Result<FlpImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_flp_blocking(&app, &path))
        .await
        .map_err(|e| format!("opening stopped: {e}"))?
}

fn import_flp_blocking(app: &AppHandle, path: &str) -> Result<FlpImportReport, String> {
    let path = Path::new(path);
    if !super::flp_package::is_zip(path) {
        let (name, built) = build_from_flp(path, &SampleFinder::new(path.parent()))?;
        return open_built(app, name, built);
    }
    use tauri::Emitter;
    let package =
        super::flp_package::unpack(path, super::flp_package::LIMITS, &mut |done, total| {
            let _ = app.emit(
                "project-load-progress",
                super::project::LoadProgress {
                    stage: "unpack",
                    done,
                    total,
                },
            );
        })?;
    let finder = SampleFinder::in_package(&package.samples);
    let (name, built) = match build_from_flp(&package.projects[0], &finder) {
        Ok(built) => built,
        Err(e) => {
            // No song plays what was unpacked, so it does not stay.
            let _ = std::fs::remove_dir_all(&package.folder);
            return Err(e);
        }
    };
    let mut report = open_built(app, name, built)?;
    report.package_folder = Some(package.folder.to_string_lossy().into_owned());
    let file_name = |p: &PathBuf| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    if package.projects.len() > 1 {
        let others: Vec<String> = package.projects[1..].iter().map(file_name).collect();
        report.left_behind.push(format!(
            "the zip's other FL project{} ({}), unpacked beside this one; import {} on {} own",
            if others.len() == 1 { "" } else { "s" },
            list_of(&others),
            if others.len() == 1 { "it" } else { "each" },
            if others.len() == 1 { "its" } else { "their" },
        ));
    }
    if !package.skipped.is_empty() {
        let names: Vec<String> = package
            .skipped
            .iter()
            .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n).to_string())
            .collect();
        report.left_behind.push(format!(
            "the zip's other files, which are neither the project nor a sample, left in the zip: {}",
            list_of(&names)
        ));
    }
    Ok(report)
}

/// Read an FL project and make the song it holds, with its samples found
/// by `finder`. Nothing is opened yet.
fn build_from_flp(
    file: &Path,
    finder: &SampleFinder,
) -> Result<(String, hardwave_project::fl_build::FlBuild), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("could not read that file: {e}"))?;
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
        if let hardwave_project::fl_import::FlClipContent::Pattern { pattern_index, .. } =
            &clip.content
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

    let name = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "FL project".into());
    let built = hardwave_project::fl_build::build_project(&fl, &name, &|p: &str| finder.find(p));
    Ok((name, built))
}

/// Open the song made from an FL project.
fn open_built(
    app: &AppHandle,
    name: String,
    built: hardwave_project::fl_build::FlBuild,
) -> Result<FlpImportReport, String> {
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
        "import_flp: {name}: {} channels, {} inserts, {} rows, {} audio, {} pattern and {} automation clips, {} samples missing",
        built.report.channels,
        built.report.mixer_inserts,
        built.report.rows,
        built.report.audio_clips,
        built.report.pattern_clips,
        built.report.automation_clips + built.report.automation_clips_moving_nothing,
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
            "plug-ins on {}: their channels and notes are here, muted; put your own plug-in on one and unmute it",
            r.plugins_left_behind.join(", ")
        ));
    }
    if r.pattern_placements > 0 {
        left_behind.push(format!(
            "pattern clips as one clip: each of the {} pattern clip{} is here as a clip per channel on its row",
            r.pattern_placements,
            if r.pattern_placements == 1 { "" } else { "s" }
        ));
    }
    if !r.sample_fades_left_behind.is_empty() {
        left_behind.push(format!(
            "the quick fades on {}",
            list_of(&r.sample_fades_left_behind)
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
            "automation of {}, which moves something that has no counterpart here; its clips are on their rows, moving nothing",
            list_of(&r.automation_left_behind)
        ));
    }
    if r.automation_curves_straightened {
        left_behind
            .push("the curves in automation clips: they run straight between their points".into());
    }

    Ok(FlpImportReport {
        bpm: r.bpm,
        tracks: r.channels + r.mixer_inserts,
        clips: r.audio_clips
            + r.pattern_clips
            + r.automation_clips
            + r.automation_clips_moving_nothing,
        notes: r.notes,
        samples: r.missing_samples,
        left_behind,
        name,
        package_folder: None,
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
        // On the track, or in one of FL's other arrangements.
        let stored = project
            .arrangements
            .iter_mut()
            .filter_map(|a| a.timelines.get_mut(&fixup.track_id))
            .flat_map(|t| t.clips.iter_mut());
        let live = project
            .tracks
            .iter_mut()
            .filter(|t| t.id == fixup.track_id)
            .flat_map(|t| t.clips.iter_mut());
        for placement in live.chain(stored) {
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
    /// The files by name come first: a zipped package carries the very
    /// samples the song was saved with.
    package_first: bool,
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
                        return Self {
                            by_name,
                            package_first: false,
                        };
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
        Self {
            by_name,
            package_first: false,
        }
    }

    /// The samples unpacked from a zipped package, found by their names
    /// before anywhere else: FL's package keeps each one under its own
    /// name, side by side, while the project still names the folders they
    /// came from on the computer that made it, often FL's own temp folder.
    /// On that computer those can still be there for a while, and a song
    /// pointing at them would lose its samples when Windows clears it.
    fn in_package(samples: &[PathBuf]) -> Self {
        let mut by_name = HashMap::new();
        for path in samples {
            if let Some(n) = path.file_name() {
                by_name
                    .entry(n.to_string_lossy().to_lowercase())
                    .or_insert_with(|| path.clone());
            }
        }
        Self {
            by_name,
            package_first: true,
        }
    }

    fn find(&self, fl_path: &str) -> Option<String> {
        let by_name = || {
            let name = fl_path.rsplit(['\\', '/']).next()?.to_lowercase();
            self.by_name
                .get(&name)
                .map(|p| p.to_string_lossy().into_owned())
        };
        if self.package_first {
            if let Some(found) = by_name() {
                return Some(found);
            }
        }
        for candidate in expand_fl_path(fl_path) {
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        by_name()
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

    /// A tiny .flp written the way FL writes one: an audio clip channel
    /// playing `sample` (named as FL names it), placed once on the
    /// playlist's first track.
    fn flp_playing(sample: &str) -> Vec<u8> {
        fn event(body: &mut Vec<u8>, id: u8, data: &[u8]) {
            body.push(id);
            // Events from 192 on carry their length first.
            if id >= 192 {
                let mut len = data.len();
                loop {
                    let byte = (len & 0x7F) as u8;
                    len >>= 7;
                    body.push(if len > 0 { byte | 0x80 } else { byte });
                    if len == 0 {
                        break;
                    }
                }
            }
            body.extend_from_slice(data);
        }
        let mut body = Vec::new();
        event(&mut body, 64, &0u16.to_le_bytes()); // a new channel, 0
        event(&mut body, 21, &[4]); // that plays an audio clip
        let mut text: Vec<u8> = sample
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        text.extend_from_slice(&[0, 0]);
        event(&mut body, 196, &text); // its sample
        let mut item = Vec::new();
        item.extend_from_slice(&0u32.to_le_bytes()); // at the start
        item.extend_from_slice(&20480u16.to_le_bytes());
        item.extend_from_slice(&0u16.to_le_bytes()); // channel 0
        item.extend_from_slice(&384u32.to_le_bytes()); // a bar long
        item.extend_from_slice(&499u16.to_le_bytes()); // playlist track 1
        item.resize(24, 0);
        item.extend_from_slice(&(-1.0f32).to_le_bytes());
        item.extend_from_slice(&(-1.0f32).to_le_bytes());
        item.resize(32, 0);
        event(&mut body, 233, &item); // the playlist
        let mut out = b"FLhd".to_vec();
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // format
        out.extend_from_slice(&1u16.to_le_bytes()); // channels
        out.extend_from_slice(&96u16.to_le_bytes()); // ticks per beat
        out.extend_from_slice(b"FLdt");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn tiny_wav() -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut bytes, spec).unwrap();
        for i in 0..441 {
            w.write_sample(((i % 50) * 400) as i16).unwrap();
        }
        w.finalize().unwrap();
        bytes.into_inner()
    }

    /// FL's zipped loop package: the .flp and its samples side by side,
    /// while the .flp still names where the samples were on the computer
    /// that made it. The song comes out playing the unpacked copy.
    #[test]
    fn a_zipped_package_imports_with_its_sample_found_in_it() {
        use super::super::flp_package::{self, tests::make_zip, tests::temp, LIMITS};
        let dir = temp("import");
        let zip = dir.join("project.zip");
        let flp = flp_playing("C:\\Users\\producer\\Documents\\Packs\\Drums\\Kick 01.wav");
        let wav = tiny_wav();
        make_zip(&zip, &[("Shared Song.flp", &flp), ("Kick 01.wav", &wav)]);

        let package = flp_package::unpack(&zip, LIMITS, &mut |_, _| {}).unwrap();
        let finder = SampleFinder::in_package(&package.samples);
        let (name, built) = build_from_flp(&package.projects[0], &finder).unwrap();
        assert_eq!(name, "Shared Song");
        assert_eq!(built.report.audio_clips, 1);
        assert!(
            built.report.missing_samples.is_empty(),
            "missing: {:?}",
            built.report.missing_samples
        );
        let played: Vec<PathBuf> = built
            .project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match &c.content {
                ClipContent::Audio(a) => Some(PathBuf::from(&a.source_path)),
                _ => None,
            })
            .collect();
        assert_eq!(played, vec![package.folder.join("Kick 01.wav")]);
        assert!(played[0].is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A package's own copy wins over a file at the path the project
    /// names: the package holds what the song was saved with.
    #[test]
    fn a_package_s_own_sample_comes_before_the_path_the_project_names() {
        let dir = std::env::temp_dir().join(format!("hw-flp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        std::fs::create_dir_all(dir.join("package")).unwrap();
        let named = dir.join("elsewhere").join("Kick.wav");
        let packed = dir.join("package").join("kick.WAV");
        std::fs::write(&named, b"RIFF").unwrap();
        std::fs::write(&packed, b"RIFF").unwrap();
        let fl_path = named.to_string_lossy().into_owned();

        let in_package = SampleFinder::in_package(std::slice::from_ref(&packed));
        assert_eq!(
            in_package.find(&fl_path).map(PathBuf::from),
            Some(packed.clone())
        );
        // A bare .flp still goes by the path first.
        let beside = SampleFinder::new(Some(&dir.join("package")));
        assert_eq!(beside.find(&fl_path).map(PathBuf::from), Some(named));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The founder's FL 26 project at 130 BPM, cut down to one clip: a
    /// 126 BPM loop of 7.62 s (a 24-bit WAV at 44.1 kHz) whose TIME knob
    /// fits it to 16 beats, placed for one bar from half way into it.
    /// Through the command's own steps: the .flp read with its sample
    /// beside it, the sample loaded as a song's open loads it, then each
    /// clip finished from what was loaded. A 24-bit WAV loaded as no audio
    /// at all, so the clip kept no stretch and started at the file's start,
    /// and played silence.
    #[test]
    fn a_stretched_24_bit_loop_is_finished_from_the_sample_the_open_loads() {
        fn event(body: &mut Vec<u8>, id: u8, data: &[u8]) {
            body.push(id);
            if id >= 192 {
                let mut len = data.len();
                loop {
                    let byte = (len & 0x7F) as u8;
                    len >>= 7;
                    body.push(if len > 0 { byte | 0x80 } else { byte });
                    if len == 0 {
                        break;
                    }
                }
            }
            body.extend_from_slice(data);
        }
        let dir = std::env::temp_dir().join(format!("hw-flp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(dir.join("126_RvbBeat_01_663.wav"), spec).unwrap();
        for i in 0..336_000i32 {
            let v = ((i % 400) - 200) * 20_000;
            w.write_sample(v).unwrap();
            w.write_sample(-v).unwrap();
        }
        w.finalize().unwrap();

        let mut body = Vec::new();
        event(&mut body, 156, &130_000u32.to_le_bytes()); // 130 BPM
        event(&mut body, 64, &0u16.to_le_bytes());
        event(&mut body, 21, &[4]); // an audio clip channel
        let mut params = vec![0u8; 168];
        // TIME 16 beats at FL's 192 a beat, the Auto stretch mode: the
        // bytes as the founder's project has them.
        params[96..112].copy_from_slice(&[
            0x00, 0x00, 0x40, 0x45, 0, 0, 0, 0, 0, 0, 0, 0, 0x05, 0, 0, 0,
        ]);
        event(&mut body, 215, &params);
        let sample =
            "C:\\Users\\DISHAI~1\\AppData\\Local\\Temp\\Image-Line\\126_RvbBeat_01_663.wav";
        let mut text: Vec<u8> = sample
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        text.extend_from_slice(&[0, 0]);
        event(&mut body, 196, &text);
        let mut item = Vec::new();
        item.extend_from_slice(&10_752u32.to_le_bytes());
        item.extend_from_slice(&20480u16.to_le_bytes());
        item.extend_from_slice(&0u16.to_le_bytes()); // channel 0
        item.extend_from_slice(&384u32.to_le_bytes()); // a bar at 96 a beat
        item.extend_from_slice(&499u16.to_le_bytes()); // playlist track 1
        item.resize(24, 0);
        item.extend_from_slice(&3_809.523_7f32.to_le_bytes()); // FL's start, ms
        item.extend_from_slice(&5_714.274f32.to_le_bytes()); // FL's end, ms
        event(&mut body, 233, &item);
        let mut flp = b"FLhd".to_vec();
        flp.extend_from_slice(&6u32.to_le_bytes());
        flp.extend_from_slice(&0u16.to_le_bytes());
        flp.extend_from_slice(&1u16.to_le_bytes());
        flp.extend_from_slice(&96u16.to_le_bytes());
        flp.extend_from_slice(b"FLdt");
        flp.extend_from_slice(&(body.len() as u32).to_le_bytes());
        flp.extend_from_slice(&body);
        let flp_path = dir.join("RyZe Take U TIme.flp");
        std::fs::write(&flp_path, &flp).unwrap();

        let (_, built) = build_from_flp(&flp_path, &SampleFinder::new(Some(&dir))).unwrap();
        let engine = hardwave_engine::DawEngine::new();
        let missing = engine
            .audio_source_loader(None)
            .load_sources_of(&built.project, |_, _| {});
        assert!(missing.is_empty(), "{missing:?}");
        let mut project = built.project;
        apply_fixups(&mut project, &engine.audio_pool, &built.audio_fixups);

        let clip = project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .find_map(|c| match &c.content {
                ClipContent::Audio(a) => Some(a.clone()),
                _ => None,
            })
            .expect("the clip");
        let frames = engine
            .audio_pool
            .get(&clip.source_path)
            .map(|b| b.num_frames as f64)
            .unwrap_or(0.0);
        assert!(frames > 0.0, "the sample's audio is loaded");
        assert!(
            (clip.stretch_ratio - 0.969).abs() < 0.001,
            "16 beats at 130 BPM from 7.62 s: {}",
            clip.stretch_ratio
        );
        let at = |f: u64| f as f64 / frames;
        assert!(
            (at(clip.source_start) - 0.5).abs() < 1e-4,
            "starts half way, as FL: {}",
            at(clip.source_start)
        );
        assert!(
            (at(clip.source_end) - 0.75).abs() < 1e-4,
            "ends three quarters in, at FL's own end: {}",
            at(clip.source_end)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

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
