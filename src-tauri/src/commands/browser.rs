//! Filesystem listing for the browser panel's Places tree.
//!
//! Before this the browser only showed files added one at a time through the
//! Open dialog, so a sample pack could not be browsed at all. This lists one
//! directory level on demand. The UI asks again whenever a folder is opened,
//! so a large library is never walked up front.

use serde::Serialize;
use std::cmp::Ordering;
use std::path::Path;

/// Extensions the engine's decoder (symphonia with all codecs) can import.
/// Only these are listed, so every file the browser shows can go on a track.
const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "aiff", "aif", "ogg", "m4a"];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// File size in bytes; 0 for folders.
    pub size_bytes: u64,
}

/// List one folder for the browser. Runs on a blocking thread so a slow
/// network drive or a huge folder cannot stall the UI.
#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<BrowserEntry>, String> {
    let dir = path.clone();
    tauri::async_runtime::spawn_blocking(move || list_directory_entries(Path::new(&dir)))
        .await
        .map_err(|e| format!("Listing {path} failed: {e}"))?
}

/// Subfolders first, then audio files, each group sorted by name ignoring
/// case. Hidden entries are skipped. An entry that cannot be read (a broken
/// symlink, a permission error on one file) is skipped instead of failing
/// the whole folder; only a folder that cannot be opened is an error.
pub fn list_directory_entries(dir: &Path) -> Result<Vec<BrowserEntry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("Can't open {}: {e}", dir.display()))?;
    let mut entries: Vec<BrowserEntry> = read
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            // `fs::metadata` follows symlinks, so a linked sample folder
            // browses like a real one. A broken link has no metadata.
            let meta = std::fs::metadata(&path).ok()?;
            if is_hidden(&name, &meta) {
                return None;
            }
            let is_dir = meta.is_dir();
            if !is_dir && !is_audio_file(&path) {
                return None;
            }
            Some(BrowserEntry {
                name,
                path: path.to_string_lossy().into_owned(),
                is_dir,
                size_bytes: if is_dir { 0 } else { meta.len() },
            })
        })
        .collect();
    entries.sort_by(compare_entries);
    Ok(entries)
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(ext)))
}

#[cfg(windows)]
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    name.starts_with('.')
        || (meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

fn compare_entries(a: &BrowserEntry, b: &BrowserEntry) -> Ordering {
    b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(entries: &[BrowserEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn lists_folders_first_then_audio_sorted_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("snares")).unwrap();
        fs::create_dir(dir.path().join("Kicks")).unwrap();
        fs::write(dir.path().join("b loop.WAV"), b"x").unwrap();
        fs::write(dir.path().join("A Loop.flac"), b"xyz").unwrap();

        let entries = list_directory_entries(dir.path()).unwrap();

        assert_eq!(
            names(&entries),
            ["Kicks", "snares", "A Loop.flac", "b loop.WAV"]
        );
        assert!(entries[0].is_dir && entries[1].is_dir);
        assert!(!entries[2].is_dir && !entries[3].is_dir);
        assert_eq!(entries[0].size_bytes, 0);
        assert_eq!(entries[2].size_bytes, 3);
        assert_eq!(
            entries[2].path,
            dir.path().join("A Loop.flac").to_string_lossy()
        );
    }

    #[test]
    fn skips_hidden_entries_and_files_the_engine_cannot_import() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".cache")).unwrap();
        fs::write(dir.path().join(".DS_Store"), b"x").unwrap();
        fs::write(dir.path().join("._kick.wav"), b"x").unwrap();
        fs::write(dir.path().join("readme.txt"), b"x").unwrap();
        fs::write(dir.path().join("song.hwp"), b"x").unwrap();
        fs::write(dir.path().join("kick.wav"), b"x").unwrap();

        let entries = list_directory_entries(dir.path()).unwrap();

        assert_eq!(names(&entries), ["kick.wav"]);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let err = list_directory_entries(&dir.path().join("gone")).unwrap_err();
        assert!(err.contains("gone"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinked_folders_and_skips_broken_links() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let listed = dir.path().join("listed");
        fs::create_dir(&listed).unwrap();
        std::os::unix::fs::symlink(&real, listed.join("linked")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("nowhere"), listed.join("broken.wav")).unwrap();

        let entries = list_directory_entries(&listed).unwrap();

        assert_eq!(names(&entries), ["linked"]);
        assert!(entries[0].is_dir);
    }
}

/// Find audio files by name under the browser's Places folders.
///
/// Search only filtered the folders that happened to be open, so finding a
/// kick in a sample pack meant opening every folder first. This walks the
/// roots and returns what matches.
///
/// Bounded on purpose: a sample library is tens of thousands of files on a
/// slow drive, and a search box that hangs the panel is worse than one that
/// says it stopped early. `hit_limit` tells the UI which happened.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySearch {
    pub matches: Vec<BrowserEntry>,
    /// True when the walk stopped at the limit rather than finishing.
    pub hit_limit: bool,
}

const SEARCH_MAX_MATCHES: usize = 300;
const SEARCH_MAX_VISITED: usize = 60_000;

#[tauri::command]
pub async fn search_library(roots: Vec<String>, query: String) -> Result<LibrarySearch, String> {
    let needle = query.trim().to_lowercase();
    if needle.len() < 2 {
        return Ok(LibrarySearch {
            matches: Vec::new(),
            hit_limit: false,
        });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut matches: Vec<BrowserEntry> = Vec::new();
        let mut visited = 0usize;
        let mut hit_limit = false;
        // Breadth first, so shallow folders (where people keep the packs
        // they use) come back before a deep archive.
        let mut queue: std::collections::VecDeque<std::path::PathBuf> =
            roots.iter().map(std::path::PathBuf::from).collect();
        while let Some(dir) = queue.pop_front() {
            if matches.len() >= SEARCH_MAX_MATCHES || visited >= SEARCH_MAX_VISITED {
                hit_limit = true;
                break;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited >= SEARCH_MAX_VISITED {
                    hit_limit = true;
                    break;
                }
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name.starts_with("._") {
                    continue;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    queue.push_back(path);
                    continue;
                }
                if !is_audio_file(&path) || !name.to_lowercase().contains(&needle) {
                    continue;
                }
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                matches.push(BrowserEntry {
                    name,
                    path: path.to_string_lossy().to_string(),
                    is_dir: false,
                    size_bytes,
                });
                if matches.len() >= SEARCH_MAX_MATCHES {
                    hit_limit = true;
                    break;
                }
            }
        }
        matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        LibrarySearch { matches, hit_limit }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Move a browser file to the recycle bin/// Move a browser file to the recycle bin (the browser's "Delete file…").
///
/// Only audio files the browser lists can be removed this way, and they go to
/// the recycle bin rather than being unlinked, so a wrong click in a sample
/// library can be undone from Explorer.
#[tauri::command]
pub async fn trash_browser_file(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = Path::new(&path);
        if !p.is_file() {
            return Err(format!("not a file: {path}"));
        }
        if !is_audio_file(p) {
            return Err(format!("not an audio file: {path}"));
        }
        trash::delete(p).map_err(|e| format!("could not move {path} to the recycle bin: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}
