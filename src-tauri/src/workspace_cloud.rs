//! Songs in Workspace.
//!
//! Workspace already stores a producer's files, with versions, sharing
//! and a desktop app that keeps a folder in sync. A song saved from the
//! DAW belongs there rather than in a second cloud of its own, so this
//! talks to Workspace's own API with the sign-in every Hardwave program
//! on the machine shares.
//!
//! A song goes up as its project file plus the samples it uses, in a
//! folder of its own under "Hardwave DAW". The samples are collected
//! next to the project first, with relative paths, so the folder opens
//! on any machine. Workspace dedupes by content, so saving the same
//! song again only sends what changed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The folder in every workspace that holds songs from the DAW.
pub const SONGS_FOLDER: &str = "/Hardwave DAW";

/// Where Workspace lives: ours, or in a development build wherever
/// HARDWAVE_WORKSPACE_URL points.
pub fn base_url() -> String {
    crate::endpoints::service_url(
        "HARDWAVE_WORKSPACE_URL",
        "https://workspace.hardwavestudios.com",
    )
}

/// The largest single file a song brings down. Far above any real
/// sample; it exists so a wrong listing cannot fill the disk.
const LARGEST_FILE: u64 = 4 * 1024 * 1024 * 1024;

/// Whether one part of a name from Workspace is safe to use as one part
/// of a path on this machine. Workspace names files; this machine
/// decides where they go, so anything that could step out of the song's
/// folder, name a drive or a device, or mean something special to
/// Windows is refused rather than cleaned up.
pub fn safe_segment(part: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = part
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    !part.is_empty()
        && part.len() <= 200
        && part != "."
        && part != ".."
        && !part.ends_with('.')
        && !part.ends_with(' ')
        && !part.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !RESERVED.contains(&stem.as_str())
}

/// Whether `path` is `folder` or inside it, comparing whole folder
/// names: "/Hardwave DAW/Song" does not contain "/Hardwave DAW/Song 2".
pub fn within_folder(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    path == folder || path.starts_with(&format!("{folder}/"))
}

/// Where one file of a song lands on this machine, or None when its
/// name or folder would put it anywhere but inside the song's folder.
pub fn local_path_for(root: &Path, song_folder: &str, file: &RemoteFile) -> Option<PathBuf> {
    if !within_folder(&file.folder_path, song_folder) || !safe_segment(&file.name) {
        return None;
    }
    let rest = file.folder_path[song_folder.trim_end_matches('/').len()..].trim_start_matches('/');
    let mut path = root.to_path_buf();
    for part in rest.split('/').filter(|p| !p.is_empty()) {
        if !safe_segment(part) {
            return None;
        }
        path.push(part);
    }
    path.push(&file.name);
    Some(path)
}

/// One workspace the signed-in person belongs to.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
}

/// One file in a workspace, as the listing returns it.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RemoteFile {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default = "root")]
    pub folder_path: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

fn root() -> String {
    "/".to_string()
}

/// A song stored in Workspace, as the open dialog lists it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudSong {
    pub name: String,
    pub file_id: u64,
    pub folder: String,
    pub updated_at: Option<String>,
    /// How many files belong to it, the project included.
    pub files: usize,
}

/// Talking to Workspace for one signed-in person.
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

#[derive(Deserialize)]
struct Workspaces {
    workspaces: Vec<WorkspaceInfo>,
}

#[derive(Deserialize)]
struct Files {
    files: Vec<RemoteFile>,
}

/// What Workspace answers when an upload starts.
///
/// It sends every field twice, `fileId` and `file_id`, `uploadUrl` and
/// `upload_url`, for clients written against either spelling. Serde
/// reads an alias and its original as the same field given twice and
/// refuses the whole answer, so the fields are picked out by hand.
struct UploadTicket {
    upload_url: Option<String>,
    file_id: u64,
    already_uploaded: bool,
}

impl UploadTicket {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let pick = |a: &str, b: &str| value.get(a).or_else(|| value.get(b)).cloned();
        let file_id = match pick("fileId", "file_id")? {
            serde_json::Value::Number(n) => n.as_u64()?,
            serde_json::Value::String(s) => s.parse().ok()?,
            _ => return None,
        };
        let upload_url = pick("uploadUrl", "upload_url").and_then(|v| v.as_str().map(String::from));
        let already_uploaded = pick("alreadyUploaded", "already_uploaded")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Some(Self {
            upload_url,
            file_id,
            already_uploaded,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Download {
    download_url: String,
}

/// What a failed call means to a producer, not to a programmer.
fn explain(status: reqwest::StatusCode, during: &str) -> String {
    match status.as_u16() {
        401 | 403 => "your Hardwave sign-in has run out; sign in again in any plug-in".into(),
        404 => format!("Workspace could not find what was asked for while {during}"),
        413 => "your Workspace is full; free some space or move to a bigger plan".into(),
        _ => format!("Workspace answered {status} while {during}"),
    }
}

impl Client {
    pub fn new(token: String) -> Self {
        Self::with_base(base_url(), token)
    }

    pub fn with_base(base: String, token: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            base,
            token,
        }
    }

    pub async fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, String> {
        let response = self
            .http
            .get(format!("{}/api/workspaces", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "could not reach Workspace".to_string())?;
        if !response.status().is_success() {
            return Err(explain(response.status(), "listing your workspaces"));
        }
        Ok(response
            .json::<Workspaces>()
            .await
            .map_err(|_| "Workspace sent a list we could not read".to_string())?
            .workspaces)
    }

    /// The workspace songs go to: the first one this person owns, or
    /// failing that the first they can write to.
    pub async fn home_workspace(&self) -> Result<WorkspaceInfo, String> {
        let all = self.workspaces().await?;
        // Only a workspace this person owns. One they were invited into
        // belongs to someone else, who would see every song saved there.
        all.iter()
            .find(|w| w.role.as_deref() == Some("owner"))
            .cloned()
            .ok_or_else(|| {
                "you have no Workspace of your own yet; open workspace.hardwavestudios.com once to make one"
                    .into()
            })
    }

    pub async fn files(&self, workspace: u64) -> Result<Vec<RemoteFile>, String> {
        let response = self
            .http
            .get(format!(
                "{}/api/workspaces/{workspace}/files?all=true",
                self.base
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "could not reach Workspace".to_string())?;
        if !response.status().is_success() {
            return Err(explain(response.status(), "listing your files"));
        }
        Ok(response
            .json::<Files>()
            .await
            .map_err(|_| "Workspace sent a list we could not read".to_string())?
            .files)
    }

    /// Put one file into a folder. Bytes Workspace already has are not
    /// sent again: it says so, and the upload is skipped.
    pub async fn upload(&self, workspace: u64, local: &Path, folder: &str) -> Result<bool, String> {
        let bytes = tokio::fs::read(local)
            .await
            .map_err(|e| format!("could not read {}: {e}", local.display()))?;
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} has no name", local.display()))?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let mime = mime_for(&name);

        let ticket: UploadTicket = {
            let response = self
                .http
                .post(format!("{}/api/workspaces/{workspace}/files", self.base))
                .bearer_auth(&self.token)
                .json(&serde_json::json!({
                    "name": name,
                    "size": bytes.len(),
                    "sha256": sha256,
                    "mime_type": mime,
                    "folder_path": folder,
                }))
                .send()
                .await
                .map_err(|_| "could not reach Workspace".to_string())?;
            if !response.status().is_success() {
                return Err(explain(
                    response.status(),
                    &format!("starting the upload of {name}"),
                ));
            }
            let value: serde_json::Value = response.json().await.map_err(|_| {
                "Workspace answered the upload with something we could not read".to_string()
            })?;
            UploadTicket::from_json(&value)
                .ok_or_else(|| "Workspace answered the upload without a file id".to_string())?
        };

        if ticket.already_uploaded {
            return Ok(false);
        }
        let url = ticket
            .upload_url
            .ok_or_else(|| "Workspace gave no address to upload to".to_string())?;
        let put = self
            .http
            .put(&url)
            .header("Content-Type", mime)
            .body(bytes)
            .send()
            .await
            .map_err(|_| format!("the upload of {name} was cut off"))?;
        if !put.status().is_success() {
            return Err(format!("storage refused {name} ({})", put.status()));
        }

        let registered = self
            .http
            .post(format!(
                "{}/api/workspaces/{workspace}/files/register",
                self.base
            ))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "file_id": ticket.file_id }))
            .send()
            .await
            .map_err(|_| "could not reach Workspace to finish the upload".to_string())?;
        if !registered.status().is_success() {
            return Err(explain(
                registered.status(),
                &format!("finishing the upload of {name}"),
            ));
        }
        Ok(true)
    }

    /// Bring one file down to `to`, as it arrives, checking it is the
    /// file the listing described before it takes its place.
    pub async fn download(
        &self,
        workspace: u64,
        file: &RemoteFile,
        to: &Path,
    ) -> Result<(), String> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;
        let link: Download = {
            let response = self
                .http
                .get(format!(
                    "{}/api/workspaces/{workspace}/files/{}",
                    self.base, file.id
                ))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|_| "could not reach Workspace".to_string())?;
            if !response.status().is_success() {
                return Err(explain(response.status(), "fetching a file"));
            }
            response
                .json()
                .await
                .map_err(|_| "Workspace sent a download we could not read".to_string())?
        };
        if !crate::endpoints::transfer_allowed(&link.download_url) {
            return Err(
                "Workspace gave a download address that is not secure; nothing was fetched".into(),
            );
        }
        let response = self
            .http
            .get(&link.download_url)
            .send()
            .await
            .map_err(|_| "the download was cut off".to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "storage refused {} ({})",
                file.name,
                response.status()
            ));
        }
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("could not make {}: {e}", parent.display()))?;
        }
        let partial = to.with_extension("part");
        let mut out = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| format!("could not write {}: {e}", partial.display()))?;
        let mut hash = Sha256::new();
        let mut written: u64 = 0;
        let mut body = response.bytes_stream();
        let result: Result<(), String> = async {
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(|_| "the download was cut off".to_string())?;
                written += chunk.len() as u64;
                if written > LARGEST_FILE || file.size.is_some_and(|size| written > size) {
                    return Err(format!(
                        "{} is larger than Workspace said; it was not kept",
                        file.name
                    ));
                }
                hash.update(&chunk);
                out.write_all(&chunk)
                    .await
                    .map_err(|e| format!("could not write {}: {e}", partial.display()))?;
            }
            out.flush().await.map_err(|e| e.to_string())?;
            if let Some(expected) = file.sha256.as_deref().filter(|h| !h.is_empty()) {
                if !format!("{:x}", hash.finalize_reset()).eq_ignore_ascii_case(expected) {
                    return Err(format!(
                        "{} did not arrive intact; it was not kept",
                        file.name
                    ));
                }
            }
            Ok(())
        }
        .await;
        drop(out);
        if let Err(e) = result {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(e);
        }
        tokio::fs::rename(&partial, to)
            .await
            .map_err(|e| format!("could not put {} in place: {e}", to.display()))
    }
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".wav") {
        "audio/wav"
    } else if lower.ends_with(".flac") {
        "audio/flac"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".aif") || lower.ends_with(".aiff") {
        "audio/aiff"
    } else {
        "application/octet-stream"
    }
}

/// The folder a song lives in, named after its project file.
pub fn song_folder(project: &Path) -> String {
    let stem = project
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string());
    format!("{SONGS_FOLDER}/{stem}")
}

/// Every file that makes up a song on disk: the project, and whatever
/// sits in the samples folder collected beside it.
pub fn song_files(project: &Path) -> Vec<(PathBuf, String)> {
    let folder = song_folder(project);
    let mut out = vec![(project.to_path_buf(), folder.clone())];
    let stem = project
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(dir) = project.parent() {
        let samples = dir.join(format!("{stem} Samples"));
        if let Ok(entries) = std::fs::read_dir(&samples) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();
            for file in files {
                out.push((file, format!("{folder}/{stem} Samples")));
            }
        }
    }
    out
}

/// The songs in a workspace: every project file under the songs folder,
/// with a count of what came with it.
pub fn songs_in(files: &[RemoteFile]) -> Vec<CloudSong> {
    let mut songs: Vec<CloudSong> = files
        .iter()
        .filter(|f| f.name.to_lowercase().ends_with(".hwp"))
        .filter(|f| f.folder_path.starts_with(&format!("{SONGS_FOLDER}/")))
        .map(|f| CloudSong {
            name: f.name.trim_end_matches(".hwp").to_string(),
            file_id: f.id,
            folder: f.folder_path.clone(),
            updated_at: f.updated_at.clone(),
            files: files
                .iter()
                .filter(|other| within_folder(&other.folder_path, &f.folder_path))
                .count(),
        })
        .collect();
    songs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    songs
}

/// Where a song from Workspace is kept on this machine, so it opens
/// like any other song and the samples resolve beside it.
pub fn local_copy_dir(folder: &str) -> Option<PathBuf> {
    if !within_folder(folder, SONGS_FOLDER) {
        return None;
    }
    let mut path = dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("Hardwave")
        .join("From Workspace");
    for part in folder[SONGS_FOLDER.len()..]
        .split('/')
        .filter(|p| !p.is_empty())
    {
        if !safe_segment(part) {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, name: &str, folder: &str, updated: &str) -> RemoteFile {
        RemoteFile {
            id,
            name: name.into(),
            size: None,
            sha256: None,
            folder_path: folder.into(),
            updated_at: Some(updated.into()),
        }
    }

    #[test]
    fn a_song_folder_is_named_after_the_project() {
        assert_eq!(
            song_folder(Path::new("/music/Raw Drop.hwp")),
            "/Hardwave DAW/Raw Drop"
        );
    }

    #[test]
    fn a_song_carries_its_collected_samples() {
        let dir = std::env::temp_dir().join(format!("hw-cloud-{}", std::process::id()));
        let samples = dir.join("Raw Drop Samples");
        std::fs::create_dir_all(&samples).unwrap();
        std::fs::write(dir.join("Raw Drop.hwp"), b"project").unwrap();
        std::fs::write(samples.join("kick.wav"), b"kick").unwrap();
        std::fs::write(samples.join("screech.wav"), b"screech").unwrap();

        let files = song_files(&dir.join("Raw Drop.hwp"));
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].1, "/Hardwave DAW/Raw Drop");
        assert_eq!(files[1].1, "/Hardwave DAW/Raw Drop/Raw Drop Samples");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_project_files_under_the_songs_folder_are_songs() {
        let files = vec![
            file(1, "Raw Drop.hwp", "/Hardwave DAW/Raw Drop", "2026-10-07"),
            file(
                2,
                "kick.wav",
                "/Hardwave DAW/Raw Drop/Raw Drop Samples",
                "2026-10-07",
            ),
            file(3, "Old Idea.hwp", "/Hardwave DAW/Old Idea", "2026-09-01"),
            file(4, "Elsewhere.hwp", "/Backups", "2026-10-08"),
            file(5, "notes.txt", "/", "2026-10-08"),
        ];
        let songs = songs_in(&files);
        assert_eq!(songs.len(), 2, "{songs:?}");
        assert_eq!(songs[0].name, "Raw Drop", "newest first");
        assert_eq!(songs[0].files, 2, "the project and its kick");
        assert_eq!(songs[1].name, "Old Idea");
    }

    #[test]
    fn a_full_workspace_is_said_in_words() {
        let message = explain(reqwest::StatusCode::PAYLOAD_TOO_LARGE, "uploading");
        assert!(message.contains("full"), "{message}");
        let signed_out = explain(reqwest::StatusCode::UNAUTHORIZED, "listing");
        assert!(signed_out.contains("sign in"), "{signed_out}");
    }

    #[test]
    fn a_downloaded_song_lands_in_its_own_folder() {
        let local = local_copy_dir("/Hardwave DAW/Raw Drop").unwrap();
        assert!(
            local.ends_with("Hardwave/From Workspace/Raw Drop"),
            "{local:?}"
        );
        assert_eq!(local_copy_dir("/Hardwave DAW/../../Startup"), None);
        assert_eq!(local_copy_dir("/Elsewhere/Raw Drop"), None);
        assert_eq!(local_copy_dir("/Hardwave DAWX/Raw Drop"), None);
    }

    fn remote(folder: &str, name: &str) -> RemoteFile {
        RemoteFile {
            id: 1,
            name: name.into(),
            size: None,
            sha256: None,
            folder_path: folder.into(),
            updated_at: None,
        }
    }

    #[test]
    fn a_name_from_workspace_cannot_place_a_file_outside_the_song() {
        let root = Path::new("/home/p/Documents/Hardwave/From Workspace/Raw Drop");
        let song = "/Hardwave DAW/Raw Drop";
        assert_eq!(
            local_path_for(root, song, &remote(song, "Raw Drop.hwp")),
            Some(root.join("Raw Drop.hwp"))
        );
        assert_eq!(
            local_path_for(
                root,
                song,
                &remote("/Hardwave DAW/Raw Drop/Raw Drop Samples", "Kick.wav")
            ),
            Some(root.join("Raw Drop Samples").join("Kick.wav"))
        );
        for (folder, name) in [
            ("/Hardwave DAW/Raw Drop/../../../AppData", "x.bat"),
            (song, "..\\..\\Startup\\x.bat"),
            (song, "../x.bat"),
            (song, "C:\\Windows\\x.bat"),
            (song, "CON.wav"),
            (song, "nul"),
            (song, "kick.wav."),
            (song, ""),
            ("/Hardwave DAW/Raw Drop 2", "Kick.wav"),
            ("/Hardwave DAW/Raw Drop/sub:stream", "Kick.wav"),
        ] {
            assert_eq!(
                local_path_for(root, song, &remote(folder, name)),
                None,
                "{folder} / {name}"
            );
        }
    }

    #[test]
    fn folders_match_by_whole_name() {
        assert!(within_folder("/Hardwave DAW/Song", "/Hardwave DAW/Song"));
        assert!(within_folder(
            "/Hardwave DAW/Song/Samples",
            "/Hardwave DAW/Song/"
        ));
        assert!(!within_folder("/Hardwave DAW/Song 2", "/Hardwave DAW/Song"));
    }
}

#[cfg(test)]
mod round_trip {
    //! Against a stand-in for Workspace that behaves like the real API:
    //! a ticket with an upload address, a PUT of the bytes, a register,
    //! and "already uploaded" for bytes it already holds.

    use super::*;
    use axum::extract::{Path as UrlPath, State};
    use axum::routing::{get, post, put};
    use axum::{Json, Router};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Store {
        /// file id -> (name, folder, sha, bytes, registered)
        files: HashMap<u64, (String, String, String, Vec<u8>, bool)>,
        next: u64,
        puts: usize,
        base: String,
    }

    type Shared = Arc<Mutex<Store>>;

    async fn ticket(
        State(store): State<Shared>,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        let mut s = store.lock().unwrap();
        let name = body["name"].as_str().unwrap_or("").to_string();
        let folder = body["folder_path"].as_str().unwrap_or("/").to_string();
        let sha = body["sha256"].as_str().unwrap_or("").to_string();
        if let Some((id, _)) = s
            .files
            .iter()
            .find(|(_, f)| f.0 == name && f.1 == folder && f.2 == sha && f.4)
        {
            return Json(
                serde_json::json!({ "fileId": id, "file_id": id.to_string(), "alreadyUploaded": true }),
            );
        }
        s.next += 1;
        let id = s.next;
        s.files.insert(id, (name, folder, sha, Vec::new(), false));
        let url = format!("{}/storage/{id}", s.base);
        Json(serde_json::json!({ "fileId": id, "file_id": id.to_string(), "uploadUrl": url }))
    }

    async fn storage(
        State(store): State<Shared>,
        UrlPath(id): UrlPath<u64>,
        body: axum::body::Bytes,
    ) {
        let mut s = store.lock().unwrap();
        s.puts += 1;
        if let Some(f) = s.files.get_mut(&id) {
            f.3 = body.to_vec();
        }
    }

    async fn register(
        State(store): State<Shared>,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        let id: u64 = body["file_id"].as_u64().unwrap_or(0);
        if let Some(f) = store.lock().unwrap().files.get_mut(&id) {
            f.4 = true;
        }
        Json(serde_json::json!({ "ok": true }))
    }

    async fn listing(State(store): State<Shared>) -> Json<serde_json::Value> {
        let s = store.lock().unwrap();
        let files: Vec<_> = s
            .files
            .iter()
            .filter(|(_, f)| f.4)
            .map(|(id, f)| serde_json::json!({ "id": id, "name": f.0, "folder_path": f.1, "sha256": f.2, "updated_at": "2026-10-07" }))
            .collect();
        Json(serde_json::json!({ "files": files }))
    }

    async fn link(
        State(store): State<Shared>,
        UrlPath((_, id)): UrlPath<(u64, u64)>,
    ) -> Json<serde_json::Value> {
        let base = store.lock().unwrap().base.clone();
        Json(serde_json::json!({ "downloadUrl": format!("{base}/blob/{id}") }))
    }

    async fn blob(State(store): State<Shared>, UrlPath(id): UrlPath<u64>) -> Vec<u8> {
        store
            .lock()
            .unwrap()
            .files
            .get(&id)
            .map(|f| f.3.clone())
            .unwrap_or_default()
    }

    async fn stand_in() -> (String, Shared) {
        let store: Shared = Arc::new(Mutex::new(Store::default()));
        let app = Router::new()
            .route("/api/workspaces/{ws}/files", post(ticket).get(listing))
            .route("/api/workspaces/{ws}/files/register", post(register))
            .route("/api/workspaces/{ws}/files/{id}", get(link))
            .route("/storage/{id}", put(storage))
            .route("/blob/{id}", get(blob))
            .with_state(store.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        store.lock().unwrap().base = base.clone();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, store)
    }

    #[tokio::test]
    async fn a_song_goes_up_comes_back_and_is_not_sent_twice() {
        let (base, store) = stand_in().await;
        let client = Client::with_base(base, "token".into());

        let dir = std::env::temp_dir().join(format!("hw-cloud-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("Raw Drop.hwp");
        std::fs::write(&project, b"the whole song").unwrap();

        // First time: ticket, bytes, register.
        let sent = client
            .upload(7, &project, "/Hardwave DAW/Raw Drop")
            .await
            .unwrap();
        assert!(sent, "a new file is sent");
        assert_eq!(store.lock().unwrap().puts, 1);

        // Second time, same bytes: Workspace already has them.
        let again = client
            .upload(7, &project, "/Hardwave DAW/Raw Drop")
            .await
            .unwrap();
        assert!(!again, "unchanged bytes are not sent twice");
        assert_eq!(store.lock().unwrap().puts, 1);

        // It is listed as a song, and it comes back byte for byte.
        let files = client.files(7).await.unwrap();
        let songs = songs_in(&files);
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].name, "Raw Drop");

        let back = dir.join("back").join("Raw Drop.hwp");
        let listed = files.iter().find(|f| f.id == songs[0].file_id).unwrap();
        client.download(7, listed, &back).await.unwrap();
        assert_eq!(std::fs::read(&back).unwrap(), b"the whole song");

        // A file that is not what the listing said is not kept.
        let mut wrong = listed.clone();
        wrong.sha256 = Some("00".repeat(32));
        let refused = dir.join("back").join("tampered.hwp");
        let err = client.download(7, &wrong, &refused).await.unwrap_err();
        assert!(err.contains("intact"), "{err}");
        assert!(!refused.exists() && !refused.with_extension("part").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
