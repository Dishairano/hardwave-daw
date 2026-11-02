//! Opening a room, joining one, and saying what the room is doing.
//!
//! The connection itself runs on its own task: it reads from the
//! socket, applies what the other person did, and writes what this
//! one does. Commands only start it, stop it, and report.

use crate::AppState;
use futures_util::{SinkExt, StreamExt};
use hardwave_project::multiplayer::{SyncKind, SyncMessage, TransportSync};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
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

/// TLS for the room connection: the service's certificate is checked against
/// the Mozilla roots built into the app (webpki-roots 1.x), as
/// tokio-tungstenite's own `rustls-tls-webpki-roots` did before it brought in
/// webpki-roots 0.26 (RUSTSEC-2026-0381).
fn room_tls() -> tokio_tungstenite::Connector {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_tungstenite::Connector::Rustls(Arc::new(config))
}

/// Open a room, or join one with a code.
///
/// Opening needs Pro and joining does not: the service decides that,
/// not this side. A guest with the right code still waits until the
/// host says yes.
#[tauri::command]
pub async fn start_collab(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    room: Option<String>,
    code: Option<String>,
) -> Result<CollabJoined, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let token = crate::commands::account::token_or_sign_in_first()?;
    let room = room.unwrap_or_default();
    let code = code.unwrap_or_default();

    let engine = Arc::clone(&state.engine);
    let collab = Arc::clone(&state.collab);
    // Leaving whatever room this DAW was in before.
    collab.stop("");

    let mut request = crate::collab::join_url(&room, 0)
        .into_client_request()
        .map_err(|e| format!("could not reach the room service: {e}"))?;
    if !code.trim().is_empty() {
        request.headers_mut().insert(
            "x-invite-code",
            code.trim()
                .parse()
                .map_err(|_| "that code has characters a code never has".to_string())?,
        );
    }
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| "the sign-in on this machine is damaged; sign in again".to_string())?,
    );
    // The songs that cross a room are capped by the service too; this
    // side refuses anything larger before reading it.
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(hardwave_room_limits::MAX_MESSAGE),
        max_frame_size: Some(hardwave_room_limits::MAX_MESSAGE),
        ..Default::default()
    };
    let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        Some(config),
        false,
        Some(room_tls()),
    )
    .await
    .map_err(|e| format!("could not reach the room service: {e}"))?;
    let (mut writer, mut reader) = socket.split();

    // The service answers first: in, waiting for the host, or why not.
    async fn hello(
        reader: &mut futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    ) -> Result<crate::collab::Hello, String> {
        let message = reader
            .next()
            .await
            .ok_or_else(|| "the room service said nothing".to_string())?
            .map_err(|e| format!("the room service dropped out: {e}"))?;
        serde_json::from_str(
            message
                .to_text()
                .map_err(|_| "the room service sent something unreadable".to_string())?,
        )
        .map_err(|e| format!("the room service sent something unreadable: {e}"))
    }
    let mut first = hello(&mut reader).await?;
    if matches!(first, crate::collab::Hello::Waiting) {
        collab.set_waiting(true);
        first = tokio::time::timeout(std::time::Duration::from_secs(150), hello(&mut reader))
            .await
            .map_err(|_| "the host did not answer".to_string())??;
        collab.set_waiting(false);
    }
    let (room_id, invite_code, missed, caught_up) = match first {
        crate::collab::Hello::Refused { reason } => return Err(reason),
        crate::collab::Hello::Waiting => return Err("the room service lost track of you".into()),
        crate::collab::Hello::Joined {
            room_id,
            invite_code,
            missed,
            caught_up,
        } => (room_id, invite_code, missed, caught_up),
    };

    let (outbox, mut to_send) = mpsc::unbounded_channel::<SyncMessage>();
    let generation = collab.begin(&room_id, &invite_code, room.is_empty(), outbox);
    // The song as it is now is what both sides start from; from here on
    // only the tracks that change are sent.
    collab.remember_tracks(&engine.lock().project.lock());

    // Whatever was missed, before anything new arrives.
    for message in &missed {
        apply_one(&app, &engine, &collab, message, generation);
    }

    // Out: what this person does.
    let collab_for_writer = Arc::clone(&collab);
    collab.hold(tokio::spawn(async move {
        while let Some(message) = to_send.recv().await {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if writer
                .send(tokio_tungstenite::tungstenite::Message::Text(text))
                .await
                .is_err()
            {
                if collab_for_writer.is_current(generation) {
                    collab_for_writer.stop("the connection closed");
                }
                break;
            }
        }
        let _ = writer.close().await;
    }));

    // In: what the other person does.
    let collab_for_reader = Arc::clone(&collab);
    let engine_for_reader = Arc::clone(&engine);
    let app_for_reader = app.clone();
    collab.hold(tokio::spawn(async move {
        while let Some(Ok(message)) = reader.next().await {
            if !collab_for_reader.is_current(generation) {
                break;
            }
            let Ok(text) = message.into_text() else {
                continue;
            };
            let text: &str = &text;
            let Ok(parsed) = serde_json::from_str::<SyncMessage>(text) else {
                continue;
            };
            apply_one(
                &app_for_reader,
                &engine_for_reader,
                &collab_for_reader,
                &parsed,
                generation,
            );
        }
        if collab_for_reader.is_current(generation) {
            collab_for_reader.stop("the other side left");
        }
    }));

    // Out, for everything one edit at a time does not carry: a few times a
    // second, every track that changed goes as a whole (clips, plug-ins,
    // automation, mute, names), and a deleted one as gone.
    let collab_for_tracks = Arc::clone(&collab);
    let engine_for_tracks = Arc::clone(&engine);
    collab.hold(tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(300));
        loop {
            tick.tick().await;
            if !collab_for_tracks.is_current(generation) {
                break;
            }
            let (changed, gone, song) = {
                let engine_guard = engine_for_tracks.lock();
                let project = engine_guard.project.lock();
                let (changed, gone) = collab_for_tracks.track_changes(&project);
                (changed, gone, collab_for_tracks.song_changes(&project))
            };
            // The song's parts first: a tempo change has to be there
            // before the clips placed by it. (Too large ones are left out
            // in song_changes.)
            for (section, version, author, blob) in song {
                collab_for_tracks.send(SyncKind::SongState {
                    section: section.to_string(),
                    version,
                    author,
                    blob,
                });
            }
            for (track_id, index, blob) in changed {
                if blob.len() > hardwave_room_limits::MAX_TRACK {
                    log::warn!(
                        "room: track {track_id} is too large to send ({} bytes)",
                        blob.len()
                    );
                    continue;
                }
                collab_for_tracks.send(SyncKind::TrackState {
                    track_id,
                    index,
                    blob,
                });
            }
            for track_id in gone {
                collab_for_tracks.send(SyncKind::TrackRemoved { track_id });
            }
        }
    }));

    // A heartbeat, so the service knows a quiet room is still open.
    let collab_for_heartbeat = Arc::clone(&collab);
    collab.hold(tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        tick.tick().await;
        loop {
            tick.tick().await;
            if !collab_for_heartbeat.is_current(generation) {
                break;
            }
            collab_for_heartbeat.send_control(hardwave_project::multiplayer::SyncKind::Heartbeat);
        }
    }));

    Ok(CollabJoined {
        room_id,
        invite_code,
        caught_up,
    })
}

/// Limits this side keeps whatever the service does.
pub(crate) mod hardwave_room_limits {
    /// The largest message read from the room: the largest song plus
    /// its envelope.
    pub const MAX_MESSAGE: usize = 16 * 1024 * 1024 + 64 * 1024;
    /// The largest track sent on its own; the room refuses larger.
    pub const MAX_TRACK: usize = 4 * 1024 * 1024;
}

/// Answer "send me the song" with the song, and take one when it
/// arrives, each only when it was agreed to.
///
/// This is the one moment the whole project crosses the network. The
/// audio files do not travel with it: a clip whose sample the other
/// machine does not have reads as missing there, exactly as it does
/// when a project is copied between machines by hand.
fn handle_project(
    app: &tauri::AppHandle,
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    collab: &Arc<crate::collab::Collab>,
    message: &SyncMessage,
) -> bool {
    use hardwave_project::multiplayer::SyncKind;
    match &message.kind {
        SyncKind::ProjectRequest => {
            // Only the host sends, and only to someone it let in.
            if !collab.may_send_song() {
                log::info!(
                    "the other side asked for the song; this side has not agreed to send it"
                );
                return true;
            }
            let (name, blob) = {
                let engine_guard = engine.lock();
                let project = engine_guard.project.lock();
                match crate::collab::prepared_for_sending(&project).to_bytes() {
                    Ok(blob) => (project.metadata.name.clone(), blob),
                    Err(e) => {
                        log::warn!("could not pack the song to send: {e}");
                        return true;
                    }
                }
            };
            collab.send_control(SyncKind::ProjectOffer { name, blob });
            // Followed by each part with its version, so the other side
            // counts on from ours (see resend_song_parts).
            collab.resend_song_parts();
            true
        }
        SyncKind::ProjectOffer { name, blob } => {
            // Only a song this side asked for replaces what is open.
            if !collab.take_song_expectation() {
                log::warn!("a song arrived that was not asked for; it was not opened");
                return true;
            }
            match hardwave_project::Project::from_bytes(blob) {
                Ok(incoming) => {
                    take_room_song(&engine.lock(), collab, incoming);
                    // Its plug-ins too, made where plug-ins are made: the
                    // song arrived without any of them playing.
                    let for_main = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        let state = for_main.state::<AppState>();
                        if let Err(e) =
                            crate::commands::plugins::hydrate_chains_from_project(&state)
                        {
                            log::warn!("room: could not load the song's plug-ins: {e}");
                        }
                    });
                    tell_windows_song_taken(app, engine, name);
                    log::info!("took the song \"{name}\" from the other side");
                }
                Err(e) => log::warn!("could not read the song they sent: {e}"),
            }
            true
        }
        _ => false,
    }
}

/// Put the song the other side sent in place of the one open here, the
/// way opening a song does.
///
/// It was taken as an edit: an undo step, with the transport left at the
/// old song's tempo and time signature and the old song's plug-ins left
/// running on tracks with the same id. The song played at the wrong speed,
/// and Ctrl+Z brought the old song back, which the room then sent to the
/// other side as edits: their tracks replaced by these, their tempo and
/// markers overwritten.
pub(crate) fn take_room_song(
    engine: &hardwave_engine::DawEngine,
    collab: &crate::collab::Collab,
    incoming: hardwave_project::Project,
) {
    let bpm = incoming
        .tempo_map
        .entries
        .first()
        .map(|e| e.bpm)
        .unwrap_or(140.0);
    *engine.project.lock() = incoming;
    // Same queue as the plug-ins the song brings, so the order holds.
    engine.clear_live_plugins();
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.send_command(hardwave_engine::TransportCommand::SetBpm(bpm));
    crate::commands::project::apply_project_time_signature(engine);
    engine.reset_history();
    engine.sync_track_meters();
    engine.rebuild_graph();
    let master = engine
        .project
        .lock()
        .tracks
        .iter()
        .find(|t| matches!(t.kind, hardwave_project::TrackKind::Master))
        .map(|t| t.id.clone());
    if let Some(master) = master {
        engine.apply_track_mix(&master);
    }
    collab.remember_tracks(&engine.project.lock());
}

/// Every window shows the song taken from the room, as after opening one.
///
/// Nothing told them: the backend had the other side's song while the
/// playlist, the mixer, the channel rack, the markers and the tempo still
/// showed this side's old one, and the next edit in a window wrote its
/// stale copy back into the song and so into the room. The parts go by
/// the events a room edit of each already uses (taken there without being
/// sent back); `daw:songTaken` has the track list read again and the song
/// named as theirs, unsaved, so saving it asks where rather than writing
/// over the song this side had open.
fn tell_windows_song_taken(
    app: &tauri::AppHandle,
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    name: &str,
) {
    let (rack, timeline) = {
        let engine_guard = engine.lock();
        let project = engine_guard.project.lock();
        (
            project.channel_rack_state.clone(),
            project.timeline_state.clone(),
        )
    };
    let _ = app.emit(
        "daw:rackChanged",
        serde_json::json!({ "payload": rack, "from": "room" }),
    );
    let _ = app.emit("daw:timelineChanged", timeline);
    let _ = app.emit("daw:tempoChanged", ());
    for section in ["arrangements", "macros", "vcas", "modulations"] {
        let _ = app.emit("daw:songChanged", section);
    }
    let _ = app.emit("daw:songTaken", serde_json::json!({ "name": name }));
}

/// Apply one message and do what it asks of the transport.
fn apply_one(
    app: &tauri::AppHandle,
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    collab: &Arc<crate::collab::Collab>,
    message: &SyncMessage,
    generation: u64,
) {
    use hardwave_project::multiplayer::SyncKind;
    if !collab.is_current(generation) {
        return;
    }
    match &message.kind {
        SyncKind::TrackState {
            track_id,
            index,
            blob,
        } => {
            apply_track_state(app, engine, collab, track_id, *index, blob);
            return;
        }
        SyncKind::SongState {
            section,
            version,
            author,
            blob,
        } => {
            apply_song_state(app, engine, collab, section, *version, *author, blob);
            return;
        }
        SyncKind::TrackRemoved { track_id } => {
            let engine_guard = engine.lock();
            engine_guard.snapshot_before_mutation();
            engine_guard.project.lock().remove_track(track_id);
            collab.forget_track(track_id);
            engine_guard.sync_track_meters();
            engine_guard.rebuild_graph();
            drop(engine_guard);
            crate::commands::plugins::tell_windows_track_changed(app, track_id);
            return;
        }
        SyncKind::PluginParam {
            track_id,
            slot_id,
            param_id,
            value,
        } => {
            let _ = engine.lock().try_send_insert_command(
                hardwave_engine::insert_chain::InsertCommand::SetParameter {
                    track_id: track_id.clone(),
                    slot_id: slot_id.clone(),
                    param_id: *param_id,
                    value: *value,
                },
            );
            return;
        }
        _ => {}
    }
    // Who is in the room, who wants in, and where the other person is
    // change what is drawn, not the song.
    match &message.kind {
        SyncKind::MembersChanged { names } => {
            collab.set_members(names.clone());
            return;
        }
        SyncKind::PresenceUpdate(presence) => {
            collab.set_peer(presence);
            return;
        }
        SyncKind::JoinRequest { request_id, name } => {
            collab.set_join_request(request_id.clone(), name.clone());
            return;
        }
        SyncKind::CodeChanged { invite_code } => {
            collab.new_code(invite_code.clone());
            return;
        }
        SyncKind::Kick { .. } => {
            collab.stop("the host removed you from the room");
            return;
        }
        SyncKind::JoinAnswer { .. }
        | SyncKind::Heartbeat
        | SyncKind::VoiceFrame { .. }
        | SyncKind::HistorySnapshot { .. }
        | SyncKind::PermissionChange { .. } => return,
        _ => {}
    }
    // The song itself is not an edit, and it is handled before the
    // engine lock is taken because packing or unpacking it is slow.
    if handle_project(app, engine, collab, message) {
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
    // Only a song asked for is opened when it arrives.
    state.collab.expect_song();
    state
        .collab
        .send_control(hardwave_project::multiplayer::SyncKind::ProjectRequest);
    Ok(())
}

/// The host's answer to someone waiting to come in. Letting them in
/// also agrees to send them the song when they ask for it.
#[tauri::command]
pub fn answer_join(state: State<AppState>, request_id: String, admit: bool) -> Result<(), String> {
    state.collab.answer_join(&request_id, admit)
}

/// Tell the other person where you are working.
///
/// Called by the window when you click in the arrangement or open the
/// piano roll, at most a few times a second. Nothing is sent when you
/// are not in a room.
#[tauri::command]
pub fn share_presence(state: State<AppState>, tick: u64, track_index: Option<u32>, panel: String) {
    use hardwave_project::multiplayer::{Panel, Presence, SyncKind};
    let active_panel = match panel.as_str() {
        "pianoroll" => Panel::PianoRoll,
        "mixer" => Panel::Mixer,
        "channelrack" => Panel::ChannelRack,
        "browser" => Panel::Browser,
        _ => Panel::Arrangement,
    };
    state.collab.send(SyncKind::PresenceUpdate(Presence {
        active_panel,
        cursor_tick: tick,
        cursor_track_index: track_index,
        last_heartbeat_ms: 0,
    }));
}

/// A whole track the other side sent: put in place of ours (or added at
/// its place), its plug-ins brought in line, and remembered as what the
/// other side has so it is not sent back.
/// What the other side sent of the song level (tempo, channel rack,
/// markers, arrangements, macros, VCAs, modulation): taken into the song,
/// and every window told so it shows it. A tempo change or a pattern drawn
/// in the rack reached the other side only when the whole song was sent
/// again.
fn apply_song_state(
    app: &tauri::AppHandle,
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    collab: &Arc<crate::collab::Collab>,
    section: &str,
    version: u64,
    author: u64,
    blob: &[u8],
) {
    use std::sync::atomic::Ordering;
    let shown = {
        let engine_guard = engine.lock();
        let shown = {
            let mut project = engine_guard.project.lock();
            let Some(current) = project.song_section(section) else {
                log::warn!("room: not a part of the song: {section}");
                return;
            };
            // Ours stands when it is newer, or as new and from the higher
            // author: the other side takes ours by the same rule.
            if !collab.should_take_section(section, &current, blob, version, author) {
                return;
            }
            // Not an undo step: undoing it would only send the old part
            // back to the room as a new edit.
            if let Err(e) = project.put_song_section(section, blob) {
                log::warn!("room: the song's {section} were not taken: {e}");
                return;
            }
            let now = project.song_section(section).unwrap_or_default();
            collab.took_section(section, &now, version, author);
            match section {
                "rack" => Some(serde_json::json!({
                    "payload": project.channel_rack_state.clone(),
                    "from": "room",
                })),
                "timeline" => Some(serde_json::json!(project.timeline_state.clone())),
                _ => None,
            }
        };
        if section == "tempo" {
            let bpm = engine_guard
                .project
                .lock()
                .tempo_map
                .entries
                .first()
                .map(|e| e.bpm)
                .unwrap_or(140.0);
            engine_guard.transport.bpm.store(bpm, Ordering::Relaxed);
            engine_guard.send_command(hardwave_engine::TransportCommand::SetBpm(bpm));
            crate::commands::project::apply_project_time_signature(&engine_guard);
        }
        engine_guard.rebuild_graph();
        shown
    };
    // The windows show what the room sent.
    match section {
        "rack" => {
            let _ = app.emit("daw:rackChanged", shown);
        }
        "timeline" => {
            let _ = app.emit("daw:timelineChanged", shown);
        }
        "tempo" => {
            let _ = app.emit("daw:tempoChanged", ());
        }
        // The arrangement list, the macros, the VCAs and the modulation
        // read the song again.
        _ => {
            let _ = app.emit("daw:songChanged", section);
        }
    }
}

fn apply_track_state(
    app: &tauri::AppHandle,
    engine: &Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    collab: &Arc<crate::collab::Collab>,
    track_id: &str,
    index: usize,
    blob: &[u8],
) {
    let (new, old) = {
        let engine_guard = engine.lock();
        engine_guard.snapshot_before_mutation();
        let mut project = engine_guard.project.lock();
        match project.put_track_blob(blob, index) {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("room: track {track_id} not taken: {e}");
                return;
            }
        }
    };
    collab.remember_track(&new.id, &hardwave_project::Project::track_blob(&new));
    {
        let engine_guard = engine.lock();
        engine_guard.sync_track_meters();
        engine_guard.rebuild_graph();
    }
    let old_slots = old.map(|t| t.inserts).unwrap_or_default();
    if old_slots
        .iter()
        .map(|s| (&s.id, &s.plugin_id, &s.state, s.enabled, s.wet))
        .ne(new
            .inserts
            .iter()
            .map(|s| (&s.id, &s.plugin_id, &s.state, s.enabled, s.wet)))
    {
        let for_main = app.clone();
        let id = new.id.clone();
        let new_slots = new.inserts.clone();
        let _ = app.run_on_main_thread(move || {
            let state = for_main.state::<AppState>();
            crate::commands::plugins::sync_track_chain(&state, &id, &old_slots, &new_slots);
        });
    }
    crate::commands::plugins::tell_windows_track_changed(app, &new.id);
}

#[cfg(test)]
mod room_tls_tests {
    use super::room_tls;

    /// The TLS set-up builds: rustls finds its crypto provider and the roots load.
    #[test]
    fn the_room_connection_has_its_tls() {
        assert!(matches!(
            room_tls(),
            tokio_tungstenite::Connector::Rustls(_)
        ));
    }

    /// The real handshake with the room service, without signing in: the service
    /// answers 401, which proves the certificate was accepted. Needs the network.
    #[ignore]
    #[tokio::test]
    async fn the_room_service_certificate_is_accepted() {
        let result = tokio_tungstenite::connect_async_tls_with_config(
            "wss://rooms.hardwavestudios.com/room",
            None,
            false,
            Some(room_tls()),
        )
        .await;
        match result {
            Ok(_) => {}
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert!(response.status().is_client_error(), "{}", response.status())
            }
            Err(e) => panic!("TLS or connection failed: {e}"),
        }
    }
}

#[cfg(test)]
mod room_song_tests {
    use super::take_room_song;
    use std::sync::atomic::Ordering;

    /// A song taken from the room plays at its own tempo, and undo cannot
    /// bring the song this side had before back (the room would send it to
    /// the other side as edits).
    #[test]
    fn a_song_taken_from_the_room_opens_like_a_song_and_is_not_an_undo_step() {
        let engine = hardwave_engine::DawEngine::new();
        let collab = crate::collab::Collab::default();
        // An edit of this side's own, before the song arrives.
        engine.snapshot_before_mutation();
        engine.project.lock().tempo_map.entries[0].bpm = 128.0;

        let mut theirs = hardwave_project::Project::default();
        theirs.tempo_map.entries[0].bpm = 150.0;
        take_room_song(&engine, &collab, theirs);

        assert_eq!(engine.transport.bpm.load(Ordering::Relaxed), 150.0);
        assert!(!engine.undo(), "the old song must not come back");
        assert_eq!(engine.project.lock().tempo_map.entries[0].bpm, 150.0);
        // What arrived is what the other side has: nothing to send back.
        let project = engine.project.lock();
        assert!(collab.song_changes(&project).is_empty());
        assert!(collab.track_changes(&project).0.is_empty());
    }
}
