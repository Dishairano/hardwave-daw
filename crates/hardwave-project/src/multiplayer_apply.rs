//! Turning what the other person did into a change in this project.
//!
//! `multiplayer.rs` has described the messages for months: a note, a
//! clip, a fader, the transport. Nothing turned one back into an edit,
//! which is the whole foundation: without it a room is two people
//! watching each other's cursors.
//!
//! Everything here is pure. A message goes in, the project changes,
//! and what the audio engine has to be told about comes back out. The
//! caller decides when to rebuild the graph and when to move the
//! playhead, because those are its business and not this module's.

use crate::clip::{ClipContent, ClipPlacement};
use crate::multiplayer::{ClipOp, ClipSync, MixerSync, NoteOp, NoteSync, SyncKind, SyncMessage};
use crate::project::Project;

/// What the caller still has to do after a message was applied.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Applied {
    /// The project changed in a way the audio graph has to be rebuilt
    /// for.
    pub rebuild_graph: bool,
    /// The transport was asked to do something: play, stop, or go to
    /// a tick.
    pub transport: Option<crate::multiplayer::TransportSync>,
    /// Nothing was found to change. Not an error: a peer can send an
    /// edit for a clip this side has already deleted, and the answer
    /// to that is to carry on.
    pub missed: bool,
}

/// Apply one message to the project.
///
/// Messages that are not edits, presence and chat and heartbeats,
/// change nothing here and say so, so the caller can hand them to the
/// part of the UI that cares.
pub fn apply(project: &mut Project, message: &SyncMessage) -> Applied {
    match &message.kind {
        SyncKind::Transport(transport) => Applied {
            transport: Some(*transport),
            ..Default::default()
        },
        SyncKind::Mixer(mixer) => apply_mixer(project, mixer),
        SyncKind::Note(note) => apply_note(project, note),
        SyncKind::Clip(clip) => apply_clip(project, clip),
        _ => Applied {
            missed: true,
            ..Default::default()
        },
    }
}

fn apply_mixer(project: &mut Project, mixer: &MixerSync) -> Applied {
    let Some(track) = project.track_mut(&mixer.track_id) else {
        return Applied {
            missed: true,
            ..Default::default()
        };
    };
    if let Some(db) = mixer.volume_db {
        track.volume_db = (db as f64).clamp(-100.0, 12.0);
    }
    if let Some(pan) = mixer.pan {
        track.pan = (pan as f64).clamp(-1.0, 1.0);
    }
    if let Some(muted) = mixer.muted {
        track.muted = muted;
    }
    Applied {
        rebuild_graph: true,
        ..Default::default()
    }
}

/// The MIDI clip with this id, wherever it is in the song.
fn midi_clip<'a>(
    project: &'a mut Project,
    clip_id: &str,
) -> Option<&'a mut hardwave_midi::MidiClip> {
    for track in project.tracks.iter_mut() {
        for placement in track.clips.iter_mut() {
            if let ClipContent::Midi(reference) = &mut placement.content {
                if reference.id == clip_id {
                    return Some(&mut reference.clip);
                }
            }
        }
    }
    None
}

fn apply_note(project: &mut Project, note: &NoteSync) -> Applied {
    let Some(clip) = midi_clip(project, &note.clip_id) else {
        return Applied {
            missed: true,
            ..Default::default()
        };
    };
    match note.operation {
        NoteOp::Insert {
            tick,
            pitch,
            velocity,
            length_ticks,
        } => {
            // The same note twice is one note. Two people writing the
            // same hit at the same moment is a normal thing to do, and
            // it should not end up doubled and twice as loud.
            let already = clip
                .notes
                .iter()
                .any(|n| n.start_tick == tick && n.pitch == pitch);
            if already {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            }
            clip.notes.push(hardwave_midi::MidiNote {
                start_tick: tick,
                duration_ticks: length_ticks.max(1),
                pitch,
                velocity: velocity as f32 / 127.0,
                channel: 0,
                muted: false,
                ..Default::default()
            });
        }
        NoteOp::Delete { tick, pitch } => {
            let before = clip.notes.len();
            clip.notes
                .retain(|n| !(n.start_tick == tick && n.pitch == pitch));
            if clip.notes.len() == before {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            }
        }
        NoteOp::Move {
            from_tick,
            from_pitch,
            to_tick,
            to_pitch,
        } => {
            let Some(note) = clip
                .notes
                .iter_mut()
                .find(|n| n.start_tick == from_tick && n.pitch == from_pitch)
            else {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            };
            note.start_tick = to_tick;
            note.pitch = to_pitch;
        }
    }
    Applied {
        rebuild_graph: true,
        ..Default::default()
    }
}

fn clip_id_of(placement: &ClipPlacement) -> &str {
    match &placement.content {
        ClipContent::Midi(reference) => &reference.id,
        ClipContent::Audio(audio) => &audio.id,
    }
}

fn apply_clip(project: &mut Project, clip: &ClipSync) -> Applied {
    match &clip.operation {
        ClipOp::Insert {
            clip_id,
            start_tick,
            length_ticks,
        } => {
            let Some(track) = project.track_mut(&clip.track_id) else {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            };
            if track.clips.iter().any(|c| clip_id_of(c) == clip_id) {
                // Already here: the same message arriving twice must
                // not make two clips.
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            }
            let length = (*length_ticks).max(1);
            track.clips.push(ClipPlacement {
                content: ClipContent::Midi(crate::clip::MidiClipRef {
                    id: clip_id.clone(),
                    clip: hardwave_midi::MidiClip::new(
                        uuid_like(clip_id),
                        "Clip".to_string(),
                        length,
                    ),
                }),
                track_id: clip.track_id.clone(),
                position_ticks: *start_tick,
                length_ticks: length,
                lane: 0,
            });
        }
        ClipOp::Move {
            clip_id,
            new_start_tick,
        } => {
            let Some(placement) = find_clip_mut(project, clip_id) else {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            };
            placement.position_ticks = *new_start_tick;
        }
        ClipOp::Resize {
            clip_id,
            new_length_ticks,
        } => {
            let Some(placement) = find_clip_mut(project, clip_id) else {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            };
            placement.length_ticks = (*new_length_ticks).max(1);
        }
        ClipOp::Delete { clip_id } => {
            let mut removed = false;
            for track in project.tracks.iter_mut() {
                let before = track.clips.len();
                track.clips.retain(|c| clip_id_of(c) != clip_id);
                removed |= track.clips.len() != before;
            }
            if !removed {
                return Applied {
                    missed: true,
                    ..Default::default()
                };
            }
        }
    }
    Applied {
        rebuild_graph: true,
        ..Default::default()
    }
}

fn find_clip_mut<'a>(project: &'a mut Project, clip_id: &str) -> Option<&'a mut ClipPlacement> {
    project
        .tracks
        .iter_mut()
        .flat_map(|track| track.clips.iter_mut())
        .find(|placement| clip_id_of(placement) == clip_id)
}

/// The inner MIDI clip needs an id of its own, and both sides have to
/// pick the same one or the next message about it misses.
fn uuid_like(clip_id: &str) -> String {
    format!("{clip_id}-midi")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplayer::TransportSync;
    use crate::track::TrackKind;

    fn message(kind: SyncKind) -> SyncMessage {
        SyncMessage {
            sender_user_id: "them".into(),
            logical_clock: 1,
            kind,
        }
    }

    fn project_with_track() -> (Project, String) {
        let mut project = Project::default();
        let id = project.add_midi_track("Lead".into());
        (project, id)
    }

    #[test]
    fn a_fader_they_moved_moves_here() {
        let (mut project, track_id) = project_with_track();
        let applied = apply(
            &mut project,
            &message(SyncKind::Mixer(MixerSync {
                track_id: track_id.clone(),
                volume_db: Some(-6.0),
                pan: Some(-0.5),
                muted: Some(true),
            })),
        );
        assert!(applied.rebuild_graph);
        let track = project.track(&track_id).unwrap();
        assert_eq!(track.volume_db, -6.0);
        assert_eq!(track.pan, -0.5);
        assert!(track.muted);
    }

    #[test]
    fn a_fader_on_a_track_we_do_not_have_is_let_go() {
        let (mut project, _) = project_with_track();
        let applied = apply(
            &mut project,
            &message(SyncKind::Mixer(MixerSync {
                track_id: "someone-elses-track".into(),
                volume_db: Some(0.0),
                pan: None,
                muted: None,
            })),
        );
        assert!(applied.missed, "a missing track is not a crash");
        assert!(!applied.rebuild_graph);
    }

    #[test]
    fn the_transport_is_handed_back_rather_than_applied_here() {
        let (mut project, _) = project_with_track();
        let applied = apply(
            &mut project,
            &message(SyncKind::Transport(TransportSync::Seek { tick: 3840 })),
        );
        assert_eq!(
            applied.transport,
            Some(TransportSync::Seek { tick: 3840 }),
            "the playhead is the engine's, not the project's"
        );
        assert!(!applied.rebuild_graph);
    }

    #[test]
    fn a_clip_they_made_appears_here_once() {
        let (mut project, track_id) = project_with_track();
        let insert = message(SyncKind::Clip(ClipSync {
            track_id: track_id.clone(),
            operation: ClipOp::Insert {
                clip_id: "clip-1".into(),
                start_tick: 1920,
                length_ticks: 3840,
            },
        }));
        assert!(apply(&mut project, &insert).rebuild_graph);
        assert_eq!(project.track(&track_id).unwrap().clips.len(), 1);

        // The same message twice is still one clip: a reconnect
        // replays what it missed and must not double anything.
        let again = apply(&mut project, &insert);
        assert!(again.missed);
        assert_eq!(project.track(&track_id).unwrap().clips.len(), 1);
    }

    #[test]
    fn a_clip_can_be_moved_resized_and_deleted() {
        let (mut project, track_id) = project_with_track();
        apply(
            &mut project,
            &message(SyncKind::Clip(ClipSync {
                track_id: track_id.clone(),
                operation: ClipOp::Insert {
                    clip_id: "clip-1".into(),
                    start_tick: 0,
                    length_ticks: 960,
                },
            })),
        );
        apply(
            &mut project,
            &message(SyncKind::Clip(ClipSync {
                track_id: track_id.clone(),
                operation: ClipOp::Move {
                    clip_id: "clip-1".into(),
                    new_start_tick: 7680,
                },
            })),
        );
        apply(
            &mut project,
            &message(SyncKind::Clip(ClipSync {
                track_id: track_id.clone(),
                operation: ClipOp::Resize {
                    clip_id: "clip-1".into(),
                    new_length_ticks: 1920,
                },
            })),
        );
        {
            let placement = &project.track(&track_id).unwrap().clips[0];
            assert_eq!(placement.position_ticks, 7680);
            assert_eq!(placement.length_ticks, 1920);
        }
        let deleted = apply(
            &mut project,
            &message(SyncKind::Clip(ClipSync {
                track_id: track_id.clone(),
                operation: ClipOp::Delete {
                    clip_id: "clip-1".into(),
                },
            })),
        );
        assert!(deleted.rebuild_graph);
        assert!(project.track(&track_id).unwrap().clips.is_empty());
        // And deleting it again is a miss, not a panic.
        assert!(
            apply(
                &mut project,
                &message(SyncKind::Clip(ClipSync {
                    track_id,
                    operation: ClipOp::Delete {
                        clip_id: "clip-1".into(),
                    },
                })),
            )
            .missed
        );
    }

    #[test]
    fn notes_are_written_moved_and_removed() {
        let (mut project, track_id) = project_with_track();
        apply(
            &mut project,
            &message(SyncKind::Clip(ClipSync {
                track_id: track_id.clone(),
                operation: ClipOp::Insert {
                    clip_id: "clip-1".into(),
                    start_tick: 0,
                    length_ticks: 3840,
                },
            })),
        );
        let note = |operation| {
            message(SyncKind::Note(NoteSync {
                clip_id: "clip-1".into(),
                operation,
            }))
        };
        assert!(
            apply(
                &mut project,
                &note(NoteOp::Insert {
                    tick: 0,
                    pitch: 36,
                    velocity: 100,
                    length_ticks: 240,
                })
            )
            .rebuild_graph
        );
        // The same hit twice is one hit: two people can write the same
        // kick at the same moment.
        assert!(
            apply(
                &mut project,
                &note(NoteOp::Insert {
                    tick: 0,
                    pitch: 36,
                    velocity: 110,
                    length_ticks: 240,
                })
            )
            .missed
        );
        assert!(
            apply(
                &mut project,
                &note(NoteOp::Move {
                    from_tick: 0,
                    from_pitch: 36,
                    to_tick: 480,
                    to_pitch: 38,
                })
            )
            .rebuild_graph
        );
        let clip = midi_clip(&mut project, "clip-1").unwrap();
        assert_eq!(clip.notes.len(), 1);
        assert_eq!(clip.notes[0].start_tick, 480);
        assert_eq!(clip.notes[0].pitch, 38);

        assert!(
            apply(
                &mut project,
                &note(NoteOp::Delete {
                    tick: 480,
                    pitch: 38
                })
            )
            .rebuild_graph
        );
        assert!(midi_clip(&mut project, "clip-1").unwrap().notes.is_empty());
    }

    #[test]
    fn a_note_for_a_clip_we_do_not_have_is_let_go() {
        let (mut project, _) = project_with_track();
        let applied = apply(
            &mut project,
            &message(SyncKind::Note(NoteSync {
                clip_id: "never-heard-of-it".into(),
                operation: NoteOp::Delete { tick: 0, pitch: 60 },
            })),
        );
        assert!(applied.missed);
    }

    #[test]
    fn chat_and_presence_change_nothing_in_the_song() {
        let (mut project, _) = project_with_track();
        let before = project.tracks.len();
        let applied = apply(
            &mut project,
            &message(SyncKind::Chat {
                body: "move the drop".into(),
            }),
        );
        assert!(applied.missed);
        assert!(!applied.rebuild_graph);
        assert_eq!(project.tracks.len(), before);
    }

    #[test]
    fn a_track_kind_is_not_changed_by_any_of_this() {
        let (mut project, track_id) = project_with_track();
        apply(
            &mut project,
            &message(SyncKind::Mixer(MixerSync {
                track_id: track_id.clone(),
                volume_db: Some(-3.0),
                pan: None,
                muted: None,
            })),
        );
        assert!(matches!(
            project.track(&track_id).unwrap().kind,
            TrackKind::Midi
        ));
    }
}
