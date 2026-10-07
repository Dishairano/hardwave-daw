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

use hardwave_project::multiplayer::{Permission, Room, RoomMember, SyncKind, SyncMessage};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// How many messages a room keeps so a peer that drops can catch up.
///
/// A few minutes of hard editing. Past that, catching up message by
/// message is slower than being sent the project again, which is what
/// the service does instead.
pub const REPLAY_DEPTH: usize = 512;

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
}

/// What the service should do after handing the room a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Relay {
    /// Send it on to the other person.
    pub forward: bool,
    /// Why it was not forwarded, for the log and for the sender.
    pub refused: Option<&'static str>,
}

impl Relay {
    fn pass() -> Self {
        Self {
            forward: true,
            refused: None,
        }
    }
    fn stop(why: &'static str) -> Self {
        Self {
            forward: false,
            refused: Some(why),
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
    history: VecDeque<SyncMessage>,
    /// The highest logical clock the room has seen, so a reconnecting
    /// peer can say where it got to.
    pub highest_clock: u64,
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
            highest_clock: 0,
        }
    }

    /// Let someone in, or say why not.
    ///
    /// The person who opens the room pays; the person they invite
    /// does not. One Pro subscription brings a second producer into
    /// the DAW, which is the point of the whole thing: the guest sees
    /// what Pro is for from the inside, and can only host once they
    /// have their own.
    ///
    /// `subscribed` is the server's answer about this account, never
    /// the client's.
    pub fn join(
        &mut self,
        user_id: impl Into<String>,
        display_name: impl Into<String>,
        code: &str,
        signed_in: bool,
    ) -> Result<(), JoinRefusal> {
        if code != self.invite_code {
            return Err(JoinRefusal::WrongCode);
        }
        if !signed_in {
            return Err(JoinRefusal::NotSignedIn);
        }
        let user_id = user_id.into();
        // Coming back after a drop is not a new person.
        if self.room.member(&user_id).is_some() {
            return Ok(());
        }
        if self.room.members.len() >= 2 {
            return Err(JoinRefusal::Full);
        }
        self.room.add_member(RoomMember {
            user_id,
            display_name: display_name.into(),
            avatar_hint: String::new(),
            // A guest can edit. Watching only is a choice the host
            // makes afterwards, not the default: two people in a room
            // are there to work.
            permission: Permission::Editor,
            presence: hardwave_project::multiplayer::Presence::default(),
        });
        Ok(())
    }

    pub fn leave(&mut self, user_id: &str) -> bool {
        self.room.remove_member(user_id)
    }

    pub fn is_empty(&self) -> bool {
        self.room.members.is_empty()
    }

    /// Decide what happens to a message, and remember it.
    pub fn handle(&mut self, message: &SyncMessage) -> Relay {
        let Some(member) = self.room.member(&message.sender_user_id) else {
            return Relay::stop("you are not in this room");
        };
        let permission = member.permission;
        let is_host = self
            .room
            .host()
            .map(|host| host.user_id == message.sender_user_id)
            .unwrap_or(false);

        let relay = match &message.kind {
            // A heartbeat is between the peer and the service.
            SyncKind::Heartbeat => Relay::stop("heartbeat"),
            // Only the host may change who can do what, or remove
            // someone. A guest asking is refused rather than ignored,
            // so the client can say why nothing happened.
            SyncKind::PermissionChange { .. } | SyncKind::Kick { .. } => {
                if is_host {
                    Relay::pass()
                } else {
                    Relay::stop("only the host can do that")
                }
            }
            // Watching is watching: presence and chat always pass.
            SyncKind::PresenceUpdate(_) | SyncKind::Chat { .. } => Relay::pass(),
            // Only the service says who is in the room. A peer sending
            // one is either confused or lying, and either way the other
            // side should not see it.
            SyncKind::MembersChanged { .. } => Relay::stop("only the room says who is in it"),
            // Asking for the song, and answering with it. A guest who
            // cannot edit may still ask, because they need the song
            // to hear anything at all.
            SyncKind::ProjectRequest => Relay::pass(),
            // The song itself is not remembered: it is megabytes, and
            // a peer catching up wants the edits since, not a copy of
            // a file it already has.
            SyncKind::ProjectOffer { .. } => Relay {
                forward: true,
                refused: None,
            },
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
        // Presence moves several times a second and would push the real
        // edits out of the history in no time.
        let worth_replaying = !matches!(
            message.kind,
            SyncKind::ProjectOffer { .. } | SyncKind::PresenceUpdate(_)
        );
        if relay.forward && worth_replaying {
            self.remember(message.clone());
        }
        relay
    }

    fn remember(&mut self, message: SyncMessage) {
        self.highest_clock = self.highest_clock.max(message.logical_clock);
        if self.history.len() == REPLAY_DEPTH {
            self.history.pop_front();
        }
        self.history.push_back(message);
    }

    /// What a peer has missed since the clock it last saw.
    ///
    /// `None` means it fell too far behind for the history to help and
    /// should be sent the project instead, which is honest: replaying
    /// five hundred edits is slower than starting again.
    pub fn catch_up(&self, since_clock: u64) -> Option<Vec<SyncMessage>> {
        let oldest = self.history.front().map(|m| m.logical_clock).unwrap_or(0);
        if since_clock + 1 < oldest {
            return None;
        }
        Some(
            self.history
                .iter()
                .filter(|m| m.logical_clock > since_clock)
                .cloned()
                .collect(),
        )
    }

    /// The names of everyone in the room, host first.
    pub fn member_names(&self) -> Vec<String> {
        let host = self.room.host_user_id.clone();
        let mut names: Vec<(bool, String)> = self
            .room
            .members
            .iter()
            .map(|m| {
                let name = if m.display_name.is_empty() {
                    m.user_id.clone()
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

    fn edit(from: &str, clock: u64) -> SyncMessage {
        SyncMessage {
            sender_user_id: from.into(),
            logical_clock: clock,
            kind: SyncKind::Mixer(MixerSync {
                track_id: "t1".into(),
                volume_db: Some(-3.0),
                pan: None,
                muted: None,
            }),
        }
    }

    #[test]
    fn the_host_is_in_the_room_and_a_guest_can_join_with_the_code() {
        let mut live = room();
        let code = live.invite_code.clone();
        assert!(live.room.member("host-1").is_some(), "the host is a member");
        assert_eq!(live.join("guest-1", "Guest", &code, true), Ok(()));
        assert_eq!(live.room.members.len(), 2);
    }

    #[test]
    fn the_wrong_code_does_not_get_in() {
        let mut live = room();
        assert_eq!(
            live.join("guest-1", "Guest", "NOT-THE-CODE", true),
            Err(JoinRefusal::WrongCode)
        );
        assert_eq!(live.room.members.len(), 1);
    }

    #[test]
    fn a_guest_needs_an_account_but_not_a_subscription() {
        let mut live = room();
        let code = live.invite_code.clone();
        // Signed in, no Pro: in. The host is paying for this room.
        assert_eq!(live.join("guest-1", "Guest", &code, true), Ok(()));
        assert_eq!(live.room.members.len(), 2);
    }

    #[test]
    fn nobody_joins_anonymously() {
        let mut live = room();
        let code = live.invite_code.clone();
        assert_eq!(
            live.join("", "", &code, false),
            Err(JoinRefusal::NotSignedIn),
            "the other person should see who is in their song"
        );
    }

    #[test]
    fn a_third_person_is_told_the_room_is_full() {
        let mut live = room();
        let code = live.invite_code.clone();
        live.join("guest-1", "One", &code, true).unwrap();
        assert_eq!(
            live.join("guest-2", "Two", &code, true),
            Err(JoinRefusal::Full)
        );
    }

    #[test]
    fn coming_back_after_a_drop_is_not_a_new_person() {
        let mut live = room();
        let code = live.invite_code.clone();
        live.join("guest-1", "One", &code, true).unwrap();
        assert_eq!(live.join("guest-1", "One", &code, true), Ok(()));
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
        let mut live = room();
        let code = live.invite_code.clone();
        live.join("guest-1", "One", &code, true).unwrap();
        live.room.member_mut("guest-1").unwrap().permission = Permission::Viewer;

        let refused = live.handle(&edit("guest-1", 1));
        assert!(!refused.forward);
        assert_eq!(refused.refused, Some("you are listening, not editing"));

        let chat = live.handle(&SyncMessage {
            sender_user_id: "guest-1".into(),
            logical_clock: 2,
            kind: SyncKind::Chat {
                body: "that kick is too long".into(),
            },
        });
        assert!(chat.forward);
    }

    #[test]
    fn only_the_host_can_change_permissions_or_remove_someone() {
        let mut live = room();
        let code = live.invite_code.clone();
        live.join("guest-1", "One", &code, true).unwrap();

        let by_guest = live.handle(&SyncMessage {
            sender_user_id: "guest-1".into(),
            logical_clock: 1,
            kind: SyncKind::Kick {
                target_user_id: "host-1".into(),
            },
        });
        assert!(!by_guest.forward);
        assert_eq!(by_guest.refused, Some("only the host can do that"));

        let by_host = live.handle(&SyncMessage {
            sender_user_id: "host-1".into(),
            logical_clock: 2,
            kind: SyncKind::Kick {
                target_user_id: "guest-1".into(),
            },
        });
        assert!(by_host.forward);
    }

    #[test]
    fn the_song_itself_passes_but_is_not_kept_in_the_history() {
        let mut live = room();
        let relay = live.handle(&SyncMessage {
            sender_user_id: "host-1".into(),
            logical_clock: 1,
            kind: SyncKind::ProjectOffer {
                name: "Untitled".into(),
                blob: vec![0u8; 4096],
            },
        });
        assert!(relay.forward, "the other side needs the song");
        assert_eq!(
            live.history_len(),
            0,
            "a copy of the file is not what a reconnecting peer wants"
        );
    }

    #[test]
    fn a_listener_may_still_ask_for_the_song() {
        let mut live = room();
        let code = live.invite_code.clone();
        live.join("guest-1", "One", &code, true).unwrap();
        live.room.member_mut("guest-1").unwrap().permission = Permission::Viewer;
        let relay = live.handle(&SyncMessage {
            sender_user_id: "guest-1".into(),
            logical_clock: 1,
            kind: SyncKind::ProjectRequest,
        });
        assert!(relay.forward, "without the song they hear nothing at all");
    }

    #[test]
    fn the_member_list_names_the_host_first() {
        let mut live = room();
        live.room.member_mut("host-1").unwrap().display_name = "Dishaion".into();
        let code = live.invite_code.clone();
        live.join("guest-1", "Alex", &code, true).unwrap();
        assert_eq!(
            live.member_names(),
            vec!["Dishaion".to_string(), "Alex".to_string()]
        );
    }

    #[test]
    fn a_peer_cannot_claim_who_is_in_the_room() {
        let mut live = room();
        let relay = live.handle(&SyncMessage {
            sender_user_id: "host-1".into(),
            logical_clock: 1,
            kind: SyncKind::MembersChanged {
                names: vec!["someone else".into()],
            },
        });
        assert!(!relay.forward);
    }

    #[test]
    fn a_heartbeat_is_not_passed_on() {
        let mut live = room();
        let relay = live.handle(&SyncMessage {
            sender_user_id: "host-1".into(),
            logical_clock: 1,
            kind: SyncKind::Heartbeat,
        });
        assert!(!relay.forward);
        assert_eq!(live.history_len(), 0, "and it is not remembered either");
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
    }

    #[test]
    fn falling_too_far_behind_is_said_plainly() {
        let mut live = room();
        for clock in 1..=(REPLAY_DEPTH as u64 + 50) {
            live.handle(&edit("host-1", clock));
        }
        assert_eq!(live.history_len(), REPLAY_DEPTH);
        assert!(
            live.catch_up(1).is_none(),
            "replaying everything is slower than sending the project again"
        );
        assert!(live.catch_up(REPLAY_DEPTH as u64 + 40).is_some());
    }

    #[test]
    fn presence_passes_without_touching_the_song() {
        let mut live = room();
        let relay = live.handle(&SyncMessage {
            sender_user_id: "host-1".into(),
            logical_clock: 1,
            kind: SyncKind::PresenceUpdate(Presence::default()),
        });
        assert!(relay.forward);
        assert_eq!(
            live.history_len(),
            0,
            "where a cursor was is not worth replaying, and it would push real edits out"
        );
    }

    #[test]
    fn the_transport_passes_like_any_other_edit() {
        let mut live = room();
        let relay = live.handle(&SyncMessage {
            sender_user_id: "host-1".into(),
            logical_clock: 1,
            kind: SyncKind::Transport(TransportSync::Seek { tick: 960 }),
        });
        assert!(relay.forward);
    }

    #[test]
    fn an_empty_room_says_so_so_the_service_can_close_it() {
        let mut live = room();
        assert!(!live.is_empty());
        live.leave("host-1");
        assert!(live.is_empty());
    }
}
