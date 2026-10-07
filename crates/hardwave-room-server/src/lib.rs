//! The service two DAWs meet in.
//!
//! Small on purpose. It holds rooms in memory, asks the site whether
//! the account signing in may use this, and passes messages from one
//! peer to the other. The rules themselves live in `hardwave-room`,
//! where they can be tested without a socket; this file is the socket.
//!
//! Nothing here stores a song. A room that empties is forgotten, and
//! the project stays on the two machines that are working on it.
//!
//! The service is reachable by anyone, so it is built to stay up when
//! someone tries to knock it over: a cap on connections and rooms, one
//! room per host, a size limit on every message, a rate limit per
//! connection, and a connection that goes quiet is closed. The account
//! token arrives in a header, never in the address, so it cannot end up
//! in a log.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use hardwave_project::multiplayer::{SyncKind, SyncMessage};
use hardwave_room::{JoinRefusal, Knock, LiveRoom};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot, Notify, OwnedSemaphorePermit, Semaphore};

pub use hardwave_account as account;

/// How much the service takes before it says no.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Connections open at once, across every room.
    pub max_sockets: usize,
    /// Rooms open at once.
    pub max_rooms: usize,
    /// The largest single message, which is the largest song.
    pub max_message: usize,
    /// A connection that says nothing for this long is closed. The DAW
    /// sends a heartbeat well inside it.
    pub idle: Duration,
    /// How long a guest waits for the host to answer.
    pub admit_wait: Duration,
    /// Messages per second a connection may send, sustained.
    pub per_second: f64,
    /// And in a burst.
    pub burst: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_sockets: 400,
            max_rooms: 150,
            max_message: hardwave_room::MAX_PROJECT_BYTES + 64 * 1024,
            idle: Duration::from_secs(90),
            admit_wait: Duration::from_secs(120),
            per_second: 40.0,
            burst: 200.0,
        }
    }
}

/// How the DAW asks to be let in. Neither the account token nor the
/// invite code is here: addresses end up in logs, so both travel in
/// headers (Authorization, and X-Invite-Code).
#[derive(Debug, Deserialize)]
struct JoinQuery {
    /// The room to open or to join. Empty means "open a new one".
    #[serde(default)]
    room: String,
    #[serde(skip)]
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
        /// False when you have fallen too far behind and should ask
        /// the other side for the project again.
        caught_up: bool,
    },
    /// The code was right; the host is being asked.
    Waiting,
    Refused {
        reason: String,
    },
}

/// Someone let in: their room, its name, and the switch that closes
/// their connection.
type Admitted = (Arc<Mutex<Session>>, String, Arc<Notify>);

pub struct Session {
    live: LiveRoom,
    /// Everything said in this room, for whoever is listening.
    chatter: broadcast::Sender<(String, SyncMessage)>,
    /// Each member's connection, so it can be closed: when they are
    /// removed, or when they connect again from somewhere else.
    connections: HashMap<String, Arc<Notify>>,
    /// Guests waiting for the host, by request.
    knocks: HashMap<String, oneshot::Sender<bool>>,
}

#[derive(Clone)]
pub struct Rooms {
    rooms: Arc<Mutex<HashMap<String, Arc<Mutex<Session>>>>>,
    /// The room each host has open: one each.
    hosted: Arc<Mutex<HashMap<String, String>>>,
    /// Where the account check is asked. The live service asks the
    /// site; a test asks its own stand-in.
    site: Arc<String>,
    sockets: Arc<Semaphore>,
    limits: Arc<Limits>,
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim()
        .to_string()
}

async fn open_socket(
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
    Query(mut query): Query<JoinQuery>,
    State(rooms): State<Rooms>,
) -> axum::response::Response {
    query.code = headers
        .get("x-invite-code")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let Ok(permit) = Arc::clone(&rooms.sockets).try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "the room service is full; try again in a minute",
        )
            .into_response();
    };
    let token = bearer(&headers);
    let max = rooms.limits.max_message;
    upgrade
        .max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| handle(socket, token, query, rooms, permit))
}

async fn say(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    hello: &Hello,
) -> bool {
    sender
        .send(Message::Text(
            serde_json::to_string(hello).unwrap_or_default().into(),
        ))
        .await
        .is_ok()
}

fn refusal_text(refusal: JoinRefusal) -> &'static str {
    match refusal {
        JoinRefusal::Full => "there are already two people in that room",
        JoinRefusal::WrongCode => "that invite code does not match",
        JoinRefusal::NotSubscribed => "opening a room to work together is part of Hardwave Pro",
        JoinRefusal::NotSignedIn => "sign in to join a room",
        JoinRefusal::Busy => "someone else is waiting for the host; try again in a minute",
        JoinRefusal::Declined => "the host did not let you in",
    }
}

/// Say something to everyone in a room, as the room.
fn from_room(session: &Session, kind: SyncKind) {
    let message = SyncMessage {
        sender_user_id: "room".to_string(),
        logical_clock: session.live.highest_clock,
        kind,
    };
    let _ = session.chatter.send(("room".to_string(), message));
}

/// Tell everyone in the room who is in it.
fn announce_members(session: &Session) {
    from_room(
        session,
        SyncKind::MembersChanged {
            names: session.live.member_names(),
        },
    );
}

/// Close a room and everyone's connection to it.
fn close_room(rooms: &Rooms, room_id: &str) {
    if let Some(session) = rooms.rooms.lock().remove(room_id) {
        let session = session.lock();
        for notify in session.connections.values() {
            notify.notify_one();
        }
    }
}

async fn handle(
    socket: WebSocket,
    token: String,
    query: JoinQuery,
    rooms: Rooms,
    _permit: OwnedSemaphorePermit,
) {
    let (mut sender, mut receiver) = socket.split();

    // Who is this, and may they? The site answers both; nothing the
    // client sends about itself is believed.
    let checked = if token.is_empty() {
        Err("sign in to work on a song together".to_string())
    } else {
        account::identify(&rooms.site, &token).await
    };
    let who = match checked {
        Ok(who) => who,
        Err(reason) => {
            say(&mut sender, &Hello::Refused { reason }).await;
            return;
        }
    };

    // In, as host or guest, or out with a reason.
    let admitted = if query.room.is_empty() {
        open_room(&rooms, &who)
    } else {
        join_room(&rooms, &who, &query, &mut sender, &mut receiver).await
    };
    let (session, room_id, closer) = match admitted {
        Ok(admitted) => admitted,
        Err(reason) => {
            say(&mut sender, &Hello::Refused { reason }).await;
            return;
        }
    };

    let (hello, mut inbox) = {
        let guard = session.lock();
        let missed = guard.live.catch_up(query.since);
        let hello = Hello::Joined {
            room_id: room_id.clone(),
            invite_code: guard.live.invite_code.clone(),
            caught_up: missed.is_some(),
            missed: missed.unwrap_or_default(),
        };
        let inbox = guard.chatter.subscribe();
        announce_members(&guard);
        (hello, inbox)
    };
    if !say(&mut sender, &hello).await {
        leave(&rooms, &session, &room_id, &who.user_id, &closer);
        return;
    }

    // Out: everything the other person says, and what the room says.
    let me = who.user_id.clone();
    let mut to_peer = tokio::spawn(async move {
        loop {
            match inbox.recv().await {
                Ok((from, message)) => {
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
                // Too slow to keep up: skip ahead rather than hang up.
                // The DAW asks for the song again if it needs to.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // In: everything this person does.
    let mine = who.user_id.clone();
    let room_for_messages = Arc::clone(&session);
    let limits = Arc::clone(&rooms.limits);
    let mut from_peer = tokio::spawn(async move {
        let mut tokens = limits.burst;
        let mut refilled = Instant::now();
        loop {
            let next = tokio::time::timeout(limits.idle, receiver.next()).await;
            let Ok(Some(Ok(message))) = next else {
                break;
            };
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(mut parsed) = serde_json::from_str::<SyncMessage>(&text) else {
                continue;
            };
            // A token bucket: a burst is fine, a flood is not. A song
            // costs more than an edit.
            tokens =
                (tokens + refilled.elapsed().as_secs_f64() * limits.per_second).min(limits.burst);
            refilled = Instant::now();
            let cost = if matches!(parsed.kind, SyncKind::ProjectOffer { .. }) {
                100.0
            } else {
                1.0
            };
            if tokens < cost {
                continue;
            }
            tokens -= cost;
            // The sender is who the socket says they are, not who the
            // message claims to be.
            parsed.sender_user_id = mine.clone();
            let mut guard = room_for_messages.lock();
            if let SyncKind::JoinAnswer { request_id, admit } = &parsed.kind {
                if guard.live.is_host(&mine) {
                    let admitted = guard.live.answer(request_id, *admit).is_ok();
                    if let Some(waiting) = guard.knocks.remove(request_id) {
                        let _ = waiting.send(admitted);
                    }
                }
                continue;
            }
            let relay = guard.live.handle(&parsed);
            if relay.forward {
                let _ = guard.chatter.send((mine.clone(), parsed.clone()));
            }
            if let Some(gone) = relay.disconnect {
                if let Some(notify) = guard.connections.remove(&gone) {
                    notify.notify_one();
                }
                announce_members(&guard);
                // A kick replaces the code; the host needs the new one.
                let code = guard.live.invite_code.clone();
                from_room(&guard, SyncKind::CodeChanged { invite_code: code });
            }
            drop(guard);
        }
    });

    // Whichever side ends first, or the room closing this connection,
    // ends it.
    tokio::select! {
        _ = &mut to_peer => from_peer.abort(),
        _ = &mut from_peer => to_peer.abort(),
        _ = closer.notified() => { to_peer.abort(); from_peer.abort(); }
    }
    leave(&rooms, &session, &room_id, &who.user_id, &closer);
}

/// Open a room for a host.
fn open_room(rooms: &Rooms, who: &account::Who) -> Result<Admitted, String> {
    // Opening a room is the paid half, and it is checked before
    // anything is made: a refused host leaves nothing behind.
    if !who.subscribed {
        return Err("opening a room to work together is part of Hardwave Pro. \
                    Joining someone else's room is free."
            .to_string());
    }
    // One room per host. Opening another closes the old one, which is
    // what happens when a DAW reconnects after its machine slept.
    let previous = rooms.hosted.lock().remove(&who.user_id);
    if let Some(previous) = previous {
        close_room(rooms, &previous);
    }
    let mut map = rooms.rooms.lock();
    if map.len() >= rooms.limits.max_rooms {
        return Err("the room service is busy; try again in a few minutes".into());
    }
    let mut live = LiveRoom::open(
        who.user_id.clone(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    );
    if let Some(host) = live.room.member_mut(&who.user_id) {
        host.display_name = who.display_name.clone();
    }
    // Eight random characters rarely meet again, but never let a new
    // room take an open one's place.
    while map.contains_key(&live.room.room_id) {
        live.room.room_id = hardwave_project::multiplayer::new_room_id();
    }
    let room_id = live.room.room_id.clone();
    let closer = Arc::new(Notify::new());
    let (chatter, _) = broadcast::channel(256);
    let mut connections = HashMap::new();
    connections.insert(who.user_id.clone(), Arc::clone(&closer));
    let session = Arc::new(Mutex::new(Session {
        live,
        chatter,
        connections,
        knocks: HashMap::new(),
    }));
    map.insert(room_id.clone(), Arc::clone(&session));
    drop(map);
    rooms
        .hosted
        .lock()
        .insert(who.user_id.clone(), room_id.clone());
    tracing::info!("room opened");
    Ok((session, room_id, closer))
}

/// Let a guest into a room, asking the host first.
async fn join_room(
    rooms: &Rooms,
    who: &account::Who,
    query: &JoinQuery,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    receiver: &mut SplitStream<WebSocket>,
) -> Result<Admitted, String> {
    let session = rooms
        .rooms
        .lock()
        .get(&query.room)
        .map(Arc::clone)
        .ok_or_else(|| "that room is not open any more".to_string())?;

    let answer = {
        let mut guard = session.lock();
        let code_before = guard.live.invite_code.clone();
        let knock = guard.live.knock(
            who.user_id.clone(),
            who.display_name.clone(),
            &query.code,
            true,
        );
        if guard.live.invite_code != code_before {
            // Too many wrong codes: the old one is dead, and the host
            // has to hear the new one.
            let code = guard.live.invite_code.clone();
            from_room(&guard, SyncKind::CodeChanged { invite_code: code });
        }
        match knock.map_err(|r| refusal_text(r).to_string())? {
            Knock::Back => None,
            Knock::AskHost { request_id } => {
                let (tell, hear) = oneshot::channel();
                guard.knocks.insert(request_id.clone(), tell);
                let name = guard.live.waiting_name(&request_id).unwrap_or_default();
                from_room(
                    &guard,
                    SyncKind::JoinRequest {
                        request_id: request_id.clone(),
                        name,
                    },
                );
                Some((request_id, hear))
            }
        }
    };

    if let Some((request_id, hear)) = answer {
        say(sender, &Hello::Waiting).await;
        // Wait for the host, but not forever, and not for a guest who
        // already gave up.
        let gave_up = async {
            loop {
                match receiver.next().await {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => continue,
                }
            }
        };
        let outcome = tokio::select! {
            answered = hear => answered.unwrap_or(false),
            _ = tokio::time::sleep(rooms.limits.admit_wait) => false,
            _ = gave_up => false,
        };
        if !outcome {
            let mut guard = session.lock();
            guard.knocks.remove(&request_id);
            guard.live.withdraw(&request_id);
            return Err(refusal_text(JoinRefusal::Declined).into());
        }
    }

    let room_id = query.room.clone();
    let closer = Arc::new(Notify::new());
    let mut guard = session.lock();
    if guard.live.room.member(&who.user_id).is_none() {
        return Err("that room is not open any more".into());
    }
    // Connected again from somewhere else: the old connection goes.
    if let Some(old) = guard
        .connections
        .insert(who.user_id.clone(), Arc::clone(&closer))
    {
        old.notify_one();
    }
    drop(guard);
    Ok((session, room_id, closer))
}

/// Leaving, and closing the room behind the last person out.
fn leave(
    rooms: &Rooms,
    session: &Arc<Mutex<Session>>,
    room_id: &str,
    user_id: &str,
    closer: &Arc<Notify>,
) {
    let mut map = rooms.rooms.lock();
    let mut guard = session.lock();
    // Only if this connection is still theirs: a newer one from the
    // same person stays.
    let current = guard
        .connections
        .get(user_id)
        .is_some_and(|c| Arc::ptr_eq(c, closer));
    if current {
        guard.connections.remove(user_id);
        // The host leaving closes the room for both: the song is theirs.
        if guard.live.is_host(user_id) {
            for notify in guard.connections.values() {
                notify.notify_one();
            }
            guard.connections.clear();
            let members: Vec<String> = guard
                .live
                .room
                .members
                .iter()
                .map(|m| m.user_id.clone())
                .collect();
            for m in members {
                guard.live.leave(&m);
            }
        } else {
            guard.live.leave(user_id);
        }
    }
    let empty = guard.live.is_empty();
    if !empty && current {
        announce_members(&guard);
    }
    drop(guard);
    if empty {
        let is_this_room = map.get(room_id).is_some_and(|s| Arc::ptr_eq(s, session));
        if is_this_room {
            map.remove(room_id);
        }
        let mut hosted = rooms.hosted.lock();
        hosted.retain(|_, r| r != room_id);
        tracing::info!("room closed");
    }
}

/// The service, without a port of its own, so a test can drive it
/// over a socket it chose.
pub fn router() -> Router {
    router_for_site(account::site_from_environment())
}

/// The same service, asking a site of your choosing about accounts.
pub fn router_for_site(site: impl Into<String>) -> Router {
    router_with(site, Limits::default())
}

/// The same service with limits of your choosing, for tests that need
/// to reach them quickly.
pub fn router_with(site: impl Into<String>, limits: Limits) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/room", get(open_socket))
        .with_state(Rooms {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            hosted: Arc::new(Mutex::new(HashMap::new())),
            site: Arc::new(site.into()),
            sockets: Arc::new(Semaphore::new(limits.max_sockets)),
            limits: Arc::new(limits),
        })
}
