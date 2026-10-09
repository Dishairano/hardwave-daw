//! Opening an FL Studio project.
//!
//! Most of the people this DAW is for work in FL. "Can I open my
//! projects" is the first question any of them ask, and until now the
//! answer was no, with a stub module describing what an answer would
//! look like.
//!
//! What comes across is the song: the tempo, an instrument track per
//! channel that plays notes (a sampler channel with its sample in the
//! built-in sampler), every sample on the playlist where FL had it, and
//! the playlist's track names. It opens as a song of its own, like a
//! .hwp. What cannot come across is everything a plug-in holds: FL
//! stores a plug-in's state as its own blob, and nothing outside FL can
//! mean anything by it. The report says what was left behind rather
//! than letting it be found later.

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
        if let hardwave_project::fl_import::FlClipContent::Pattern { pattern_index } = &clip.content
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
    // it opens.
    let mut samplers_failed = Vec::new();
    for (track_id, sample) in &built.sampler_samples {
        match hardwave_dsp::audio_file::AudioFileReader::read(Path::new(sample)) {
            Ok((info, channels)) => {
                let state = hardwave_native_plugins::sampler::encode_sample_state(
                    info.sample_rate,
                    60,
                    channels,
                );
                let slot_id = uuid::Uuid::new_v4().to_string();
                if let Some(track) = project.track_mut(track_id) {
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
                log::warn!("import_flp: sampler sample {sample}: {e:?}");
                samplers_failed.push(sample.clone());
            }
        }
    }

    let offsets = built.audio_offsets;
    crate::commands::project::open_project_blocking(app, project, None, move |song, pool| {
        apply_offsets(song, pool, &offsets)
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
    if !r.plugins_left_behind.is_empty() {
        left_behind.push(format!(
            "plug-ins on {}: their notes are here, add your own plug-in on those tracks",
            r.plugins_left_behind.join(", ")
        ));
    }
    if r.automation_clips > 0 {
        left_behind.push(format!("{} automation clips", r.automation_clips));
    }
    left_behind.push("the mixer: effects, routing and levels".into());

    Ok(FlpImportReport {
        bpm: r.bpm,
        tracks: r.instrument_tracks + r.audio_tracks,
        clips: r.audio_clips + r.pattern_clips,
        notes: r.notes,
        samples: r.missing_samples,
        left_behind,
    })
}

/// Where each trimmed clip starts and ends in its file. FL gives
/// milliseconds; a clip counts in the pool's frames, at the rate the
/// file was loaded at.
fn apply_offsets(
    project: &mut hardwave_project::Project,
    pool: &hardwave_engine::audio_pool::AudioPool,
    offsets: &[(String, String, f32, f32)],
) {
    for (track_id, clip_id, start_ms, end_ms) in offsets {
        let Some(track) = project.track_mut(track_id) else {
            continue;
        };
        for placement in &mut track.clips {
            let ClipContent::Audio(clip) = &mut placement.content else {
                continue;
            };
            if clip.id != *clip_id {
                continue;
            }
            let Some(buffer) = pool.get(&clip.source_path) else {
                continue;
            };
            let frames = buffer.num_frames as u64;
            let at = |ms: f32| ((ms as f64) * buffer.sample_rate as f64 / 1000.0).round() as u64;
            if *start_ms > 0.0 {
                clip.source_start = at(*start_ms).min(frames);
            }
            clip.source_end = if *end_ms > 0.0 {
                at(*end_ms).min(frames)
            } else {
                frames
            };
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
