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
    project_sources(&state)
        .into_iter()
        .filter(|(_, (file, _, _))| !Path::new(file).is_file())
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
