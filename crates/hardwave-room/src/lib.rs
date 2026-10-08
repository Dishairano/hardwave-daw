//! The rules of a room: who may join, what gets passed on, and what a
//! reconnecting peer has missed.
//!
//! Deliberately away from the socket. A room is a small state machine
//! and the awkward cases, a second person arriving, a guest editing
//! when the host said they may not, someone coming back after their
//! wifi dropped, are cheaper to settle in tests than over a network.
//!
//! Nothing here opens a connection or reads a clock. The service
//! around it does that and calls in.
//!
//! Who gets in is settled in three steps, each of which a stranger has
//! to pass: a code that cannot be worked out (random, 50 bits, compared
//! in constant time, replaced after ten wrong tries), an account, and
//! the host saying yes to the name that wants in.

use hardwave_project::multiplayer::{
    codes_match, new_invite_code, Permission, Room, RoomMember, SyncKind, SyncMessage,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// How many messages a room keeps so a peer that drops can catch up.
///
/// A few minutes of hard editing. Past that, catching up message by
/// message is slower than being sent the project again, which is what
/// the service does instead.
pub const REPLAY_DEPTH: usize = 512;
/// And at most this many bytes of them, whatever their number.
pub const REPLAY_BYTES: usize = 4 * 1024 * 1024;
/// The largest song that may cross the room in one offer.
pub const MAX_PROJECT_BYTES: usize = 16 * 1024 * 1024;
/// The largest chat message.
pub const MAX_CHAT_CHARS: usize = 2000;
/// The largest single track a room carries (its clips and plug-in state).
pub const MAX_TRACK_BYTES: usize = 4 * 1024 * 1024;
/// Wrong codes a room takes before its code is replaced.
pub const MAX_WRONG_CODES: u32 = 10;

/// Why someone was not let in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinRefusal {
    /// Two people is the room. A third is told so rather than being
    /// dropped into a session that was not built for them.
    Full,
    /// The invite code does not match this room.
    WrongCode,
    /// Opening a room is a Pro feature, and this is where that is
    /// checked: the one place a client cannot edit around. Joining
    /// one is not: a guest needs an account, not a subscription.
    NotSubscribed,
    /// A guest with no account at all. Joining is free, but not
    /// anonymous: the other person should see who is in their song.
    NotSignedIn,
    /// Someone else is already waiting for the host to answer.
    Busy,
    /// The host said no.
    Declined,
}

/// What happens next for someone with the right code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Knock {
    /// Already a member, coming back after a drop.
    Back,
    /// The host has to say yes; the request carries this id.
    AskHost { request_id: String },
}

/// What the service should do after handing the room a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Relay {
    /// Send it on to the other person.
    pub forward: bool,
    /// Why it was not forwarded, for the log and for the sender.
    pub refused: Option<&'static str>,
    /// Someone the room no longer has: their connection must close.
    pub disconnect: Option<String>,
}

impl Relay {
    fn pass() -> Self {
        Self {
            forward: true,
            refused: None,
            disconnect: None,
        }
    }
    fn stop(why: &'static str) -> Self {
        Self {
            forward: false,
            refused: Some(why),
            disconnect: None,
        }
    }
}

/// A live room: the members, the invite code, and what has been said
/// recently.
#[derive(Debug, Clone)]
pub struct LiveRoom {
    pub room: Room,
    pub invite_code: String,
    /// The last messages, oldest first, for a peer catching up.
    history: VecDeque<(SyncMessage, usize)>,
    history_bytes: usize,
    /// The highest logical clock the room has seen, so a reconnecting
    /// peer can say where it got to.
    pub highest_clock: u64,
    /// Wrong codes since the code was last replaced.
    wrong_codes: u32,
    /// Someone with the right code, waiting for the host: request id,
    /// account, name.
    waiting: HashMap<String, (String, String)>,
    /// Who asked for the song and has not been sent it yet. An offer
    /// is only passed on as the answer to a request.
    asked_for_song: Option<String>,
}

impl LiveRoom {
    /// Open a room. The caller has already checked that this account
    /// has Pro: hosting is the paid half of working together.
    pub fn open(host_user_id: impl Into<String>, created_at_unix: i64) -> Self {
        let host_user_id = host_user_id.into();
        let mut room = Room::new(host_user_id.clone(), created_at_unix);
        // Room::new records who the host is but puts nobody in the
        // room. The host is in their own room.
        room.add_member(RoomMember {
            user_id: host_user_id,
            display_name: String::new(),
            avatar_hint: String::new(),
            permission: Permission::Host,
            presence: hardwave_project::multiplayer::Presence::default(),
        });
        let invite_code = room.invite_code.clone();
        Self {
            room,
            invite_code,
            history: VecDeque::with_capacity(REPLAY_DEPTH),
            history_bytes: 0,
            highest_clock: 0,
            wrong_codes: 0,
            waiting: HashMap::new(),
            asked_for_song: None,
        }
    }

    /// Someone knocks with a code.
    ///
    /// The person who opens the room pays; the person they invite does
    /// not. One Pro subscription brings a second producer into the DAW,
    /// and the host still decides who that is.
    ///
    /// `signed_in` is the server's answer about this account, never the
    /// client's.
    pub fn knock(
        &mut self,
        user_id: impl Into<String>,
        display_name: impl Into<String>,
        code: &str,
        signed_in: bool,
    ) -> Result<Knock, JoinRefusal> {
        if !codes_match(code, &self.invite_code) {
            self.wrong_codes += 1;
            if self.wrong_codes >= MAX_WRONG_CODES {
                self.replace_code();
            }
            return Err(JoinRefusal::WrongCode);
        }
        if !signed_in {
            return Err(JoinRefusal::NotSignedIn);
        }
        let user_id = user_id.into();
        // Coming back after a drop is not a new person.
        if self.room.member(&user_id).is_some() {
            return Ok(Knock::Back);
        }
        if self.room.members.len() >= 2 {
            return Err(JoinRefusal::Full);
        }
        if !self.waiting.is_empty() {
            return Err(JoinRefusal::Busy);
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        self.waiting
            .insert(request_id.clone(), (user_id, display_name.into()));
        Ok(Knock::AskHost { request_id })
    }

    /// The name waiting behind a request, to show the host.
    pub fn waiting_name(&self, request_id: &str) -> Option<String> {
        self.waiting.get(request_id).map(|(_, name)| {
            if name.trim().is_empty() {
                "A producer".to_string()
            } else {
                name.clone()
            }
        })
    }

    /// The host answered. Returns the account let in, or why not.
    pub fn answer(&mut self, request_id: &str, admit: bool) -> Result<String, JoinRefusal> {
        let (user_id, name) = self
            .waiting
            .remove(request_id)
            .ok_or(JoinRefusal::Declined)?;
        if !admit {
            return Err(JoinRefusal::Declined);
        }
        if self.room.members.len() >= 2 {
            return Err(JoinRefusal::Full);
        }
        self.room.add_member(RoomMember {
            user_id: user_id.clone(),
            display_name: name,
            avatar_hint: String::new(),
            // A guest can edit. Watching only is a choice the host
            // makes afterwards, not the default: two people in a room
            // are there to work.
            permission: Permission::Editor,
            presence: hardwave_project::multiplayer::Presence::default(),
        });
        Ok(user_id)
    }

    /// A request nobody answered in time, or whose guest gave up.
    pub fn withdraw(&mut self, request_id: &str) {
        self.waiting.remove(request_id);
    }

    /// A new code, so one that got out stops working.
    pub fn replace_code(&mut self) -> String {
        self.invite_code = new_invite_code();
        self.room.invite_code = self.invite_code.clone();
        self.wrong_codes = 0;
        self.invite_code.clone()
    }

    pub fn leave(&mut self, user_id: &str) -> bool {
        if self.asked_for_song.as_deref() == Some(user_id) {
            self.asked_for_song = None;
        }
        self.room.remove_member(user_id)
    }

    pub fn is_empty(&self) -> bool {
        self.room.members.is_empty()
    }

    pub fn is_host(&self, user_id: &str) -> bool {
        self.room.host_user_id == user_id
    }

    /// Decide what happens to a message, and remember it.
    pub fn handle(&mut self, message: &SyncMessage) -> Relay {
        let Some(member) = self.room.member(&message.sender_user_id) else {
            return Relay::stop("you are not in this room");
        };
        let permission = member.permission;
        let sender = message.sender_user_id.clone();
        let is_host = self.is_host(&sender);

        let relay = match &message.kind {
            // A heartbeat is between the peer and the service.
            SyncKind::Heartbeat => Relay::stop("heartbeat"),
            // Only the room says who is in it, who wants in, and what
            // the code is. A peer sending one is confused or lying.
            SyncKind::MembersChanged { .. }
            | SyncKind::JoinRequest { .. }
            | SyncKind::CodeChanged { .. } => Relay::stop("only the room says that"),
            // The host's answer is for the service, not the guest.
            SyncKind::JoinAnswer { .. } => Relay::stop("answered by the room"),
            // Not part of the DAW; nothing here should carry them.
            SyncKind::VoiceFrame { .. } | SyncKind::HistorySnapshot { .. } => {
                Relay::stop("not supported")
            }
            SyncKind::PermissionChange {
                target_user_id,
                new_permission,
            } => {
                if !is_host {
                    Relay::stop("only the host can do that")
                } else if self.is_host(target_user_id) || *new_permission == Permission::Host {
                    Relay::stop("the host stays the host")
                } else if let Some(target) = self.room.member_mut(target_user_id) {
                    // Applied here, so it holds whatever the other
                    // DAW does with the message.
                    target.permission = *new_permission;
                    Relay::pass()
                } else {
                    Relay::stop("they are not in the room")
                }
            }
            SyncKind::Kick { target_user_id } => {
                if !is_host {
                    Relay::stop("only the host can do that")
                } else if self.is_host(target_user_id) {
                    Relay::stop("the host cannot remove themselves")
                } else if self.leave(target_user_id) {
                    // Gone from the room, and a code they knew is no
                    // longer any use to them.
                    self.replace_code();
                    Relay {
                        forward: true,
                        refused: None,
                        disconnect: Some(target_user_id.clone()),
                    }
                } else {
                    Relay::stop("they are not in the room")
                }
            }
            // Watching is watching: presence always passes.
            SyncKind::PresenceUpdate(_) => Relay::pass(),
            SyncKind::Chat { body } => {
                if body.chars().count() > MAX_CHAT_CHARS {
                    Relay::stop("that message is too long")
                } else {
                    Relay::pass()
                }
            }
            // Asking for the song: anyone in the room, because without
            // it a listener hears nothing at all.
            SyncKind::ProjectRequest => {
                self.asked_for_song = Some(sender.clone());
                Relay::pass()
            }
            // The song: only as the answer to the other person's
            // request, only from someone who may edit, and only so big.
            SyncKind::ProjectOffer { blob, .. } => {
                let asked_by_other = self
                    .asked_for_song
                    .as_deref()
                    .is_some_and(|asker| asker != sender);
                if !permission.can_edit() {
                    Relay::stop("you are listening, not editing")
                } else if !asked_by_other {
                    Relay::stop("nobody asked for the song")
                } else if blob.len() > MAX_PROJECT_BYTES {
                    Relay::stop("that song is too large to send")
                } else {
                    self.asked_for_song = None;
                    Relay::pass()
                }
            }
            SyncKind::TrackState { blob, .. } if blob.len() > MAX_TRACK_BYTES => {
                Relay::stop("that track is too large to send")
            }
            // Everything that changes the song needs edit rights.
            _ => {
                if permission.can_edit() {
                    Relay::pass()
                } else {
                    Relay::stop("you are listening, not editing")
                }
            }
        };

        // Remembered for a peer catching up: edits, not where someone's
        // cursor was a minute ago, and not a copy of the whole song.
        let worth_replaying = matches!(
            message.kind,
            SyncKind::Transport(_)
                | SyncKind::Mixer(_)
                | SyncKind::Note(_)
                | SyncKind::Clip(_)
                | SyncKind::Chat { .. }
                | SyncKind::TrackState { .. }
                | SyncKind::TrackRemoved { .. }
                | SyncKind::PluginParam { .. }
        );
        if relay.forward && worth_replaying {
            self.remember(message.clone());
        }
        relay
    }

    fn remember(&mut self, message: SyncMessage) {
        self.highest_clock = self.highest_clock.max(message.logical_clock);
        let size = serde_json::to_vec(&message)
            .map(|v| v.len())
            .unwrap_or(1024);
        self.history_bytes += size;
        self.history.push_back((message, size));
        while self.history.len() > REPLAY_DEPTH || self.history_bytes > REPLAY_BYTES {
            match self.history.pop_front() {
                Some((_, old)) => self.history_bytes -= old,
                None => break,
            }
        }
    }

    /// What a peer has missed since the clock it last saw.
    ///
    /// `None` means it fell too far behind for the history to help and
    /// should be sent the project instead, which is honest: replaying
    /// five hundred edits is slower than starting again.
    pub fn catch_up(&self, since_clock: u64) -> Option<Vec<SyncMessage>> {
        let oldest = self
            .history
            .front()
            .map(|(m, _)| m.logical_clock)
            .unwrap_or(0);
        if since_clock.saturating_add(1) < oldest {
            return None;
        }
        Some(
            self.history
                .iter()
                .filter(|(m, _)| m.logical_clock > since_clock)
                .map(|(m, _)| m.clone())
                .collect(),
        )
    }

    /// The names of everyone in the room, host first. Never an account
    /// id or an email address: those are not the other person's to see.
    pub fn member_names(&self) -> Vec<String> {
        let host = self.room.host_user_id.clone();
        let mut names: Vec<(bool, String)> = self
            .room
            .members
            .iter()
            .map(|m| {
                let name = if m.display_name.trim().is_empty() {
                    "A producer".to_string()
                } else {
                    m.display_name.clone()
                };
                (m.user_id != host, name)
            })
            .collect();
        names.sort();
        names.into_iter().map(|(_, name)| name).collect()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::multiplayer::{MixerSync, Presence, TransportSync};

    fn room() -> LiveRoom {
        LiveRoom::open("host-1", 1_760_000_000)
    }

    fn msg(from: &str, clock: u64, kind: SyncKind) -> SyncMessage {
        SyncMessage {
            sender_user_id: from.into(),
            logical_clock: clock,
            kind,
        }
    }

    fn edit(from: &str, clock: u64) -> SyncMessage {
        msg(
            from,
            clock,
            SyncKind::Mixer(MixerSync {
                track_id: "t1".into(),
                volume_db: Some(-3.0),
                pan: None,
                muted: None,
            }),
        )
    }

    /// A guest the host has let in.
    fn with_guest() -> LiveRoom {
        let mut live = room();
        let code = live.invite_code.clone();
        let Knock::AskHost { request_id } = live.knock("guest-1", "Alex", &code, true).unwrap()
        else {
            panic!("a new guest has to be let in")
        };
        assert_eq!(live.answer(&request_id, true), Ok("guest-1".to_string()));
        live
    }

    #[test]
    fn a_guest_with_the_code_still_waits_for_the_host() {
        let mut live = room();
        let code = live.invite_code.clone();
        let knock = live.knock("guest-1", "Alex", &code, true).unwrap();
        let Knock::AskHost { request_id } = knock else {
            panic!("{knock:?}")
        };
        assert_eq!(live.room.members.len(), 1, "not in until the host says so");
        assert_eq!(live.waiting_name(&request_id).as_deref(), Some("Alex"));
        assert_eq!(live.answer(&request_id, true), Ok("guest-1".into()));
        assert_eq!(live.room.members.len(), 2);
    }

    #[test]
    fn the_host_can_say_no() {
        let mut live = room();
        let code = live.invite_code.clone();
        let Ok(Knock::AskHost { request_id }) = live.knock("guest-1", "Alex", &code, true) else {
            panic!()
        };
        assert_eq!(live.answer(&request_id, false), Err(JoinRefusal::Declined));
        assert_eq!(live.room.members.len(), 1);
        assert_eq!(
            live.answer(&request_id, true),
            Err(JoinRefusal::Declined),
            "an answer counts once"
        );
    }

    #[test]
    fn one_person_waits_at_a_time() {
        let mut live = room();
        let code = live.invite_code.clone();
        live.knock("guest-1", "One", &code, true).unwrap();
        assert_eq!(
            live.knock("guest-2", "Two", &code, true),
            Err(JoinRefusal::Busy)
        );
    }

    #[test]
    fn the_wrong_code_does_not_get_in_and_ten_of_them_replace_it() {
        let mut live = room();
        let original = live.invite_code.clone();
        for _ in 0..(MAX_WRONG_CODES - 1) {
            assert_eq!(
                live.knock("x", "X", "AAAAAAAAAA", true),
                Err(JoinRefusal::WrongCode)
            );
        }
        assert_eq!(
            live.invite_code, original,
            "nine wrong tries leave the code"
        );
        assert_eq!(
            live.knock("x", "X", "AAAAAAAAAA", true),
            Err(JoinRefusal::WrongCode)
        );
        assert_ne!(live.invite_code, original, "the tenth replaces it");
        assert_eq!(
            live.knock("guest-1", "Alex", &original, true),
            Err(JoinRefusal::WrongCode),
            "and the old code no longer works"
        );
    }

    #[test]
    fn a_code_typed_in_lower_case_still_works() {
        let mut live = room();
        let code = live.invite_code.to_lowercase();
        assert!(live.knock("guest-1", "Alex", &code, true).is_ok());
    }

    #[test]
    fn nobody_joins_anonymously() {
        let mut live = room();
        let code = live.invite_code.clone();
        assert_eq!(
            live.knock("", "", &code, false),
            Err(JoinRefusal::NotSignedIn)
        );
    }

    #[test]
    fn a_third_person_is_told_the_room_is_full() {
        let mut live = with_guest();
        let code = live.invite_code.clone();
        assert_eq!(
            live.knock("guest-2", "Two", &code, true),
            Err(JoinRefusal::Full)
        );
    }

    #[test]
    fn coming_back_after_a_drop_is_not_a_new_person() {
        let mut live = with_guest();
        let code = live.invite_code.clone();
        assert_eq!(live.knock("guest-1", "Alex", &code, true), Ok(Knock::Back));
        assert_eq!(live.room.members.len(), 2, "still two, not three");
    }

    #[test]
    fn an_edit_from_someone_outside_the_room_goes_nowhere() {
        let mut live = room();
        let relay = live.handle(&edit("a-stranger", 1));
        assert!(!relay.forward);
        assert_eq!(relay.refused, Some("you are not in this room"));
        assert_eq!(live.history_len(), 0);
    }

    #[test]
    fn a_listener_cannot_change_the_song_but_can_still_talk() {
        let mut live = with_guest();
        live.room.member_mut("guest-1").unwrap().permission = Permission::Viewer;
        let refused = live.handle(&edit("guest-1", 1));
        assert_eq!(refused.refused, Some("you are listening, not editing"));
        let chat = live.handle(&msg(
            "guest-1",
            2,
            SyncKind::Chat {
                body: "that kick is too long".into(),
            },
        ));
        assert!(chat.forward);
        let essay = live.handle(&msg(
            "guest-1",
            3,
            SyncKind::Chat {
                body: "x".repeat(MAX_CHAT_CHARS + 1),
            },
        ));
        assert!(!essay.forward);
    }

    /// A whole track needs edit rights and a sane size.
    #[test]
    fn a_track_needs_edit_rights_and_a_sane_size() {
        let mut live = with_guest();
        let track = |n: usize| SyncKind::TrackState {
            track_id: "t1".into(),
            index: 0,
            blob: vec![0; n],
        };
        assert!(live.handle(&msg("guest-1", 1, track(1024))).forward);
        let big = live.handle(&msg("guest-1", 2, track(MAX_TRACK_BYTES + 1)));
        assert_eq!(big.refused, Some("that track is too large to send"));
        live.room.member_mut("guest-1").unwrap().permission = Permission::Viewer;
        let listening = live.handle(&msg("guest-1", 3, track(10)));
        assert_eq!(listening.refused, Some("you are listening, not editing"));
    }

    #[test]
    fn the_host_changes_permissions_and_the_room_holds_them() {
        let mut live = with_guest();
        let relay = live.handle(&msg(
            "host-1",
            1,
            SyncKind::PermissionChange {
                target_user_id: "guest-1".into(),
                new_permission: Permission::Viewer,
            },
        ));
        assert!(relay.forward);
        assert_eq!(
            live.room.member("guest-1").unwrap().permission,
            Permission::Viewer
        );
        assert!(
            !live.handle(&edit("guest-1", 2)).forward,
            "the change holds, whatever the guest's DAW does"
        );

        let by_guest = live.handle(&msg(
            "guest-1",
            3,
            SyncKind::PermissionChange {
                target_user_id: "host-1".into(),
                new_permission: Permission::Viewer,
            },
        ));
        assert_eq!(by_guest.refused, Some("only the host can do that"));
        let demote_host = live.handle(&msg(
            "host-1",
            4,
            SyncKind::PermissionChange {
                target_user_id: "host-1".into(),
                new_permission: Permission::Viewer,
            },
        ));
        assert!(!demote_host.forward);
    }

    #[test]
    fn a_kick_removes_them_and_closes_their_connection() {
        let mut live = with_guest();
        let code = live.invite_code.clone();
        let by_guest = live.handle(&msg(
            "guest-1",
            1,
            SyncKind::Kick {
                target_user_id: "host-1".into(),
            },
        ));
        assert_eq!(by_guest.refused, Some("only the host can do that"));

        let kick = live.handle(&msg(
            "host-1",
            2,
            SyncKind::Kick {
                target_user_id: "guest-1".into(),
            },
        ));
        assert_eq!(kick.disconnect.as_deref(), Some("guest-1"));
        assert!(live.room.member("guest-1").is_none());
        assert_ne!(live.invite_code, code, "the code they knew stops working");
        assert_eq!(
            live.knock("guest-1", "Alex", &code, true),
            Err(JoinRefusal::WrongCode)
        );
    }

    #[test]
    fn the_song_crosses_only_as_the_answer_to_the_other_persons_request() {
        let mut live = with_guest();
        let offer = |from: &str| {
            msg(
                from,
                9,
                SyncKind::ProjectOffer {
                    name: "Song".into(),
                    blob: vec![0u8; 4096],
                },
            )
        };

        assert_eq!(
            live.handle(&offer("host-1")).refused,
            Some("nobody asked for the song")
        );
        assert!(
            live.handle(&msg("guest-1", 1, SyncKind::ProjectRequest))
                .forward
        );
        assert_eq!(
            live.handle(&offer("guest-1")).refused,
            Some("nobody asked for the song"),
            "not your own request"
        );
        assert!(live.handle(&offer("host-1")).forward);
        assert_eq!(
            live.handle(&offer("host-1")).refused,
            Some("nobody asked for the song"),
            "one answer per request"
        );
        assert_eq!(
            live.history_len(),
            0,
            "a copy of the file is not what a reconnecting peer wants"
        );
    }

    #[test]
    fn a_listener_may_ask_for_the_song_but_not_send_one() {
        let mut live = with_guest();
        live.room.member_mut("guest-1").unwrap().permission = Permission::Viewer;
        assert!(
            live.handle(&msg("host-1", 1, SyncKind::ProjectRequest))
                .forward
        );
        let offer = live.handle(&msg(
            "guest-1",
            2,
            SyncKind::ProjectOffer {
                name: "x".into(),
                blob: vec![1],
            },
        ));
        assert_eq!(offer.refused, Some("you are listening, not editing"));
        assert!(
            live.handle(&msg("guest-1", 3, SyncKind::ProjectRequest))
                .forward
        );
    }

    #[test]
    fn a_song_too_large_is_not_passed_on() {
        let mut live = with_guest();
        live.handle(&msg("guest-1", 1, SyncKind::ProjectRequest));
        let huge = live.handle(&msg(
            "host-1",
            2,
            SyncKind::ProjectOffer {
                name: "x".into(),
                blob: vec![0u8; MAX_PROJECT_BYTES + 1],
            },
        ));
        assert_eq!(huge.refused, Some("that song is too large to send"));
    }

    #[test]
    fn the_member_list_shows_names_never_accounts() {
        let mut live = with_guest();
        live.room.member_mut("host-1").unwrap().display_name = "Dishaion".into();
        assert_eq!(
            live.member_names(),
            vec!["Dishaion".to_string(), "Alex".to_string()]
        );
        live.room.member_mut("guest-1").unwrap().display_name = String::new();
        assert_eq!(live.member_names()[1], "A producer");
    }

    #[test]
    fn a_peer_cannot_speak_for_the_room() {
        let mut live = room();
        for kind in [
            SyncKind::MembersChanged {
                names: vec!["someone".into()],
            },
            SyncKind::JoinRequest {
                request_id: "r".into(),
                name: "x".into(),
            },
            SyncKind::CodeChanged {
                invite_code: "AAAAAAAAAA".into(),
            },
            SyncKind::VoiceFrame {
                payload: vec![0; 10],
            },
            SyncKind::HistorySnapshot {
                label: "x".into(),
                blob: vec![],
            },
            SyncKind::Heartbeat,
        ] {
            assert!(!live.handle(&msg("host-1", 1, kind)).forward);
        }
        assert_eq!(live.history_len(), 0);
    }

    #[test]
    fn catching_up_returns_what_was_missed_and_nothing_else() {
        let mut live = room();
        for clock in 1..=5 {
            live.handle(&edit("host-1", clock));
        }
        let missed = live.catch_up(3).expect("the history still reaches back");
        assert_eq!(missed.len(), 2);
        assert_eq!(missed[0].logical_clock, 4);
        assert_eq!(live.highest_clock, 5);
        assert_eq!(
            live.catch_up(u64::MAX).map(|m| m.len()),
            Some(0),
            "a silly clock is not a crash"
        );
    }

    #[test]
    fn falling_too_far_behind_is_said_plainly() {
        let mut live = room();
        for clock in 1..=(REPLAY_DEPTH as u64 + 50) {
            live.handle(&edit("host-1", clock));
        }
        assert_eq!(live.history_len(), REPLAY_DEPTH);
        assert!(live.catch_up(1).is_none());
        assert!(live.catch_up(REPLAY_DEPTH as u64 + 40).is_some());
    }

    #[test]
    fn the_history_is_capped_by_size_as_well_as_count() {
        let mut live = with_guest();
        for clock in 1..=200 {
            live.handle(&msg(
                "host-1",
                clock,
                SyncKind::Chat {
                    body: "x".repeat(MAX_CHAT_CHARS),
                },
            ));
        }
        assert!(live.history_bytes <= REPLAY_BYTES);
        assert!(live.history_len() < 200 || 200 * MAX_CHAT_CHARS <= REPLAY_BYTES);
    }

    #[test]
    fn presence_passes_without_being_remembered() {
        let mut live = room();
        assert!(
            live.handle(&msg(
                "host-1",
                1,
                SyncKind::PresenceUpdate(Presence::default())
            ))
            .forward
        );
        assert_eq!(live.history_len(), 0);
    }

    #[test]
    fn the_transport_passes_like_any_other_edit() {
        let mut live = room();
        assert!(
            live.handle(&msg(
                "host-1",
                1,
                SyncKind::Transport(TransportSync::Seek { tick: 960 })
            ))
            .forward
        );
    }

    #[test]
    fn an_empty_room_says_so_so_the_service_can_close_it() {
        let mut live = room();
        assert!(!live.is_empty());
        live.leave("host-1");
        assert!(live.is_empty());
    }
}
