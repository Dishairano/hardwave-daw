//! What one undo step changed, in a few words, for the history list.
//!
//! Well over a hundred commands snapshot the song before they change it,
//! and few of them say what they are about to do. The history list used
//! to be a log the app kept beside the real undo steps, and the two
//! drifted apart: a note drawn in the piano roll or a fader moved on a
//! desk was an undo step the list never showed, so clicking a row undid
//! something else than the row said. A step without a label of its own
//! is now described from the song before and after it.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use crate::clip::{ClipContent, ClipPlacement};
use crate::project::Project;
use crate::snapshot::{clip_id as kept_clip_id, same, same_items, same_track, Snapshot, TrackPart};

/// What an edit changed: a template such as `Add {n} notes: "{name}"`,
/// which the app translates, and what goes into it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct Change {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A track's name before a rename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<usize>,
}

impl Change {
    fn plain(label: &str) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    fn on(label: &str, name: &str) -> Self {
        Self {
            label: label.into(),
            name: Some(name.into()),
            ..Self::default()
        }
    }

    fn counted(label: &str, n: usize, name: Option<&str>) -> Self {
        Self {
            label: label.into(),
            name: name.map(str::to_string),
            n: Some(n),
            ..Self::default()
        }
    }

    /// The template filled in, in English.
    pub fn text(&self) -> String {
        let mut out = self.label.clone();
        if let Some(name) = &self.name {
            out = out.replace("{name}", name);
        }
        if let Some(old) = &self.old {
            out = out.replace("{old}", old);
        }
        if let Some(n) = self.n {
            out = out.replace("{n}", &n.to_string());
        }
        out
    }
}

/// What changed from `before` to `after`, such as `Add {n} notes:
/// "{name}"` with the count and the track's name.
pub fn describe_change(before: &Project, after: &Project) -> Change {
    match (serde_json::to_value(before), serde_json::to_value(after)) {
        (Ok(a), Ok(b)) => describe_values(&a, &b),
        _ => Change::plain("Edit"),
    }
}

/// What changed from `before` to `after`, read off two undo snapshots
/// rather than whole songs: the same words [`describe_change`] finds.
///
/// Comparing two whole songs as JSON took about 190 ms per step on an FL
/// song with 501 tracks and 3,166 clips, and the history list did it on
/// the window's thread for every step it had not described yet. Two
/// neighbouring snapshots share every part an edit did not change, so
/// equal parts are told apart by their pointers and only the changed
/// ones are looked into.
pub fn describe_snapshots(before: &Snapshot, after: &Snapshot) -> Change {
    if let Some(change) = describe_track_parts(&before.tracks, &after.tracks) {
        return change;
    }
    let (a, b) = (&*before.rest, &*after.rest);
    let rest_same = Arc::ptr_eq(&before.rest, &after.rest);
    let arrangements_same = same(&before.arrangements, &after.arrangements);
    if rest_same && arrangements_same {
        return Change::plain("No change");
    }
    for (key, label) in PARTS {
        let differs = match *key {
            "arrangements" => !arrangements_same,
            _ if rest_same => false,
            "tempo_map" => a.tempo_map != b.tempo_map,
            "channel_rack_state" => a.channel_rack_state != b.channel_rack_state,
            // Not written with the song: it is read from the channel rack,
            // which is looked at just before.
            "step_rack" => false,
            "active_arrangement" => a.active_arrangement != b.active_arrangement,
            "macros" => a.macros != b.macros,
            "vcas" => a.vcas != b.vcas,
            "modulations" => a.modulations != b.modulations,
            "scenes" => a.scenes != b.scenes,
            "timeline_state" => a.timeline_state != b.timeline_state,
            "midi_mappings" => a.midi_mappings != b.midi_mappings,
            "plugin_states" => a.plugin_states != b.plugin_states,
            "video" => a.video != b.video,
            "keyboard_track" => a.keyboard_track != b.keyboard_track,
            "metadata" => a.metadata != b.metadata,
            _ => unreachable!("a part of the song with no comparison: {key}"),
        };
        if differs {
            return Change::plain(label);
        }
    }
    // Something else outside the tracks: rare, so read as the song file
    // writes it, which leaves out what is never saved.
    let same_as_saved = match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    Change::plain(if same_as_saved { "No change" } else { "Edit" })
}

fn describe_track_parts(a: &[Arc<TrackPart>], b: &[Arc<TrackPart>]) -> Option<Change> {
    if a.len() == b.len() && a.iter().zip(b).all(|(ta, tb)| same_track(ta, tb)) {
        return None;
    }
    let id = part_id;
    let ids_a: HashSet<&str> = a.iter().map(id).collect();
    let ids_b: HashSet<&str> = b.iter().map(id).collect();
    let added: Vec<&Arc<TrackPart>> = b.iter().filter(|t| !ids_a.contains(id(t))).collect();
    let removed: Vec<&Arc<TrackPart>> = a.iter().filter(|t| !ids_b.contains(id(t))).collect();
    match (added.len(), removed.len()) {
        (0, 0) => {}
        (1, 0) => return Some(Change::on("Add track \"{name}\"", &added[0].settings.name)),
        (n, 0) => return Some(Change::counted("Add {n} tracks", n, None)),
        (0, 1) => {
            return Some(Change::on(
                "Delete track \"{name}\"",
                &removed[0].settings.name,
            ))
        }
        (0, n) => return Some(Change::counted("Delete {n} tracks", n, None)),
        _ => return Some(Change::plain("Change tracks")),
    }
    if !a.iter().map(id).eq(b.iter().map(id)) {
        return Some(Change::plain("Move tracks"));
    }
    if let Some(to) = clip_part_moved_to(a, b) {
        return Some(Change::on("Move clip to \"{name}\"", &to.settings.name));
    }
    a.iter()
        .zip(b)
        .find(|(ta, tb)| !same_track(ta, tb))
        .map(|(ta, tb)| describe_track_part(ta, tb))
}

fn part_id(track: &Arc<TrackPart>) -> &str {
    &track.settings.id
}

/// The track a clip went to, when one moved from one track to another
/// (see `clip_moved_to`). Tracks whose clips are the very same ones
/// gained none and are passed over.
fn clip_part_moved_to<'a>(
    a: &[Arc<TrackPart>],
    b: &'a [Arc<TrackPart>],
) -> Option<&'a Arc<TrackPart>> {
    for (ta, tb) in a.iter().zip(b) {
        if same_items(&ta.clips, &tb.clips) {
            continue;
        }
        let before: HashSet<&str> = ta.clips.iter().map(|c| kept_clip_id(c)).collect();
        let Some(gained) = tb
            .clips
            .iter()
            .map(|c| kept_clip_id(c))
            .find(|id| !before.contains(id))
        else {
            continue;
        };
        let came_from_elsewhere = a
            .iter()
            .filter(|other| other.settings.id != ta.settings.id)
            .any(|other| other.clips.iter().any(|c| kept_clip_id(c) == gained));
        if came_from_elsewhere {
            return Some(tb);
        }
    }
    None
}

fn describe_track_part(a: &Arc<TrackPart>, b: &Arc<TrackPart>) -> Change {
    let name = b.settings.name.as_str();
    if a.settings.name != b.settings.name {
        return Change {
            old: Some(a.settings.name.clone()),
            ..Change::on("Rename \"{old}\" to \"{name}\"", name)
        };
    }
    if !same_items(&a.clips, &b.clips) {
        return describe_clip_parts(&a.clips, &b.clips, name);
    }
    if a.settings.frozen != b.settings.frozen {
        let label = if b.settings.frozen {
            "Freeze: \"{name}\""
        } else {
            "Unfreeze: \"{name}\""
        };
        return Change::on(label, name);
    }
    // The track's own settings, without its clips, read as the song file
    // writes them: only when they changed, and only this one track.
    let settings = (!same(&a.settings, &b.settings)).then(|| {
        (
            serde_json::to_value(&*a.settings),
            serde_json::to_value(&*b.settings),
        )
    });
    let changed = |key: &str| match (key, &settings) {
        ("automation_clips", _) => !same_items(&a.automation_clips, &b.automation_clips),
        (_, Some((Ok(va), Ok(vb)))) => va.get(key) != vb.get(key),
        (_, Some(_)) => true,
        (_, None) => false,
    };
    let label = TRACK_SETTINGS
        .iter()
        .find(|(key, _)| changed(key))
        .map_or("Edit \"{name}\"", |(_, label)| *label);
    Change::on(label, name)
}

/// The notes of a MIDI clip; an audio clip has none.
fn notes_of(clip: &ClipPlacement) -> Option<&Vec<hardwave_midi::MidiNote>> {
    match &clip.content {
        ClipContent::Midi(midi) => Some(&midi.clip.notes),
        ClipContent::Audio(_) => None,
    }
}

fn describe_clip_parts(a: &[Arc<ClipPlacement>], b: &[Arc<ClipPlacement>], name: &str) -> Change {
    if b.len() > a.len() {
        return counted(
            b.len() - a.len(),
            "Add clip: \"{name}\"",
            "Add {n} clips: \"{name}\"",
            name,
        );
    }
    if a.len() > b.len() {
        return counted(
            a.len() - b.len(),
            "Delete clip: \"{name}\"",
            "Delete {n} clips: \"{name}\"",
            name,
        );
    }
    let Some((ca, cb)) = a.iter().zip(b).find(|(ca, cb)| !same(ca, cb)) else {
        return Change::on("Clips: \"{name}\"", name);
    };
    let (na, nb) = (notes_of(ca), notes_of(cb));
    if na != nb {
        return match (na.map(Vec::len), nb.map(Vec::len)) {
            (Some(x), Some(y)) if y > x => counted(
                y - x,
                "Add note: \"{name}\"",
                "Add {n} notes: \"{name}\"",
                name,
            ),
            (Some(x), Some(y)) if x > y => counted(
                x - y,
                "Delete note: \"{name}\"",
                "Delete {n} notes: \"{name}\"",
                name,
            ),
            _ => Change::on("Edit notes: \"{name}\"", name),
        };
    }
    let label = if ca.position_ticks != cb.position_ticks
        || ca.lane != cb.lane
        || ca.track_id != cb.track_id
    {
        "Move clip: \"{name}\""
    } else if ca.length_ticks != cb.length_ticks {
        "Resize clip: \"{name}\""
    } else {
        "Edit clip: \"{name}\""
    };
    Change::on(label, name)
}

/// The parts of the song outside the tracks, in the order they are
/// looked at, with what a change to each is called.
const PARTS: &[(&str, &str)] = &[
    ("tempo_map", "Tempo"),
    ("channel_rack_state", "Channel rack"),
    ("step_rack", "Channel rack"),
    ("arrangements", "Arrangements"),
    ("active_arrangement", "Switch arrangement"),
    ("macros", "Macros"),
    ("vcas", "VCA faders"),
    ("modulations", "Modulation"),
    ("scenes", "Scenes"),
    ("timeline_state", "Timeline"),
    ("midi_mappings", "MIDI mappings"),
    ("plugin_states", "Plug-in settings"),
    ("video", "Video"),
    ("keyboard_track", "Keyboard track"),
    ("metadata", "Song info"),
];

/// A track setting, with what a change to it is called.
const TRACK_SETTINGS: &[(&str, &str)] = &[
    ("volume_db", "Volume: \"{name}\""),
    ("pan", "Pan: \"{name}\""),
    ("muted", "Mute: \"{name}\""),
    ("soloed", "Solo: \"{name}\""),
    ("solo_safe", "Solo safe: \"{name}\""),
    ("armed", "Arm: \"{name}\""),
    ("inserts", "Plug-ins: \"{name}\""),
    ("sends", "Sends: \"{name}\""),
    ("automation_lanes", "Automation: \"{name}\""),
    ("automation_clips", "Automation clips: \"{name}\""),
    ("stereo_separation", "Width: \"{name}\""),
    ("phase_invert", "Polarity: \"{name}\""),
    ("swap_lr", "Swap left and right: \"{name}\""),
    ("delay_samples", "Track delay: \"{name}\""),
    ("pitch_semitones", "Pitch: \"{name}\""),
    ("fine_tune_cents", "Fine tune: \"{name}\""),
    ("filter_type", "Filter: \"{name}\""),
    ("filter_cutoff_hz", "Filter: \"{name}\""),
    ("filter_resonance", "Filter: \"{name}\""),
    ("output_bus", "Output: \"{name}\""),
    ("monitor_input", "Monitoring: \"{name}\""),
    ("monitor_auto", "Monitoring: \"{name}\""),
    ("input_source", "Input: \"{name}\""),
    ("input_channel", "Input: \"{name}\""),
    ("color", "Colour: \"{name}\""),
    ("instrument", "Instrument: \"{name}\""),
    ("kick_patch", "Kick: \"{name}\""),
    ("midi_fx", "MIDI effects: \"{name}\""),
    ("midi_route_to", "MIDI route: \"{name}\""),
    ("mpe", "MPE: \"{name}\""),
    ("tuning", "Tuning: \"{name}\""),
    ("session_slots", "Launcher clips: \"{name}\""),
    ("stem_excluded", "Stems: \"{name}\""),
    ("kind", "Track type: \"{name}\""),
];

fn describe_values(a: &Value, b: &Value) -> Change {
    if let Some(change) = describe_tracks(a.get("tracks"), b.get("tracks")) {
        return change;
    }
    for (key, label) in PARTS {
        if a.get(key) != b.get(key) {
            return Change::plain(label);
        }
    }
    Change::plain(if a == b { "No change" } else { "Edit" })
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn describe_tracks(a: Option<&Value>, b: Option<&Value>) -> Option<Change> {
    let (Some(Value::Array(a)), Some(Value::Array(b))) = (a, b) else {
        return None;
    };
    if a == b {
        return None;
    }
    let ids_a: Vec<&str> = a.iter().map(|t| text(t, "id")).collect();
    let ids_b: Vec<&str> = b.iter().map(|t| text(t, "id")).collect();
    let added: Vec<&Value> = b
        .iter()
        .filter(|t| !ids_a.contains(&text(t, "id")))
        .collect();
    let removed: Vec<&Value> = a
        .iter()
        .filter(|t| !ids_b.contains(&text(t, "id")))
        .collect();
    match (added.len(), removed.len()) {
        (0, 0) => {}
        (1, 0) => return Some(Change::on("Add track \"{name}\"", text(added[0], "name"))),
        (n, 0) => return Some(Change::counted("Add {n} tracks", n, None)),
        (0, 1) => {
            return Some(Change::on(
                "Delete track \"{name}\"",
                text(removed[0], "name"),
            ))
        }
        (0, n) => return Some(Change::counted("Delete {n} tracks", n, None)),
        _ => return Some(Change::plain("Change tracks")),
    }
    if ids_a != ids_b {
        return Some(Change::plain("Move tracks"));
    }
    // A clip dragged to another track leaves one track and lands on
    // another; read one at a time, that is a delete on the first.
    if let Some(to) = clip_moved_to(a, b) {
        return Some(Change::on("Move clip to \"{name}\"", text(to, "name")));
    }
    a.iter()
        .zip(b)
        .find(|(ta, tb)| ta != tb)
        .map(|(ta, tb)| describe_track(ta, tb))
}

/// The id of a clip, audio or MIDI.
fn clip_id(clip: &Value) -> Option<&str> {
    clip.pointer("/content/Audio/id")
        .or_else(|| clip.pointer("/content/Midi/id"))
        .and_then(Value::as_str)
}

/// The track a clip went to, when one moved from one track to another.
fn clip_moved_to<'a>(a: &[Value], b: &'a [Value]) -> Option<&'a Value> {
    let clips = |t: &Value| -> Vec<String> {
        t.get("clips")
            .and_then(Value::as_array)
            .map(|cs| cs.iter().filter_map(clip_id).map(str::to_string).collect())
            .unwrap_or_default()
    };
    for (ta, tb) in a.iter().zip(b) {
        let before = clips(ta);
        let after = clips(tb);
        if let Some(gained) = after.iter().find(|id| !before.contains(id)) {
            let came_from_elsewhere = a
                .iter()
                .filter(|other| text(other, "id") != text(ta, "id"))
                .any(|other| clips(other).contains(gained));
            if came_from_elsewhere {
                return Some(tb);
            }
        }
    }
    None
}

fn describe_track(a: &Value, b: &Value) -> Change {
    let name = text(b, "name");
    let changed = |key: &str| a.get(key) != b.get(key);
    if changed("name") {
        return Change {
            old: Some(text(a, "name").into()),
            ..Change::on("Rename \"{old}\" to \"{name}\"", name)
        };
    }
    if changed("clips") {
        return describe_clips(a.get("clips"), b.get("clips"), name);
    }
    if changed("frozen") {
        let frozen = b.get("frozen").and_then(Value::as_bool).unwrap_or(false);
        let label = if frozen {
            "Freeze: \"{name}\""
        } else {
            "Unfreeze: \"{name}\""
        };
        return Change::on(label, name);
    }
    let label = TRACK_SETTINGS
        .iter()
        .find(|(key, _)| changed(key))
        .map_or("Edit \"{name}\"", |(_, label)| *label);
    Change::on(label, name)
}

/// A count of one has a label of its own ("Add note"), more one with
/// `{n}` in it ("Add {n} notes").
fn counted(n: usize, one: &str, many: &str, name: &str) -> Change {
    if n == 1 {
        Change::on(one, name)
    } else {
        Change::counted(many, n, Some(name))
    }
}

fn describe_clips(a: Option<&Value>, b: Option<&Value>, name: &str) -> Change {
    let (Some(Value::Array(a)), Some(Value::Array(b))) = (a, b) else {
        return Change::on("Clips: \"{name}\"", name);
    };
    if b.len() > a.len() {
        return counted(
            b.len() - a.len(),
            "Add clip: \"{name}\"",
            "Add {n} clips: \"{name}\"",
            name,
        );
    }
    if a.len() > b.len() {
        return counted(
            a.len() - b.len(),
            "Delete clip: \"{name}\"",
            "Delete {n} clips: \"{name}\"",
            name,
        );
    }
    let Some((ca, cb)) = a.iter().zip(b).find(|(ca, cb)| ca != cb) else {
        return Change::on("Clips: \"{name}\"", name);
    };
    const NOTES: &str = "/content/Midi/clip/notes";
    let notes = |c: &Value| c.pointer(NOTES).and_then(Value::as_array).map(Vec::len);
    if ca.pointer(NOTES) != cb.pointer(NOTES) {
        return match (notes(ca), notes(cb)) {
            (Some(x), Some(y)) if y > x => counted(
                y - x,
                "Add note: \"{name}\"",
                "Add {n} notes: \"{name}\"",
                name,
            ),
            (Some(x), Some(y)) if x > y => counted(
                x - y,
                "Delete note: \"{name}\"",
                "Delete {n} notes: \"{name}\"",
                name,
            ),
            _ => Change::on("Edit notes: \"{name}\"", name),
        };
    }
    let moved = |key: &str| ca.get(key) != cb.get(key);
    let label = if moved("position_ticks") || moved("lane") || moved("track_id") {
        "Move clip: \"{name}\""
    } else if moved("length_ticks") {
        "Resize clip: \"{name}\""
    } else {
        "Edit clip: \"{name}\""
    };
    Change::on(label, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{ClipContent, ClipPlacement, MidiClipRef};
    use crate::track::Track;

    fn song() -> Project {
        let mut p = Project::default();
        let mut lead = Track::new_midi("lead".into(), "Lead".into());
        lead.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "c1".into(),
                clip: hardwave_midi::MidiClip::new("m1".into(), "Lead".into(), 3840),
            }),
            track_id: "lead".into(),
            position_ticks: 0,
            length_ticks: 3840,
            lane: 0,
            color: None,
            row: None,
        });
        p.tracks.push(lead);
        p
    }

    #[test]
    fn a_drawn_note_is_named_with_its_track() {
        let before = song();
        let mut after = before.clone();
        if let ClipContent::Midi(m) = &mut after.tracks.last_mut().unwrap().clips[0].content {
            m.clip.notes.push(hardwave_midi::MidiNote {
                pitch: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
                channel: 0,
                muted: false,
                pan: 0.0,
                fine_cents: 0.0,
                release_velocity: 0.5,
            });
        }
        assert_eq!(
            describe_change(&before, &after).text(),
            "Add note: \"Lead\""
        );
        assert_eq!(
            describe_change(&after, &before).text(),
            "Delete note: \"Lead\""
        );
    }

    #[test]
    fn tracks_added_deleted_and_mixed_are_told_apart() {
        let before = song();
        let mut after = before.clone();
        after
            .tracks
            .push(Track::new_audio("kick".into(), "Kick".into()));
        assert_eq!(
            describe_change(&before, &after).text(),
            "Add track \"Kick\""
        );
        assert_eq!(
            describe_change(&after, &before).text(),
            "Delete track \"Kick\""
        );

        let mut louder = before.clone();
        louder.tracks.last_mut().unwrap().volume_db = -3.0;
        assert_eq!(describe_change(&before, &louder).text(), "Volume: \"Lead\"");

        let mut moved = before.clone();
        moved.tracks.last_mut().unwrap().clips[0].position_ticks = 960;
        assert_eq!(
            describe_change(&before, &moved).text(),
            "Move clip: \"Lead\""
        );
    }

    #[test]
    fn a_clip_dragged_to_another_track_is_a_move_there() {
        let mut before = song();
        before
            .tracks
            .push(Track::new_midi("bass".into(), "Bass".into()));
        let mut after = before.clone();
        let lead = after.tracks.len() - 2;
        let clip = after.tracks[lead].clips.remove(0);
        after.tracks.last_mut().unwrap().clips.push(clip);
        assert_eq!(
            describe_change(&before, &after).text(),
            "Move clip to \"Bass\""
        );
    }

    #[test]
    fn a_step_that_changed_nothing_says_so() {
        let before = song();
        assert_eq!(
            describe_change(&before, &before.clone()).text(),
            "No change"
        );
    }

    /// Every kind of edit, as (what it is, the song before, the song after).
    fn edits() -> Vec<(&'static str, Project, Project)> {
        let mut base = song();
        base.tracks
            .push(Track::new_midi("bass".into(), "Bass".into()));
        base.ensure_arrangements();
        let mut out = Vec::new();
        let mut edit = |what: &'static str, f: &dyn Fn(&mut Project)| {
            let mut after = base.clone();
            f(&mut after);
            out.push((what, base.clone(), after));
        };
        let lead = |p: &mut Project| p.tracks.len() - 2;
        let note = hardwave_midi::MidiNote::default();
        edit("nothing", &|_| {});
        edit("a note", &|p| {
            let i = lead(p);
            if let ClipContent::Midi(m) = &mut p.tracks[i].clips[0].content {
                m.clip.notes.push(note.clone());
                m.clip.notes.push(note.clone());
            }
        });
        edit("a pattern made longer", &|p| {
            let i = lead(p);
            if let ClipContent::Midi(m) = &mut p.tracks[i].clips[0].content {
                m.clip.length_ticks += 1;
            }
        });
        edit("a track", &|p| {
            p.tracks
                .push(Track::new_audio("kick".into(), "Kick".into()))
        });
        edit("two tracks", &|p| {
            p.tracks
                .push(Track::new_audio("kick".into(), "Kick".into()));
            p.tracks.push(Track::new_audio("hat".into(), "Hat".into()));
        });
        edit("a track gone", &|p| {
            p.tracks.pop();
        });
        edit("a track swapped", &|p| {
            p.tracks.pop();
            p.tracks
                .push(Track::new_audio("kick".into(), "Kick".into()));
        });
        edit("tracks moved", &|p| {
            let n = p.tracks.len();
            p.tracks.swap(n - 1, n - 2);
        });
        edit("renamed", &|p| {
            let i = lead(p);
            p.tracks[i].name = "Lead 2".into();
        });
        edit("a clip to another track", &|p| {
            let i = lead(p);
            let clip = p.tracks[i].clips.remove(0);
            p.tracks.last_mut().unwrap().clips.push(clip);
        });
        edit("a clip moved", &|p| {
            let i = lead(p);
            p.tracks[i].clips[0].position_ticks = 960;
        });
        edit("a clip resized", &|p| {
            let i = lead(p);
            p.tracks[i].clips[0].length_ticks = 960;
        });
        edit("a clip coloured", &|p| {
            let i = lead(p);
            p.tracks[i].clips[0].color = Some("#ff0000".into());
        });
        edit("a clip added", &|p| {
            let i = lead(p);
            let clip = p.tracks[i].clips[0].clone();
            p.tracks[i].clips.push(clip);
        });
        edit("a clip deleted", &|p| {
            let i = lead(p);
            p.tracks[i].clips.clear();
        });
        edit("frozen", &|p| {
            let i = lead(p);
            p.tracks[i].frozen = true;
        });
        edit("volume", &|p| {
            let i = lead(p);
            p.tracks[i].volume_db = -6.0;
        });
        edit("muted and panned", &|p| {
            let i = lead(p);
            p.tracks[i].muted = true;
            p.tracks[i].pan = 0.5;
        });
        edit("an automation clip", &|p| {
            let i = lead(p);
            p.tracks[i]
                .automation_clips
                .push(crate::automation_clip::AutomationClip::new(
                    "a1",
                    crate::automation::AutomationTarget::TrackVolume,
                    0,
                    3840,
                ));
        });
        edit("colour", &|p| {
            let i = lead(p);
            p.tracks[i].color = "#00ff00".into();
        });
        edit("tempo", &|p| p.tempo_map.entries[0].bpm = 150.0);
        edit("channel rack", &|p| {
            p.channel_rack_state = Some("{\"patterns\":[]}".into())
        });
        edit("markers", &|p| p.timeline_state = Some("{}".into()));
        edit("arrangement", &|p| p.arrangements[0].name = "B".into());
        edit("song info", &|p| p.metadata.title = "Song".into());
        edit("a playlist row", &|p| p.rows[2].name = "Drums".into());
        edit("a rack group", &|p| p.rack_groups.push("Audio".into()));
        edit("tempo and a track", &|p| {
            p.tempo_map.entries[0].bpm = 150.0;
            let i = lead(p);
            p.tracks[i].volume_db = -6.0;
        });
        out
    }

    /// Read off two undo snapshots, an edit is described in the very words
    /// a comparison of the two whole songs finds, whether the snapshots
    /// share their unchanged parts or not.
    #[test]
    fn snapshots_describe_an_edit_as_the_whole_songs_do() {
        for (what, mut before, mut after) in edits() {
            let expected = describe_change(&before, &after);
            let (a, _) = Snapshot::take(&mut before, None);
            let (shared, _) = Snapshot::take(&mut after, Some(&a));
            let (alone, _) = Snapshot::take(&mut after, None);
            assert_eq!(describe_snapshots(&a, &shared), expected, "{what}");
            assert_eq!(
                describe_snapshots(&a, &alone),
                expected,
                "{what}, nothing shared"
            );
            // And the edit undone.
            let back = describe_change(&after, &before);
            assert_eq!(describe_snapshots(&shared, &a), back, "{what} undone");
        }
    }

    #[test]
    fn every_part_of_the_song_is_compared() {
        let mut a = song();
        let mut b = a.clone();
        let (a, _) = Snapshot::take(&mut a, None);
        let (b, _) = Snapshot::take(&mut b, None);
        // Not shared, so every part is looked at; none may be missing.
        assert_eq!(describe_snapshots(&a, &b).text(), "No change");
    }

    #[test]
    fn a_tempo_change_is_named() {
        let before = song();
        let mut after = before.clone();
        after.tempo_map.entries[0].bpm = 150.0;
        assert_eq!(describe_change(&before, &after).text(), "Tempo");
    }
}
