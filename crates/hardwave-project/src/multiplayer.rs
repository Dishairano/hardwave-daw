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
        let room_id = format!("room-{}", created_at_unix);
        Self {
            invite_code: generate_invite_code(&room_id),
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

/// Generate a short human-readable invite code from a room id. FNV-
/// 1a hash mapped into Base32 for copy-pasteability.
pub fn generate_invite_code(seed: &str) -> String {
    let mut hash: u64 = 0xCBF29CE484222325;
    for b in seed.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001B3);
    }
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O/1/I
    let mut out = String::with_capacity(8);
    for i in 0..8 {
        let idx = ((hash >> (i * 5)) & 0x1F) as usize;
        out.push(ALPHABET[idx % ALPHABET.len()] as char);
    }
    out
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
    /// Heartbeat — keep the connection alive without a state edit.
    Heartbeat,
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
    fn invite_code_is_deterministic_per_room() {
        let a = generate_invite_code("room-1");
        let b = generate_invite_code("room-1");
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        let c = generate_invite_code("room-2");
        assert_ne!(a, c);
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
