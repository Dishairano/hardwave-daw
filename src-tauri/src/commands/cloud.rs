//! Saving a song to Workspace and opening one from it.
//!
//! The song is saved and its samples collected beside it first, the
//! same way "collect samples" always worked, so what goes up is a
//! folder that opens on any machine. Coming back down, it lands in its
//! own folder under Documents and opens like any other song.

use crate::workspace_cloud::{self, Client, CloudSong};
use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

fn signed_in_client() -> Result<Client, String> {
    let token = crate::collab::load_token().ok_or_else(|| {
        "sign in first. Opening any Hardwave plug-in and signing in there is enough: \
         every Hardwave program on this machine shares one sign-in."
            .to_string()
    })?;
    Ok(Client::new(token))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSaveResult {
    pub workspace: String,
    pub folder: String,
    /// Files actually sent; unchanged ones are skipped by Workspace.
    pub sent: usize,
    pub unchanged: usize,
    /// Samples the song uses that could not be found to send.
    pub missing: Vec<String>,
}

/// Save the open song to Workspace, samples and all.
///
/// The song must have been saved to disk once, because the samples are
/// collected into a folder beside the project file and that is the
/// folder that goes up.
#[tauri::command]
pub async fn save_to_workspace(
    app: AppHandle,
    project_path: String,
) -> Result<CloudSaveResult, String> {
    let project = PathBuf::from(&project_path);
    {
        let state: State<AppState> = app.state();
        crate::commands::project::save_project(state.clone(), project_path.clone())?;
    }
    let missing = {
        let state: State<AppState> = app.state();
        crate::commands::sources::collect_project_samples(state.clone(), project_path.clone())?
            .missing
    };
    // Collecting rewrote the sample paths to the copies, so the project
    // is saved once more before it is sent.
    {
        let state: State<AppState> = app.state();
        crate::commands::project::save_project(state, project_path.clone())?;
    }

    let client = signed_in_client()?;
    let home = client.home_workspace().await?;
    let mut sent = 0usize;
    let mut unchanged = 0usize;
    for (file, folder) in workspace_cloud::song_files(&project) {
        if client.upload(home.id, &file, &folder).await? {
            sent += 1;
        } else {
            unchanged += 1;
        }
    }
    Ok(CloudSaveResult {
        workspace: home.name,
        folder: workspace_cloud::song_folder(&project),
        sent,
        unchanged,
        missing,
    })
}

/// The songs in your Workspace.
#[tauri::command]
pub async fn list_workspace_songs() -> Result<Vec<CloudSong>, String> {
    let client = signed_in_client()?;
    let home = client.home_workspace().await?;
    let files = client.files(home.id).await?;
    Ok(workspace_cloud::songs_in(&files))
}

/// Bring a song down from Workspace, ready to open.
///
/// Everything in the song's folder comes with it, so the samples sit
/// beside the project as they did when it was saved. Returns where the
/// project landed.
#[tauri::command]
pub async fn open_from_workspace(file_id: u64) -> Result<String, String> {
    let client = signed_in_client()?;
    let home = client.home_workspace().await?;
    let files = client.files(home.id).await?;
    let project = files
        .iter()
        .find(|f| f.id == file_id)
        .ok_or_else(|| "that song is no longer in your Workspace".to_string())?
        .clone();

    let local_root = workspace_cloud::local_copy_dir(&project.folder_path)
        .ok_or_else(|| "that song's folder name cannot be used on this machine".to_string())?;
    // Every file is placed by this machine, inside the song's folder,
    // or not at all: a name that would land anywhere else stops the
    // whole download before anything is written.
    let mut plan = Vec::new();
    for file in files
        .iter()
        .filter(|f| workspace_cloud::within_folder(&f.folder_path, &project.folder_path))
    {
        let target = workspace_cloud::local_path_for(&local_root, &project.folder_path, file)
            .ok_or_else(|| {
                format!(
                    "\"{}\" has a name this machine cannot safely use; nothing was downloaded",
                    file.name
                )
            })?;
        plan.push((file, target));
    }
    for (file, target) in plan {
        client.download(home.id, file, &target).await?;
    }

    // Opened by the window, the same way any song is opened, so the
    // channel rack, the timeline and the recent list all follow.
    if !workspace_cloud::safe_segment(&project.name) {
        return Err("that song's name cannot be used on this machine".into());
    }
    Ok(local_root
        .join(&project.name)
        .to_string_lossy()
        .into_owned())
}
