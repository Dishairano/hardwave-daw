//! Opening a room, joining one, and saying what the room is doing.
//!
//! The connection itself runs on its own task: it reads from the
//! socket, applies what the other person did, and writes what this
//! one does. Commands only start it, stop it, and report.

use crate::AppState;
use futures_util::{SinkExt, StreamExt};
use hardwave_project::multiplayer::{SyncMessage, TransportSync};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::State;
use tokio::sync::mpsc;

/// What the UI needs after asking to open or join.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollabJoined {
    pub room_id: String,
    pub invite_code: String,
    /// Whether everything missed while away could be replayed. False
    /// means the project should be taken from the other side again.
    pub caught_up: bool,
}

/// Open a room, or join one with a code.
///
/// Opening needs Pro and joining does not: the service decides that,
/// not this side.
#[tauri::command]
pub async fn start_collab(
    state: State<'_, AppState>,
    room: Option<String>,
    code: Option<String>,
) -> Result<CollabJoined, String> {
    let token = crate::collab::load_token().ok_or_else(|| {
        "sign in first. Opening any Hardwave plug-in and signing in there is enough: \
         every Hardwave program on this machine shares one sign-in."
            .to_string()
    })?;
    let room = room.unwrap_or_default();
    let code = code.unwrap_or_default();

    let engine = Arc::clone(&state.engine);
    let collab = Arc::clone(&state.collab);
    let url = crate::collab::join_url(&token, &room, &code, 0);

    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("could not reach the room service: {e}"))?;
    let (mut writer, mut reader) = socket.split();

    // The service answers first: in, or why not.
    let hello = reader
        .next()
        .await
        .ok_or_else(|| "the room service said nothing".to_string())?
        .map_err(|e| format!("the room service dropped out: {e}"))?;
    let hello: crate::collab::Hello = serde_json::from_str(
        hello
            .to_text()
            .map_err(|_| "the room service sent something unreadable".to_string())?,
    )
    .map_err(|e| format!("the room service sent something unreadable: {e}"))?;

    let (room_id, invite_code, missed, caught_up) = match hello {
        crate::collab::Hello::Refused { reason } => return Err(reason),
        crate::collab::Hello::Joined {
            room_id,
            invite_code,
            missed,
            caught_up,
        } => (room_id, invite_code, missed, caught_up),
    };

    // Whatever was missed, before anything new arrives.
    for message in &missed {
        apply_one(&engine, &collab, message);
    }

    let (outbox, mut to_send) = mpsc::unbounded_channel::<SyncMessage>();
    collab.begin(&room_id, &invite_code, room.is_empty(), outbox);

    // Out: what this person does.
    let collab_for_writer = Arc::clone(&collab);
    tokio::spawn(async move {
        while let Some(message) = to_send.recv().await {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if writer
                .send(tokio_tungstenite::tungstenite::Message::Text(text))
                .await
                .is_err()
            {
                collab_for_writer.stop("the connection closed");
                break;
            }
        }
    });

    // In: what the other person does.
    let collab_for_reader = Arc::clone(&collab);
    let engine_for_reader = Arc::clone(&engine);
    tokio::spawn(async move {
        while let Some(Ok(message)) = reader.next().await {
            let Ok(text) = message.into_text() else {
                continue;
            };
            let text: &str = &text;
            let Ok(parsed) = serde_json::from_str::<SyncMessage>(text) else {
                continue;
            };
            apply_one(&engine_for_reader, &collab_for_reader, &parsed);
        }
        collab_for_reader.stop("the other side left");
    });

    Ok(CollabJoined {
        room_id,
        invite_code,
        caught_up,
    })
}

/// Answer "send me the song" with the song, and take one when it
/// arrives.
///
/// This is the one moment the whole project crosses the network. The
/// audio files do not travel with it: a clip whose sample the other
/// machine does not have reads as missing there, exactly as it does
/// when a project is copied between machines by hand.
fn handle_project(
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    collab: &Arc<crate::collab::Collab>,
    message: &SyncMessage,
) -> bool {
    use hardwave_project::multiplayer::SyncKind;
    match &message.kind {
        SyncKind::ProjectRequest => {
            let (name, blob) = {
                let engine_guard = engine.lock();
                let project = engine_guard.project.lock();
                match project.to_bytes() {
                    Ok(blob) => (project.metadata.name.clone(), blob),
                    Err(e) => {
                        log::warn!("could not pack the song to send: {e}");
                        return true;
                    }
                }
            };
            collab.send(SyncKind::ProjectOffer { name, blob });
            true
        }
        SyncKind::ProjectOffer { name, blob } => {
            match hardwave_project::Project::from_bytes(blob) {
                Ok(incoming) => {
                    let engine_guard = engine.lock();
                    engine_guard.snapshot_before_mutation();
                    *engine_guard.project.lock() = incoming;
                    engine_guard.sync_track_meters();
                    engine_guard.rebuild_graph();
                    log::info!("took the song \"{name}\" from the other side");
                }
                Err(e) => log::warn!("could not read the song they sent: {e}"),
            }
            true
        }
        _ => false,
    }
}

/// Apply one message and do what it asks of the transport.
fn apply_one(
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    collab: &Arc<crate::collab::Collab>,
    message: &SyncMessage,
) {
    // Who is in the room changes the panel, not the song.
    if let hardwave_project::multiplayer::SyncKind::MembersChanged { names } = &message.kind {
        collab.set_members(names.clone());
        return;
    }
    // The song itself is not an edit, and it is handled before the
    // engine lock is taken because packing or unpacking it is slow.
    if handle_project(engine, collab, message) {
        return;
    }
    let engine_guard = engine.lock();
    let transport = {
        let mut project = engine_guard.project.lock();
        crate::collab::apply_incoming(collab, &mut project, message)
    };
    engine_guard.rebuild_graph();

    match transport {
        Some(TransportSync::Play) => {
            engine_guard.send_command(hardwave_engine::transport::TransportCommand::Play)
        }
        Some(TransportSync::Stop) => {
            engine_guard.send_command(hardwave_engine::transport::TransportCommand::Stop)
        }
        Some(TransportSync::Seek { tick }) => {
            let sample_rate = engine_guard.current_sample_rate() as f64;
            let bpm = engine_guard.transport.bpm.load(Ordering::Relaxed);
            let beats = tick as f64 / hardwave_midi::PPQ as f64;
            engine_guard
                .transport
                .set_position((beats * 60.0 / bpm * sample_rate) as u64);
        }
        None => {}
    }
}

#[tauri::command]
pub fn stop_collab(state: State<AppState>) {
    state.collab.stop("you left the room");
}

#[tauri::command]
pub fn collab_status(state: State<AppState>) -> crate::collab::CollabStatus {
    state.collab.status()
}

/// Ask the other side for the song.
///
/// A guest who joins a room usually does not have the project, and
/// one edit at a time will never build it for them.
#[tauri::command]
pub fn request_project(state: State<AppState>) -> Result<(), String> {
    if !state.collab.status().connected {
        return Err("you are not in a room".into());
    }
    state
        .collab
        .send(hardwave_project::multiplayer::SyncKind::ProjectRequest);
    Ok(())
}
