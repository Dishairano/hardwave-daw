//! The service two DAWs meet in.
//!
//! Small on purpose. It holds rooms in memory, asks the site whether
//! the account signing in may use this, and passes messages from one
//! peer to the other. The rules themselves live in `hardwave-room`,
//! where they can be tested without a socket; this file is the socket.
//!
//! Nothing here stores a song. A room that empties is forgotten, and
//! the project stays on the two machines that are working on it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use hardwave_project::multiplayer::SyncMessage;
use hardwave_room::{JoinRefusal, LiveRoom};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub mod account;

/// How the DAW asks to be let in.
#[derive(Debug, Deserialize)]
struct JoinQuery {
    /// The account token the plug-ins and the DAW already share.
    /// Optional here so a connection without one is answered with a
    /// reason rather than a bare 400 from the query parser.
    #[serde(default)]
    token: String,
    /// The room to open or to join. Empty means "open a new one".
    #[serde(default)]
    room: String,
    /// The invite code, when joining someone else's room.
    #[serde(default)]
    code: String,
    /// What this peer last saw, so it can be caught up.
    #[serde(default)]
    since: u64,
}

/// What the service says before anything else.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Hello {
    /// You are in. The code is what you send the other person.
    Joined {
        room_id: String,
        invite_code: String,
        /// What you missed, oldest first.
        missed: Vec<SyncMessage>,
        /// Empty when you have fallen too far behind and should ask
        /// the other side for the project again.
        caught_up: bool,
    },
    Refused {
        reason: String,
    },
}

pub struct Session {
    live: LiveRoom,
    /// Everything said in this room, for whoever is listening.
    chatter: broadcast::Sender<(String, SyncMessage)>,
}

#[derive(Clone)]
pub struct Rooms {
    rooms: Arc<Mutex<HashMap<String, Arc<Mutex<Session>>>>>,
    /// Where the account check is asked. The live service asks the
    /// site; a test asks its own stand-in.
    site: Arc<String>,
}

async fn open_socket(
    upgrade: WebSocketUpgrade,
    Query(query): Query<JoinQuery>,
    State(rooms): State<Rooms>,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| handle(socket, query, rooms))
}

async fn handle(socket: WebSocket, query: JoinQuery, rooms: Rooms) {
    let (mut sender, mut receiver) = socket.split();

    // Who is this, and may they? The site answers both; nothing the
    // client sends about itself is believed.
    let who = match account::identify(&rooms.site, &query.token).await {
        Ok(who) => who,
        Err(reason) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&Hello::Refused { reason })
                        .unwrap_or_default()
                        .into(),
                ))
                .await;
            return;
        }
    };

    // Everything that touches a lock happens in here, and nothing in
    // here waits on the network: a lock held across an await would
    // hold up every other room on the machine.
    enum Admission {
        In {
            room_id: String,
            invite_code: String,
            missed: Vec<SyncMessage>,
            caught_up: bool,
            inbox: broadcast::Receiver<(String, SyncMessage)>,
            session: Arc<Mutex<Session>>,
        },
        Out(String),
    }

    let mut return_refused: Option<String> = None;
    let admission = {
        let mut map = rooms.rooms.lock();
        let session = if query.room.is_empty() {
            // Opening a room is the paid half. A guest joining one
            // needs an account and nothing else, which is how one
            // subscription brings a second producer into the DAW.
            if !who.subscribed {
                return_refused = Some(
                    "opening a room to work together is part of Hardwave Pro. \
                     Joining someone else's room is free."
                        .to_string(),
                );
            }
            let live = LiveRoom::open(
                who.user_id.clone(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            );
            let mut live = live;
            if let Some(host) = live.room.member_mut(&who.user_id) {
                host.display_name = who.display_name.clone();
            }
            let id = live.room.room_id.clone();
            let (chatter, _) = broadcast::channel(256);
            let session = Arc::new(Mutex::new(Session { live, chatter }));
            map.insert(id, Arc::clone(&session));
            Some(session)
        } else {
            map.get(&query.room).map(Arc::clone)
        };
        drop(map);

        match (return_refused.take(), session) {
            (Some(reason), _) => Admission::Out(reason),
            (None, None) => Admission::Out("that room is not open any more".into()),
            (None, Some(session)) => {
                let mut guard = session.lock();
                // Opening a room needs no code of its own: the host
                // already has it, and is about to read it out to the
                // other person.
                let code = if query.room.is_empty() {
                    guard.live.invite_code.clone()
                } else {
                    query.code.clone()
                };
                match guard.live.join(
                    who.user_id.clone(),
                    who.display_name.clone(),
                    &code,
                    // Signed in is enough to join; the host pays.
                    true,
                ) {
                    Err(refusal) => Admission::Out(
                        match refusal {
                            JoinRefusal::Full => "there are already two people in that room",
                            JoinRefusal::WrongCode => "that invite code does not match",
                            JoinRefusal::NotSubscribed => {
                                "opening a room to work together is part of Hardwave Pro"
                            }
                            JoinRefusal::NotSignedIn => "sign in to join a room",
                        }
                        .to_string(),
                    ),
                    Ok(()) => {
                        let missed = guard.live.catch_up(query.since);
                        let inbox = guard.chatter.subscribe();
                        announce_members(&guard);
                        let room_id = guard.live.room.room_id.clone();
                        let invite_code = guard.live.invite_code.clone();
                        drop(guard);
                        Admission::In {
                            room_id,
                            invite_code,
                            missed: missed.clone().unwrap_or_default(),
                            caught_up: missed.is_some(),
                            inbox,
                            session,
                        }
                    }
                }
            }
        }
    };

    let (room_id, invite_code, missed, caught_up, mut inbox, session) = match admission {
        Admission::Out(reason) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&Hello::Refused { reason })
                        .unwrap_or_default()
                        .into(),
                ))
                .await;
            return;
        }
        Admission::In {
            room_id,
            invite_code,
            missed,
            caught_up,
            inbox,
            session,
        } => (room_id, invite_code, missed, caught_up, inbox, session),
    };

    let hello = Hello::Joined {
        room_id: room_id.clone(),
        invite_code,
        missed,
        caught_up,
    };
    if sender
        .send(Message::Text(
            serde_json::to_string(&hello).unwrap_or_default().into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // Out: everything the other person says.
    let me = who.user_id.clone();
    let mut to_peer = tokio::spawn(async move {
        while let Ok((from, message)) = inbox.recv().await {
            if from == me {
                continue;
            }
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // In: everything this person does.
    let mine = who.user_id.clone();
    let room_for_messages = Arc::clone(&session);
    let mut from_peer = tokio::spawn(async move {
        while let Some(Ok(message)) = receiver.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(mut parsed) = serde_json::from_str::<SyncMessage>(&text) else {
                continue;
            };
            // The sender is who the socket says they are, not who the
            // message claims to be.
            parsed.sender_user_id = mine.clone();
            let mut guard = room_for_messages.lock();
            let relay = guard.live.handle(&parsed);
            if relay.forward {
                let _ = guard.chatter.send((mine.clone(), parsed));
            }
        }
    });

    // Whichever side ends first, the other is no longer needed.
    tokio::select! {
        _ = &mut to_peer => from_peer.abort(),
        _ = &mut from_peer => to_peer.abort(),
    }

    // Leaving, and closing the room behind the last person out.
    let mut map = rooms.rooms.lock();
    let mut guard = session.lock();
    guard.live.leave(&who.user_id);
    let empty = guard.live.is_empty();
    if !empty {
        announce_members(&guard);
    }
    drop(guard);
    if empty {
        map.remove(&room_id);
        tracing::info!("room {room_id} closed");
    }
}

/// Tell everyone in the room who is in it.
///
/// Sent as coming from the room itself rather than from a person, so
/// every peer receives it, the one who just arrived included.
fn announce_members(session: &Session) {
    let message = SyncMessage {
        sender_user_id: "room".to_string(),
        logical_clock: session.live.highest_clock,
        kind: hardwave_project::multiplayer::SyncKind::MembersChanged {
            names: session.live.member_names(),
        },
    };
    let _ = session.chatter.send(("room".to_string(), message));
}

/// The service, without a port of its own, so a test can drive it
/// over a socket it chose.
pub fn router() -> Router {
    router_for_site(account::site_from_environment())
}

/// The same service, asking a site of your choosing about accounts.
pub fn router_for_site(site: impl Into<String>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/room", get(open_socket))
        .with_state(Rooms {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            site: Arc::new(site.into()),
        })
}
