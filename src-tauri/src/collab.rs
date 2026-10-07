//! Working on a song with someone else, from this end.
//!
//! The rules are in `hardwave-room`, the service in
//! `hardwave-room-server`, and turning a message back into an edit is
//! in `hardwave-project`. What is left, and what lives here, is the
//! connection: send what this person does, apply what the other one
//! does, and say which.
//!
//! One rule runs through all of it: an edit that arrived from the
//! other side is never sent back out. Without that, two DAWs hand the
//! same fader change back and forth until one of them gives up.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use hardwave_project::multiplayer::{LogicalClock, SyncKind, SyncMessage, TransportSync};
use hardwave_project::multiplayer_apply;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::mpsc;

/// What the UI shows about the room.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollabStatus {
    pub connected: bool,
    pub room_id: String,
    /// What to read out to the other person.
    pub invite_code: String,
    /// Whether this side opened the room, which is the side that
    /// needs Pro.
    pub hosting: bool,
    /// Empty when all is well, otherwise why it stopped.
    pub message: String,
    /// How many edits have come in and gone out, so the panel can
    /// show that something is actually happening.
    pub received: u64,
    pub sent: u64,
    /// Who is in the room, host first, as the room last said.
    pub members: Vec<String>,
    /// Where the other person is working, as they last said.
    pub peer: Option<PeerCursor>,
}

/// Where the other person is in the song.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerCursor {
    pub name: String,
    pub tick: u64,
    pub track_index: Option<u32>,
    /// Which window they have in front of them, so "they are in the
    /// piano roll" can be said rather than guessed.
    pub panel: String,
}

/// The live session, or nothing when this DAW is on its own.
#[derive(Default)]
pub struct Collab {
    status: Mutex<CollabStatus>,
    outbox: Mutex<Option<mpsc::UnboundedSender<SyncMessage>>>,
    clock: Mutex<LogicalClock>,
    running: AtomicBool,
    received: AtomicU64,
    sent: AtomicU64,
    /// Set while a message from the other side is being applied, so
    /// the edit it causes is not sent straight back to them.
    applying: AtomicBool,
}

impl Collab {
    pub fn status(&self) -> CollabStatus {
        let mut status = self.status.lock().clone();
        status.received = self.received.load(Ordering::Relaxed);
        status.sent = self.sent.load(Ordering::Relaxed);
        status.connected = self.running.load(Ordering::Relaxed);
        status
    }

    /// Send something this person did. Silently does nothing when
    /// there is no room, which is the normal case.
    pub fn send(&self, kind: SyncKind) {
        if !self.running.load(Ordering::Relaxed) || self.applying.load(Ordering::Relaxed) {
            return;
        }
        let Some(outbox) = self.outbox.lock().clone() else {
            return;
        };
        let logical_clock = self.clock.lock().tick();
        let message = SyncMessage {
            // The service replaces this with whoever the socket
            // signed in as, so what is put here does not matter.
            sender_user_id: String::new(),
            logical_clock,
            kind,
        };
        if outbox.send(message).is_ok() {
            self.sent.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A room is open: remember what it is and where to send.
    pub fn begin(
        &self,
        room_id: &str,
        invite_code: &str,
        hosting: bool,
        outbox: mpsc::UnboundedSender<SyncMessage>,
    ) {
        *self.outbox.lock() = Some(outbox);
        self.running.store(true, Ordering::Relaxed);
        self.received.store(0, Ordering::Relaxed);
        self.sent.store(0, Ordering::Relaxed);
        let mut status = self.status.lock();
        status.connected = true;
        status.members = Vec::new();
        status.peer = None;
        status.room_id = room_id.to_string();
        status.invite_code = invite_code.to_string();
        status.hosting = hosting;
        status.message = String::new();
    }

    /// The room says who is in it now.
    pub fn set_members(&self, names: Vec<String>) {
        self.status.lock().members = names;
    }

    /// The other person moved. With two in a room the other person is
    /// whoever this side is not: the guest if this side opened the
    /// room, the host otherwise.
    pub fn set_peer(&self, presence: &hardwave_project::multiplayer::Presence) {
        let mut status = self.status.lock();
        let name = if status.hosting {
            status.members.get(1).cloned()
        } else {
            status.members.first().cloned()
        }
        .unwrap_or_else(|| "The other person".to_string());
        status.peer = Some(PeerCursor {
            name,
            tick: presence.cursor_tick,
            track_index: presence.cursor_track_index,
            panel: format!("{:?}", presence.active_panel),
        });
    }

    pub fn stop(&self, why: &str) {
        self.running.store(false, Ordering::Relaxed);
        *self.outbox.lock() = None;
        let mut status = self.status.lock();
        status.connected = false;
        status.message = why.to_string();
    }
}

/// Apply one message from the other side.
///
/// Returns what the caller still has to do with the transport. While
/// this runs, anything the apply touches must not be sent back out,
/// which is what the flag is for.
pub fn apply_incoming(
    collab: &Collab,
    project: &mut hardwave_project::Project,
    message: &SyncMessage,
) -> Option<TransportSync> {
    collab.applying.store(true, Ordering::Relaxed);
    let applied = multiplayer_apply::apply(project, message);
    collab.applying.store(false, Ordering::Relaxed);
    collab.received.fetch_add(1, Ordering::Relaxed);
    applied.transport
}

/// The account token every Hardwave program on this machine shares.
///
/// The plug-ins and the Suite write it when you sign in, so the DAW
/// does not ask again: if you are signed in anywhere, you are signed
/// in here.
pub fn load_token() -> Option<String> {
    let path = dirs::data_dir()?.join("hardwave").join("auth_token");
    std::fs::read_to_string(path)
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

/// Where the room service lives. Ours, unless a tester points it
/// somewhere else.
pub fn service_url() -> String {
    std::env::var("HARDWAVE_ROOM_URL")
        .unwrap_or_else(|_| "wss://rooms.hardwavestudios.com/room".to_string())
}

/// Build the URL for opening or joining a room.
pub fn join_url(token: &str, room: &str, code: &str, since: u64) -> String {
    let mut url = format!(
        "{}?token={}&since={}",
        service_url(),
        urlencode(token),
        since
    );
    if !room.is_empty() {
        url.push_str(&format!("&room={}", urlencode(room)));
    }
    if !code.is_empty() {
        url.push_str(&format!("&code={}", urlencode(code)));
    }
    url
}

/// Enough escaping for a token and a room id, which are the only
/// things that go in. A dependency for this would be a dependency
/// for nothing.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// What the service says first.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Hello {
    Joined {
        room_id: String,
        invite_code: String,
        #[serde(default)]
        missed: Vec<SyncMessage>,
        #[serde(default)]
        caught_up: bool,
    },
    Refused {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::multiplayer::MixerSync;

    fn fader() -> SyncKind {
        SyncKind::Mixer(MixerSync {
            track_id: "kick".into(),
            volume_db: Some(-3.0),
            pan: None,
            muted: None,
        })
    }

    #[test]
    fn with_no_room_open_nothing_is_sent() {
        let collab = Collab::default();
        collab.send(fader());
        assert_eq!(collab.status().sent, 0);
        assert!(!collab.status().connected);
    }

    #[test]
    fn what_came_from_them_is_not_sent_back_to_them() {
        let collab = Collab::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        *collab.outbox.lock() = Some(tx);
        collab.running.store(true, Ordering::Relaxed);

        // An ordinary local change goes out.
        collab.send(fader());
        assert_eq!(collab.status().sent, 1);
        assert!(rx.try_recv().is_ok());

        // The same change, while applying what they sent, does not.
        let mut project = hardwave_project::Project::default();
        let track_id = project.add_midi_track("Kick".into());
        let incoming = SyncMessage {
            sender_user_id: "them".into(),
            logical_clock: 7,
            kind: SyncKind::Mixer(MixerSync {
                track_id: track_id.clone(),
                volume_db: Some(-9.0),
                pan: None,
                muted: None,
            }),
        };
        collab.applying.store(true, Ordering::Relaxed);
        collab.send(fader());
        collab.applying.store(false, Ordering::Relaxed);
        assert_eq!(collab.status().sent, 1, "no echo back to the sender");

        // And applying it really does change the song.
        apply_incoming(&collab, &mut project, &incoming);
        assert_eq!(project.track(&track_id).unwrap().volume_db, -9.0);
        assert_eq!(collab.status().received, 1);
    }

    #[test]
    fn the_clock_moves_on_with_every_message() {
        let collab = Collab::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        *collab.outbox.lock() = Some(tx);
        collab.running.store(true, Ordering::Relaxed);
        collab.send(fader());
        collab.send(fader());
        let first = rx.try_recv().unwrap().logical_clock;
        let second = rx.try_recv().unwrap().logical_clock;
        assert!(second > first, "{second} should come after {first}");
    }

    #[test]
    fn the_url_carries_what_the_service_asks_for() {
        std::env::set_var("HARDWAVE_ROOM_URL", "ws://127.0.0.1:8787/room");
        let url = join_url("tok en/1", "room-5", "ABC-123", 42);
        assert!(url.starts_with("ws://127.0.0.1:8787/room?token=tok%20en%2F1&since=42"));
        assert!(url.contains("&room=room-5"));
        assert!(url.contains("&code=ABC-123"));
        std::env::remove_var("HARDWAVE_ROOM_URL");
    }

    #[test]
    fn stopping_says_why_and_stops_sending() {
        let collab = Collab::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        *collab.outbox.lock() = Some(tx);
        collab.running.store(true, Ordering::Relaxed);
        collab.stop("the other side closed the room");
        collab.send(fader());
        let status = collab.status();
        assert!(!status.connected);
        assert_eq!(status.sent, 0);
        assert_eq!(status.message, "the other side closed the room");
    }
}
