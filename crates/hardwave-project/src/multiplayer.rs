//! Multiplayer session model — data types for rooms, invite codes,
//! presence, chat, CRDT conflict resolution, and the wire-protocol
//! envelope for live state sync.
//!
//! This is the data + protocol tier. The WebSocket relay server and
//! actual networking live in a separate binary; the DAW side holds
//! the state machine + message types so the UI and engine can work
//! against a fixture feed during development.

use serde::{Deserialize, Serialize};

/// Permission tier — host can admin the room, editor can change
/// state, viewer is read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    Host,
    Editor,
    Viewer,
}

impl Permission {
    pub fn can_edit(&self) -> bool {
        matches!(self, Permission::Host | Permission::Editor)
    }

    pub fn can_kick(&self) -> bool {
        matches!(self, Permission::Host)
    }
}

/// One member of a multiplayer room — user id + display + avatar
/// hint + permission + presence state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub user_id: String,
    pub display_name: String,
    pub avatar_hint: String,
    pub permission: Permission,
    pub presence: Presence,
}

/// Live presence indicator — what panel the user is focused on and
/// where their cursor is (timeline tick + track index).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub active_panel: Panel,
    pub cursor_tick: u64,
    pub cursor_track_index: Option<u32>,
    pub last_heartbeat_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    Arrangement,
    PianoRoll,
    Mixer,
    ChannelRack,
    Browser,
    PluginEditor,
    Settings,
}

impl Default for Presence {
    fn default() -> Self {
        Self {
            active_panel: Panel::Arrangement,
            cursor_tick: 0,
            cursor_track_index: None,
            last_heartbeat_ms: 0,
        }
    }
}

/// A room — the top-level multiplayer container. `invite_code` is
/// user-shareable (Discord / DM); `members` is the live roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub room_id: String,
    pub invite_code: String,
    pub host_user_id: String,
    pub members: Vec<RoomMember>,
    pub created_at_unix: i64,
}

impl Room {
    pub fn new(host_user_id: impl Into<String>, created_at_unix: i64) -> Self {
        let host = host_user_id.into();
        // Both are drawn from the operating system's random source.
        // They used to be derived from the time the room opened, which
        // anyone could work out.
        let room_id = new_room_id();
        Self {
            invite_code: new_invite_code(),
            room_id,
            host_user_id: host,
            members: Vec::new(),
            created_at_unix,
        }
    }

    pub fn add_member(&mut self, member: RoomMember) -> bool {
        if self.members.iter().any(|m| m.user_id == member.user_id) {
            return false;
        }
        self.members.push(member);
        true
    }

    pub fn remove_member(&mut self, user_id: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.user_id != user_id);
        self.members.len() != before
    }

    pub fn member(&self, user_id: &str) -> Option<&RoomMember> {
        self.members.iter().find(|m| m.user_id == user_id)
    }

    pub fn member_mut(&mut self, user_id: &str) -> Option<&mut RoomMember> {
        self.members.iter_mut().find(|m| m.user_id == user_id)
    }

    pub fn host(&self) -> Option<&RoomMember> {
        self.member(&self.host_user_id)
    }
}

/// The characters an invite code is made of: no 0/O or 1/I, so it
/// can be read out over a call without spelling it.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Ten characters of 32 is 50 bits: guessing one, even at the rate the
/// service allows nobody, takes longer than any room stays open.
pub const INVITE_CODE_LEN: usize = 10;

/// A room's name: eight characters, short enough to read out, random
/// so nobody can work out which rooms are open.
pub fn new_room_id() -> String {
    random_chars(8)
}

/// A fresh invite code from the operating system's random source.
pub fn new_invite_code() -> String {
    random_chars(INVITE_CODE_LEN)
}

fn random_chars(len: usize) -> String {
    debug_assert!(len * 5 <= 64);
    let random = uuid::Uuid::new_v4();
    let bytes = random.as_bytes();
    // 128 random bits less the 6 a v4 id fixes; the first 8 bytes are
    // all random, which is 64 bits for 50 needed.
    let mut bits = u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes"));
    (0..len)
        .map(|_| {
            let c = CODE_ALPHABET[(bits & 0x1F) as usize] as char;
            bits >>= 5;
            c
        })
        .collect()
}

/// Compare a code someone typed with the room's, taking the same time
/// however many characters are right, so the comparison itself says
/// nothing about how close a guess was.
pub fn codes_match(typed: &str, actual: &str) -> bool {
    let typed = typed.trim().to_ascii_uppercase();
    let (a, b) = (typed.as_bytes(), actual.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Wire-protocol envelope — everything the DAW sends / receives
/// over the WebSocket relay flows through here. Latency-tolerant
/// because `logical_clock` carries a causal ordering independent of
/// arrival time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncMessage {
    pub sender_user_id: String,
    pub logical_clock: u64,
    pub kind: SyncKind,
}

/// All message kinds the DAW understands over the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SyncKind {
    /// Transport sync — play / stop / seek.
    Transport(TransportSync),
    /// Mixer state change — volume, pan, mute.
    Mixer(MixerSync),
    /// Piano roll note edit.
    Note(NoteSync),
    /// Arrangement clip edit.
    Clip(ClipSync),
    /// Presence / cursor update from a peer.
    PresenceUpdate(Presence),
    /// Chat message.
    Chat { body: String },
    /// Voice chat audio frame — opaque opus payload.
    VoiceFrame { payload: Vec<u8> },
    /// Session history snapshot — one tick of replay.
    HistorySnapshot { label: String, blob: Vec<u8> },
    /// Permission change (host-only).
    PermissionChange {
        target_user_id: String,
        new_permission: Permission,
    },
    /// Kick member (host-only).
    Kick { target_user_id: String },
    /// "Send me the song." A guest who joins a room usually does not
    /// have the project yet, and one edit at a time will never build
    /// it for them.
    ProjectRequest,
    /// The song itself, as the bytes a .hwp file holds. The one
    /// moment the whole project crosses the network rather than one
    /// edit at a time.
    ProjectOffer {
        name: String,
        #[serde(deserialize_with = "bytes_or_base64")]
        blob: Vec<u8>,
    },
    /// Who is in the room now, sent by the service whenever someone
    /// arrives or leaves. Without it the person who opened the room
    /// cannot tell whether the other one has actually come in.
    MembersChanged { names: Vec<String> },
    /// Heartbeat — keep the connection alive without a state edit.
    Heartbeat,
    /// Sent by the room to the host: someone with the right code wants
    /// in. Nobody joins until the host says yes, so a code that got
    /// out is not enough on its own.
    JoinRequest { request_id: String, name: String },
    /// The host's answer to a JoinRequest.
    JoinAnswer { request_id: String, admit: bool },
    /// Sent by the room to the host when too many wrong codes were
    /// tried: the old code no longer works, and this is the new one.
    CodeChanged { invite_code: String },
    /// A whole track as it is now, at its place in the song: what one edit
    /// at a time could not carry (clips added, resized or deleted, plug-ins
    /// and their order, automation, names, colours). MessagePack of the
    /// track, as `Project::track_blob` writes it.
    TrackState {
        track_id: String,
        index: usize,
        #[serde(deserialize_with = "bytes_or_base64")]
        blob: Vec<u8>,
    },
    /// A track was deleted.
    TrackRemoved { track_id: String },
    /// What belongs to the song rather than one track: the tempo map, the
    /// channel rack, markers and sections, the arrangements, macros, VCAs
    /// and modulation. MessagePack, as `Project::song_level_blob` writes it.
    SongState {
        section: String,
        /// Lamport version of this part, and who wrote it: two people
        /// editing it at once end with the same one on both sides.
        version: u64,
        author: u64,
        #[serde(deserialize_with = "bytes_or_base64")]
        blob: Vec<u8>,
    },
    /// A plug-in knob moved (live, so not yet in the track's saved state).
    PluginParam {
        track_id: String,
        slot_id: String,
        param_id: u32,
        value: f64,
    },
}

/// The wire version a peer or the room service speaks. From 2 on, the
/// bytes of a song, a track or a song part may travel as base64 text
/// instead of a list of numbers: a list spends about 3.6 characters on a
/// byte, so a song of more than about 4.5 MB went over a room's message
/// size and the room closed. Base64 spends 1.33. Both sides say which
/// they read (the DAW in its address, the room in its hello), and each
/// side still reads either.
pub const WIRE_VERSION: u32 = 2;

/// A message as text for a room: the bytes inside as base64 when the
/// other end reads it (`base64`), else as the list of numbers every
/// version reads.
pub fn to_wire_json(message: &SyncMessage, base64: bool) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct Wire<'a> {
        sender_user_id: &'a str,
        logical_clock: u64,
        kind: WireKind<'a>,
    }
    // The same variant and field names as `SyncKind`, so it reads back as one.
    #[derive(Serialize)]
    enum WireKind<'a> {
        ProjectOffer {
            name: &'a str,
            blob: Base64<'a>,
        },
        TrackState {
            track_id: &'a str,
            index: usize,
            blob: Base64<'a>,
        },
        SongState {
            section: &'a str,
            version: u64,
            author: u64,
            blob: Base64<'a>,
        },
    }
    struct Base64<'a>(&'a [u8]);
    impl Serialize for Base64<'_> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use base64::Engine;
            s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(self.0))
        }
    }
    let kind = match (&message.kind, base64) {
        (SyncKind::ProjectOffer { name, blob }, true) => WireKind::ProjectOffer {
            name,
            blob: Base64(blob),
        },
        (
            SyncKind::TrackState {
                track_id,
                index,
                blob,
            },
            true,
        ) => WireKind::TrackState {
            track_id,
            index: *index,
            blob: Base64(blob),
        },
        (
            SyncKind::SongState {
                section,
                version,
                author,
                blob,
            },
            true,
        ) => WireKind::SongState {
            section,
            version: *version,
            author: *author,
            blob: Base64(blob),
        },
        _ => return serde_json::to_string(message),
    };
    serde_json::to_string(&Wire {
        sender_user_id: &message.sender_user_id,
        logical_clock: message.logical_clock,
        kind,
    })
}

/// Bytes as either form a room carries: a list of numbers or base64 text.
fn bytes_or_base64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    struct Bytes;
    impl<'de> serde::de::Visitor<'de> for Bytes {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bytes as a list of numbers or as base64")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 20));
            while let Some(byte) = seq.next_element::<u8>()? {
                out.push(byte);
            }
            Ok(out)
        }
        fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Vec<u8>, E> {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .map_err(E::custom)
        }
        fn visit_bytes<E: serde::de::Error>(self, bytes: &[u8]) -> Result<Vec<u8>, E> {
            Ok(bytes.to_vec())
        }
    }
    d.deserialize_any(Bytes)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportSync {
    Play,
    Stop,
    Seek { tick: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MixerSync {
    pub track_id: String,
    pub volume_db: Option<f32>,
    pub pan: Option<f32>,
    pub muted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteSync {
    pub clip_id: String,
    pub operation: NoteOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NoteOp {
    Insert {
        tick: u64,
        pitch: u8,
        velocity: u8,
        length_ticks: u64,
    },
    Delete {
        tick: u64,
        pitch: u8,
    },
    Move {
        from_tick: u64,
        from_pitch: u8,
        to_tick: u64,
        to_pitch: u8,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipSync {
    pub track_id: String,
    pub operation: ClipOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClipOp {
    Insert {
        clip_id: String,
        start_tick: u64,
        length_ticks: u64,
    },
    Move {
        clip_id: String,
        new_start_tick: u64,
    },
    Resize {
        clip_id: String,
        new_length_ticks: u64,
    },
    Delete {
        clip_id: String,
    },
}

/// A Lamport-style logical clock — every time the DAW emits a
/// `SyncMessage` it tags the message with `tick()` so the remote
/// peers can order messages independent of network arrival time.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogicalClock {
    counter: u64,
}

impl LogicalClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    /// Observe a remote message's clock value — the local counter
    /// jumps to `max(local, remote) + 1` (standard Lamport rule).
    pub fn observe(&mut self, remote: u64) {
        self.counter = self.counter.max(remote) + 1;
    }

    pub fn value(&self) -> u64 {
        self.counter
    }
}

/// CRDT last-writer-wins register over a parameter — the audio
/// engine uses this for mixer fader / pan / mute sync. Concurrent
/// writes are resolved by `logical_clock` first, then by `user_id`
/// lexicographic order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LwwRegister<T: Clone + PartialEq> {
    pub value: T,
    pub logical_clock: u64,
    pub writer_user_id: String,
}

impl<T: Clone + PartialEq> LwwRegister<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            logical_clock: 0,
            writer_user_id: String::new(),
        }
    }

    /// Apply a remote write. Returns `true` if the local register
    /// was updated (remote was newer); `false` if the local value
    /// won.
    pub fn apply(&mut self, value: T, logical_clock: u64, writer_user_id: &str) -> bool {
        let remote_wins = match logical_clock.cmp(&self.logical_clock) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => writer_user_id > self.writer_user_id.as_str(),
        };
        if remote_wins {
            self.value = value;
            self.logical_clock = logical_clock;
            self.writer_user_id = writer_user_id.to_string();
            true
        } else {
            false
        }
    }
}

/// Session history recorder — stores the last `capacity` messages
/// for replay. The UI uses this to scrub through a session after
/// recording.
#[derive(Debug, Clone)]
pub struct SessionHistory {
    messages: Vec<SyncMessage>,
    capacity: usize,
}

impl SessionHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            messages: Vec::with_capacity(capacity),
            capacity: capacity.max(1),
        }
    }

    pub fn record(&mut self, message: SyncMessage) {
        self.messages.push(message);
        if self.messages.len() > self.capacity {
            let drop = self.messages.len() - self.capacity;
            self.messages.drain(..drop);
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn messages(&self) -> &[SyncMessage] {
        &self.messages
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_blob(kind: fn(Vec<u8>) -> SyncKind) -> SyncMessage {
        SyncMessage {
            sender_user_id: "u".into(),
            logical_clock: 9,
            kind: kind((0..=255u8).cycle().take(3000).collect()),
        }
    }

    /// A song, a track and a song part go as base64 to a side that reads
    /// it, read back the same, and take far less room than a number list.
    #[test]
    fn bytes_travel_as_base64_and_read_back_the_same() {
        let messages = [
            with_blob(|blob| SyncKind::ProjectOffer {
                name: "Song".into(),
                blob,
            }),
            with_blob(|blob| SyncKind::TrackState {
                track_id: "t".into(),
                index: 3,
                blob,
            }),
            with_blob(|blob| SyncKind::SongState {
                section: "tempo".into(),
                version: 4,
                author: u64::MAX,
                blob,
            }),
        ];
        for message in messages {
            let old = to_wire_json(&message, false).unwrap();
            assert_eq!(old, serde_json::to_string(&message).unwrap());
            let new = to_wire_json(&message, true).unwrap();
            assert!(new.len() * 2 < old.len(), "{} vs {}", new.len(), old.len());
            for text in [old, new] {
                let back: SyncMessage = serde_json::from_str(&text).unwrap();
                assert_eq!(back, message);
            }
        }
    }

    /// Messages without bytes are written as they always were.
    #[test]
    fn other_messages_are_written_as_before() {
        let message = SyncMessage {
            sender_user_id: String::new(),
            logical_clock: 1,
            kind: SyncKind::Heartbeat,
        };
        assert_eq!(
            to_wire_json(&message, true).unwrap(),
            serde_json::to_string(&message).unwrap()
        );
    }

    fn member(id: &str, perm: Permission) -> RoomMember {
        RoomMember {
            user_id: id.into(),
            display_name: format!("User {}", id),
            avatar_hint: "avatar-1".into(),
            permission: perm,
            presence: Presence::default(),
        }
    }

    #[test]
    fn rooms_and_codes_cannot_be_worked_out() {
        let a = Room::new("host", 1_760_000_000);
        let b = Room::new("host", 1_760_000_000);
        assert_ne!(
            a.room_id, b.room_id,
            "same host, same second, different room"
        );
        assert_ne!(a.invite_code, b.invite_code);
        assert_eq!(a.invite_code.len(), INVITE_CODE_LEN);
        assert!(a.invite_code.bytes().all(|c| CODE_ALPHABET.contains(&c)));
        let many: std::collections::HashSet<String> =
            (0..2000).map(|_| new_invite_code()).collect();
        assert_eq!(many.len(), 2000, "no repeats in two thousand codes");
    }

    #[test]
    fn a_code_matches_only_itself() {
        assert!(
            codes_match("abcdefghjk", "ABCDEFGHJK"),
            "typed in lower case is fine"
        );
        assert!(codes_match(" ABCDEFGHJK ", "ABCDEFGHJK"));
        assert!(!codes_match("ABCDEFGHJ", "ABCDEFGHJK"));
        assert!(!codes_match("ABCDEFGHJL", "ABCDEFGHJK"));
        assert!(!codes_match("", "ABCDEFGHJK"));
    }

    #[test]
    fn room_add_member_is_idempotent() {
        let mut r = Room::new("host", 1);
        r.add_member(member("host", Permission::Host));
        assert!(!r.add_member(member("host", Permission::Editor)));
        assert_eq!(r.members.len(), 1);
        r.add_member(member("guest", Permission::Editor));
        assert_eq!(r.members.len(), 2);
    }

    #[test]
    fn permission_capabilities_are_correct() {
        assert!(Permission::Host.can_edit());
        assert!(Permission::Host.can_kick());
        assert!(Permission::Editor.can_edit());
        assert!(!Permission::Editor.can_kick());
        assert!(!Permission::Viewer.can_edit());
        assert!(!Permission::Viewer.can_kick());
    }

    #[test]
    fn logical_clock_observe_jumps_ahead_of_remote() {
        let mut c = LogicalClock::new();
        c.tick();
        c.observe(10);
        assert_eq!(c.value(), 11);
        c.tick();
        assert_eq!(c.value(), 12);
    }

    #[test]
    fn lww_register_newer_clock_wins() {
        let mut reg: LwwRegister<f32> = LwwRegister::new(0.0);
        assert!(reg.apply(0.5, 5, "alice"));
        assert!(!reg.apply(0.8, 3, "bob")); // older clock → loses
        assert!(reg.apply(0.9, 6, "bob")); // newer clock → wins
        assert_eq!(reg.value, 0.9);
    }

    #[test]
    fn lww_register_equal_clock_resolved_by_user_id() {
        let mut reg: LwwRegister<f32> = LwwRegister::new(0.0);
        reg.apply(0.5, 5, "alice");
        // Equal clock → later user_id (lexicographic) wins.
        assert!(reg.apply(0.8, 5, "bob"));
        assert_eq!(reg.value, 0.8);
        assert!(!reg.apply(0.9, 5, "alice"));
    }

    #[test]
    fn session_history_drops_oldest_when_over_capacity() {
        let mut h = SessionHistory::new(3);
        for i in 0..5 {
            h.record(SyncMessage {
                sender_user_id: "u".into(),
                logical_clock: i,
                kind: SyncKind::Heartbeat,
            });
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.messages()[0].logical_clock, 2);
        assert_eq!(h.messages()[2].logical_clock, 4);
    }

    #[test]
    fn sync_message_serde_roundtrip() {
        let m = SyncMessage {
            sender_user_id: "u1".into(),
            logical_clock: 42,
            kind: SyncKind::Transport(TransportSync::Seek { tick: 1000 }),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: SyncMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }
}
