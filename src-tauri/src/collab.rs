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
    /// Each track's hash as last sent or received, so what goes out is
    /// only the tracks that changed since.
    track_hashes: Mutex<std::collections::HashMap<String, u64>>,
    /// The same for each part of the song that belongs to no track (see
    /// `Project::SONG_SECTIONS`), with the version that settles two people
    /// editing one part at the same moment.
    song: Mutex<std::collections::HashMap<&'static str, SectionClock>>,
    /// Who this side is when two versions of a part tie: random per room.
    author: AtomicU64,
    /// The project format the other side said it has, once it has said.
    peer_format: Mutex<Option<u32>>,
}

/// Why a room with a build that would lose this side's work ends.
const ANOTHER_VERSION: &str =
    "The other person has a different version of Hardwave DAW. Update both to work together.";

/// One part of the song as last sent or taken.
///
/// Two people editing the same part at once each send their version; each
/// side then takes the higher (version, author) of the two, so both end
/// with the same one. Sent as one blob, a marker moved on one side and a
/// tempo changed on the other overwrote each other and the two songs
/// stayed different.
#[derive(Clone, Copy, Default)]
struct SectionClock {
    hash: u64,
    /// Lamport version: the highest seen, one more for an edit of ours.
    version: u64,
    author: u64,
    /// An edit of ours that has its version but has not gone out yet.
    unsent: bool,
    /// The hash of a version too large to send, so it is said once.
    too_large: u64,
}

/// One part of the song to send: (name, version, author, bytes).
pub type ChangedSections = Vec<(&'static str, u64, u64, Vec<u8>)>;

/// Tracks that changed, as (id, place in the song, bytes).
pub type ChangedTracks = Vec<(String, usize, Vec<u8>)>;

/// A message as the text that goes to the room, or why it cannot go.
///
/// The room takes one message of up to MAX_MESSAGE and closes a connection
/// that sends a larger one; the host's connection closing closes the room
/// for both. A song whose bytes went over it (about 4.5 MB packed as a list
/// of numbers, about 12 MB as base64) did exactly that when the guest asked
/// for it. It is now not sent, and the host is told why in plain words.
pub fn wire_text(message: &SyncMessage, base64: bool) -> Result<String, String> {
    let text = hardwave_project::multiplayer::to_wire_json(message, base64)
        .map_err(|e| format!("could not write a room message: {e}"))?;
    let limit = crate::commands::collab::hardwave_room_limits::MAX_MESSAGE;
    if text.len() <= limit {
        return Ok(text);
    }
    let mb = |n: usize| n as f64 / (1024.0 * 1024.0);
    Err(match &message.kind {
        SyncKind::ProjectOffer { blob, .. } => {
            // How much song fits, in the same packed size as this one.
            let fits = blob.len() as f64 * limit as f64 / text.len() as f64;
            format!(
                "This song is too large to send through a room: it packs to {:.1} MB and a room \
                 carries about {:.1} MB. Send them the project file another way; you are both \
                 still in the room.",
                mb(blob.len()),
                fits / (1024.0 * 1024.0)
            )
        }
        SyncKind::TrackState { blob, .. } => format!(
            "A track is too large to send through the room ({:.1} MB with its plug-in \
             settings), so the other side does not have its latest change.",
            mb(blob.len())
        ),
        _ => format!(
            "Something too large for the room ({:.1} MB) was not sent.",
            mb(text.len())
        ),
    })
}

/// How fast this side sends to the room.
///
/// The room drops whatever comes faster than its allowance (SEND_RATE a
/// second, SEND_BURST at once), without a word: a change to more tracks
/// than that at once (the default song has 500 mixer tracks) reached the
/// other side in part, and the two songs stayed different. Messages now
/// wait their turn under an allowance a tenth below the room's, so a big
/// change arrives whole, a few seconds later.
pub struct Pacer(hardwave_room::TokenBucket);

impl Default for Pacer {
    fn default() -> Self {
        Self(hardwave_room::TokenBucket::new(
            hardwave_room::SEND_RATE * 0.9,
            hardwave_room::SEND_BURST * 0.9,
        ))
    }
}

impl Pacer {
    /// When a message of this kind may go, at `now` or later; its share of
    /// the allowance is taken for then.
    pub fn slot(&mut self, kind: &SyncKind, now: std::time::Instant) -> std::time::Instant {
        let cost = hardwave_room::message_cost(kind);
        let at = now + self.0.wait_for(cost, now);
        // Always there by then: no message costs more than the burst.
        let _ = self.0.try_take(cost, at);
        at
    }
}

fn hash_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

impl Collab {
    /// Take the song as it is now as what the other side has, so only
    /// later edits are sent (when a room opens or a song arrives).
    pub fn remember_tracks(&self, project: &hardwave_project::Project) {
        {
            let mut song = self.song.lock();
            song.clear();
            for name in hardwave_project::Project::SONG_SECTIONS {
                let blob = project.song_section(name).unwrap_or_default();
                song.insert(
                    name,
                    SectionClock {
                        hash: hash_of(&blob),
                        ..SectionClock::default()
                    },
                );
            }
        }
        if self.author.load(Ordering::Relaxed) == 0 {
            self.author
                .store(uuid::Uuid::new_v4().as_u64_pair().0 | 1, Ordering::Relaxed);
        }
        let mut hashes = self.track_hashes.lock();
        hashes.clear();
        for t in &project.tracks {
            hashes.insert(
                t.id.clone(),
                hash_of(&hardwave_project::Project::track_blob(t)),
            );
        }
    }

    /// A track the other side sent: it has it, so it is not sent back.
    pub fn remember_track(&self, track_id: &str, blob: &[u8]) {
        self.track_hashes
            .lock()
            .insert(track_id.to_string(), hash_of(blob));
    }

    /// The parts of the song changed since last sent or taken, each with
    /// its new version. A part too large for a room message is left out
    /// (said once) and does not hold up the others.
    pub fn song_changes(&self, project: &hardwave_project::Project) -> ChangedSections {
        let me = self.author.load(Ordering::Relaxed);
        let mut song = self.song.lock();
        let mut out = Vec::new();
        for name in hardwave_project::Project::SONG_SECTIONS {
            let blob = project.song_section(name).unwrap_or_default();
            let h = hash_of(&blob);
            let clock = song.entry(name).or_default();
            if h != clock.hash {
                clock.hash = h;
                clock.version += 1;
                clock.author = me;
                clock.unsent = true;
            }
            if !clock.unsent {
                continue;
            }
            if blob.len() > crate::commands::collab::hardwave_room_limits::MAX_TRACK {
                if clock.too_large != h {
                    clock.too_large = h;
                    log::warn!(
                        "room: the song's {name} are too large to send ({} bytes)",
                        blob.len()
                    );
                }
                continue;
            }
            clock.unsent = false;
            out.push((name, clock.version, clock.author, blob));
        }
        out
    }

    /// Whether a part of the song the other side sent should be taken:
    /// it is newer than ours, or as new and from the higher author. An
    /// edit of ours not sent yet counts as ours first, so the two compare
    /// fairly; it then goes out on the next send.
    ///
    /// When theirs loses and is not what we have, ours goes out again on
    /// the next send. They may never have had it (they took the whole
    /// song since, or came back after a drop, and count from 0 again):
    /// without it they kept their edit, we kept ours, and the two songs
    /// stayed different with nothing left to put them right.
    pub fn should_take_section(
        &self,
        name: &str,
        current: &[u8],
        incoming: &[u8],
        version: u64,
        author: u64,
    ) -> bool {
        let me = self.author.load(Ordering::Relaxed);
        let Some(name) = hardwave_project::Project::SONG_SECTIONS
            .into_iter()
            .find(|n| *n == name)
        else {
            return false;
        };
        let mut song = self.song.lock();
        let clock = song.entry(name).or_default();
        let h = hash_of(current);
        if h != clock.hash {
            clock.hash = h;
            clock.version += 1;
            clock.author = me;
            clock.unsent = true;
        }
        let take = (version, author) > (clock.version, clock.author);
        if !take {
            // Ours stands; the version still counts for the next edit.
            clock.version = clock.version.max(version);
            if hash_of(incoming) != clock.hash {
                clock.unsent = true;
            }
        }
        take
    }

    /// A part of the song taken from the other side, as it reads now: it
    /// has it, so it is not sent back.
    pub fn took_section(&self, name: &str, now: &[u8], version: u64, author: u64) {
        let Some(name) = hardwave_project::Project::SONG_SECTIONS
            .into_iter()
            .find(|n| *n == name)
        else {
            return;
        };
        self.song.lock().insert(
            name,
            SectionClock {
                hash: hash_of(now),
                version,
                author,
                unsent: false,
                too_large: 0,
            },
        );
    }

    /// The whole song went to the other side, which counts every part
    /// from 0 again when it takes it. Each part edited in this room goes
    /// out once more with its version, so their next edit counts on from
    /// ours: without it, their first tempo change or marker after taking
    /// the song lost to ours and was not taken here.
    pub fn resend_song_parts(&self) {
        for clock in self.song.lock().values_mut() {
            if clock.version > 0 {
                clock.unsent = true;
            }
        }
    }

    pub fn forget_track(&self, track_id: &str) {
        self.track_hashes.lock().remove(track_id);
    }

    /// The tracks that changed since last time, as (id, place, bytes), and
    /// the ones that are gone. Marks them as sent.
    pub fn track_changes(
        &self,
        project: &hardwave_project::Project,
    ) -> (ChangedTracks, Vec<String>) {
        let mut hashes = self.track_hashes.lock();
        let mut changed = Vec::new();
        for (i, t) in project.tracks.iter().enumerate() {
            let blob = hardwave_project::Project::track_blob(t);
            let h = hash_of(&blob);
            if hashes.get(&t.id) != Some(&h) {
                hashes.insert(t.id.clone(), h);
                changed.push((t.id.clone(), i, blob));
            }
        }
        let gone: Vec<String> = hashes
            .keys()
            .filter(|id| !project.tracks.iter().any(|t| &t.id == *id))
            .cloned()
            .collect();
        for id in &gone {
            hashes.remove(id);
        }
        (changed, gone)
    }

    /// Count what came in from the other side, for the panel's "received":
    /// every change to the song, whole tracks and song parts included. Only
    /// the few messages that went through the generic edit path were
    /// counted, so the panel said 0 received while tracks, the tempo and
    /// markers arrived.
    pub fn count_received(&self, kind: &SyncKind) {
        let edit = matches!(
            kind,
            SyncKind::Transport(_)
                | SyncKind::Mixer(_)
                | SyncKind::Note(_)
                | SyncKind::Clip(_)
                | SyncKind::Chat { .. }
                | SyncKind::TrackState { .. }
                | SyncKind::TrackRemoved { .. }
                | SyncKind::SongState { .. }
                | SyncKind::PluginParam { .. }
                | SyncKind::ProjectOffer { .. }
        );
        if edit {
            self.received.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Something to tell the person in the room's panel.
    pub fn say(&self, what: &str) {
        self.status.lock().message = what.to_string();
    }

    /// True while a message from the other side is being applied.
    pub fn set_applying(&self, on: bool) {
        self.applying.store(on, Ordering::Relaxed);
    }

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
        *self.peer_format.lock() = None;
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

    /// This side's project format, sent on joining and whenever someone
    /// arrives, as a part of the song (`Project::FORMAT_SECTION`) so the
    /// room service passes it on unchanged. A build from before it ignores
    /// a part it does not know.
    pub fn format_message(&self) -> SyncKind {
        SyncKind::SongState {
            section: hardwave_project::Project::FORMAT_SECTION.to_string(),
            version: 0,
            author: self.author.load(Ordering::Relaxed),
            blob: hardwave_project::Project::format_section(),
        }
    }

    /// Check what the other side sent against this build's project format,
    /// before it is applied.
    ///
    /// Two builds that keep different fields would each drop the other's
    /// from every track they send back: an older build has no playlist
    /// rows, so taking its tracks would put every clip of this side's song
    /// back on no row. Such a room ends with a message saying why. The
    /// other side's format part is answered here (`Ok(true)`: nothing more
    /// to do with it). A build too old to send one is known by its tracks
    /// and arrangements, which lack the fields; a track that has them is
    /// taken, even before that side's format arrives.
    ///
    /// Two more moments come before any edit. The song a host sends says
    /// its own format, and one in another format is never opened: a build
    /// on the newer format took an older host's song, brought it up to
    /// date and sent its edits back, and the room only ended when the
    /// older side edited a track. And a host is asked for its song by a
    /// guest that said its format first (every build that says one says
    /// it before anything else), so a request from one that did not is
    /// from an older build: the song is not sent and the room ends.
    pub fn check_peer(&self, kind: &SyncKind) -> Result<bool, String> {
        use hardwave_project::project::FORMAT_VERSION;
        use hardwave_project::Project;
        match kind {
            SyncKind::SongState { section, blob, .. } if section == Project::FORMAT_SECTION => {
                match Project::read_format_section(blob) {
                    Some(v) if v == FORMAT_VERSION => {
                        *self.peer_format.lock() = Some(v);
                        Ok(true)
                    }
                    _ => Err(ANOTHER_VERSION.into()),
                }
            }
            SyncKind::ProjectOffer { blob, .. } => match Project::bytes_format(blob) {
                Some(v) if v == FORMAT_VERSION => Ok(false),
                _ => Err(ANOTHER_VERSION.into()),
            },
            SyncKind::ProjectRequest
                if self.status.lock().hosting && self.peer_format.lock().is_none() =>
            {
                Err(ANOTHER_VERSION.into())
            }
            _ if self.peer_format.lock().is_some() => Ok(false),
            SyncKind::TrackState { blob, .. } if !Project::track_blob_knows_layout(blob) => {
                Err(ANOTHER_VERSION.into())
            }
            SyncKind::SongState { section, blob, .. }
                if section == "arrangements"
                    && !Project::arrangements_section_knows_layout(blob) =>
            {
                Err(ANOTHER_VERSION.into())
            }
            _ => Ok(false),
        }
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

    /// The room says who is in it now. When the other person has left,
    /// their cursor goes with them: it stayed drawn on the arrangement,
    /// with their name, where they last were.
    ///
    /// So does the format they said: whoever comes in next says their own,
    /// and a build too old to say one must not pass as the one before.
    pub fn set_members(&self, names: Vec<String>) {
        let mut status = self.status.lock();
        if names.len() < 2 {
            status.peer = None;
            *self.peer_format.lock() = None;
        }
        status.members = names;
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
    applied.transport
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
    let mut url = format!(
        "{}?since={}&wire={}",
        service_url(),
        since,
        hardwave_project::multiplayer::WIRE_VERSION
    );
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
///
/// A song travels between systems, so a path counts as some machine's by
/// any system's rules, not this one's: on Windows "/home/..." is not
/// absolute and on a Mac "C:\Users\..." is not, and each was sent whole.
pub fn prepared_for_sending(project: &hardwave_project::Project) -> hardwave_project::Project {
    use hardwave_project::clip::ClipContent;
    let mut copy = project.clone();
    let reduce = |path: &mut String| {
        if is_machine_path(path) {
            if let Some(name) = path.rsplit(['/', '\\']).find(|part| !part.is_empty()) {
                *path = name.to_string();
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

/// A path that points somewhere on a particular computer, on any system:
/// "/home/...", "~/...", "C:\..." or "C:/...", "\\server\share". A path
/// relative to the song ("Song Samples/kick.wav") is not.
fn is_machine_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || path.starts_with("~/")
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
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
        /// The wire version the service reads; a service from before
        /// says none, and is sent bytes as lists of numbers.
        #[serde(default)]
        wire: u32,
    },
    Refused {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::multiplayer::MixerSync;

    /// One side of a room: its song and its session.
    struct Side {
        project: hardwave_project::Project,
        collab: Collab,
    }

    fn side(song: &hardwave_project::Project) -> Side {
        let collab = Collab::default();
        collab.remember_tracks(song);
        Side {
            project: song.clone(),
            collab,
        }
    }

    /// What `to` does with the parts `from` sends: as apply_song_state.
    fn deliver(sent: ChangedSections, to: &mut Side) {
        for (name, version, author, blob) in sent {
            let current = to.project.song_section(name).unwrap();
            if to
                .collab
                .should_take_section(name, &current, &blob, version, author)
            {
                to.project.put_song_section(name, &blob).unwrap();
                let now = to.project.song_section(name).unwrap();
                to.collab.took_section(name, &now, version, author);
            }
        }
    }

    fn song() -> hardwave_project::Project {
        let mut p = hardwave_project::Project::default();
        p.ensure_arrangements();
        p
    }

    #[test]
    fn two_people_editing_different_parts_at_once_both_keep_their_edit() {
        let start = song();
        let (mut a, mut b) = (side(&start), side(&start));
        a.project.timeline_state = Some("{\"markers\":[1]}".into());
        b.project.tempo_map.entries[0].bpm = 150.0;
        // Both send before either hears from the other.
        let from_a = a.collab.song_changes(&a.project);
        let from_b = b.collab.song_changes(&b.project);
        deliver(from_b, &mut a);
        deliver(from_a, &mut b);
        for s in [&a, &b] {
            assert_eq!(s.project.tempo_map.entries[0].bpm, 150.0);
            assert_eq!(
                s.project.timeline_state.as_deref(),
                Some("{\"markers\":[1]}")
            );
        }
        // Nothing left to send: neither echoes what it took.
        assert!(a.collab.song_changes(&a.project).is_empty());
        assert!(b.collab.song_changes(&b.project).is_empty());
    }

    #[test]
    fn two_people_editing_the_same_part_at_once_end_with_the_same_one() {
        let start = song();
        let (mut a, mut b) = (side(&start), side(&start));
        a.project.tempo_map.entries[0].bpm = 150.0;
        b.project.tempo_map.entries[0].bpm = 160.0;
        let from_a = a.collab.song_changes(&a.project);
        let from_b = b.collab.song_changes(&b.project);
        deliver(from_b, &mut a);
        deliver(from_a, &mut b);
        assert_eq!(
            a.project.tempo_map.entries[0].bpm, b.project.tempo_map.entries[0].bpm,
            "both sides keep the same tempo"
        );
    }

    #[test]
    fn an_edit_made_after_hearing_the_other_side_wins() {
        let start = song();
        let (mut a, mut b) = (side(&start), side(&start));
        a.project.tempo_map.entries[0].bpm = 150.0;
        let from_a = a.collab.song_changes(&a.project);
        deliver(from_a, &mut b);
        assert_eq!(b.project.tempo_map.entries[0].bpm, 150.0);
        // B changes it after taking A's: B's is newer, whoever's author
        // number is higher.
        b.project.tempo_map.entries[0].bpm = 128.0;
        let from_b = b.collab.song_changes(&b.project);
        deliver(from_b, &mut a);
        assert_eq!(a.project.tempo_map.entries[0].bpm, 128.0);
    }

    #[test]
    fn an_edit_not_sent_yet_is_weighed_before_the_other_sides_is_taken() {
        let start = song();
        let (mut a, mut b) = (side(&start), side(&start));
        a.project.tempo_map.entries[0].bpm = 150.0;
        let from_a = a.collab.song_changes(&a.project);
        // B edits the same part and hears A before its next send.
        b.project.tempo_map.entries[0].bpm = 160.0;
        deliver(from_a, &mut b);
        let from_b = b.collab.song_changes(&b.project);
        deliver(from_b, &mut a);
        assert_eq!(
            a.project.tempo_map.entries[0].bpm,
            b.project.tempo_map.entries[0].bpm
        );
    }

    /// A side that counts from 0 again (it took the whole song, or came
    /// back after a drop) edits a part the other side has a newer version
    /// of. Theirs loses there, and the newer one goes back to them, so the
    /// two songs end the same rather than each keeping its own.
    #[test]
    fn an_edit_that_loses_gets_the_newer_part_sent_back() {
        let start = song();
        let (mut a, mut b) = (side(&start), side(&start));
        for bpm in [150.0, 151.0] {
            a.project.tempo_map.entries[0].bpm = bpm;
            let sent = a.collab.song_changes(&a.project);
            deliver(sent, &mut b);
        }
        b.collab.remember_tracks(&b.project);
        b.project.tempo_map.entries[0].bpm = 133.0;
        let from_b = b.collab.song_changes(&b.project);
        deliver(from_b, &mut a);
        assert_eq!(a.project.tempo_map.entries[0].bpm, 151.0, "A's is newer");
        let from_a = a.collab.song_changes(&a.project);
        deliver(from_a, &mut b);
        assert_eq!(b.project.tempo_map.entries[0].bpm, 151.0);
        // Settled: nothing more either way.
        assert!(a.collab.song_changes(&a.project).is_empty());
        assert!(b.collab.song_changes(&b.project).is_empty());
    }

    /// After the guest takes the whole song, the host sends each part
    /// again with its version, and the guest's next edit then wins.
    #[test]
    fn an_edit_made_after_taking_the_whole_song_reaches_the_other_side() {
        let start = song();
        let (mut host, mut guest) = (side(&start), side(&start));
        // The host edits the tempo twice before the guest asks for the song.
        for bpm in [150.0, 151.0] {
            host.project.tempo_map.entries[0].bpm = bpm;
            host.collab.song_changes(&host.project);
        }
        // The guest takes it whole, as handle_project does.
        guest.project = host.project.clone();
        guest.collab.remember_tracks(&guest.project);
        host.collab.resend_song_parts();
        let again = host.collab.song_changes(&host.project);
        assert_eq!(
            again.len(),
            1,
            "only the part edited in the room goes again"
        );
        deliver(again, &mut guest);
        // Nothing changed for the guest, and nothing goes back.
        assert_eq!(guest.project.tempo_map.entries[0].bpm, 151.0);
        assert!(guest.collab.song_changes(&guest.project).is_empty());

        guest.project.tempo_map.entries[0].bpm = 133.0;
        let from_guest = guest.collab.song_changes(&guest.project);
        deliver(from_guest, &mut host);
        assert_eq!(host.project.tempo_map.entries[0].bpm, 133.0);
    }

    /// When the other person leaves, their cursor is no longer drawn.
    #[test]
    fn the_other_persons_cursor_goes_when_they_leave() {
        let collab = Collab::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        collab.begin("r", "CODE", true, tx);
        collab.set_members(vec!["Host".into(), "Alex".into()]);
        collab.set_peer(&hardwave_project::multiplayer::Presence::default());
        assert_eq!(collab.status().peer.unwrap().name, "Alex");
        collab.set_members(vec!["Host".into()]);
        assert!(collab.status().peer.is_none());
    }

    fn fader() -> SyncKind {
        SyncKind::Mixer(MixerSync {
            track_id: "kick".into(),
            volume_db: Some(-3.0),
            pan: None,
            muted: None,
        })
    }

    /// Only tracks that changed since the last look go out, and a deleted
    /// one is reported once; one the other side sent is not sent back.
    #[test]
    fn only_changed_tracks_go_out() {
        let collab = Collab::default();
        let mut p = hardwave_project::Project::default();
        let a = p.add_midi_track("A".into());
        collab.remember_tracks(&p);
        assert_eq!(collab.track_changes(&p).0.len(), 0, "nothing changed yet");
        p.track_mut(&a).unwrap().muted = true;
        let (changed, gone) = collab.track_changes(&p);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, a);
        assert!(gone.is_empty());
        assert_eq!(collab.track_changes(&p).0.len(), 0, "sent once");
        p.track_mut(&a).unwrap().name = "Theirs".into();
        let blob = hardwave_project::Project::track_blob(p.track(&a).unwrap());
        collab.remember_track(&a, &blob);
        assert_eq!(
            collab.track_changes(&p).0.len(),
            0,
            "what they sent is not echoed"
        );
        p.remove_track(&a);
        assert_eq!(collab.track_changes(&p).1, vec![a.clone()]);
        assert!(collab.track_changes(&p).1.is_empty());
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
    }

    /// The panel's "received" counts every change from the other side,
    /// whole tracks and song parts included; who is in the room does not.
    #[test]
    fn received_counts_tracks_and_song_parts() {
        let collab = Collab::default();
        for kind in [
            SyncKind::TrackState {
                track_id: "t".into(),
                index: 1,
                blob: Vec::new(),
            },
            SyncKind::SongState {
                section: "tempo".into(),
                version: 1,
                author: 1,
                blob: Vec::new(),
            },
            SyncKind::TrackRemoved {
                track_id: "t".into(),
            },
            fader(),
            SyncKind::MembersChanged { names: Vec::new() },
            SyncKind::Heartbeat,
        ] {
            collab.count_received(&kind);
        }
        assert_eq!(collab.status().received, 4);
    }

    /// A song too large for one room message is not sent (the room would
    /// close the connection, and with the host's the room), and the reason
    /// is said in words. As base64 the same song fits.
    #[test]
    fn a_song_too_large_for_the_room_is_not_sent_and_says_why() {
        let song = |len: usize| SyncMessage {
            sender_user_id: String::new(),
            logical_clock: 1,
            kind: SyncKind::ProjectOffer {
                name: "Big".into(),
                blob: vec![200; len],
            },
        };
        let why = wire_text(&song(5 * 1024 * 1024), false).unwrap_err();
        assert!(
            why.contains("too large to send through a room") && why.contains("5.0 MB"),
            "{why}"
        );
        assert!(wire_text(&song(5 * 1024 * 1024), true).is_ok());
        assert_eq!(
            wire_text(&song(10), false).unwrap(),
            serde_json::to_string(&song(10)).unwrap()
        );
    }

    /// A change to 500 tracks at once: sent all at once the room keeps 200
    /// and drops the rest; paced, all 500 get in, even when the network
    /// holds some back and lets them through together.
    #[test]
    fn a_change_to_500_tracks_arrives_whole() {
        use std::time::{Duration, Instant};
        let track = SyncKind::TrackState {
            track_id: "t".into(),
            index: 0,
            blob: vec![0; 2000],
        };
        let cost = hardwave_room::message_cost(&track);
        let room =
            || hardwave_room::TokenBucket::new(hardwave_room::SEND_RATE, hardwave_room::SEND_BURST);
        let start = Instant::now();
        let mut at_once = room();
        let kept = (0..500).filter(|_| at_once.try_take(cost, start)).count();
        assert_eq!(kept, 200, "unpaced, the room drops the rest");

        let mut pace = Pacer::default();
        let mut relay = room();
        let (mut now, mut arrived, mut accepted) = (start, start, 0);
        for i in 0..500 {
            now = pace.slot(&track, now);
            // In order, some held back 300 ms and then let through together.
            let delay = Duration::from_millis(if i % 50 < 25 { 300 } else { 0 });
            arrived = arrived.max(now + delay);
            if relay.try_take(cost, arrived) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 500);
        // Paced, not stalled: (500 - 180) at 36 a second is about 9 s.
        assert!(now - start < Duration::from_secs(10), "{:?}", now - start);
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
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            }),
            track_id: id.clone(),
            position_ticks: 0,
            length_ticks: 1,
            lane: 0,
            color: None,
            row: None,
        };
        let t = project.track_mut(&id).unwrap();
        t.clips.push(clip("/home/dishaion/Secret Project/vox.wav"));
        t.clips.push(clip("Song Samples/kick.wav"));
        // Paths from the other systems count the same on this one.
        t.clips
            .push(clip("C:\\Users\\dishaion\\Secret Project\\snare.wav"));
        t.clips.push(clip("D:/Packs/clap.wav"));
        t.clips.push(clip("\\\\studio-nas\\Samples\\hat.wav"));
        t.clips.push(clip("~/Music/ride.wav"));
        t.clips.push(clip("Song Samples\\crash.wav"));
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
            vec![
                "vox.wav".to_string(),
                "Song Samples/kick.wav".into(),
                "snare.wav".into(),
                "clap.wav".into(),
                "hat.wav".into(),
                "ride.wav".into(),
                "Song Samples\\crash.wav".into(),
            ]
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

    fn part(section: &str, blob: Vec<u8>) -> SyncKind {
        SyncKind::SongState {
            section: section.into(),
            version: 1,
            author: 1,
            blob,
        }
    }

    /// Both sides say their project format on joining; a side on another
    /// one ends the room instead of dropping this side's playlist rows.
    #[test]
    fn a_room_partner_on_another_project_format_ends_the_room() {
        let ours = Collab::default();
        let SyncKind::SongState { section, blob, .. } = ours.format_message() else {
            panic!("the format goes as a part of the song");
        };
        assert_eq!(section, "format");
        assert!(
            !hardwave_project::Project::SONG_SECTIONS.contains(&"format"),
            "not a part this build or an older one takes into the song"
        );
        let theirs = Collab::default();
        assert_eq!(theirs.check_peer(&part("format", blob)), Ok(true));
        assert_eq!(
            Collab::default().check_peer(&part("format", vec![2])),
            Err(ANOTHER_VERSION.to_string()),
            "a song in format 2 on the other side"
        );
        // Once the other side said the same format, its tracks are taken.
        let track = hardwave_project::Track::new_audio("a".into(), "A".into());
        let state = |blob| SyncKind::TrackState {
            track_id: "a".into(),
            index: 1,
            blob,
        };
        assert_eq!(
            theirs.check_peer(&state(hardwave_project::Project::track_blob(&track))),
            Ok(false)
        );
    }

    /// A build from before FL's three lists sends no format at all: its
    /// first track ends the room, before it is taken. A current build's
    /// track is taken even when it arrives before that side's format.
    #[test]
    fn a_track_from_a_build_before_the_three_lists_ends_the_room() {
        let track = hardwave_project::Track::new_audio("a".into(), "A".into());
        let state = |blob| SyncKind::TrackState {
            track_id: "a".into(),
            index: 1,
            blob,
        };
        let collab = Collab::default();
        assert_eq!(
            collab.check_peer(&state(hardwave_project::Project::track_blob(&track))),
            Ok(false)
        );
        assert_eq!(
            collab.check_peer(&state(hardwave_project::Project::older_track_blob(&track))),
            Err(ANOTHER_VERSION.to_string())
        );
        // Edits that carry no fields of the song are fine either way.
        assert_eq!(collab.check_peer(&fader()), Ok(false));
        let song = song();
        assert_eq!(
            collab.check_peer(&part(
                "arrangements",
                song.song_section("arrangements").unwrap()
            )),
            Ok(false)
        );
        // A new room forgets what the last one said.
        let (tx, _rx) = mpsc::unbounded_channel();
        let blob = hardwave_project::Project::format_section();
        assert_eq!(collab.check_peer(&part("format", blob)), Ok(true));
        collab.begin("r", "CODE", true, tx);
        assert!(collab.peer_format.lock().is_none());
    }

    /// A host on an older build sends its song in its own format. The
    /// newer guest ends the room on it, before the song is opened, instead
    /// of taking it and sending its edits back.
    #[test]
    fn a_song_from_a_host_on_an_older_format_is_never_taken() {
        let offer = |blob| SyncKind::ProjectOffer {
            name: "Theirs".into(),
            blob,
        };
        let guest = Collab::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        guest.begin("r", "CODE", false, tx);
        guest.expect_song();
        let older = song().older_format_bytes();
        assert_eq!(
            hardwave_project::Project::bytes_format(&older),
            Some(2),
            "the song as v0.266.40 sends it"
        );
        assert_eq!(
            guest.check_peer(&offer(older.clone())),
            Err(ANOTHER_VERSION.to_string())
        );
        // Even from a side that said this format: the song is what counts.
        let said = Collab::default();
        let blob = hardwave_project::Project::format_section();
        assert_eq!(said.check_peer(&part("format", blob)), Ok(true));
        assert_eq!(
            said.check_peer(&offer(older)),
            Err(ANOTHER_VERSION.to_string())
        );
        assert_eq!(
            said.check_peer(&offer(b"not a song".to_vec())),
            Err(ANOTHER_VERSION.to_string())
        );
        // A song in this format is taken, said or not.
        let current = song().to_bytes().unwrap();
        assert_eq!(said.check_peer(&offer(current.clone())), Ok(false));
        assert_eq!(Collab::default().check_peer(&offer(current)), Ok(false));
    }

    /// A newer host is asked for its song by a guest on an older build,
    /// which never says its format: the song is not sent and the room
    /// ends. A guest that said its format first gets the song.
    #[test]
    fn a_guest_on_an_older_build_is_not_sent_the_song() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let host = Collab::default();
        host.begin("r", "CODE", true, tx);
        host.set_members(vec!["Host".into(), "Guest".into()]);
        assert_eq!(
            host.check_peer(&SyncKind::ProjectRequest),
            Err(ANOTHER_VERSION.to_string())
        );
        let blob = hardwave_project::Project::format_section();
        assert_eq!(host.check_peer(&part("format", blob)), Ok(true));
        assert_eq!(host.check_peer(&SyncKind::ProjectRequest), Ok(false));
        // The guest leaves; whoever comes in next says their own format.
        host.set_members(vec!["Host".into()]);
        host.set_members(vec!["Host".into(), "Another".into()]);
        assert_eq!(
            host.check_peer(&SyncKind::ProjectRequest),
            Err(ANOTHER_VERSION.to_string()),
            "an older build that came in after a current one"
        );
        // Only the host's song is guarded so: a guest is never asked.
        let (tx, _rx) = mpsc::unbounded_channel();
        let guest = Collab::default();
        guest.begin("r", "CODE", false, tx);
        assert_eq!(guest.check_peer(&SyncKind::ProjectRequest), Ok(false));
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
