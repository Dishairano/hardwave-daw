//! Finding audio files a project can no longer see, and pointing it at them
//! again.
//!
//! A clip stores two different strings: `source_path`, the id its audio has
//! in the engine's pool, and `source_file`, where that audio was read from.
//! Relinking only ever rewrites `source_file` and reloads the audio under the
//! unchanged id, so every clip that shares the sample follows along and none
//! of them has to be touched.

use crate::AppState;
use hardwave_project::clip::ClipContent;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;

/// Extensions worth considering when searching a folder for a lost sample.
const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "aiff", "aif", "ogg", "m4a"];

/// How deep a folder search descends. A sample library is nested, but an
/// unbounded walk of a whole drive would hang the search.
const MAX_SEARCH_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MissingSource {
    /// Pool id the clips reference. This is what relinking keeps stable.
    pub source_id: String,
    /// Last known location, which is what makes the name recognisable.
    pub file: String,
    /// Just the file name, for display.
    pub name: String,
    /// SHA-256 recorded at import, empty on older projects.
    pub hash: String,
    /// How many clips play this sample.
    pub clip_count: usize,
}

/// Every distinct audio source in the project, with where it was read from.
fn project_sources(state: &State<AppState>) -> BTreeMap<String, (String, String, usize)> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let mut sources: BTreeMap<String, (String, String, usize)> = BTreeMap::new();
    let mut collect = |content: &ClipContent| {
        if let ClipContent::Audio(ac) = content {
            let file = if ac.source_file.is_empty() {
                // Projects older than `source_file` kept the path here.
                ac.source_path.clone()
            } else {
                ac.source_file.clone()
            };
            let entry =
                sources
                    .entry(ac.source_path.clone())
                    .or_insert((file, ac.source_hash.clone(), 0));
            entry.2 += 1;
        }
    };
    for track in &project.tracks {
        for clip in &track.clips {
            collect(&clip.content);
        }
    }
    for arrangement in &project.arrangements {
        for timeline in arrangement.timelines.values() {
            for clip in &timeline.clips {
                collect(&clip.content);
            }
        }
    }
    sources
}

/// Audio the project references that is not on disk where it expects it.
#[tauri::command]
pub fn list_missing_sources(state: State<AppState>) -> Vec<MissingSource> {
    // Resolved, not raw: a collected project stores its samples relative to
    // the .hwp, and checking those strings directly would report every one of
    // them as missing.
    let resolve = |file: &str| state.engine.lock().resolve_source_file(file);
    project_sources(&state)
        .into_iter()
        .filter(|(_, (file, _, _))| !resolve(file).is_file())
        .map(|(source_id, (file, hash, clip_count))| MissingSource {
            name: Path::new(&file)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                // A project written before `source_file` has a bare pool id
                // here, which is a hash and means nothing to anyone reading
                // it. Say so rather than printing it as a file name.
                .unwrap_or_else(|| "unknown file".to_string()),
            source_id,
            file,
            hash,
            clip_count,
        })
        .collect()
}

/// Point one source at a file the user picked. Every clip using it follows,
/// because they reference the pool id, which does not change.
#[tauri::command]
pub fn relink_source(
    state: State<AppState>,
    source_id: String,
    new_path: String,
) -> Result<usize, String> {
    let path = PathBuf::from(&new_path);
    if !path.is_file() {
        return Err(format!("{new_path} is not a file"));
    }
    let engine = state.engine.lock();
    engine.load_audio_file_as(&path, &source_id)?;

    let mut repointed = 0usize;
    {
        let mut project = engine.project.lock();
        let mut repoint = |content: &mut ClipContent| {
            if let ClipContent::Audio(ac) = content {
                if ac.source_path == source_id {
                    ac.source_file = new_path.clone();
                    repointed += 1;
                }
            }
        };
        for track in &mut project.tracks {
            for clip in &mut track.clips {
                repoint(&mut clip.content);
            }
        }
        for arrangement in &mut project.arrangements {
            for timeline in arrangement.timelines.values_mut() {
                for clip in &mut timeline.clips {
                    repoint(&mut clip.content);
                }
            }
        }
    }
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(repointed)
}

/// Search `dirs` for the missing files and relink whatever is found.
/// Returns the file names that were recovered.
#[tauri::command]
pub fn auto_relink_sources(state: State<AppState>, dirs: Vec<String>) -> Vec<String> {
    let missing = list_missing_sources(state.clone());
    if missing.is_empty() {
        return Vec::new();
    }
    let index = index_audio_files(&dirs);

    let mut recovered = Vec::new();
    for source in missing {
        let Some(found) = pick_match(&index, &source) else {
            continue;
        };
        if relink_source(
            state.clone(),
            source.source_id.clone(),
            found.to_string_lossy().into_owned(),
        )
        .is_ok()
        {
            recovered.push(source.name.clone());
        }
    }
    recovered
}

/// File name (lowercased) to every path carrying that name.
fn index_audio_files(dirs: &[String]) -> BTreeMap<String, Vec<PathBuf>> {
    let mut index: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for dir in dirs {
        walk(Path::new(dir), 0, &mut index);
    }
    index
}

fn walk(dir: &Path, depth: usize, index: &mut BTreeMap<String, Vec<PathBuf>>) {
    if depth > MAX_SEARCH_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            walk(&path, depth + 1, index);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(ext)))
        {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                index.entry(name.to_lowercase()).or_default().push(path);
            }
        }
    }
}

/// Choose which candidate is the lost file.
///
/// Name alone is not proof: sample packs are full of files called kick.wav.
/// When the project recorded a hash, only a file whose contents match counts,
/// so a same-named different sample is never silently swapped in. Without a
/// hash (older projects) a unique name match is accepted, and an ambiguous one
/// is left for the user to resolve by hand.
fn pick_match<'a>(
    index: &'a BTreeMap<String, Vec<PathBuf>>,
    source: &MissingSource,
) -> Option<&'a Path> {
    let candidates = index.get(&source.name.to_lowercase())?;
    if !source.hash.is_empty() {
        return candidates
            .iter()
            .find(|c| hash_file(c) == source.hash)
            .map(|p| p.as_path());
    }
    match candidates.as_slice() {
        [only] => Some(only.as_path()),
        _ => None,
    }
}

fn hash_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut hasher = Sha256::new();
    if std::io::copy(&mut file, &mut hasher).is_err() {
        return String::new();
    }
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectResult {
    pub copied: usize,
    pub already_there: usize,
    pub missing: Vec<String>,
    pub folder: String,
}

/// Copy every sample the project uses into a folder beside the project file,
/// and point the project at those copies.
///
/// A project normally references samples wherever they happened to be when
/// they were imported: a pack on another drive, a download folder, a path that
/// only exists on one machine. Collecting makes the song self-contained, and
/// the copies are stored as paths relative to the .hwp, so the folder can be
/// moved, copied to another drive or zipped and still open.
#[tauri::command]
pub fn collect_project_samples(
    state: State<AppState>,
    project_path: String,
) -> Result<CollectResult, String> {
    let project_file = PathBuf::from(&project_path);
    let dir = project_file
        .parent()
        .ok_or_else(|| format!("{project_path} has no folder"))?
        .to_path_buf();
    let stem = project_file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Project".to_string());
    let folder_name = format!("{stem} Samples");
    let samples_dir = dir.join(&folder_name);
    std::fs::create_dir_all(&samples_dir)
        .map_err(|e| format!("Could not create {}: {e}", samples_dir.display()))?;

    let sources = project_sources(&state);
    let mut copied = 0usize;
    let mut already_there = 0usize;
    let mut missing = Vec::new();
    let mut relink: Vec<(String, String)> = Vec::new();
    let mut taken: Vec<String> = Vec::new();

    for (source_id, (file, _hash, _count)) in sources {
        let resolved = state.engine.lock().resolve_source_file(&file);
        if !resolved.is_file() {
            missing.push(
                Path::new(&file)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(file.clone()),
            );
            continue;
        }
        if resolved.starts_with(&samples_dir) {
            already_there += 1;
            // Still rewrite it: an older collect stored an absolute path,
            // which stops the folder being movable.
            if let Some(name) = resolved.file_name().and_then(|n| n.to_str()) {
                relink.push((source_id, format!("{folder_name}/{name}")));
            }
            continue;
        }
        let name = unique_name_in(&samples_dir, &resolved, &taken);
        std::fs::copy(&resolved, samples_dir.join(&name))
            .map_err(|e| format!("Could not copy {}: {e}", resolved.display()))?;
        taken.push(name.clone());
        copied += 1;
        relink.push((source_id, format!("{folder_name}/{name}")));
    }

    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let repoint = |content: &mut ClipContent| {
            if let ClipContent::Audio(ac) = content {
                if let Some((_, new_file)) = relink.iter().find(|(id, _)| *id == ac.source_path) {
                    ac.source_file = new_file.clone();
                }
            }
        };
        for track in &mut project.tracks {
            for clip in &mut track.clips {
                repoint(&mut clip.content);
            }
        }
        for arrangement in &mut project.arrangements {
            for timeline in arrangement.timelines.values_mut() {
                for clip in &mut timeline.clips {
                    repoint(&mut clip.content);
                }
            }
        }
    }
    state.engine.lock().set_project_dir(Some(dir));

    Ok(CollectResult {
        copied,
        already_there,
        missing,
        folder: folder_name,
    })
}

/// A name that does not collide with a file already in the folder or one
/// copied earlier in this run. Two different samples can easily both be
/// called kick.wav, and copying one over the other would silently replace
/// audio in the song.
fn unique_name_in(dir: &Path, source: &Path, taken: &[String]) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sample.wav".to_string());
    let is_free =
        |candidate: &str| !dir.join(candidate).exists() && !taken.iter().any(|t| t == candidate);
    if is_free(&name) {
        return name;
    }
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sample".to_string());
    let ext = source
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 2..10_000 {
        let candidate = format!("{stem} ({n}){ext}");
        if is_free(&candidate) {
            return candidate;
        }
    }
    format!("{stem} ({}){ext}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn missing(name: &str, hash: &str) -> MissingSource {
        MissingSource {
            source_id: "id".into(),
            file: format!("/gone/{name}"),
            name: name.into(),
            hash: hash.into(),
            clip_count: 1,
        }
    }

    #[test]
    fn a_collected_name_never_overwrites_a_different_sample() {
        let dir = tempfile::tempdir().unwrap();
        let samples = dir.path().join("Song Samples");
        std::fs::create_dir_all(&samples).unwrap();
        write(&samples.join("kick.wav"), b"the one already collected");

        // A second, different kick.wav from another pack.
        let other = dir.path().join("packB/kick.wav");
        write(&other, b"a different kick");

        let name = unique_name_in(&samples, &other, &[]);

        assert_eq!(name, "kick (2).wav");
        assert_eq!(
            std::fs::read(samples.join("kick.wav")).unwrap(),
            b"the one already collected",
            "the existing sample is untouched"
        );
    }

    #[test]
    fn two_same_named_samples_in_one_collect_get_separate_names() {
        let dir = tempfile::tempdir().unwrap();
        let samples = dir.path().join("Song Samples");
        std::fs::create_dir_all(&samples).unwrap();
        let first = dir.path().join("packA/snare.wav");
        let second = dir.path().join("packB/snare.wav");
        write(&first, b"one");
        write(&second, b"two");

        let a = unique_name_in(&samples, &first, &[]);
        let b = unique_name_in(&samples, &second, std::slice::from_ref(&a));

        assert_eq!(a, "snare.wav");
        assert_eq!(b, "snare (2).wav");
    }

    #[test]
    fn finds_a_moved_sample_by_name_in_a_nested_folder() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("Packs/Hard/Kicks/kick.wav"), b"audio");
        let index = index_audio_files(&[dir.path().to_string_lossy().into_owned()]);

        let found = pick_match(&index, &missing("kick.wav", "")).expect("found");

        assert!(found.ends_with("Packs/Hard/Kicks/kick.wav"));
    }

    #[test]
    fn a_recorded_hash_picks_the_right_one_of_two_same_named_samples() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("packA/kick.wav"), b"the wrong one");
        write(&dir.path().join("packB/kick.wav"), b"the right one");
        let wanted = hash_file(&dir.path().join("packB/kick.wav"));
        let index = index_audio_files(&[dir.path().to_string_lossy().into_owned()]);

        let found = pick_match(&index, &missing("kick.wav", &wanted)).expect("found");

        assert_eq!(std::fs::read(found).unwrap(), b"the right one");
    }

    #[test]
    fn without_a_hash_an_ambiguous_name_is_left_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("packA/kick.wav"), b"one");
        write(&dir.path().join("packB/kick.wav"), b"two");
        let index = index_audio_files(&[dir.path().to_string_lossy().into_owned()]);

        // Two files answer to the name and nothing distinguishes them, so
        // guessing would quietly put the wrong sample in someone's song.
        assert!(pick_match(&index, &missing("kick.wav", "")).is_none());
    }

    #[test]
    fn a_hash_that_matches_nothing_present_is_not_relinked() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("kick.wav"), b"a different sample");
        let index = index_audio_files(&[dir.path().to_string_lossy().into_owned()]);

        assert!(pick_match(&index, &missing("kick.wav", &"0".repeat(64))).is_none());
    }

    #[test]
    fn the_search_stops_before_it_walks_a_whole_drive() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c/d/e/f/g/h/i");
        write(&deep.join("kick.wav"), b"audio");
        let index = index_audio_files(&[dir.path().to_string_lossy().into_owned()]);

        assert!(pick_match(&index, &missing("kick.wav", "")).is_none());
    }

    #[test]
    fn non_audio_files_are_ignored_even_with_a_matching_name() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("kick.wav.txt"), b"notes");
        let index = index_audio_files(&[dir.path().to_string_lossy().into_owned()]);

        assert!(pick_match(&index, &missing("kick.wav", "")).is_none());
    }
}
