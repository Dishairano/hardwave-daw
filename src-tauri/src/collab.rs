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
    /// A guest with the right code is waiting for this side, the host,
    /// to say yes.
    pub join_request: Option<JoinAsk>,
    /// This side has the right code and is waiting for the host.
    pub waiting_for_host: bool,
}

/// Someone asking to come into the room.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinAsk {
    pub request_id: String,
    pub name: String,
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
    /// The host let someone in, which is agreeing to send them the song
    /// when they ask. Without it a request for the song is ignored.
    song_consent: AtomicBool,
    /// This side asked for the song. A song that arrives without being
    /// asked for is not opened.
    awaiting_song: AtomicBool,
    /// Which connection is current. Messages from an older one, still
    /// arriving after a stop, are dropped.
    generation: AtomicU64,
    /// The connection's tasks, so leaving really ends them.
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
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

    /// A room is open: remember what it is and where to send. Returns
    /// the generation the new connection's messages must carry.
    pub fn begin(
        &self,
        room_id: &str,
        invite_code: &str,
        hosting: bool,
        outbox: mpsc::UnboundedSender<SyncMessage>,
    ) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.outbox.lock() = Some(outbox);
        self.song_consent.store(false, Ordering::SeqCst);
        self.awaiting_song.store(false, Ordering::SeqCst);
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
        status.join_request = None;
        status.waiting_for_host = false;
        generation
    }

    /// Whether a message belongs to the connection that is open now.
    pub fn is_current(&self, generation: u64) -> bool {
        self.running.load(Ordering::SeqCst) && self.generation.load(Ordering::SeqCst) == generation
    }

    /// Keep the connection's tasks, so `stop` can end them.
    pub fn hold(&self, task: tokio::task::JoinHandle<()>) {
        self.tasks.lock().push(task);
    }

    /// Send something that is not an edit (an answer, a heartbeat): it
    /// goes out even while an incoming message is being applied.
    pub fn send_control(&self, kind: SyncKind) {
        if !self.running.load(Ordering::Relaxed) {
            return;
        }
        if let Some(outbox) = self.outbox.lock().clone() {
            let _ = outbox.send(SyncMessage {
                sender_user_id: String::new(),
                logical_clock: self.clock.lock().tick(),
                kind,
            });
        }
    }

    pub fn set_waiting(&self, waiting: bool) {
        self.status.lock().waiting_for_host = waiting;
    }

    /// Someone wants in; only the host is asked.
    pub fn set_join_request(&self, request_id: String, name: String) {
        let mut status = self.status.lock();
        if status.hosting {
            status.join_request = Some(JoinAsk { request_id, name });
        }
    }

    /// The host's answer. Letting someone in is also agreeing to send
    /// them the song when they ask for it.
    pub fn answer_join(&self, request_id: &str, admit: bool) -> Result<(), String> {
        let pending = self.status.lock().join_request.take();
        match pending {
            Some(ask) if ask.request_id == request_id => {
                if admit {
                    self.song_consent.store(true, Ordering::SeqCst);
                }
                self.send_control(SyncKind::JoinAnswer {
                    request_id: request_id.to_string(),
                    admit,
                });
                Ok(())
            }
            _ => Err("nobody is waiting to come in".into()),
        }
    }

    pub fn new_code(&self, invite_code: String) {
        let mut status = self.status.lock();
        status.invite_code = invite_code;
        status.message =
            "The invite code changed. Send the new one if someone still has to join.".into();
    }

    pub fn expect_song(&self) {
        self.awaiting_song.store(true, Ordering::SeqCst);
    }

    /// Whether a song that arrived was asked for; asking counts once.
    pub fn take_song_expectation(&self) -> bool {
        self.awaiting_song.swap(false, Ordering::SeqCst)
    }

    /// Whether this side may send its song: it is the host and it let
    /// the other person in.
    pub fn may_send_song(&self) -> bool {
        self.status.lock().hosting && self.song_consent.load(Ordering::SeqCst)
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

    /// Leave the room: the connection's tasks end, the socket closes,
    /// and nothing that was still on its way is applied.
    pub fn stop(&self, why: &str) {
        self.running.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.outbox.lock() = None;
        for task in self.tasks.lock().drain(..) {
            task.abort();
        }
        self.song_consent.store(false, Ordering::SeqCst);
        self.awaiting_song.store(false, Ordering::SeqCst);
        let mut status = self.status.lock();
        status.connected = false;
        status.join_request = None;
        status.waiting_for_host = false;
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

/// Where the room service lives: ours, or in a development build
/// wherever HARDWAVE_ROOM_URL points.
pub fn service_url() -> String {
    crate::endpoints::service_url("HARDWAVE_ROOM_URL", "wss://rooms.hardwavestudios.com/room")
}

/// Build the URL for opening or joining a room. Neither the account
/// token nor the invite code is in it: addresses end up in logs, so
/// both go in headers.
pub fn join_url(room: &str, since: u64) -> String {
    let mut url = format!("{}?since={}", service_url(), since);
    if !room.is_empty() {
        url.push_str(&format!("&room={}", urlencode(room)));
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

/// The song as it is sent to the other person: the same song, with
/// where its files sit on this machine reduced to their names. The
/// other machine cannot use this one's paths anyway, and they say
/// things about this machine (an account name, a folder layout) that
/// are not the other person's business.
pub fn prepared_for_sending(project: &hardwave_project::Project) -> hardwave_project::Project {
    use hardwave_project::clip::ClipContent;
    let mut copy = project.clone();
    let reduce = |path: &mut String| {
        if std::path::Path::new(path.as_str()).is_absolute() || path.starts_with("\\\\") {
            if let Some(name) = std::path::Path::new(path.as_str()).file_name() {
                *path = name.to_string_lossy().into_owned();
            }
        }
    };
    let clean = |clips: &mut Vec<hardwave_project::clip::ClipPlacement>| {
        for placement in clips {
            if let ClipContent::Audio(a) = &mut placement.content {
                reduce(&mut a.source_file);
            }
        }
    };
    for track in &mut copy.tracks {
        clean(&mut track.clips);
    }
    for arrangement in &mut copy.arrangements {
        for timeline in arrangement.timelines.values_mut() {
            clean(&mut timeline.clips);
        }
    }
    if let Some(video) = &mut copy.video {
        reduce(&mut video.path);
    }
    copy
}

/// What the service says first.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Hello {
    /// The code was right and the host is being asked.
    Waiting,
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
    fn the_url_carries_the_room_but_never_the_token_or_the_code() {
        let url = join_url("K7QM2XPA", 42);
        assert!(url.contains("?since=42"));
        assert!(url.contains("&room=K7QM2XPA"));
        assert!(!url.contains("token") && !url.contains("code"), "{url}");
    }

    #[test]
    fn the_song_is_sent_without_this_machines_paths() {
        use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement};
        let mut project = hardwave_project::Project::default();
        let id = project.add_audio_track("Vox".into());
        let clip = |file: &str| ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "c".into(),
                name: "c".into(),
                source_path: "p".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 1,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                source_file: file.into(),
            }),
            track_id: id.clone(),
            position_ticks: 0,
            length_ticks: 1,
            lane: 0,
        };
        let t = project.track_mut(&id).unwrap();
        t.clips.push(clip("/home/dishaion/Secret Project/vox.wav"));
        t.clips.push(clip("Song Samples/kick.wav"));
        let sent = prepared_for_sending(&project);
        let files: Vec<String> = sent
            .track(&id)
            .unwrap()
            .clips
            .iter()
            .map(|c| match &c.content {
                ClipContent::Audio(a) => a.source_file.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(
            files,
            vec!["vox.wav".to_string(), "Song Samples/kick.wav".to_string()]
        );
    }

    #[test]
    fn a_song_nobody_asked_for_is_not_taken_and_asking_counts_once() {
        let collab = Collab::default();
        assert!(!collab.take_song_expectation());
        collab.expect_song();
        assert!(collab.take_song_expectation());
        assert!(!collab.take_song_expectation());
    }

    #[test]
    fn the_song_is_only_sent_after_the_host_let_someone_in() {
        let collab = Collab::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        collab.begin("r", "CODE", true, tx);
        assert!(!collab.may_send_song());
        collab.set_join_request("req-1".into(), "Alex".into());
        assert!(collab.answer_join("someone-else", true).is_err());
        collab.set_join_request("req-1".into(), "Alex".into());
        collab.answer_join("req-1", true).unwrap();
        assert!(collab.may_send_song());
        assert!(matches!(
            rx.try_recv().unwrap().kind,
            SyncKind::JoinAnswer { admit: true, .. }
        ));
        collab.stop("left");
        assert!(!collab.may_send_song(), "leaving takes the consent back");
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
