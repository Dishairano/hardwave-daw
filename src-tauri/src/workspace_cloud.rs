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

/// Where Workspace lives. Ours, unless a test points it elsewhere.
pub fn base_url() -> String {
    std::env::var("HARDWAVE_WORKSPACE_URL")
        .unwrap_or_else(|_| "https://workspace.hardwavestudios.com".to_string())
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
        all.iter()
            .find(|w| w.role.as_deref() == Some("owner"))
            .or_else(|| all.first())
            .cloned()
            .ok_or_else(|| {
                "you have no Workspace yet; open workspace.hardwavestudios.com once to make one"
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

    pub async fn download(&self, workspace: u64, file: u64, to: &Path) -> Result<(), String> {
        let link: Download = {
            let response = self
                .http
                .get(format!(
                    "{}/api/workspaces/{workspace}/files/{file}",
                    self.base
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
        let bytes = self
            .http
            .get(&link.download_url)
            .send()
            .await
            .map_err(|_| "the download was cut off".to_string())?
            .bytes()
            .await
            .map_err(|_| "the download was cut off".to_string())?;
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("could not make {}: {e}", parent.display()))?;
        }
        tokio::fs::write(to, &bytes)
            .await
            .map_err(|e| format!("could not write {}: {e}", to.display()))
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
                .filter(|other| other.folder_path.starts_with(&f.folder_path))
                .count(),
        })
        .collect();
    songs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    songs
}

/// Where a song from Workspace is kept on this machine, so it opens
/// like any other song and the samples resolve beside it.
pub fn local_copy_dir(folder: &str) -> PathBuf {
    let base = dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("Hardwave")
        .join("From Workspace");
    let relative = folder
        .trim_start_matches(SONGS_FOLDER)
        .trim_start_matches('/');
    base.join(relative)
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
        let local = local_copy_dir("/Hardwave DAW/Raw Drop");
        assert!(
            local.ends_with("Hardwave/From Workspace/Raw Drop"),
            "{local:?}"
        );
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
        client.download(7, songs[0].file_id, &back).await.unwrap();
        assert_eq!(std::fs::read(&back).unwrap(), b"the whole song");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
