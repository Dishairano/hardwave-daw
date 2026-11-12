//! DawEngine — the top-level orchestrator that owns the audio graph, transport,
//! plugin host, and project state. Bridges between the audio callback and the UI.

use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use std::sync::Arc;

use hardwave_audio_io::{AudioCallback, AudioDeviceManager};
use hardwave_metering::{ChannelMeter, MeterSnapshot};
use hardwave_midi::{MidiCaptureRing, MidiInputManager};
use hardwave_plugin_host::PluginScanner;
use hardwave_project::Project;

/// Factory that instantiates a plug-in by its scanner id, used to
/// hydrate insert chains for offline render (so exports apply the same FX
/// as live playback). The closure returning `None` skips that insert.
/// Implemented by the command layer, which owns the native/VST3/CLAP
/// factory + scanner.
pub type OfflineInsertFactory<'a> =
    &'a dyn Fn(&str) -> Option<Box<dyn hardwave_plugin_host::types::HostedPlugin>>;

/// Separator that marks an audio-pool id as a derived (baked stretch /
/// pitch) variant rather than a real imported source. Real source ids are
/// file paths or import hashes and never contain it.
const STRETCH_KEY_MARKER: &str = "#hw-stretch:";
const WARP_KEY_MARKER: &str = "#hw-warp:";
const TUNE_KEY_MARKER: &str = "#hw-tune:";

/// Pool id for a take played with its notes tuned by hand (the sample
/// window's NOTES view): the source and a fingerprint of its notes. The
/// clip's stretch and warp are baked from this, as from the file.
pub(crate) fn tune_cache_key(
    source_id: &str,
    notes: &[hardwave_project::clip::TunedNote],
) -> String {
    format!(
        "{source_id}{TUNE_KEY_MARKER}{:016x}",
        hardwave_project::clip::tuned_notes_hash(notes)
    )
}

/// What a clip plays from before its stretch: its take with the notes
/// tuned by hand when it has any, else the file.
pub(crate) fn playing_source(audio_clip: &hardwave_project::clip::AudioClip) -> String {
    if audio_clip.has_tuned_notes() {
        tune_cache_key(&audio_clip.source_path, &audio_clip.tuned_notes)
    } else {
        audio_clip.source_path.clone()
    }
}

/// The tuned-take id a pool id was made from (itself, or a stretch or
/// warp of it), or None when it was not made from one.
fn tuned_take_of(id: &str) -> Option<&str> {
    let at = id.find(TUNE_KEY_MARKER)?;
    id.get(..at + TUNE_KEY_MARKER.len() + 16)
}

/// One take with notes tuned by hand, to bake.
#[derive(Debug, Clone)]
struct TuneBake {
    key: String,
    source_id: String,
    notes: Vec<hardwave_project::clip::TunedNote>,
}

/// Pool id for a warped clip rendered with its pitch kept: the source, how
/// its markers cut it, its transpose, and the rate it was rendered at (an
/// export at another rate needs its own).
fn warp_cache_key(
    source_id: &str,
    pieces: &[hardwave_dsp::stretch_apply::WarpPiece],
    semitones: f64,
    sample_rate: f64,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    for p in pieces {
        p.out_start.hash(&mut h);
        p.src_start.to_bits().hash(&mut h);
        p.step.to_bits().hash(&mut h);
    }
    semitones.to_bits().hash(&mut h);
    sample_rate.to_bits().hash(&mut h);
    format!("{source_id}{WARP_KEY_MARKER}{:016x}", h.finish())
}

/// How a warped clip's markers cut its source, at `sample_rate`: output
/// sample n is the clip's nth sample on the timeline. Marker ticks are
/// clip-relative; going through the tempo map (absolute tick to samples,
/// minus the clip's start) keeps them honest under tempo changes.
fn warp_pieces(
    clip: &hardwave_project::clip::ClipPlacement,
    audio_clip: &hardwave_project::clip::AudioClip,
    tempo_map: &hardwave_project::tempo::TempoMap,
    sample_rate: f64,
    unity_step: f64,
) -> Vec<hardwave_dsp::stretch_apply::WarpPiece> {
    let timeline_start = tempo_map.tick_to_samples(clip.position_ticks, sample_rate);
    let anchors: Vec<(u64, f64)> = audio_clip
        .warp_markers
        .iter()
        .map(|m| {
            let abs = tempo_map.tick_to_samples(clip.position_ticks + m.clip_tick, sample_rate);
            (abs.saturating_sub(timeline_start), m.source_sample as f64)
        })
        .collect();
    crate::track_node::build_warp_segments(&anchors, unity_step, audio_clip.source_start as f64)
        .into_iter()
        .map(|seg| hardwave_dsp::stretch_apply::WarpPiece {
            out_start: seg.start_sample,
            src_start: seg.source_start,
            step: seg.step,
        })
        .collect()
}

/// One warped clip's render with its pitch kept, to bake.
struct WarpBake {
    key: String,
    source_id: String,
    pieces: Vec<hardwave_dsp::stretch_apply::WarpPiece>,
    semitones: f64,
}

/// The part of the song a render plays, in samples at its rate: only the
/// clips that sound in it need their bakes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BakeSpan {
    from: u64,
    to: u64,
    sample_rate: f64,
}

impl BakeSpan {
    /// Whether `clip` sounds in `span`; every clip does when there is none.
    fn plays(
        span: Option<BakeSpan>,
        clip: &hardwave_project::clip::ClipPlacement,
        tempo_map: &hardwave_project::tempo::TempoMap,
    ) -> bool {
        let Some(span) = span else {
            return true;
        };
        let start = tempo_map.tick_to_samples(clip.position_ticks, span.sample_rate);
        let end = tempo_map.tick_to_samples(
            clip.position_ticks.saturating_add(clip.length_ticks),
            span.sample_rate,
        );
        start < span.to && end > span.from
    }
}

/// Pool id for a baked stretch / pitch-shift variant of a source. The
/// original id stays untouched in the project, so `rehydrate_audio_pool`
/// (which walks clips' `source_path`) never sees these derived keys.
fn stretch_cache_key(source_id: &str, stretch: f64, semitones: f64) -> String {
    format!("{source_id}{STRETCH_KEY_MARKER}{stretch:.6}:{semitones:.6}")
}

/// Drops a set of stretch-bake keys out of the in-flight set when it goes out
/// of scope, so a panicking bake can't leave them permanently claimed.
struct InflightRelease<'a> {
    set: &'a Arc<Mutex<std::collections::HashSet<String>>>,
    keys: Vec<String>,
}

impl Drop for InflightRelease<'_> {
    fn drop(&mut self) {
        let mut guard = self.set.lock();
        for key in &self.keys {
            guard.remove(key);
        }
    }
}

/// Does this clip want a pitch-preserving bake? Warped clips are excluded —
/// warp markers define their own piecewise source map and take precedence.
/// Returns the effective (stretch, semitones) when a bake applies.
pub(crate) fn bake_params(audio_clip: &hardwave_project::clip::AudioClip) -> Option<(f64, f64)> {
    // A warped clip follows its markers, and a resampled one plays like a
    // tape: neither is baked.
    if !audio_clip.warp_markers.is_empty() || audio_clip.treatment.resample {
        return None;
    }
    let stretch = if audio_clip.stretch_ratio <= 0.01 {
        1.0
    } else {
        audio_clip.stretch_ratio
    };
    if (stretch - 1.0).abs() > 1e-4 || audio_clip.pitch_semitones.abs() > 1e-4 {
        Some((stretch, audio_clip.pitch_semitones))
    } else {
        None
    }
}
use rtrb::RingBuffer;

use std::collections::HashMap;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioGraph, ProcessContext};
use crate::input_node::{
    CaptureTap, InputBlock, InputHub, InputNode, InputRing, RecordTapNode, SharedInputBlock,
    SharedInputConsumer, MAX_RECORD_SOURCES,
};
use crate::master_node::MasterNode;
use crate::master_tap::{self, SharedMasterTap};
use crate::track_node::{ClipRegion, TrackMeterState, TrackNode};
use crate::transport::{TransportCommand, TransportState};
use hardwave_project::describe::describe_snapshots;
use hardwave_project::snapshot::Snapshot;

mod graph_build;
mod offline_bakes;
pub use offline_bakes::OfflineBakes;
#[cfg(test)]
mod graph_swap_tests;
use graph_build::{
    BuildMode, CarryKind, ChainBoard, GraphBuilder, MonitorEdges, PreparedGraph, SendEdges,
};
use hardwave_project::{InputChannel, RecordSource};

/// Shared per-track meter map. Rebuilt when the audio graph is rebuilt.
pub type TrackMeterMap = Arc<Mutex<HashMap<String, Arc<TrackMeterState>>>>;

/// Maximum number of snapshots kept on each side of the history.
const HISTORY_CAP: usize = 256;

/// Roughly how many bytes the history may hold before its oldest steps
/// go. Steps share what did not change, so a small edit costs a few
/// kilobytes even on a big song; this only bites when edit after edit
/// changes the whole song (moving every clip of a 3,000-clip FL song,
/// over and over), where each step is a whole copy again.
const HISTORY_BYTES: usize = 128 * 1024 * 1024;

/// How many modulation routes the audio thread follows. Its state is sized
/// for this many up front, so it never allocates; routes past it are left
/// still.
const MAX_MOD_ROUTES: usize = 256;

/// Snapshot-based undo/redo. Each step keeps the song in parts it shares
/// with the step next to it ([`Snapshot`]), so a step costs what its edit
/// changed; audio samples live in the AudioPool and are referenced by
/// source id.
pub struct History {
    /// The song before each edit, oldest first.
    undo: Vec<Step>,
    /// The song after each undone edit, the next one to redo last.
    redo: Vec<Step>,
    /// Steps were dropped off the bottom (past the cap), so the oldest
    /// one kept is not the song as it was opened.
    trimmed: bool,
    /// Every edit, undo and redo counts one, so the song can tell whether
    /// it changed since it was last saved or opened.
    edits: u64,
    /// `edits` when the song was last saved or opened.
    saved_at: u64,
    /// Roughly how many bytes the steps hold together.
    bytes: usize,
    /// `HISTORY_BYTES`, smaller in tests.
    max_bytes: usize,
    /// Counts every change to the steps, their names or the song the
    /// newest one leads to, so the list is worked out once per change
    /// and the window can be told when to read it again.
    version: u64,
    /// The list as last worked out, for `version`.
    listed: Option<Listed>,
}

/// The history list as worked out for one version of the history.
struct Listed {
    version: u64,
    rows: Vec<HistoryEntry>,
    applied: usize,
}

/// One edit: a song it can be undone or redone to, and what the edit was.
struct Step {
    song: Snapshot,
    /// What the app called the edit, when it said.
    label: Option<String>,
    /// What the edit changed, read off the two snapshots on either side
    /// of it as soon as both exist: when the next step is taken, or when
    /// the edit is undone or redone. Two neighbouring snapshots share what
    /// the edit did not change, so this costs what the edit changed.
    described: Option<hardwave_project::describe::Change>,
    /// When the edit was made, in milliseconds since 1970.
    at_ms: u64,
}

/// One row of the history list: the app's own name for the edit, or a
/// template describing it (`Add {n} notes: "{name}"`) with what goes in.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryEntry {
    #[serde(flatten)]
    pub change: hardwave_project::describe::Change,
    pub time: u64,
}

impl Step {
    fn new(song: Snapshot) -> Self {
        Self {
            song,
            label: None,
            described: None,
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Move the edit to the other stack: `song` becomes the song it holds
    /// and the song it held comes back. `undoing` says which way: the step
    /// holds the song before the edit when it is undone, and the song
    /// after it when it is redone. Returns the bytes the history grew by,
    /// which can be less than nothing.
    fn swap(&mut self, song: &mut Project, undoing: bool) -> isize {
        let back = self.song.to_project();
        // The song as it is shares its parts with the one it goes back to:
        // the two differ by this one edit.
        let (now, added) = Snapshot::take(song, Some(&self.song));
        // Both sides of the edit are snapshots now: the moment to say what
        // it was, if nothing has yet.
        let (before, after) = if undoing {
            (&self.song, &now)
        } else {
            (&now, &self.song)
        };
        describe_if_needed(&self.label, &mut self.described, before, after);
        let freed = self.song.bytes_freed_by_drop();
        self.song = now;
        *song = back;
        added as isize - freed as isize
    }
}

/// The row for an edit: the app's name for it, or what it changed.
fn history_row(step: &Step, change: Option<&hardwave_project::describe::Change>) -> HistoryEntry {
    let change = match (&step.label, change.or(step.described.as_ref())) {
        (Some(label), _) => hardwave_project::describe::Change {
            label: label.clone(),
            ..Default::default()
        },
        (None, Some(change)) => change.clone(),
        (None, None) => hardwave_project::describe::Change {
            label: "Edit".into(),
            ..Default::default()
        },
    };
    HistoryEntry {
        change,
        time: step.at_ms,
    }
}

/// What turned `before` into `after`, for a step that has no name or
/// description of its own yet.
fn describe_if_needed(
    label: &Option<String>,
    described: &mut Option<hardwave_project::describe::Change>,
    before: &Snapshot,
    after: &Snapshot,
) {
    if label.is_none() && described.is_none() {
        *described = Some(describe_snapshots(before, after));
    }
}

impl History {
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            trimmed: false,
            edits: 0,
            saved_at: 0,
            bytes: 0,
            max_bytes: HISTORY_BYTES,
            version: 0,
            listed: None,
        }
    }

    /// Counts every change the history list would show: a new step, an
    /// undo or redo, a name, a later move of the newest edit, a clear.
    pub fn version(&self) -> u64 {
        self.version
    }

    fn changed(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.listed = None;
    }

    /// Keep `song` as it is before an edit. It shares every part the edit
    /// does not change with the step before it, and is left as it was.
    pub fn push(&mut self, song: &mut Project) {
        self.note_edit();
        while let Some(step) = self.redo.pop() {
            self.forget(step);
        }
        let (snapshot, added) = Snapshot::take(song, self.undo.last().map(|step| &step.song));
        // The step before ends here: its edit is what lies between the
        // two, and the two share everything it did not change.
        if let Some(previous) = self.undo.last_mut() {
            describe_if_needed(
                &previous.label,
                &mut previous.described,
                &previous.song,
                &snapshot,
            );
        }
        self.bytes += added;
        self.undo.push(Step::new(snapshot));
        self.trim();
    }

    /// Drop a step, giving back what only it held.
    fn forget(&mut self, step: Step) {
        self.bytes = self.bytes.saturating_sub(step.song.bytes_freed_by_drop());
    }

    /// Keep the history within its caps: steps past `HISTORY_CAP` on
    /// either side go, then the oldest undo steps (and after them the
    /// furthest redo steps) while it holds more than `HISTORY_BYTES`. The
    /// newest step on each side always stays, so the last edit can be
    /// undone however big it is.
    fn trim(&mut self) {
        while self.undo.len() > HISTORY_CAP {
            let step = self.undo.remove(0);
            self.forget(step);
            self.trimmed = true;
        }
        while self.redo.len() > HISTORY_CAP {
            let step = self.redo.remove(0);
            self.forget(step);
        }
        while self.bytes > self.max_bytes {
            if self.undo.len() > 1 {
                let step = self.undo.remove(0);
                self.forget(step);
                self.trimmed = true;
            } else if self.redo.len() > 1 {
                let step = self.redo.remove(0);
                self.forget(step);
            } else {
                break;
            }
        }
    }

    /// Forget every step. Whether the song changed since it was saved is
    /// left as it is: clearing the list is not saving the song.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.trimmed = false;
        self.bytes = 0;
        self.changed();
    }

    /// The song changed without a step of its own: a later move of a
    /// gesture whose first move took the step.
    pub fn note_edit(&mut self) {
        self.edits += 1;
        self.changed();
    }

    /// The song is now what is on disk: it was just saved or opened.
    pub fn mark_saved(&mut self) {
        self.saved_at = self.edits;
    }

    /// Whether the song changed since it was last saved or opened.
    ///
    /// The window kept its own "changed" flag, set by some edits and not
    /// others: a new tempo, an added plug-in or an automation point left
    /// it clean, so closing the DAW did not ask to save and autosave never
    /// ran. Every edit that can be undone passes through here.
    pub fn has_unsaved_edits(&self) -> bool {
        self.edits != self.saved_at
    }

    /// Put `song` back as it was before the newest edit. Returns whether
    /// there was an edit to undo.
    pub fn undo(&mut self, song: &mut Project) -> bool {
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        self.note_edit();
        let grew = step.swap(song, true);
        self.bytes = self.bytes.saturating_add_signed(grew);
        self.redo.push(step);
        self.trim();
        true
    }

    /// Make the newest undone edit again. Returns whether there was one.
    pub fn redo(&mut self, song: &mut Project) -> bool {
        let Some(mut step) = self.redo.pop() else {
            return false;
        };
        self.note_edit();
        let grew = step.swap(song, false);
        self.bytes = self.bytes.saturating_add_signed(grew);
        self.undo.push(step);
        self.trim();
        true
    }

    pub fn sizes(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    /// Roughly how many bytes the steps hold together.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Name the newest edit, if it was made at or after `since_ms` (when
    /// the app started the command the name is for, in milliseconds since
    /// 1970) and has no name yet. A command that took no step finds an
    /// older one and names nothing. Returns whether it was named.
    pub fn label_newest(&mut self, label: String, since_ms: u64) -> bool {
        match self.undo.last_mut() {
            Some(step) if step.label.is_none() && step.at_ms >= since_ms => {
                step.label = Some(label);
                self.changed();
                true
            }
            _ => false,
        }
    }

    /// Whether the oldest step kept is the song as it was opened.
    pub fn starts_at_open(&self) -> bool {
        !self.trimmed
    }

    /// The history list as last worked out, if nothing changed since.
    pub fn listed(&self) -> Option<(Vec<HistoryEntry>, usize)> {
        self.listed
            .as_ref()
            .filter(|listed| listed.version == self.version)
            .map(|listed| (listed.rows.clone(), listed.applied))
    }

    /// Every edit, oldest first, and how many of them are applied: the
    /// undo steps, then the redo steps. `current` is the song as it is.
    ///
    /// Every step but the newest was described when the step after it was
    /// taken, or when it was undone or redone. The newest leads to the
    /// song as it is, which a snapshot sharing the newest step's parts
    /// describes. The list is kept until the history changes, so asking
    /// again costs nothing. `current` is only borrowed mutably to take
    /// that snapshot; it is left as it was.
    pub fn entries(&mut self, current: &mut Project) -> (Vec<HistoryEntry>, usize) {
        if let Some(listed) = self.listed() {
            return listed;
        }
        // Steps whose neighbour is a snapshot and that no step change
        // described yet. None are expected; this keeps the list whole if
        // a path missed one.
        for i in 0..self.undo.len().saturating_sub(1) {
            let (head, tail) = self.undo.split_at_mut(i + 1);
            let step = &mut head[i];
            describe_if_needed(&step.label, &mut step.described, &step.song, &tail[0].song);
        }
        for i in 0..self.redo.len().saturating_sub(1) {
            let (head, tail) = self.redo.split_at_mut(i + 1);
            let step = &mut head[i];
            describe_if_needed(&step.label, &mut step.described, &tail[0].song, &step.song);
        }
        // The two steps next to the song as it is: the newest edit, and
        // the next one to redo. Worked out for this version only, since
        // the song can still change under them.
        let undescribed = |step: &Step| step.label.is_none() && step.described.is_none();
        let newest = self.undo.last().filter(|s| undescribed(s)).map(|step| {
            let (now, _) = Snapshot::take(current, Some(&step.song));
            describe_snapshots(&step.song, &now)
        });
        let next_redo = self.redo.last().filter(|s| undescribed(s)).map(|step| {
            let (now, _) = Snapshot::take(current, Some(&step.song));
            describe_snapshots(&now, &step.song)
        });

        let applied = self.undo.len();
        let mut rows = Vec::with_capacity(applied + self.redo.len());
        for (i, step) in self.undo.iter().enumerate() {
            let change = (i + 1 == applied).then_some(newest.as_ref()).flatten();
            rows.push(history_row(step, change));
        }
        // The redo stack holds the next one to redo last.
        let undone = self.redo.len();
        for (k, step) in self.redo.iter().rev().enumerate() {
            let change = (k == 0).then_some(next_redo.as_ref()).flatten();
            rows.push(history_row(step, change));
        }
        debug_assert_eq!(rows.len(), applied + undone);
        self.listed = Some(Listed {
            version: self.version,
            rows: rows.clone(),
            applied,
        });
        (rows, applied)
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod history_list_tests {
    use super::*;

    fn song(bpm: f64) -> Project {
        let mut p = Project::default();
        p.tempo_map.entries[0].bpm = bpm;
        p
    }

    #[test]
    fn the_list_is_the_undo_steps_with_their_names_and_follows_undo() {
        let mut h = History::new();
        // Edit one, named by the app; edit two, not named.
        h.push(&mut song(120.0));
        assert!(h.label_newest("Set tempo".into(), 0));
        h.push(&mut song(130.0));
        let mut current = song(140.0);
        let (rows, applied) = h.entries(&mut current);
        let labels: Vec<&str> = rows.iter().map(|r| r.change.label.as_str()).collect();
        assert_eq!(labels, vec!["Set tempo", "Tempo"]);
        assert_eq!(applied, 2);

        // Undo moves the cursor, not the names.
        assert!(h.undo(&mut current));
        assert_eq!(current.tempo_map.entries[0].bpm, 130.0);
        let (rows, applied) = h.entries(&mut current);
        let labels: Vec<&str> = rows.iter().map(|r| r.change.label.as_str()).collect();
        assert_eq!(labels, vec!["Set tempo", "Tempo"]);
        assert_eq!(applied, 1);
        assert!(h.undo(&mut current));
        assert_eq!(current.tempo_map.entries[0].bpm, 120.0);
        assert_eq!(h.entries(&mut current).1, 0);
        assert!(!h.undo(&mut current), "nothing older");
        assert!(h.redo(&mut current));
        assert_eq!(current.tempo_map.entries[0].bpm, 130.0);
        let (rows, applied) = h.entries(&mut current);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].change.label, "Set tempo");
        assert_eq!(applied, 1);
    }

    /// Each step is described once, when the step after it is taken or
    /// when it is undone, from the two snapshots on either side; the list
    /// is worked out once per change and then handed back as it was.
    #[test]
    fn steps_are_described_as_they_are_taken_and_the_list_kept_until_it_changes() {
        let mut h = History::new();
        let mut song = busy_song();
        let mut songs = vec![song.clone()];
        for n in 0..6usize {
            h.push(&mut song);
            match n % 3 {
                0 => song.tracks[2].clips[n].position_ticks += 96,
                1 => song.tracks[3].volume_db = -(n as f64),
                _ => song.tempo_map.entries[0].bpm += 1.0,
            }
            songs.push(song.clone());
        }
        // Every step but the newest was described when the next was taken.
        assert!(h.undo[..5].iter().all(|step| step.described.is_some()));
        assert!(h.undo[5].described.is_none());
        let words = |rows: &[HistoryEntry]| -> Vec<String> {
            rows.iter().map(|r| r.change.text()).collect()
        };
        let expected: Vec<String> = songs
            .windows(2)
            .map(|w| hardwave_project::describe::describe_change(&w[0], &w[1]).text())
            .collect();
        assert!(expected[0].starts_with("Move clip"), "{}", expected[0]);
        let (rows, applied) = h.entries(&mut song);
        assert_eq!((words(&rows), applied), (expected.clone(), 6));
        assert!(h.listed().is_some(), "kept");
        let version = h.version();

        // A later move of the newest edit changes what it did.
        h.note_edit();
        song.tracks[4].muted = true;
        assert!(h.listed().is_none());
        assert!(h.version() != version);
        let (rows, _) = h.entries(&mut song);
        let now = hardwave_project::describe::describe_change(&songs[5], &song).text();
        assert_eq!(rows[5].change.text(), now);
        assert!(now.starts_with("Mute"), "{now}");
        h.note_edit();
        song.tracks[4].muted = false;

        // Undo and redo keep every row, the cursor moves.
        assert!(h.undo(&mut song));
        assert!(h.undo(&mut song));
        assert!(h.redo.iter().all(|step| step.described.is_some()));
        let (rows, applied) = h.entries(&mut song);
        assert_eq!((words(&rows), applied), (expected.clone(), 4));
        assert!(h.redo(&mut song));
        let (rows, applied) = h.entries(&mut song);
        assert_eq!((words(&rows), applied), (expected, 5));
        assert!(h.undo[..5].iter().all(|step| step.described.is_some()));
    }

    #[test]
    fn a_name_is_only_for_an_edit_made_since_its_command_started() {
        let mut h = History::new();
        assert!(!h.label_newest("Nothing to name".into(), 0), "no edit yet");
        h.push(&mut song(120.0));
        let made = h.undo.last().unwrap().at_ms;
        // A command started after the step and took none of its own: its
        // name must not land on the step before it.
        assert!(!h.label_newest("Set volume".into(), made + 1));
        assert!(h.label_newest("First".into(), made));
        assert!(!h.label_newest("Second".into(), made), "named once");
        let mut current = song(125.0);
        assert!(h.undo(&mut current));
        assert!(h.redo(&mut current));
        assert!(
            !h.label_newest("After redo".into(), made + 1),
            "a redo is not a new edit"
        );
    }

    #[test]
    fn an_edit_since_the_last_save_is_known_and_clearing_the_list_does_not_hide_it() {
        let mut h = History::new();
        assert!(!h.has_unsaved_edits(), "a song as opened");
        h.push(&mut song(150.0));
        assert!(h.has_unsaved_edits(), "a new tempo is an edit");
        h.mark_saved();
        assert!(!h.has_unsaved_edits());

        // A later move of the same gesture takes no step but still counts.
        h.note_edit();
        assert!(h.has_unsaved_edits());
        h.mark_saved();

        // Undo changes the song on screen from the one on disk.
        let mut current = song(155.0);
        assert!(h.undo(&mut current));
        assert!(h.has_unsaved_edits());
        h.mark_saved();
        assert!(h.redo(&mut current));
        assert!(h.has_unsaved_edits());

        // The history list's Clear forgets the steps, not the edit.
        h.clear();
        assert!(h.has_unsaved_edits());
    }

    #[test]
    fn the_oldest_step_says_whether_it_is_the_song_as_opened() {
        let mut h = History::new();
        h.push(&mut song(120.0));
        assert!(h.starts_at_open());
        for i in 0..HISTORY_CAP {
            h.push(&mut song(120.0 + i as f64));
        }
        assert!(!h.starts_at_open(), "the first step went past the cap");
    }

    /// A song with a few busy tracks among the inserts, like an import.
    fn busy_song() -> Project {
        let mut p = Project::default();
        for t in 1..=6 {
            let track_id = p.tracks[t].id.clone();
            for c in 0..100u64 {
                let mut clip =
                    hardwave_midi::MidiClip::new(format!("m{t}-{c}"), "Bass".into(), 3840);
                clip.notes = vec![hardwave_midi::MidiNote::default(); 16];
                p.tracks[t]
                    .clips
                    .push(hardwave_project::clip::ClipPlacement {
                        content: hardwave_project::clip::ClipContent::Midi(
                            hardwave_project::clip::MidiClipRef {
                                id: format!("c{t}-{c}"),
                                clip,
                            },
                        ),
                        track_id: track_id.clone(),
                        position_ticks: c * 3840,
                        length_ticks: 3840,
                        lane: 0,
                        color: None,
                        row: Some(t as u32),
                    });
            }
        }
        p
    }

    fn file(p: &Project) -> Vec<u8> {
        p.to_bytes().unwrap()
    }

    #[test]
    fn undo_and_redo_give_back_every_song_exactly() {
        let mut h = History::new();
        let mut song = busy_song();
        let mut songs = vec![file(&song)];
        for n in 0..40usize {
            h.push(&mut song);
            match n % 5 {
                0 => song.tracks[1 + n % 6].clips[n].position_ticks += 96,
                1 => song.tracks[n].volume_db = -3.0,
                2 => {
                    song.tracks[2].clips.drain(0..3);
                }
                3 => {
                    let t = song.tracks.remove(10);
                    song.tracks.insert(20, t);
                }
                _ => song.tempo_map.entries[0].bpm += 1.0,
            }
            songs.push(file(&song));
        }
        for n in (0..40).rev() {
            assert!(h.undo(&mut song));
            assert!(file(&song) == songs[n], "undo to song {n}");
        }
        for (n, after) in songs.iter().enumerate().skip(1) {
            assert!(h.redo(&mut song));
            assert!(file(&song) == *after, "redo to song {n}");
        }
        // Halfway back, a new edit drops the redo steps and still undoes.
        for _ in 0..20 {
            assert!(h.undo(&mut song));
        }
        h.push(&mut song);
        song.tracks[3].muted = true;
        assert_eq!(h.sizes(), (21, 0));
        assert!(h.undo(&mut song));
        assert!(file(&song) == songs[20]);
    }

    #[test]
    fn a_small_edit_on_a_busy_song_holds_a_small_step() {
        let mut h = History::new();
        let mut song = busy_song();
        h.push(&mut song);
        let whole = h.bytes();
        song.tracks[3].clips[50].position_ticks += 96;
        h.push(&mut song);
        song.tracks[4].volume_db = -6.0;
        h.push(&mut song);
        let two_edits = h.bytes() - whole;
        assert!(two_edits * 20 < whole, "{two_edits} bytes against {whole}");
        // Undo and redo cost about as little.
        assert!(h.undo(&mut song));
        assert!(h.redo(&mut song));
        assert!(h.bytes() - whole < 2 * two_edits);
    }

    #[test]
    fn the_history_stays_within_its_bytes_and_keeps_the_newest_steps() {
        let mut h = History::new();
        let mut song = busy_song();
        h.push(&mut song);
        let whole = h.bytes();
        h.clear();
        // Room for a few whole songs.
        h.max_bytes = whole * 5;
        let mut songs = Vec::new();
        for _ in 0..30 {
            songs.push(file(&song));
            h.push(&mut song);
            // Every clip moves: nothing to share with the step before.
            for t in song.tracks.iter_mut() {
                for c in t.clips.iter_mut() {
                    c.position_ticks += 96;
                }
            }
            assert!(
                h.bytes() <= h.max_bytes,
                "{} over {}",
                h.bytes(),
                h.max_bytes
            );
        }
        let (kept, _) = h.sizes();
        assert!((3..30).contains(&kept), "{kept} steps kept");
        assert!(!h.starts_at_open());
        for n in (30 - kept..30).rev() {
            assert!(h.undo(&mut song));
            assert!(file(&song) == songs[n], "undo to song {n}");
        }
        assert!(!h.undo(&mut song));
        while h.redo(&mut song) {
            assert!(h.bytes() <= h.max_bytes);
        }

        // One step bigger than the cap is still kept: the last edit can
        // always be undone.
        h.clear();
        h.max_bytes = 1;
        h.push(&mut song);
        h.push(&mut song);
        assert_eq!(h.sizes(), (1, 0));
    }
}

/// Channel sender the audio thread uses to ship its end-of-block
/// plug-in state snapshot map back to the UI thread. Keyed by
/// `(track_id, slot_id)` so the project save path can serialise each
/// slot's opaque state blob.
pub type PluginStateSnapshotSender =
    std::sync::mpsc::SyncSender<HashMap<(String, String), Vec<u8>>>;

/// UI-thread → audio-thread parking slot for `PluginStateSnapshotSender`.
/// The UI places a sender here before `save_project`, the audio thread
/// services exactly one snapshot request per audio block.
pub type PluginStateSnapshotSlot = Arc<Mutex<Option<PluginStateSnapshotSender>>>;

/// The passes of a recorded take: where each starts on the timeline, and the
/// range of the take's samples it is.
pub type TakePasses = Vec<(u64, std::ops::Range<usize>)>;

/// Why the audio input did not open, as something a person can act on.
/// Windows answers "Access is denied" when its privacy settings keep
/// desktop programs off the microphone, which covers every audio input.
fn input_open_error(e: &str) -> String {
    if e.contains("0x80070005") || e.contains("Access is denied") {
        "Windows does not let the DAW use the audio input. Turn on Settings > Privacy & \
         security > Microphone > Let desktop apps access your microphone, then arm the \
         track again."
            .to_string()
    } else {
        format!("The audio input could not be opened: {e}")
    }
}

#[cfg(test)]
mod input_open_error_tests {
    #[test]
    fn a_privacy_refusal_says_where_to_allow_it() {
        let m = super::input_open_error(
            "Stream error: A backend-specific error has occurred: Access is denied. (0x80070005)",
        );
        assert!(m.contains("Let desktop apps access your microphone"), "{m}");
        assert!(
            super::input_open_error("no device").starts_with("The audio input could not be opened")
        );
    }
}

/// One input's take when recording stops: what it recorded, its file, and
/// its loop passes.
pub struct SourceTake {
    pub source: RecordSource,
    pub file: crate::input_node::TakeFile,
    pub passes: TakePasses,
    /// Audio was lost: the ring filled, or the disk failed part-way.
    pub truncated: bool,
}

/// Seconds of audio the ring between the audio thread and the disk holds
/// while recording. The take itself goes to a file, so this only has to
/// ride out a slow disk; it used to be the whole take, reserved in memory for
/// twenty minutes (about 460 MB at 48 kHz) and cut off after that.
const CAPTURE_RING_SECS: u32 = 10;

/// The parts of the recording latency, for the settings read-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingLatency {
    pub sample_rate: u32,
    pub output_frames: u64,
    pub input_frames: u64,
    pub queue_frames: u64,
    pub offset_ms: i32,
    pub total_frames: u64,
}

pub struct DawEngine {
    /// When audio was last restarted after its stream failed: a stream that
    /// fails again soon after a restart on the same device moves to the
    /// system default (see `poll_audio_health`).
    last_audio_restart: Option<std::time::Instant>,
    pub transport: TransportState,
    pub project: Arc<Mutex<Project>>,
    pub plugin_scanner: Arc<Mutex<PluginScanner>>,
    pub midi_input: Arc<Mutex<MidiInputManager>>,
    pub audio_pool: AudioPool,
    /// The clip launcher: which slot each track is playing, and what
    /// has been asked for. Shared with the audio thread through each
    /// track's node.
    pub session: Arc<crate::session::SessionState>,
    /// Critical-path latency in samples, published by the audio thread after
    /// each graph rebuild. Coarse proxy for "total project latency" until
    /// full PDC lands.
    pub graph_latency_samples: Arc<std::sync::atomic::AtomicU32>,
    /// The threads that share the per-block work. Empty is the audio
    /// thread alone, which is what a machine with one core wants.
    worker_pool: Arc<Mutex<Option<Arc<crate::parallel::WorkerPool>>>>,
    /// The Ableton Link session, when the setting asks for one.
    link: Arc<Mutex<Option<crate::link::LinkSession>>>,
    /// A commercial track to compare the mix against, and how it is
    /// being played.
    #[allow(clippy::type_complexity)]
    reference: Arc<Mutex<Option<Arc<crate::audio_pool::AudioBuffer>>>>,
    reference_on: Arc<std::sync::atomic::AtomicBool>,
    reference_gain_centi_db: Arc<std::sync::atomic::AtomicI64>,

    /// Folder the current .hwp lives in, set on save and load.
    ///
    /// A clip's `source_file` may be relative to it, which is what lets a
    /// project folder be moved, copied to another drive or zipped and still
    /// open: an absolute path only describes where the audio was on the
    /// machine that imported it.
    project_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// Network servers the person has picked samples from on this
    /// machine. A project may only make the DAW open files on these;
    /// see `source_paths`.
    trusted_servers: Mutex<Vec<String>>,

    /// How much of each audio block's time budget the engine actually spends,
    /// in per mille, written by the audio thread after every block.
    ///
    /// This is the number a CPU meter should show. The toolbar meter used to
    /// estimate load from WebView frame jitter, which measures how busy the
    /// UI is: it moved when a window was dragged and sat still while the
    /// audio thread was close to dropping out.
    audio_load_permille: Arc<std::sync::atomic::AtomicU32>,
    /// Blocks that overran their budget since the session started. One xrun
    /// is an audible click, so the count matters even when the average looks
    /// comfortable.
    audio_xruns: Arc<std::sync::atomic::AtomicU32>,

    /// Click settings, shared with the audio thread.
    metronome: crate::metronome::MetronomeSettings,

    /// Browser audition requests, shared with the audio thread.
    preview: crate::preview_player::PreviewRequest,

    /// The test tone after Apply in Settings > Audio, shared with the audio
    /// thread.
    test_tone: crate::test_tone::TestToneRequest,

    /// The plain sine voice the first run's Controller step plays keys on.
    check_voice: crate::check_voice::CheckVoiceSwitch,

    /// Behaviour switches from the Audio settings panel, read by the audio
    /// thread every block.
    audio_prefs: crate::audio_prefs::AudioPrefs,

    audio_device: AudioDeviceManager,
    command_tx: Sender<EngineCommand>,
    command_rx: Receiver<EngineCommand>,

    /// Stretch-bake keys currently being rendered on a background thread.
    /// Dragging a stretch control fires a rebuild per frame; without this
    /// every one of them would spawn a duplicate bake of the same source.
    stretch_inflight: Arc<Mutex<std::collections::HashSet<String>>>,

    // Metering: lock-free ring buffer (audio thread → UI thread)
    meter_consumer: Option<rtrb::Consumer<MeterSnapshot>>,
    meter_cache: MeterSnapshot,

    /// Per-track post-fader meter state, keyed by track id.
    pub track_meters: TrackMeterMap,

    /// Shared consumer side of the input monitor ring buffer. The audio-io
    /// input callback pushes interleaved stereo samples into the matching
    /// producer; the graph's InputNode drains from this consumer. Held in a
    /// Mutex<Option<_>> so we can swap it when the input stream is stopped
    /// and restarted without rebuilding the whole engine.
    input_consumer: SharedInputConsumer,

    /// Recording capture target. The audio thread's InputNode appends
    /// interleaved L/R samples here when [`CaptureTap::recording`] is
    /// true. The UI thread can drain this on stop to write a WAV file
    /// and place a clip on the armed track. The first of `captures`.
    pub capture: Arc<CaptureTap>,
    /// One capture per input recording at once: armed tracks on different
    /// inputs (or recording different channels' output) each get a take of
    /// their own.
    captures: Arc<[Arc<CaptureTap>]>,
    /// What each capture records in the current or last take, in order.
    record_sources: Mutex<Vec<RecordSource>>,
    /// The interface's input was opened because a track that records it
    /// was armed, not by the monitor switch in Audio settings, so it is let
    /// go again when the last such track is disarmed.
    input_opened_for_arm: bool,

    /// Undo/redo history. Take a snapshot BEFORE mutating the project.
    pub history: Arc<Mutex<History>>,
    /// Playhead where the current take started, so its first pass can be
    /// placed where it was played.
    record_start: std::sync::atomic::AtomicU64,
    /// Which loop pass the recording is on, and the playhead at the previous
    /// block, so a jump back can be spotted. Counted here rather than in the
    /// capture tap because a MIDI-only recording has no input node at all.
    record_pass: Arc<std::sync::atomic::AtomicU32>,
    record_last_pos: Arc<std::sync::atomic::AtomicU64>,
    /// True while a gesture is being treated as one undo step.
    history_group_open: Arc<std::sync::atomic::AtomicBool>,
    /// True once the open group has taken its snapshot.
    history_group_taken: Arc<std::sync::atomic::AtomicBool>,
    /// A rebuild inside the open group wanted bakes, which wait for its end.
    bake_after_group: Arc<std::sync::atomic::AtomicBool>,

    /// Circular buffer of recent master-bus output samples. Used by the UI
    /// for oscilloscope / spectrum / correlation visualizations.
    pub master_tap: SharedMasterTap,

    /// UI-side handle for queueing per-track plug-in commands toward
    /// the audio thread. Populated when `start()` builds the audio
    /// callback; `None` before start or after stop. Wrapped in a Mutex
    /// because Tauri command handlers fire concurrently — the
    /// underlying rtrb Producer is single-producer.
    pub insert_command_sender: Arc<Mutex<Option<crate::insert_chain::InsertCommandSender>>>,

    /// Receiver side of the audio→drop graveyard. The UI thread (or
    /// engine shutdown) drains and drops vacated plug-in instances so
    /// the audio thread never frees memory directly.
    pub insert_graveyard: Arc<Mutex<Option<crate::insert_chain::PluginGraveyardReceiver>>>,

    /// One-shot plug-in state snapshot request channel. The UI thread
    /// places a SyncSender here just before `save_project`, then waits
    /// on the matching Receiver. The audio thread polls this slot on
    /// every block and, if it finds a Sender, walks all chains, calls
    /// `plugin.get_state()` per slot, and sends the resulting map back.
    /// Used to capture plug-in state into `Project.plugin_states` so
    /// save/load round-trips preserve the user's knob tweaks.
    pub pending_state_snapshot: PluginStateSnapshotSlot,
    /// Rolling 3-minute capture of every live MIDI event seen by the
    /// engine. Audio thread pushes per-block; UI thread can dump the
    /// last N seconds into a clip without arming a track. The ring is
    /// always recording — there is no start/stop, just "dump from now
    /// minus N seconds". Capacity sized for ~8k events which covers
    /// dense input over the 3-minute window.
    pub midi_capture_ring: Arc<Mutex<MidiCaptureRing>>,
    /// How many times `start` has succeeded; see `start_count`.
    starts: u64,
    /// The last `start` failed: audio is meant to be on and is not. A new
    /// device, rate, buffer or driver starts it, as it restarts a running
    /// one.
    start_failed: bool,
}

/// Commands sent from UI thread to audio thread.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    Transport(TransportCommand),
    /// Rebuild the audio graph from the current project state.
    RebuildGraph,
    /// Which MIDI tracks hear the keyboard changed (a track was picked):
    /// set that on the nodes, without rebuilding the graph.
    RefreshLiveMidi,
}

impl DawEngine {
    pub fn new() -> Self {
        let (tx, rx) = bounded(256);
        let captures = crate::input_node::capture_set();

        Self {
            last_audio_restart: None,
            transport: TransportState::default(),
            project: Arc::new(Mutex::new(Project::default())),
            plugin_scanner: Arc::new(Mutex::new(PluginScanner::new())),
            midi_input: Arc::new(Mutex::new(MidiInputManager::new())),
            audio_pool: AudioPool::new(),
            session: Arc::new(crate::session::SessionState::new()),
            audio_device: AudioDeviceManager::new(),
            command_tx: tx,
            command_rx: rx,
            stretch_inflight: Arc::new(Mutex::new(std::collections::HashSet::new())),
            meter_consumer: None,
            meter_cache: MeterSnapshot::default(),
            track_meters: Arc::new(Mutex::new(HashMap::new())),
            input_consumer: Arc::new(Mutex::new(None)),
            capture: Arc::clone(&captures[0]),
            captures,
            record_sources: Mutex::new(Vec::new()),
            input_opened_for_arm: false,
            history: Arc::new(Mutex::new(History::new())),
            record_start: std::sync::atomic::AtomicU64::new(0),
            record_pass: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            record_last_pos: Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            history_group_open: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            history_group_taken: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            bake_after_group: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            master_tap: master_tap::new_shared(),
            graph_latency_samples: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            worker_pool: Arc::new(Mutex::new(None)),
            link: Arc::new(Mutex::new(None)),
            reference: Arc::new(Mutex::new(None)),
            reference_on: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            reference_gain_centi_db: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            project_dir: Arc::new(Mutex::new(None)),
            trusted_servers: Mutex::new(Vec::new()),
            audio_load_permille: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            audio_xruns: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            metronome: crate::metronome::MetronomeSettings::new(),
            preview: crate::preview_player::PreviewRequest::new(),
            test_tone: crate::test_tone::TestToneRequest::new(),
            check_voice: crate::check_voice::CheckVoiceSwitch::new(),
            audio_prefs: crate::audio_prefs::AudioPrefs::new(),
            insert_command_sender: Arc::new(Mutex::new(None)),
            insert_graveyard: Arc::new(Mutex::new(None)),
            pending_state_snapshot: Arc::new(Mutex::new(None)),
            midi_capture_ring: Arc::new(Mutex::new(MidiCaptureRing::new(8192))),
            starts: 0,
            start_failed: false,
        }
    }

    /// Block the calling thread (UI) up to `timeout` while the audio
    /// thread snapshots the current state of every loaded plug-in
    /// instance into a `(track_id, slot_id) -> state_bytes` map. Returns
    /// `None` on timeout or if the engine hasn't been started yet.
    pub fn snapshot_plugin_states(
        &self,
        timeout: std::time::Duration,
    ) -> Option<std::collections::HashMap<(String, String), Vec<u8>>> {
        // With no audio thread nothing answers: say so at once instead of
        // waiting out the timeout, which cost every export and every step of
        // the performance test half a second.
        if !self.is_running() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        {
            let mut slot = self.pending_state_snapshot.lock();
            // If a previous request is still pending, just bail — caller
            // can retry. We don't queue requests because the audio thread
            // only services one per block anyway.
            if slot.is_some() {
                return None;
            }
            *slot = Some(tx);
        }
        rx.recv_timeout(timeout).ok()
    }

    /// Try to push a per-track plug-in command onto the audio-thread
    /// queue. Returns `Err(cmd)` if the engine is not started yet or
    /// the queue is full so the caller can retry on the next UI tick.
    pub fn try_send_insert_command(
        &self,
        cmd: crate::insert_chain::InsertCommand,
    ) -> Result<(), crate::insert_chain::InsertCommand> {
        let mut sender = self.insert_command_sender.lock();
        match sender.as_mut() {
            Some(s) => s.try_send(cmd),
            None => Err(cmd),
        }
    }

    /// A different song is coming in: have the audio thread drop every live
    /// plug-in (`InsertCommand::ClearAll`) before the new song's own arrive.
    /// Waits a moment for room in the queue rather than lose it, since the
    /// old song's plug-ins would otherwise go on playing in the new one.
    pub fn clear_live_plugins(&self) {
        let mut cmd = crate::insert_chain::InsertCommand::ClearAll;
        for _ in 0..50 {
            if self.insert_command_sender.lock().is_none() {
                // No audio thread yet, so no live plug-ins either.
                return;
            }
            match self.try_send_insert_command(cmd) {
                Ok(()) => return,
                Err(back) => {
                    cmd = back;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        log::warn!("clear_live_plugins: insert queue stayed full; old plug-ins may linger");
    }

    /// Drop everything pending in the plug-in graveyard. Call on a
    /// non-RT cadence (UI tick, on quit, etc) so vacated plug-in
    /// instances actually run their destructors. Returns the number
    /// dropped.
    pub fn drain_insert_graveyard(&self) -> usize {
        let mut graveyard = self.insert_graveyard.lock();
        match graveyard.as_mut() {
            Some(g) => g.drain_and_drop(),
            None => 0,
        }
    }

    /// Snapshot the project before a mutation. Call this as the first line of any
    /// command that changes `Project`. Redo stack is cleared on new edits.
    pub fn snapshot_before_mutation(&self) {
        // Inside an open group only the first mutation is snapshotted, so one
        // gesture is one undo. Painting twenty clips in a drag used to leave
        // twenty snapshots behind, and undoing the drag meant pressing undo
        // twenty times.
        use std::sync::atomic::Ordering::Relaxed;
        if self.history_group_open.load(Relaxed) && self.history_group_taken.swap(true, Relaxed) {
            self.history.lock().note_edit();
            return;
        }
        // The song, then the history: the order every caller takes them in.
        let mut song = self.project.lock();
        // What the last edit made without a row (a new, imported, recorded
        // or bounced clip) is written down on the row it is drawn on before
        // this edit and its undo step see the song, so a row edit, a row's
        // mute or solo and undo all treat it as it looks. Nothing moves.
        song.settle_rows();
        self.history.lock().push(&mut song);
    }

    /// Start treating the mutations that follow as one undo step.
    ///
    /// Balanced by `end_history_group`. Re-entrant calls are ignored rather
    /// than nested: a gesture is a gesture.
    pub fn begin_history_group(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        if self.history_group_open.swap(true, Relaxed) {
            return;
        }
        // Nothing snapshotted for this group yet.
        self.history_group_taken.store(false, Relaxed);
    }

    /// End the group, so the next mutation snapshots again, and start the
    /// bakes its rebuilds held back.
    pub fn end_history_group(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.history_group_open.store(false, Relaxed);
        self.history_group_taken.store(false, Relaxed);
        if self.bake_after_group.swap(false, Relaxed) {
            self.prebake_stretch_sources_async();
        }
    }

    pub fn history_group_open(&self) -> bool {
        self.history_group_open
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Roll the project back to the most recent snapshot. Returns true when a snapshot
    /// was consumed. The caller is responsible for calling `rebuild_graph()` afterwards.
    pub fn undo(&self) -> bool {
        let mut song = self.project.lock();
        self.history.lock().undo(&mut song)
    }

    /// Re-apply the most recently undone mutation. Returns true on success.
    pub fn redo(&self) -> bool {
        let mut song = self.project.lock();
        self.history.lock().redo(&mut song)
    }

    /// Name the edit just made, for the history list. See
    /// [`History::label_newest`].
    pub fn label_newest_edit(&self, label: String, since_ms: u64) -> bool {
        self.history.lock().label_newest(label, since_ms)
    }

    /// The history list: every edit, oldest first, and how many are
    /// applied.
    ///
    /// Takes the song and the history by their own locks, not the
    /// engine's. Unchanged since it was last asked for, the list is handed
    /// back as it was, without touching the song; otherwise the song is
    /// locked first, as undo locks them, and read in place.
    pub fn history_entries(
        project: &Mutex<Project>,
        history: &Mutex<History>,
    ) -> (Vec<HistoryEntry>, usize, bool) {
        {
            let history = history.lock();
            if let Some((entries, applied)) = history.listed() {
                return (entries, applied, history.starts_at_open());
            }
        }
        let mut song = project.lock();
        let mut history = history.lock();
        let (entries, applied) = history.entries(&mut song);
        (entries, applied, history.starts_at_open())
    }

    /// Counts every change to the undo steps (see [`History::version`]),
    /// so the window knows when to read the history list again.
    pub fn history_version(&self) -> u64 {
        self.history.lock().version()
    }

    /// Reset the history — e.g. after loading or creating a new project.
    pub fn reset_history(&self) {
        self.history.lock().clear();
    }

    /// The song is now what is on disk: just saved, opened or made new.
    pub fn mark_song_saved(&self) {
        self.history.lock().mark_saved();
    }

    /// Whether the song changed since it was last saved or opened.
    pub fn song_has_unsaved_edits(&self) -> bool {
        self.history.lock().has_unsaved_edits()
    }

    /// (undoDepth, redoDepth) — exposed so the UI can enable/disable menu entries.
    pub fn history_sizes(&self) -> (usize, usize) {
        self.history.lock().sizes()
    }

    /// How many inputs the audio interface offers: the width of the
    /// running input stream, or the one the next stream would open.
    pub fn input_channel_count(&self) -> u16 {
        match self.audio_device.input_active_channels() {
            0 => self.audio_device.planned_input_channels(),
            w => w,
        }
    }

    /// The input chosen in Audio settings, which a track records unless it
    /// has picked one of its own.
    pub fn default_input(&self) -> InputChannel {
        InputChannel {
            first: self.audio_device.input_channel_offset,
            mono: self.audio_device.input_channels == 1,
        }
    }

    /// Begin a recording capture. Clears any previously buffered samples
    /// and flips the `recording` flag so the audio thread starts pushing
    /// input blocks into the capture buffer immediately. Idempotent — a
    /// second call while already recording is a no-op.
    ///
    /// Every input the armed tracks record from gets a capture of its own,
    /// up to [`MAX_RECORD_SOURCES`]; with nothing armed the first capture
    /// records the input chosen in Audio settings, as it always did.
    pub fn start_capture(&self) {
        use std::sync::atomic::Ordering;
        if self.capture.recording.load(Ordering::Relaxed) {
            return;
        }
        // The rings, the takes' files and the threads that write them are
        // made here, on this thread, so the audio thread never allocates.
        let record_start = self.transport.position();
        let default = self.default_input();
        let mut sources = self.project.lock().record_sources(default);
        sources.truncate(MAX_RECORD_SOURCES);
        if sources.is_empty() {
            sources.push(RecordSource::Interface(default));
        }
        // The song's own output reaches the tap late only by what plug-in
        // delay compensation holds it back; the interface's input by the
        // whole round trip.
        let interface_latency = self.recording_latency().total_frames;
        let inside_latency = if self.transport.pdc_enabled.load(Ordering::Relaxed) {
            self.graph_latency_samples.load(Ordering::Relaxed) as u64
        } else {
            0
        };
        for (tap, source) in self.captures.iter().zip(&sources) {
            let latency = match source {
                RecordSource::Interface(_) => interface_latency,
                RecordSource::Track(_) => inside_latency,
            };
            tap.arm_compensated(
                self.current_sample_rate(),
                CAPTURE_RING_SECS,
                latency,
                record_start,
            );
        }
        let active = sources.len();
        *self.record_sources.lock() = sources;
        self.record_start.store(record_start, Ordering::Relaxed);
        self.record_pass.store(0, Ordering::Relaxed);
        self.record_last_pos.store(u64::MAX, Ordering::Relaxed);
        for tap in &self.captures[..active] {
            tap.recording.store(true, Ordering::Relaxed);
        }
    }

    /// The captures of the current or last take.
    fn active_captures(&self) -> &[Arc<CaptureTap>] {
        let n = self
            .record_sources
            .lock()
            .len()
            .clamp(1, MAX_RECORD_SOURCES);
        &self.captures[..n]
    }

    /// How late a played sample reaches the recording, and so how far each
    /// take is moved back: what the output takes to be heard, what the
    /// input takes to arrive, what waits in the queue between the two, and
    /// the user's own correction.
    pub fn recording_latency(&self) -> RecordingLatency {
        let sr = self.current_sample_rate() as u64;
        let stream = self.audio_device.latency();
        let to_frames = |nanos: u64| nanos.saturating_mul(sr) / 1_000_000_000;
        let output_frames = to_frames(stream.output_nanos());
        let input_frames = to_frames(stream.input_nanos());
        // Every input node reads the same ring, so they all see the same
        // queue; one that has not run yet reads zero.
        let queue_frames = self
            .captures
            .iter()
            .map(|c| c.queue_frames())
            .max()
            .unwrap_or(0) as u64;
        let offset_ms = self.audio_prefs.record_offset_ms();
        let measured = (output_frames + input_frames + queue_frames) as i64;
        let offset_frames = offset_ms as i64 * sr as i64 / 1000;
        // Capped at a second: a larger figure is a broken report, and moving
        // a take that far would be worse than not moving it.
        let total_frames = (measured + offset_frames).clamp(0, sr as i64) as u64;
        RecordingLatency {
            sample_rate: sr as u32,
            output_frames,
            input_frames,
            queue_frames,
            offset_ms,
            total_frames,
        }
    }

    /// Set the punch window from timeline ticks.
    ///
    /// Ticks rather than samples at the boundary, because the playlist thinks
    /// in ticks and the tempo map lives here: converting in the UI would drift
    /// from the engine as soon as a project had a tempo change.
    pub fn set_punch_ticks(&self, enabled: bool, in_ticks: u64, out_ticks: u64) {
        let sr = self.current_sample_rate() as f64;
        let (in_samples, out_samples) = {
            let project = self.project.lock();
            (
                project.tempo_map.tick_to_samples(in_ticks, sr),
                project.tempo_map.tick_to_samples(out_ticks, sr),
            )
        };
        for tap in self.captures.iter() {
            tap.set_punch(enabled, in_samples, out_samples);
        }
    }

    /// How many loop passes the current or last recording ran for.
    pub fn record_passes(&self) -> u32 {
        self.record_pass
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_add(1)
    }

    /// The punch window in samples, as the audio thread sees it.
    pub fn punch_samples(&self) -> (bool, u64, u64) {
        self.capture.punch()
    }

    /// Whether the last take ran out of reserved room, so the caller can say
    /// so instead of presenting a truncated file as complete.
    pub fn capture_overflowed(&self) -> bool {
        self.active_captures().iter().any(|c| c.overflowed())
    }

    /// Seconds captured so far in the active take.
    pub fn capture_seconds(&self) -> f64 {
        self.capture.seconds_captured(self.current_sample_rate())
    }

    /// Stop every capture writing. The audio thread stops at its next block.
    fn stop_captures(&self) {
        for tap in self.captures.iter() {
            tap.recording
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Finish the active capture and return the interleaved L/R samples
    /// captured since `start_capture()`. The buffer is left empty so a
    /// follow-up start_capture begins from scratch.
    pub fn stop_capture(&self) -> Vec<f32> {
        self.stop_captures();
        for tap in &self.captures[1..] {
            drop(tap.finish());
        }
        self.capture.take()
    }

    /// Throw the take away: every capture stops and its file is removed,
    /// without reading a sample of it back. Cancelling used to read the
    /// whole take into memory first, gigabytes for a long one, only to drop
    /// it.
    pub fn discard_capture(&self) {
        self.stop_captures();
        self.record_sources.lock().clear();
        for tap in self.captures.iter() {
            drop(tap.finish());
        }
    }

    /// Finish every capture of the take: for each input recorded, the
    /// take's file and each loop pass in it as a range of samples with the
    /// timeline position it starts at.
    pub fn stop_capture_sources(&self) -> Vec<SourceTake> {
        self.stop_captures();
        let sources = std::mem::take(&mut *self.record_sources.lock());
        let mut takes = Vec::with_capacity(sources.len());
        for (tap, source) in self.captures.iter().zip(sources) {
            if let Some((file, passes)) = self.finish_tap(tap) {
                takes.push(SourceTake {
                    source,
                    file,
                    passes,
                    truncated: tap.overflowed(),
                });
            }
        }
        // A capture past the sources holds nothing; finishing it removes
        // any file an earlier take left behind.
        for tap in self.captures.iter().skip(takes.len()) {
            drop(tap.finish());
        }
        takes
    }

    /// The first capture's take, for one input: see `stop_capture_sources`.
    pub fn stop_capture_takes(&self) -> Option<(crate::input_node::TakeFile, TakePasses)> {
        self.stop_capture_sources()
            .into_iter()
            .next()
            .map(|t| (t.file, t.passes))
    }

    /// One capture's file and its loop passes.
    ///
    /// Each pass except the last is trimmed to where the loop (or the punch
    /// window) ends: the transport wraps at block granularity, so without
    /// the trim every pass would carry a few milliseconds of audio from past
    /// the loop end. The last pass is left as played, since recording may
    /// have stopped part-way through it.
    fn finish_tap(&self, tap: &CaptureTap) -> Option<(crate::input_node::TakeFile, TakePasses)> {
        use std::sync::atomic::Ordering::Relaxed;
        let record_start = self.record_start.load(Relaxed);
        // Where the first kept sample was played. With latency compensation
        // that is not always the record start: a punch-in or a pass that
        // began late moves it.
        let first = tap.first_position().unwrap_or(record_start);
        let (file, ranges) = tap.take_pass_ranges()?;
        let looping = self.transport.looping.load(Relaxed);
        let loop_start = self.transport.loop_start.load(Relaxed);
        let loop_end = self.transport.loop_end.load(Relaxed);
        let (punched, punch_in, punch_out) = tap.punch();

        let last = ranges.len().saturating_sub(1);
        let passes = ranges
            .into_iter()
            .enumerate()
            .map(|(i, (mut range, played_at))| {
                // Each pass starts where it was played: the loop start for a
                // loop, but also wherever a jump took the playhead (a
                // timecode master, a Link session, a clock master's song
                // position). Every pass after the first used to be put at
                // the loop start, loop or not.
                let mut start = if i == 0 {
                    first
                } else {
                    played_at.unwrap_or(loop_start)
                };
                let mut end = if looping && loop_end > loop_start {
                    loop_end
                } else {
                    u64::MAX
                };
                if punched {
                    start = start.max(punch_in);
                    end = end.min(punch_out);
                }
                if i < last && end > start && end != u64::MAX {
                    let frames = (end - start) as usize;
                    range.end = range.end.min(range.start + frames * 2);
                }
                (start, range)
            })
            .collect();
        Some((file, passes))
    }

    /// `stop_capture_takes`, read back into memory: for short takes and
    /// tests.
    pub fn stop_capture_passes(&self) -> Vec<(u64, Vec<f32>)> {
        let Some((file, passes)) = self.stop_capture_takes() else {
            return Vec::new();
        };
        passes
            .into_iter()
            .map(|(start, range)| (start, file.read(range).unwrap_or_default()))
            .collect()
    }

    /// Where takes are written while recording.
    pub fn set_capture_dir(&self, dir: std::path::PathBuf) {
        for tap in self.captures.iter() {
            tap.set_dir(dir.clone());
        }
    }

    /// Sample rate of the running audio device, or the offline default
    /// when the engine isn't started yet. Needed by the recording
    /// pipeline to write the right WAV header.
    pub fn current_sample_rate(&self) -> u32 {
        let sr = self.audio_device.engine_rate();
        if sr > 0 {
            sr
        } else {
            48_000
        }
    }

    /// Start the audio engine.
    pub fn start(&mut self) -> Result<(), String> {
        let started = self.start_stream();
        self.start_failed = started.is_err();
        started
    }

    fn start_stream(&mut self) -> Result<(), String> {
        let transport = self.transport.clone();
        let project = Arc::clone(&self.project);
        let command_rx = self.command_rx.clone();
        let audio_pool = self.audio_pool.clone();

        // Negotiate the actual stream rate BEFORE building the callback.
        // cpal can silently fall back from the requested rate to the device's
        // default if the device doesn't accept the request — doing the
        // negotiation up front guarantees the callback, transport, and any
        // already-loaded audio buffers all agree on the same rate.
        self.audio_device
            .negotiate_output_rate()
            .map_err(|e| e.to_string())?;

        // The rate the mix runs at, which is a half or a quarter of the
        // device's when Windows has the output at 96 or 192 kHz (the
        // callback raises it; see hardwave_audio_io::upsample).
        let sample_rate = self.audio_device.engine_rate();
        let buffer_size = self.audio_device.buffer_size;

        // Any audio buffer previously loaded at a different rate would play
        // back at the wrong pitch after this restart — re-resample them to
        // the negotiated rate before the audio thread sees the new pool.
        let resampled = self.audio_pool.resample_all(sample_rate, |chs, src, dst| {
            hardwave_dsp::resample_channels(chs, src, dst).ok()
        });
        if resampled > 0 {
            log::info!(
                "Re-resampled {} cached audio buffer(s) to {} Hz",
                resampled,
                sample_rate
            );
        }

        transport
            .sample_rate
            .store(sample_rate as u64, std::sync::atomic::Ordering::Relaxed);

        let (meter_producer, meter_consumer) = RingBuffer::new(16);
        self.meter_consumer = Some(meter_consumer);

        // Fresh insert-chain channels per audio session. UI-side handles
        // are exposed through `insert_command_sender` and `insert_graveyard`
        // so Tauri command handlers can route plug-in mutations to the
        // audio thread, and drain freed instances on a non-RT cadence.
        // Each TrackNode owns its own chain; the engine routes commands
        // by track_id_to_node lookup, so we build the channels directly
        // and don't use the bundled InsertRouter helper.
        let (cmd_tx, cmd_rx) = crate::insert_chain::command_channel(256);
        let (grave_tx, grave_rx) = crate::insert_chain::graveyard_channel(64);
        *self.insert_command_sender.lock() = Some(cmd_tx);
        *self.insert_graveyard.lock() = Some(grave_rx);

        let mut callback = EngineCallback::new(
            transport,
            project,
            meter_producer,
            command_rx,
            audio_pool,
            Arc::clone(&self.track_meters),
            Arc::clone(&self.input_consumer),
            Arc::clone(&self.captures),
            Arc::clone(&self.master_tap),
            Arc::clone(&self.graph_latency_samples),
            cmd_rx,
            grave_tx,
            Arc::clone(&self.pending_state_snapshot),
            Arc::clone(&self.midi_input),
            Arc::clone(&self.midi_capture_ring),
            sample_rate,
            buffer_size,
            LoadCounters {
                load_permille: Arc::clone(&self.audio_load_permille),
                xruns: Arc::clone(&self.audio_xruns),
            },
            self.metronome.clone(),
            self.preview.clone(),
            self.audio_prefs.clone(),
            Arc::clone(&self.record_pass),
            Arc::clone(&self.record_last_pos),
            Arc::clone(&self.worker_pool),
            Arc::clone(&self.reference),
            Arc::clone(&self.reference_on),
            Arc::clone(&self.reference_gain_centi_db),
            Arc::clone(&self.session),
        );
        // Only the live callback hears the test tone and the check voice;
        // an export's callback keeps the ones nobody can start.
        callback.test_tone = crate::test_tone::TestTone::new(self.test_tone.clone());
        callback.check_voice = crate::check_voice::CheckVoice::new(self.check_voice.clone());
        // From here on graphs are built on a thread of their own and only
        // swapped in on the audio thread.
        callback.attach_builder_thread();

        self.audio_device
            .start(callback)
            .map_err(|e| e.to_string())?;
        self.starts += 1;
        Ok(())
    }

    /// Stop the audio engine.
    pub fn stop(&mut self) {
        self.audio_device.stop();
    }

    /// Stop on the way to starting again (another device, rate, buffer,
    /// driver or output). A start builds the engine's processing anew,
    /// without the plug-ins, which the app then loads again from the song;
    /// so their settings as they are now go into the song first, or every
    /// knob turned since the last save would be lost.
    pub fn stop_for_restart(&mut self) {
        self.stop_for_restart_reading(true);
    }

    /// The same, saying whether the audio callback can still be asked for
    /// the plug-ins' settings. After the stream failed it cannot: the ask
    /// waited half a second with the engine locked for an answer that
    /// never came. The settings as last saved are what is loaded then.
    fn stop_for_restart_reading(&mut self, stream_alive: bool) {
        if !stream_alive {
            log::warn!(
                "audio stream failed: plug-in settings changed since the last save could not be read"
            );
        } else if let Some(states) =
            self.snapshot_plugin_states(std::time::Duration::from_millis(500))
        {
            let mut project = self.project.lock();
            for ((_, slot_id), bytes) in states {
                project.set_plugin_state(slot_id, "unknown", bytes);
            }
        } else {
            log::warn!(
                "plug-in settings were not read before the audio restart (no answer in 500 ms)"
            );
        }
        self.audio_device.stop();
    }

    /// How many times the engine has started. A start drops the plug-ins
    /// it was hosting, so when this changes the app loads them again.
    pub fn start_count(&self) -> u64 {
        self.starts
    }

    /// Send a transport command from the UI thread.
    pub fn send_command(&self, cmd: TransportCommand) {
        let _ = self.command_tx.try_send(EngineCommand::Transport(cmd));
    }

    /// Which MIDI tracks hear the keyboard changed: set it on the audio
    /// thread without rebuilding the graph.
    pub fn refresh_live_midi(&self) {
        let _ = self.command_tx.try_send(EngineCommand::RefreshLiveMidi);
    }

    /// Ask for the audio graph to be built again from the current project
    /// state. It is built on the graph builder thread and swapped in by the
    /// audio thread when ready.
    pub fn rebuild_graph(&self) {
        // Kick the bake off in the background and signal the rebuild now.
        // Baking a long stem takes seconds (~16 s for 30 s of stereo), so
        // doing it inline would freeze the caller; until it lands the clip
        // simply plays varispeed, and the bake triggers its own rebuild.
        //
        // In the middle of a gesture (a tempo point being dragged) every step
        // asked for bakes of a stretch gone a moment later, each on a thread
        // of its own: they wait for the gesture to end.
        if self.history_group_open() {
            self.bake_after_group
                .store(true, std::sync::atomic::Ordering::Relaxed);
        } else {
            self.prebake_stretch_sources_async();
        }
        let _ = self.command_tx.try_send(EngineCommand::RebuildGraph);
    }

    /// The takes with notes tuned by hand this project's clips play, of
    /// the clips in `span` when there is one.
    fn tune_bake_plan(&self, span: Option<BakeSpan>) -> Vec<TuneBake> {
        use hardwave_project::clip::ClipContent;
        let project = self.project.lock();
        let mut acc: Vec<TuneBake> = Vec::new();
        for track in &project.tracks {
            for clip in &track.clips {
                let ClipContent::Audio(audio_clip) = &clip.content else {
                    continue;
                };
                if !BakeSpan::plays(span, clip, &project.tempo_map) {
                    continue;
                }
                if !audio_clip.has_tuned_notes() {
                    continue;
                }
                let key = tune_cache_key(&audio_clip.source_path, &audio_clip.tuned_notes);
                if !acc.iter().any(|b| b.key == key) {
                    acc.push(TuneBake {
                        key,
                        source_id: audio_clip.source_path.clone(),
                        notes: audio_clip.tuned_notes.clone(),
                    });
                }
            }
        }
        acc
    }

    /// Render one take with its notes tuned into the pool, outside the
    /// pool's lock like a stretch. The take's notes and periods are worked
    /// out once per file and kept, so moving a note again only renders.
    fn bake_tune_variant(pool: &AudioPool, bake: &TuneBake) {
        if pool.contains(&bake.key) {
            return;
        }
        let Some(src) = pool.get(&bake.source_id) else {
            return;
        };
        let Some(take) = pool.take_analysis(&bake.source_id, &mut |_| {}) else {
            return;
        };
        let rate = src.sample_rate as f64;
        let moves: Vec<hardwave_dsp::note_tune::NoteMove> = bake
            .notes
            .iter()
            .map(|n| hardwave_dsp::note_tune::NoteMove {
                start: n.start * rate,
                end: n.end * rate,
                sung: n.sung,
                pitch: n.pitch,
                drift: n.drift,
                new_start: n.new_start * rate,
                new_end: n.new_end * rate,
                formants: n.formants,
            })
            .collect();
        let channels = hardwave_dsp::note_tune::render(&src.channels, &take, &moves);
        let num_frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        if num_frames == 0 {
            return;
        }
        pool.insert(
            bake.key.clone(),
            AudioBuffer {
                channels,
                sample_rate: src.sample_rate,
                num_frames,
            },
        );
    }

    /// Take the tuned takes no clip plays any more out of the pool, with
    /// everything stretched or warped from them, and hand them back. Only
    /// for a file whose current tuning is already baked, so a clip never
    /// drops back to the untuned take while its new one is on the way.
    /// Handed back rather than dropped: the audio thread may hold one
    /// until its next rebuild, and the last reference should not be let
    /// go there.
    fn evict_stale_tunes(pool: &AudioPool, project: &Mutex<Project>) -> Vec<Arc<AudioBuffer>> {
        use hardwave_project::clip::ClipContent;
        let live: std::collections::HashSet<String> = {
            let project = project.lock();
            project
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .filter_map(|c| match &c.content {
                    ClipContent::Audio(a) if a.has_tuned_notes() => {
                        Some(tune_cache_key(&a.source_path, &a.tuned_notes))
                    }
                    _ => None,
                })
                .collect()
        };
        // Whether every tuning of a file the song plays is in the pool.
        let ready = |source: &str| {
            live.iter()
                .filter(|key| {
                    key.strip_prefix(source).is_some_and(|rest| {
                        rest.starts_with(TUNE_KEY_MARKER)
                            && rest.len() == TUNE_KEY_MARKER.len() + 16
                    })
                })
                .all(|key| pool.contains(key))
        };
        pool.remove_where(|id| match tuned_take_of(id) {
            Some(take) if !live.contains(take) => {
                let source = &take[..take.len() - TUNE_KEY_MARKER.len() - 16];
                ready(source)
            }
            _ => false,
        })
    }

    /// The (source, stretch, semitones) variants this project's clips need,
    /// the clips in `span` when there is one.
    fn stretch_bake_plan(&self, span: Option<BakeSpan>) -> Vec<(String, f64, f64)> {
        use hardwave_project::clip::ClipContent;
        let project = self.project.lock();
        let mut acc: Vec<(String, f64, f64)> = Vec::new();
        for track in &project.tracks {
            for clip in &track.clips {
                if !BakeSpan::plays(span, clip, &project.tempo_map) {
                    continue;
                }
                if let ClipContent::Audio(audio_clip) = &clip.content {
                    // A clip that follows the tempo, as it plays at this map.
                    let file_rate = self
                        .audio_pool
                        .get(&audio_clip.source_path)
                        .map(|b| b.sample_rate as f64);
                    let followed = crate::tempo_follow::tempo_followed(
                        clip,
                        audio_clip,
                        &project.tempo_map,
                        file_rate,
                    );
                    if let Some((stretch, semis)) = bake_params(&followed) {
                        let entry = (playing_source(audio_clip), stretch, semis);
                        if !acc.contains(&entry) {
                            acc.push(entry);
                        }
                    }
                }
            }
        }
        acc
    }

    /// Render one stretch variant into the pool. The stretch itself runs
    /// outside the pool lock, so the audio thread's reads are never blocked
    /// while a bake is in flight — only the final insert takes the write lock.
    fn bake_stretch_variant(pool: &AudioPool, source_id: &str, stretch: f64, semitones: f64) {
        let key = stretch_cache_key(source_id, stretch, semitones);
        if pool.contains(&key) {
            return;
        }
        let Some(src) = pool.get(source_id) else {
            return;
        };
        let req = hardwave_dsp::stretch_apply::StretchRequest {
            // Phase vocoder: the codebase's own "higher quality for tonal
            // sources" choice. The cost is paid once per
            // (source, ratio, pitch) and then cached.
            algorithm: hardwave_dsp::stretch_apply::StretchAlgorithm::PhaseVocoder,
            stretch_ratio: stretch as f32,
            pitch_semitones: semitones as f32,
            sample_rate: src.sample_rate as f32,
        };
        let channels: Vec<Vec<f32>> = src
            .channels
            .iter()
            .map(|ch| hardwave_dsp::stretch_apply::apply_stretch(ch, req))
            .collect();
        let num_frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        if num_frames == 0 {
            return;
        }
        pool.insert(
            key,
            AudioBuffer {
                channels,
                sample_rate: src.sample_rate,
                num_frames,
            },
        );
    }

    /// The warped clips' renders the project needs at `sample_rate`, of the
    /// clips in `span` when there is one. A clip set to resample plays its
    /// warp like a tape and is not baked.
    fn warp_bake_plan(&self, sample_rate: f64, span: Option<BakeSpan>) -> Vec<WarpBake> {
        use hardwave_project::clip::ClipContent;
        let project = self.project.lock();
        let mut acc: Vec<WarpBake> = Vec::new();
        for track in &project.tracks {
            for clip in &track.clips {
                let ClipContent::Audio(audio_clip) = &clip.content else {
                    continue;
                };
                if !BakeSpan::plays(span, clip, &project.tempo_map) {
                    continue;
                }
                let Some(buffer_rate) = self
                    .audio_pool
                    .get(&audio_clip.source_path)
                    .map(|b| b.sample_rate as f64)
                else {
                    continue;
                };
                // A clip that follows the tempo across a change is played as
                // warp markers, rendered here like any warped clip.
                let followed = crate::tempo_follow::tempo_followed(
                    clip,
                    audio_clip,
                    &project.tempo_map,
                    Some(buffer_rate),
                );
                let audio_clip: &hardwave_project::clip::AudioClip = &followed;
                if audio_clip.warp_markers.is_empty() || audio_clip.treatment.resample {
                    continue;
                }
                let unity = if buffer_rate > 0.0 {
                    buffer_rate / sample_rate
                } else {
                    1.0
                };
                let pieces = warp_pieces(clip, audio_clip, &project.tempo_map, sample_rate, unity);
                let source_id = playing_source(audio_clip);
                let key =
                    warp_cache_key(&source_id, &pieces, audio_clip.pitch_semitones, sample_rate);
                if !acc.iter().any(|b| b.key == key) {
                    acc.push(WarpBake {
                        key,
                        source_id,
                        pieces,
                        semitones: audio_clip.pitch_semitones,
                    });
                }
            }
        }
        acc
    }

    /// Render one warped clip with its pitch kept into the pool, outside
    /// the pool's lock like a stretch.
    fn bake_warp_variant(pool: &AudioPool, bake: &WarpBake, sample_rate: f64) {
        if pool.contains(&bake.key) {
            return;
        }
        let Some(src) = pool.get(&bake.source_id) else {
            return;
        };
        let channels: Vec<Vec<f32>> = src
            .channels
            .iter()
            .map(|ch| {
                hardwave_dsp::stretch_apply::warp_render(
                    ch,
                    &bake.pieces,
                    bake.semitones as f32,
                    sample_rate as f32,
                )
            })
            .collect();
        let num_frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        if num_frames == 0 {
            return;
        }
        pool.insert(
            bake.key.clone(),
            AudioBuffer {
                channels,
                sample_rate: sample_rate.round() as u32,
                num_frames,
            },
        );
    }

    /// Bake every warped clip's render the project needs at `sample_rate`,
    /// BLOCKING until done.
    fn prebake_warps(&self, sample_rate: f64) {
        for bake in self.warp_bake_plan(sample_rate, None) {
            Self::bake_warp_variant(&self.audio_pool, &bake, sample_rate);
        }
    }

    /// Bake every stretch variant the project needs, BLOCKING until done.
    ///
    /// A bounce must not race the bake, or it would render varispeed and
    /// not match playback; a render makes its own with
    /// [`Self::offline_bakes`]. Never call it from the audio thread — see
    /// [`Self::prebake_stretch_sources_async`] for the UI path.
    pub fn prebake_stretch_sources(&self) {
        // Tuned takes first: a clip's stretch is baked from its tuned take.
        for bake in self.tune_bake_plan(None) {
            Self::bake_tune_variant(&self.audio_pool, &bake);
        }
        for (source_id, stretch, semitones) in self.stretch_bake_plan(None) {
            Self::bake_stretch_variant(&self.audio_pool, &source_id, stretch, semitones);
        }
        self.prebake_warps(self.current_sample_rate() as f64);
    }

    /// Same, but hands the work to a background thread and returns at once.
    ///
    /// Playback stays responsive: an unbaked clip falls back to the varispeed
    /// resample path, and when the bake lands this triggers another rebuild so
    /// the audio thread picks it up. Keys already in flight are skipped, so
    /// dragging a stretch control doesn't spawn a bake per frame.
    pub fn prebake_stretch_sources_async(&self) {
        let sample_rate = self.current_sample_rate() as f64;
        type Plans = (Vec<TuneBake>, Vec<(String, f64, f64)>, Vec<WarpBake>);
        let (tunes, todo, warps): Plans = {
            let mut inflight = self.stretch_inflight.lock();
            let tunes = self
                .tune_bake_plan(None)
                .into_iter()
                .filter(|b| !self.audio_pool.contains(&b.key) && inflight.insert(b.key.clone()))
                .collect();
            let todo = self
                .stretch_bake_plan(None)
                .into_iter()
                .filter(|(s, st, se)| {
                    let key = stretch_cache_key(s, *st, *se);
                    !self.audio_pool.contains(&key) && inflight.insert(key)
                })
                .collect();
            let warps = self
                .warp_bake_plan(sample_rate, None)
                .into_iter()
                .filter(|b| !self.audio_pool.contains(&b.key) && inflight.insert(b.key.clone()))
                .collect();
            (tunes, todo, warps)
        };
        if tunes.is_empty() && todo.is_empty() && warps.is_empty() {
            return;
        }
        let pool = self.audio_pool.clone();
        let tx = self.command_tx.clone();
        let inflight = Arc::clone(&self.stretch_inflight);
        let project = Arc::clone(&self.project);
        std::thread::spawn(move || {
            let stale = {
                // Release the in-flight keys on the way out whatever happens. If a
                // bake panicked and left them claimed, that source could never be
                // retried and the clip would stay varispeed for the rest of the
                // session with nothing to indicate why.
                let _release = InflightRelease {
                    set: &inflight,
                    keys: tunes
                        .iter()
                        .map(|b| b.key.clone())
                        .chain(
                            todo.iter()
                                .map(|(s, st, se)| stretch_cache_key(s, *st, *se)),
                        )
                        .chain(warps.iter().map(|b| b.key.clone()))
                        .collect(),
                };
                // A tuned take plays as the file until its render lands.
                for bake in &tunes {
                    Self::bake_tune_variant(&pool, bake);
                }
                // A stretch or warp is made from the tuned take, which another
                // bake may still be making.
                let ours = |id: &str| tunes.iter().any(|b| b.key == id);
                for id in todo
                    .iter()
                    .map(|t| t.0.as_str())
                    .chain(warps.iter().map(|b| b.source_id.as_str()))
                {
                    if !ours(id) {
                        Self::wait_for_tuned_take(&pool, &inflight, id);
                    }
                }
                for (source_id, stretch, semitones) in &todo {
                    Self::bake_stretch_variant(&pool, source_id, *stretch, *semitones);
                }
                // A warped clip plays like a tape until its render lands.
                for bake in &warps {
                    Self::bake_warp_variant(&pool, bake, sample_rate);
                }
                if tunes.is_empty() {
                    Vec::new()
                } else {
                    Self::evict_stale_tunes(&pool, &project)
                }
            };
            // Ask the audio thread to rebuild so the fresh bakes are picked up.
            let _ = tx.try_send(EngineCommand::RebuildGraph);
            // The tunings nothing plays any more are let go here, once the
            // audio thread has moved on to the new ones, not there.
            if !stale.is_empty() {
                std::thread::sleep(std::time::Duration::from_secs(2));
                drop(stale);
            }
        });
    }

    /// Wait while another bake is still making a tuned take this one is
    /// made from. A long take's first tuning works out its notes first,
    /// which takes a few seconds; anything but a tuned take returns at once.
    fn wait_for_tuned_take(
        pool: &AudioPool,
        inflight: &Mutex<std::collections::HashSet<String>>,
        id: &str,
    ) {
        if tuned_take_of(id) != Some(id) {
            return;
        }
        for _ in 0..3000 {
            if pool.contains(id) || !inflight.lock().contains(id) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Audition a file on the engine's own output device.
    ///
    /// Decodes it only once: clicking back and forth through a folder would
    /// otherwise re-decode and re-resample the same file on every click,
    /// which on a long sample is a pause before the sound starts.
    pub fn preview_file(&self, path: &std::path::Path) -> Result<(), String> {
        self.preview_file_at_speed(path, 1.0)
    }

    /// Load a file for auditioning, once, and say its pool id and length in
    /// seconds.
    pub fn preview_load(&self, path: &std::path::Path) -> Result<(String, f64), String> {
        let source_id = source_id_for_path(&path.to_string_lossy());
        if !self.audio_pool.contains(&source_id) {
            self.load_audio_file_as(path, &source_id)?;
        }
        let seconds = self
            .audio_pool
            .get(&source_id)
            .map(|b| b.num_frames as f64 / b.sample_rate.max(1) as f64)
            .unwrap_or(0.0);
        Ok((source_id, seconds))
    }

    /// A pool source played `speed` times faster with its pitch kept, for
    /// auditioning, cached under its own id. Overlap-add, not the phase
    /// vocoder clips get: the vocoder took about five seconds for a
    /// ten-second loop before the first click sounded, and a preview is
    /// for judging a loop against the song, not for the mix. None when the
    /// stretch gave nothing.
    pub fn bake_preview_stretch(pool: &AudioPool, source_id: &str, speed: f64) -> Option<String> {
        let stretch = 1.0 / speed.clamp(0.25, 4.0);
        let key = format!("{}:preview", stretch_cache_key(source_id, stretch, 0.0));
        if pool.contains(&key) {
            return Some(key);
        }
        let src = pool.get(source_id)?;
        let req = hardwave_dsp::stretch_apply::StretchRequest {
            algorithm: hardwave_dsp::stretch_apply::StretchAlgorithm::OverlapAdd,
            stretch_ratio: stretch as f32,
            pitch_semitones: 0.0,
            sample_rate: src.sample_rate as f32,
        };
        let channels: Vec<Vec<f32>> = src
            .channels
            .iter()
            .map(|ch| hardwave_dsp::stretch_apply::apply_stretch(ch, req))
            .collect();
        let num_frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        if num_frames == 0 {
            return None;
        }
        pool.insert(
            key.clone(),
            AudioBuffer {
                channels,
                sample_rate: src.sample_rate,
                num_frames,
            },
        );
        Some(key)
    }

    /// Audition a file, optionally sped up or slowed to the song's tempo.
    pub fn preview_file_at_speed(&self, path: &std::path::Path, speed: f64) -> Result<(), String> {
        let source_id = source_id_for_path(&path.to_string_lossy());
        if !self.audio_pool.contains(&source_id) {
            self.load_audio_file_as(path, &source_id)?;
        }
        self.preview.play_at_speed(&source_id, speed);
        Ok(())
    }

    /// Load an audio file into the pool and return its source ID and info.
    pub fn load_audio_file(
        &self,
        path: &std::path::Path,
    ) -> Result<(String, hardwave_dsp::AudioFileInfo), String> {
        let source_id = source_id_for_path(&path.to_string_lossy());
        let info = self.load_audio_file_as(path, &source_id)?;
        Ok((source_id, info))
    }

    /// Load an audio file into the pool under an id the caller chooses.
    ///
    /// Reopening a project needs this: clips hold the pool id they were
    /// imported with, so the file has to come back under that same id even
    /// when it now lives somewhere else. Deriving the id from the path
    /// instead would give a relinked file a new id that no clip references,
    /// and the clip would stay silent.
    pub fn load_audio_file_as(
        &self,
        path: &std::path::Path,
        source_id: &str,
    ) -> Result<hardwave_dsp::AudioFileInfo, String> {
        let target_sr = self.audio_device.engine_rate();
        let (info, channels) = hardwave_dsp::AudioFileReader::read_resampled(path, Some(target_sr))
            .map_err(|e| e.to_string())?;

        let num_frames = channels.first().map(|c| c.len()).unwrap_or(0);

        let buffer = AudioBuffer {
            channels,
            sample_rate: info.sample_rate,
            num_frames,
        };

        self.audio_pool.insert(source_id.to_string(), buffer);

        Ok(info)
    }

    /// Click settings. The UI writes these; the audio thread reads them.
    pub fn metronome(&self) -> &crate::metronome::MetronomeSettings {
        &self.metronome
    }

    /// Browser auditions. Previewing through the engine means a sample comes
    /// out of the device the DAW is using, which the WebView's own audio
    /// could not do.
    pub fn preview(&self) -> &crate::preview_player::PreviewRequest {
        &self.preview
    }

    /// The test tone: a soft sine on the left output, then the right,
    /// straight to the sound card (see `test_tone`).
    pub fn test_tone(&self) -> &crate::test_tone::TestToneRequest {
        &self.test_tone
    }

    /// The first run's Controller step turns this on, so the keys sound
    /// in a song with no instrument (see `check_voice`).
    pub fn check_voice(&self) -> &crate::check_voice::CheckVoiceSwitch {
        &self.check_voice
    }

    /// The Audio settings panel's behaviour switches.
    pub fn audio_prefs(&self) -> &crate::audio_prefs::AudioPrefs {
        &self.audio_prefs
    }

    /// Audio-thread load: percentage of each block's time budget in use, and
    /// how many blocks have overrun it this session.
    ///
    /// Replaces a toolbar meter that estimated load from WebView frame
    /// timing, which measured UI business rather than audio work.
    pub fn audio_load(&self) -> (f32, u32) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.audio_load_permille.load(Relaxed) as f32 / 10.0,
            self.audio_xruns.load(Relaxed),
        )
    }

    /// Remember where the current project file lives, so sample paths stored
    /// relative to it can be resolved. Set on save and on load.
    pub fn set_project_dir(&self, dir: Option<std::path::PathBuf>) {
        *self.project_dir.lock() = dir;
    }

    pub fn project_dir(&self) -> Option<std::path::PathBuf> {
        self.project_dir.lock().clone()
    }

    /// Absolute location of a clip's audio, or why a project may not
    /// make the DAW open it.
    ///
    /// A collected project stores its samples relative to the .hwp, so the
    /// whole folder can be moved or zipped and still open. An imported sample
    /// that lives elsewhere on disk keeps its absolute path. What a project
    /// may name is decided in `source_paths`: audio files only, inside the
    /// project folder when relative, and no strangers' network servers.
    pub fn resolve_source_file(&self, file: &str) -> Result<std::path::PathBuf, String> {
        let dir = self.project_dir();
        let trusted = self.trusted_servers.lock().clone();
        crate::source_paths::check(file, dir.as_deref(), &trusted).map_err(|why| why.to_string())
    }

    /// Where the project's video really is, under the same rules.
    pub fn resolve_video_file(&self, file: &str) -> Result<std::path::PathBuf, String> {
        let dir = self.project_dir();
        let trusted = self.trusted_servers.lock().clone();
        crate::source_paths::check_video(file, dir.as_deref(), &trusted)
            .map_err(|why| why.to_string())
    }

    /// Play a track at this fader, pan and width from the next block on,
    /// without rebuilding the graph or touching the project. What the
    /// mixer sends while a fader is being dragged; the project is written
    /// once, when the drag ends. The master's fader is the master volume.
    pub fn set_live_mix(&self, track_id: &str, volume_db: f64, pan: f64, width: f64) {
        if !(volume_db.is_finite() && pan.is_finite() && width.is_finite()) {
            return;
        }
        let (is_master, vca_db) = {
            let project = self.project.lock();
            let is_master = project
                .tracks
                .iter()
                .any(|t| t.id == track_id && matches!(t.kind, hardwave_project::TrackKind::Master));
            (
                is_master,
                hardwave_project::vca::offset_for(&project.vcas, track_id).0,
            )
        };
        if is_master {
            self.transport.master_volume_db.store(
                volume_db.clamp(-200.0, 24.0),
                std::sync::atomic::Ordering::Relaxed,
            );
            return;
        }
        if let Some(meter) = self.track_meters.lock().get(track_id) {
            meter.set_vca_gain(db_to_linear_f32(vca_db));
            meter.set_mix(
                db_to_linear_f32(volume_db.clamp(-200.0, 24.0) + vca_db),
                pan.clamp(-1.0, 1.0) as f32,
                width.clamp(0.0, 2.0) as f32,
            );
        }
    }

    /// Play a track's fader, pan and width as the project now has them.
    pub fn apply_track_mix(&self, track_id: &str) {
        let values = {
            let project = self.project.lock();
            project
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .map(|t| (t.volume_db, t.pan, t.stereo_separation))
        };
        if let Some((volume_db, pan, width)) = values {
            self.set_live_mix(track_id, volume_db, pan, width);
        }
    }

    /// Play every member of a VCA group at its own fader plus the group's
    /// level as the project now has it, from the next block. Moving the
    /// group's fader rebuilt the whole graph at every step of the drag,
    /// which clicked and reset every delay line and filter, as a track
    /// fader used to.
    pub fn apply_vca_mix(&self, vca_id: &str) {
        let members: Vec<String> = {
            let project = self.project.lock();
            project
                .vcas
                .iter()
                .find(|v| v.id == vca_id)
                .map(|v| v.members.clone())
                .unwrap_or_default()
        };
        for member in members {
            self.apply_track_mix(&member);
        }
    }

    /// The person picked a file on a network server themselves: samples
    /// on that server may load from projects from now on.
    pub fn trust_network_location(&self, path: &str) {
        if let Some(server) = crate::source_paths::network_server(path) {
            let mut trusted = self.trusted_servers.lock();
            if !trusted.contains(&server) {
                trusted.push(server);
            }
        }
    }

    /// Servers trusted so far, to keep in the preferences.
    pub fn trusted_servers(&self) -> Vec<String> {
        self.trusted_servers.lock().clone()
    }

    pub fn set_trusted_servers(&self, servers: Vec<String>) {
        *self.trusted_servers.lock() = servers
            .into_iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
    }

    /// Re-load every audio source referenced by the current project into the
    /// audio pool. The pool only ever fills at import time (`load_audio_file`),
    /// so without this every project reopened after an app restart has SILENT
    /// audio clips — the clips reference pool ids that no longer exist.
    /// Covers the live tracks AND every stored arrangement timeline. Returns
    /// the source paths that failed to load (missing/unreadable files) so the
    /// UI can surface them like missing plugins; the project still opens.
    pub fn rehydrate_audio_pool(&self) -> Vec<String> {
        let project = self.project.lock().clone();
        self.audio_source_loader(self.project_dir())
            .load_sources_of(&project, |_, _| {})
    }

    /// What reading a project's audio needs from the engine, taken now, so
    /// the reading can run without the engine's lock. Opening a song decoded
    /// every sample under that lock on the window's own thread, and the
    /// window stopped responding until the last one was read.
    pub fn audio_source_loader(
        &self,
        project_dir: Option<std::path::PathBuf>,
    ) -> AudioSourceLoader {
        AudioSourceLoader {
            pool: self.audio_pool.clone(),
            target_sr: self.audio_device.engine_rate(),
            project_dir,
            trusted: self.trusted_servers.lock().clone(),
        }
    }

    /// Ensure every non-master track in the project has an entry in the
    /// track meter map. Called from the UI thread so that `dev_dump_state`
    /// can see newly added tracks before the audio thread rebuilds.
    pub fn sync_track_meters(&self) {
        let project = self.project.lock();
        let mut meters = self.track_meters.lock();
        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }
            meters
                .entry(track.id.clone())
                .or_insert_with(|| Arc::new(TrackMeterState::default()));
        }
        let live_ids: std::collections::HashSet<String> = project
            .tracks
            .iter()
            .filter(|t| t.kind.is_audio_bearing())
            .map(|t| t.id.clone())
            .collect();
        meters.retain(|id, _| live_ids.contains(id));
    }

    /// Snapshot per-track post-fader meters.
    /// Returns (id, peak_l, peak_r, rms, pre_fader_peak).
    pub fn track_meter_snapshots(&self) -> Vec<(String, f32, f32, f32, f32)> {
        use std::sync::atomic::Ordering;
        let meters = self.track_meters.lock();
        meters
            .iter()
            .map(|(id, m)| {
                (
                    id.clone(),
                    m.peak_db_l.load(Ordering::Relaxed),
                    m.peak_db_r.load(Ordering::Relaxed),
                    m.rms_db.load(Ordering::Relaxed),
                    m.pre_fader_peak_db.load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// What automation is doing to each track's fader and pan right now:
    /// (track id, volume in dB, pan -1..1), each `None` when not automated.
    /// The mixer follows these so a fader moves with its automation.
    pub fn track_automation(&self) -> Vec<(String, Option<f32>, Option<f32>)> {
        let meters = self.track_meters.lock();
        meters
            .iter()
            .filter_map(|(id, m)| {
                let (v, p) = m.automation();
                (v.is_some() || p.is_some()).then(|| {
                    let db = v.map(|lin| {
                        if lin > 1e-6 {
                            20.0 * lin.log10()
                        } else {
                            -120.0
                        }
                    });
                    (id.clone(), db, p)
                })
            })
            .collect()
    }

    /// How much of each block every track is taking, as a percentage of the
    /// time the block has. Returns (track id, percent).
    ///
    /// The whole-mix CPU meter can say the load is high and not which track
    /// is causing it, which is the question anyone actually has.
    pub fn track_cpu_percent(&self) -> Vec<(String, f32)> {
        use std::sync::atomic::Ordering;
        let block_ns = {
            let frames = self.audio_device.buffer_size.max(1) as f64;
            let rate = self.current_sample_rate().max(1) as f64;
            (frames / rate) * 1_000_000_000.0
        };
        let meters = self.track_meters.lock();
        meters
            .iter()
            .map(|(id, m)| {
                let ns = m.cpu_ns.load(Ordering::Relaxed) as f64;
                (id.clone(), ((ns / block_ns) * 100.0) as f32)
            })
            .collect()
    }

    /// Snapshot the most recent `n_frames` stereo frames from the master
    /// tap, interleaved. Returns fewer samples if the tap hasn't filled.
    pub fn master_tap_snapshot(&self, n_frames: usize) -> Vec<f32> {
        self.master_tap.lock().snapshot_interleaved(n_frames)
    }

    /// Clear the master-tap circular buffer (e.g. after transport reset).
    pub fn master_tap_reset(&self) {
        self.master_tap.lock().reset();
    }

    /// Get the latest master meter snapshot (drains the lock-free ring buffer).
    pub fn master_meter(&mut self) -> MeterSnapshot {
        if let Some(ref mut consumer) = self.meter_consumer {
            while let Ok(snapshot) = consumer.pop() {
                self.meter_cache = snapshot;
            }
        }
        self.meter_cache
    }

    /// Scan for plugins.
    pub fn scan_plugins(&self) -> Vec<hardwave_plugin_host::PluginDescriptor> {
        let mut scanner = self.plugin_scanner.lock();
        scanner.scan().to_vec()
    }

    pub fn is_running(&self) -> bool {
        self.audio_device.is_running()
    }

    /// Poll for runtime device failures (e.g. USB interface disconnected) and
    /// transparently fall back to the system default device.
    ///
    /// Returns `Ok(true)` if a recovery restart happened.
    /// Restart audio when its stream has failed. Returns the device it now
    /// plays on when that is a different one than before (the window says
    /// so), or `None` when nothing failed or it is the same device.
    ///
    /// A failed stream is not a lost device. A remote-desktop reconnect, a
    /// sample rate changed in Windows' sound settings, another program
    /// taking the device: each ends the stream while the device is still
    /// there. This used to drop the chosen device and move to the system
    /// default every time, so audio left the interface for the laptop's
    /// speakers. Now the chosen device is started again first (when it is
    /// gone, starting it falls back to the default by itself); the default
    /// is forced only when it will not start, or when it fails again within
    /// five seconds of the last restart.
    pub fn poll_audio_health(&mut self) -> Result<Option<String>, String> {
        if !self.audio_device.take_stream_error() {
            return Ok(None);
        }
        let before = self.audio_device.active_device_name().map(str::to_string);
        self.stop_for_restart_reading(false);
        let failed_again = self
            .last_audio_restart
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5));
        self.last_audio_restart = Some(std::time::Instant::now());
        if !failed_again && self.start().is_ok() {
            let now = self.audio_device.active_device_name().map(str::to_string);
            log::warn!(
                "Audio stream failed; started again on {}",
                now.as_deref().unwrap_or("the default output")
            );
            return Ok(moved_to(before, now));
        }
        log::warn!("Audio stream failed again; falling back to the default output device");
        self.stop_for_restart();
        // With no device chosen, resolve_output_device() opens the system
        // default on restart.
        self.audio_device.selected_device = None;
        self.start()?;
        let now = self.audio_device.active_device_name().map(str::to_string);
        Ok(moved_to(before, now).or_else(|| Some("system default output".into())))
    }

    /// Get a reference to the audio device manager for device listing.
    pub fn audio_device_manager(&self) -> &AudioDeviceManager {
        &self.audio_device
    }

    /// Fingerprint of the current output device set — used by the UI thread
    /// to detect hot-plug events.
    pub fn output_device_fingerprint(&self) -> u64 {
        self.audio_device.output_device_fingerprint()
    }

    /// List audio host backends available in this build.
    pub fn list_audio_hosts() -> Vec<String> {
        AudioDeviceManager::list_hosts()
    }

    /// Name of the currently active audio host backend.
    pub fn audio_host_name(&self) -> String {
        self.audio_device.host_name()
    }

    /// Switch audio host backend. Restarts the stream if it was running,
    /// or starts it if the last start failed.
    pub fn set_audio_host(&mut self, host_name: &str) -> Result<(), String> {
        let restart = self.audio_device.is_running() || self.start_failed;
        self.audio_device
            .set_host(host_name)
            .map_err(|e| e.to_string())?;
        if restart {
            self.start()?;
        }
        Ok(())
    }

    /// Whether WASAPI exclusive mode is currently requested.
    pub fn wasapi_exclusive(&self) -> bool {
        self.audio_device.wasapi_exclusive
    }

    /// Whether WASAPI exclusive mode is applicable on this host (Windows + WASAPI).
    pub fn wasapi_exclusive_available(&self) -> bool {
        self.audio_device.exclusive_available()
    }

    /// Enable/disable WASAPI exclusive mode. Restarts the stream if running
    /// so the change takes effect immediately. If the restart fails (e.g. the
    /// device does not support exclusive at the current sample rate/format),
    /// the flag is rolled back and the stream is restarted in the previous
    /// mode so the engine is never left in a stopped+committed state.
    pub fn set_wasapi_exclusive(&mut self, enabled: bool) -> Result<(), String> {
        if self.audio_device.wasapi_exclusive == enabled {
            return Ok(());
        }
        let previous = self.audio_device.wasapi_exclusive;
        let was_running = self.audio_device.is_running();
        if was_running {
            self.stop_for_restart();
        }
        self.audio_device.wasapi_exclusive = enabled;
        if was_running {
            if let Err(e) = self.start() {
                self.audio_device.wasapi_exclusive = previous;
                let _ = self.start();
                return Err(e);
            }
        }
        Ok(())
    }

    /// Get current audio config.
    pub fn audio_config(&self) -> (Option<String>, u32, u32) {
        (
            self.audio_device.selected_device.clone(),
            self.audio_device.sample_rate,
            self.audio_device.buffer_size,
        )
    }

    /// Apply new audio settings. Restarts the audio stream if running.
    /// Share the per-block work across this many threads beside the
    /// audio thread. Zero puts it back on the audio thread alone.
    ///
    /// The threads are started here, on the caller's thread, and the
    /// audio thread only picks up the handle at its next graph
    /// rebuild: starting a thread inside the callback is the kind of
    /// thing that makes a block late.
    pub fn set_worker_threads(&self, threads: usize) {
        let pool = if threads == 0 {
            None
        } else {
            Some(Arc::new(crate::parallel::WorkerPool::new(threads)))
        };
        *self.worker_pool.lock() = pool;
        self.rebuild_graph();
    }

    /// How many threads are set to help, zero when none are.
    pub fn worker_threads(&self) -> usize {
        self.worker_pool
            .lock()
            .as_ref()
            .map(|p| p.threads())
            .unwrap_or(0)
    }

    /// Join or leave an Ableton Link session.
    ///
    /// On, the tempo and the start and stop are shared with everything
    /// else on the network that speaks Link. Off, nothing is sent and
    /// nothing is listened to.
    /// Put a record beside the mix to compare against.
    ///
    /// The buffer is whatever the pool holds for `source_id`, so the
    /// file is loaded the same way a clip's audio is.
    pub fn set_reference_source(&self, source_id: Option<&str>) {
        let buffer = source_id.and_then(|id| self.audio_pool.get(id));
        *self.reference.lock() = buffer;
    }

    /// Hear the reference instead of the mix.
    pub fn set_reference_on(&self, on: bool) {
        self.reference_on
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// How much to lift or drop the reference so it sits at the mix's
    /// loudness. Comparing at different levels compares the levels.
    pub fn set_reference_gain_db(&self, db: f64) {
        self.reference_gain_centi_db.store(
            (db.clamp(-40.0, 40.0) * 100.0) as i64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Whether a reference is loaded, whether it is playing and at what
    /// gain.
    pub fn reference_status(&self) -> (bool, bool, f64) {
        (
            self.reference.lock().is_some(),
            self.reference_on.load(std::sync::atomic::Ordering::Relaxed),
            self.reference_gain_centi_db
                .load(std::sync::atomic::Ordering::Relaxed) as f64
                / 100.0,
        )
    }

    pub fn set_link_enabled(&self, enabled: bool) {
        let mut slot = self.link.lock();
        if enabled {
            if slot.is_none() {
                let bpm = self
                    .transport
                    .bpm
                    .load(std::sync::atomic::Ordering::Relaxed);
                *slot = Some(crate::link::LinkSession::start(
                    self.transport.clone(),
                    bpm,
                    self.audio_device.latency(),
                ));
            }
        } else {
            *slot = None;
        }
    }

    /// Whether a session is joined, and how many others are in it.
    pub fn link_status(&self) -> (bool, u64) {
        let slot = self.link.lock();
        match slot.as_ref() {
            Some(session) => (true, session.peers()),
            None => (false, 0),
        }
    }

    /// Starts audio as well when the last start failed: a sound card that
    /// would not open (another program had it) left audio off, and picking
    /// another one only stored the choice, so the song stayed silent until
    /// the app was started again.
    pub fn set_audio_config(
        &mut self,
        device: Option<String>,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Result<(), String> {
        let was_running = self.audio_device.is_running();
        let restart = was_running || self.start_failed;
        if was_running {
            self.stop_for_restart();
        }

        self.audio_device.selected_device = device;
        self.audio_device.sample_rate = sample_rate;
        self.audio_device.buffer_size = buffer_size;

        if restart {
            self.start()?;
        }

        Ok(())
    }

    /// Current input config: (selected device name, channels).
    pub fn input_config(&self) -> (Option<String>, u16) {
        (
            self.audio_device.selected_input_device.clone(),
            self.audio_device.input_channels,
        )
    }

    /// Which pair of the interface is recorded from, and which pair the
    /// mix goes out of. Both are the first of a pair, counting from zero.
    pub fn channel_offsets(&self) -> (u16, u16) {
        (
            self.audio_device.input_channel_offset,
            self.audio_device.output_channel_offset,
        )
    }

    /// Record from another pair of the interface, or send the mix out of
    /// one. Restarts whichever stream is running so the change is heard
    /// straight away.
    pub fn set_channel_offsets(&mut self, input: u16, output: u16) {
        let input_changed = self.audio_device.input_channel_offset != input;
        let output_changed = self.audio_device.output_channel_offset != output;
        self.audio_device.input_channel_offset = input;
        self.audio_device.output_channel_offset = output;
        if input_changed && self.audio_device.is_input_running() {
            let _ = self.start_input_monitoring();
        }
        if output_changed && self.audio_device.is_running() {
            // The output pair is fixed when the stream opens, so the
            // stream has to come back up on the new one.
            self.stop_for_restart();
            let _ = self.start();
        }
    }

    /// The output pair the metronome is asked to go out of (none is with
    /// the mix), and the pair it goes out of in the running stream.
    pub fn click_output(&self) -> (Option<u16>, Option<u16>) {
        (
            self.audio_device.click_channel_offset,
            self.audio_device.active_click_offset(),
        )
    }

    /// Send the metronome out of an output pair of its own, or back into
    /// the mix with none. The pairs are fixed when the stream opens, so a
    /// running stream comes back up on the new layout.
    pub fn set_click_output(&mut self, offset: Option<u16>) {
        if self.audio_device.click_channel_offset == offset {
            return;
        }
        self.audio_device.click_channel_offset = offset;
        if self.audio_device.is_running() {
            self.stop_for_restart();
            let _ = self.start();
        }
    }

    /// Update the input device preferences. The engine does not restart any
    /// input stream yet — recording isn't live — but the choice is stored so
    /// the recording pipeline picks it up when we ship it.
    pub fn set_input_config(&mut self, device: Option<String>, channels: u16) {
        self.audio_device.selected_input_device = device;
        self.audio_device.input_channels = channels.clamp(1, 2);
        // If monitoring is running, restart it so the new device/channel
        // count takes effect immediately without the caller re-toggling.
        if self.audio_device.is_input_running() {
            let _ = self.start_input_monitoring();
        }
    }

    /// Open a cpal input stream that feeds the pre-record peak meter AND
    /// streams live samples into a ring buffer the graph's InputNode drains.
    /// Armed tracks with `monitor_input` enabled then hear live input.
    pub fn start_input_monitoring(&mut self) -> Result<(), String> {
        // Create a fresh ring buffer pair every time we (re)start — this
        // guarantees no stale samples from a previous session survive into
        // the new stream, and that every frame in it has the new stream's
        // width. Capacity is a few output blocks of every channel, so small
        // drift between input and output buffer sizes absorbs without
        // audible glitching.
        let width = self.audio_device.planned_input_channels().max(1);
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(8192 * width as usize);
        *self.input_consumer.lock() = None;
        self.audio_device.set_input_monitor_producer(Some(producer));
        let started = self
            .audio_device
            .start_input_stream()
            .map_err(|e| e.to_string());
        let width = match self.audio_device.input_active_channels() {
            0 => width,
            w => w,
        };
        *self.input_consumer.lock() = Some(InputRing {
            consumer,
            width,
            default: self.default_input(),
        });
        // Which input each armed track reads from depends on the stream's
        // width and the input chosen in Audio settings.
        self.rebuild_graph();
        started
    }

    /// Open the interface's input while an armed track records it, and let
    /// it go when none does.
    ///
    /// The input only ran while the monitor switch in Audio settings was
    /// on, so arming a track and recording gave a silent take unless that
    /// switch had been found first. An input the person opened themselves
    /// is left open. An input that will not open says why, in words that
    /// say what to do: it was only logged, and the take came out silent.
    pub fn follow_armed_inputs(&mut self) -> Result<(), String> {
        let needed = self
            .project
            .lock()
            .record_sources(self.default_input())
            .iter()
            .any(|s| matches!(s, RecordSource::Interface(_)));
        let running = self.audio_device.is_input_running();
        if needed && !running {
            match self.start_input_monitoring() {
                Ok(()) => self.input_opened_for_arm = true,
                Err(e) => {
                    log::warn!("could not open the input for an armed track: {e}");
                    // A stream that failed to start leaves nothing to read.
                    self.stop_input_monitoring();
                    return Err(input_open_error(&e));
                }
            }
        } else if !needed && running && self.input_opened_for_arm {
            self.stop_input_monitoring();
        }
        Ok(())
    }

    /// The monitor switch in Audio settings: the input stays open until
    /// switched off, whatever is armed.
    pub fn start_input_monitoring_by_hand(&mut self) -> Result<(), String> {
        self.input_opened_for_arm = false;
        if self.audio_device.is_input_running() {
            return Ok(());
        }
        self.start_input_monitoring()
    }

    /// Switching the monitor off in Audio settings. An armed track that
    /// records the interface keeps it open, or its take would be silent.
    pub fn stop_input_monitoring_by_hand(&mut self) {
        self.stop_input_monitoring();
        let _ = self.follow_armed_inputs();
    }

    /// Stop the input monitor stream and detach the ring buffer.
    pub fn stop_input_monitoring(&mut self) {
        self.input_opened_for_arm = false;
        self.audio_device.stop_input_stream();
        self.audio_device.set_input_monitor_producer(None);
        *self.input_consumer.lock() = None;
    }

    pub fn is_input_monitoring(&self) -> bool {
        self.audio_device.is_input_running()
    }

    /// Read and reset the current input peak. Returns linear (0..1+) L/R
    /// values.
    pub fn input_peak_snapshot(&self) -> (f32, f32) {
        self.audio_device.take_input_peak()
    }

    /// Sample rate / buffer size the input stream is currently running at.
    /// Both are 0 when the monitor stream isn't active.
    pub fn input_active_config(&self) -> (u32, u32) {
        (
            self.audio_device.input_active_sample_rate(),
            self.audio_device.input_active_buffer_size(),
        )
    }

    /// End of the project timeline in samples — the latest clip end across all
    /// tracks. Returns 0 when the project has no clips.
    pub fn project_end_samples(&self, sample_rate: u32) -> u64 {
        let project = self.project.lock();
        let sr = sample_rate as f64;
        let mut max_tick: u64 = 0;
        for track in &project.tracks {
            for clip in &track.clips {
                let end = clip.position_ticks + clip.length_ticks;
                if end > max_tick {
                    max_tick = end;
                }
            }
        }
        if max_tick == 0 {
            return 0;
        }
        project.tempo_map.tick_to_samples(max_tick, sr)
    }

    /// Offline-render the current project to an interleaved-stereo output
    /// stream. `on_block` is invoked with each rendered block (length =
    /// frames * 2). Rendering stops after `total_samples` frames.
    ///
    /// The render uses a private project snapshot and a fresh transport so
    /// the live audio thread is unaffected. `on_block` is called on the UI
    /// thread — typically it streams samples to disk.
    pub fn render_offline(
        &self,
        sample_rate: u32,
        total_samples: u64,
        on_block: impl FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        self.render_offline_with(sample_rate, total_samples, 0, None, |_| {}, on_block)
    }

    /// Like [`Self::render_offline`] but allows the caller to mutate the
    /// project snapshot before rendering — used for stems (mute all tracks
    /// except one) and similar isolation renders. The `on_block` callback
    /// returns `false` to halt rendering early (used for user cancellation).
    pub fn render_offline_with(
        &self,
        sample_rate: u32,
        total_samples: u64,
        start_samples: u64,
        instantiate: Option<OfflineInsertFactory<'_>>,
        prepare: impl FnOnce(&mut Project),
        on_block: impl FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        if total_samples == 0 {
            return Ok(());
        }
        self.prepare_offline_render(sample_rate, start_samples, instantiate, prepare)
            .run(total_samples, on_block);
        Ok(())
    }

    /// Everything a render needs, taken from the engine now: a copy of the
    /// project, its plug-ins and its audio. The render itself
    /// ([`OfflineRender::run`]) needs nothing from the engine, so callers can
    /// let go of the engine's lock before running it. An export used to hold
    /// that lock for the whole render, and every window command that touched
    /// the engine (the load meters poll it several times a second) waited
    /// behind it: the app stopped responding until the export ended.
    ///
    /// Every clip's bakes are made first; see [`Self::prepare_offline_span`].
    pub fn prepare_offline_render(
        &self,
        sample_rate: u32,
        start_samples: u64,
        instantiate: Option<OfflineInsertFactory<'_>>,
        prepare: impl FnOnce(&mut Project),
    ) -> OfflineRender {
        self.prepare_offline_span(sample_rate, start_samples, None, instantiate, prepare)
    }

    /// As [`Self::prepare_offline_render`], making only the bakes of the
    /// clips that sound between `bake_span`'s two sample positions when
    /// there is one. Made here, the bakes hold the caller's lock: make them
    /// before with [`Self::offline_bakes`], without it, and this finds them
    /// done.
    pub fn prepare_offline_span(
        &self,
        sample_rate: u32,
        start_samples: u64,
        bake_span: Option<(u64, u64)>,
        instantiate: Option<OfflineInsertFactory<'_>>,
        prepare: impl FnOnce(&mut Project),
    ) -> OfflineRender {
        let buffer_size: usize = 1024;
        // A pool of its own with as many helpers as playback has. Not the
        // live pool: it serves one caller at a time, and an export running
        // beside playback wrote into the same batch.
        let threads = self.worker_threads();
        let pool = (threads > 0).then(|| Arc::new(crate::parallel::WorkerPool::new(threads)));
        OfflineRender {
            callback: self.offline_callback(
                sample_rate,
                buffer_size,
                start_samples,
                bake_span,
                instantiate,
                prepare,
                Arc::new(Mutex::new(pool)),
            ),
            buffer_size,
            output_gain: 1.0,
        }
    }

    /// Play the song from the top as a sound card does, without one: the graph is
    /// built for the engine's buffer size, as playback builds it, and each
    /// block asks for `device_frames`, which a driver can make fewer than
    /// that (Windows hands 480-frame blocks to a graph built for 512).
    /// Plug-ins on the project's inserts are made with `instantiate`, as an
    /// export makes them. Each interleaved stereo block goes to `on_block`
    /// (false stops early).
    ///
    /// For checking that playback and an export of the same song agree:
    /// an export always fills its blocks, so a part of playback that went
    /// wrong only on a short block was never heard in one. Like
    /// [`Self::play_live_without_device`] it reads the engine's own MIDI
    /// input, and refuses while the engine is running.
    pub fn play_live_in_device_blocks(
        &self,
        sample_rate: u32,
        total_samples: u64,
        device_frames: usize,
        instantiate: Option<OfflineInsertFactory<'_>>,
        mut on_block: impl FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        if self.is_running() {
            return Err("the engine is running: its audio thread reads the live input".into());
        }
        let buffer_size = (self.audio_device.buffer_size as usize).max(1);
        let device_frames = device_frames.max(1);
        let mut callback = self.offline_callback(
            sample_rate,
            buffer_size,
            0,
            None,
            instantiate,
            |_| {},
            Arc::new(Mutex::new(None)),
        );
        callback.midi_input = Arc::clone(&self.midi_input);
        callback.midi_capture_ring = Arc::clone(&self.midi_capture_ring);
        let mut buf = vec![0.0_f32; device_frames * 2];
        let mut left = total_samples;
        while left > 0 {
            let frames = left.min(device_frames as u64) as usize;
            buf.fill(0.0);
            callback.process(&mut buf, frames, 2);
            if !on_block(&buf[..frames * 2]) {
                break;
            }
            left -= frames as u64;
        }
        Ok(())
    }

    /// [`Self::play_live_in_device_blocks`] with a stopwatch on it: how long
    /// the callback took for each device block, and how long each node of
    /// the graph took over the whole run. For finding what playback spends
    /// its time on. Plug-ins are made with `instantiate`, as an export
    /// makes them. `on_block` gets each block, outside the timing.
    #[doc(hidden)]
    pub fn profile_live_blocks(
        &self,
        sample_rate: u32,
        total_samples: u64,
        device_frames: usize,
        instantiate: Option<OfflineInsertFactory<'_>>,
        mut on_block: impl FnMut(&[f32]),
    ) -> LiveProfile {
        let buffer_size = (self.audio_device.buffer_size as usize).max(1);
        let device_frames = device_frames.max(1);
        let mut callback = self.offline_callback(
            sample_rate,
            buffer_size,
            0,
            None,
            instantiate,
            |_| {},
            Arc::new(Mutex::new(None)),
        );
        // As on the audio thread, which the sound card's callback sets up
        // this way.
        let _flushed = hardwave_audio_io::denormals::flush_denormals_while();
        let mut profile = LiveProfile::default();
        let mut buf = vec![0.0_f32; device_frames * 2];
        let mut left = total_samples;
        let mut timed_graph = None;
        while left > 0 {
            let frames = left.min(device_frames as u64) as usize;
            buf.fill(0.0);
            // A graph swapped in starts its own count.
            let graph_id = callback.graph_seq;
            if timed_graph != Some(graph_id) {
                if timed_graph.is_some() {
                    profile.swaps += 1;
                }
                callback.graph.time_nodes(true);
                timed_graph = Some(graph_id);
            }
            let before: Vec<[u64; 2]> = callback
                .graph
                .node_nanos()
                .map(|n| n.to_vec())
                .unwrap_or_default();
            let started = std::time::Instant::now();
            callback.process(&mut buf, frames, 2);
            let took = started.elapsed();
            // What each kind of node took in this block.
            let mut kinds = [0u64; LIVE_PROFILE_KINDS.len()];
            let after: Vec<[u64; 2]> = callback
                .graph
                .node_nanos()
                .map(|n| n.to_vec())
                .unwrap_or_default();
            for (id, (a, b)) in after.iter().zip(&before).enumerate() {
                let gather = a[0] - b[0];
                let ran = a[1] - b[1];
                if gather + ran == 0 {
                    continue;
                }
                let (name, kind) = callback
                    .graph
                    .node_label(id)
                    .unwrap_or_else(|| (String::new(), "other"));
                let k = LIVE_PROFILE_KINDS
                    .iter()
                    .position(|&n| n == kind)
                    .unwrap_or(LIVE_PROFILE_KINDS.len() - 1);
                kinds[k] += ran;
                kinds[LIVE_PROFILE_KINDS.len() - 2] += gather;
                let entry = match profile
                    .nodes
                    .iter_mut()
                    .position(|n| n.name == name && n.kind == kind)
                {
                    Some(i) => &mut profile.nodes[i],
                    None => {
                        profile.nodes.push(NodeCost {
                            name,
                            kind,
                            ..NodeCost::default()
                        });
                        profile.nodes.last_mut().unwrap()
                    }
                };
                entry.gather_ns += gather;
                entry.run_ns += ran;
                entry.blocks += 1;
                entry.max_ns = entry.max_ns.max(gather + ran);
            }
            profile.blocks.push(took);
            profile.kinds.push(kinds);
            on_block(&buf[..frames * 2]);
            left -= frames as u64;
        }
        profile
    }

    /// What one graph costs, `runs` times over, with the song playing from
    /// half a minute in: `build` is the work done off the audio thread,
    /// `swap` what the audio thread does to put the graph in. The plug-ins
    /// are left out; clips, notes, automation and routing are all there.
    #[doc(hidden)]
    pub fn graph_build_times(&self, runs: usize) -> Vec<GraphBuildTime> {
        let mut callback = self.offline_callback(
            48_000,
            512,
            0,
            None,
            None,
            |_| {},
            Arc::new(Mutex::new(None)),
        );
        callback.transport.set_position(48_000 * 30);
        let mut out = vec![0.0f32; 512 * 2];
        for _ in 0..4 {
            callback.process(&mut out, 512, 2);
        }
        (0..runs)
            .map(|_| {
                let started = std::time::Instant::now();
                let seq = callback.graph_seq + 1;
                let prepared = {
                    let BuildMode::Inline(builder) = &mut callback.builder else {
                        unreachable!("an offline callback builds inline");
                    };
                    let project = callback.project.lock();
                    let mut meters = callback.track_meters.lock();
                    builder.build(&project, &mut meters, seq)
                };
                let build = started.elapsed();
                let started = std::time::Instant::now();
                let old = callback.install_graph(prepared);
                let swap = started.elapsed();
                drop(old);
                callback.process(&mut out, 512, 2);
                GraphBuildTime { build, swap }
            })
            .collect()
    }

    /// How long each block of the project takes to compute, block by
    /// block, at a given buffer size.
    ///
    /// It runs the same callback playback runs, on the calling thread
    /// and the engine's worker pool, as fast as it can rather than in
    /// time with a sound card. A block that takes longer than its own
    /// length in time is one a sound card would have played late, which
    /// is what a crackle is. The performance test reads these to find
    /// how much a machine can play.
    pub fn measure_block_times(
        &self,
        sample_rate: u32,
        buffer_size: usize,
        total_samples: u64,
        instantiate: Option<OfflineInsertFactory<'_>>,
    ) -> Vec<std::time::Duration> {
        let buffer_size = buffer_size.max(16);
        // This engine's own pool: the performance test builds an engine of
        // its own per measurement, with the helper threads it is testing.
        let mut callback = self.offline_callback(
            sample_rate,
            buffer_size,
            0,
            None,
            instantiate,
            |_| {},
            Arc::clone(&self.worker_pool),
        );
        let mut buf = vec![0.0_f32; buffer_size * 2];
        let blocks = total_samples.div_ceil(buffer_size as u64) as usize;
        let mut times = Vec::with_capacity(blocks);
        for _ in 0..blocks {
            buf.fill(0.0);
            let started = std::time::Instant::now();
            callback.process(&mut buf, buffer_size, 2);
            times.push(started.elapsed());
        }
        times
    }

    /// [`Self::measure_block_times`], but in time: each block starts when
    /// a sound card would ask for it, not as soon as the last one ends.
    /// Back to back, caches stay warm and the CPU stays at full clock,
    /// which playback never gets; the numbers came out better than real.
    /// Stops early, with what it has, when `cancel` is set.
    pub fn measure_block_times_paced(
        &self,
        sample_rate: u32,
        buffer_size: usize,
        total_samples: u64,
        instantiate: Option<OfflineInsertFactory<'_>>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Vec<std::time::Duration> {
        let buffer_size = buffer_size.max(16);
        let mut callback = self.offline_callback(
            sample_rate,
            buffer_size,
            0,
            None,
            instantiate,
            |_| {},
            Arc::clone(&self.worker_pool),
        );
        let mut buf = vec![0.0_f32; buffer_size * 2];
        let blocks = total_samples.div_ceil(buffer_size as u64) as usize;
        let block_len =
            std::time::Duration::from_secs_f64(buffer_size as f64 / sample_rate.max(1) as f64);
        let mut times = Vec::with_capacity(blocks);
        let begun = std::time::Instant::now();
        for i in 0..blocks {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let due = begun + block_len * i as u32;
            let now = std::time::Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
            buf.fill(0.0);
            let started = std::time::Instant::now();
            callback.process(&mut buf, buffer_size, 2);
            times.push(started.elapsed());
        }
        times
    }

    /// Play `total_samples` frames of the project through the callback
    /// playback runs, block by block on the calling thread, without a sound
    /// card, handing each interleaved stereo block to `on_block` (false
    /// stops early).
    ///
    /// Unlike a render, it reads the engine's own MIDI input
    /// ([`Self::midi_input`]), fills its capture ring
    /// ([`Self::midi_capture_ring`]), and plays or stands still as the
    /// engine's transport does. It is how a key played into the engine is
    /// checked end to end without a device: the live MIDI smoke tests use
    /// it.
    ///
    /// Never for an export, a freeze or a bounce: those must not hear the
    /// keyboard (see [`Self::prepare_offline_render`]). It refuses while the
    /// engine is running, as the audio thread reads the same input and the
    /// two would split the keys between them.
    pub fn play_live_without_device(
        &self,
        sample_rate: u32,
        total_samples: u64,
        mut on_block: impl FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        if self.is_running() {
            return Err("the engine is running: its audio thread reads the live input".into());
        }
        let buffer_size: usize = 1024;
        let mut callback = self.offline_callback(
            sample_rate,
            buffer_size,
            0,
            None,
            None,
            |_| {},
            Arc::new(Mutex::new(None)),
        );
        // What makes it the live path: the engine's own MIDI input and
        // capture ring in place of the render's, and the engine's transport
        // deciding whether the song plays.
        callback.midi_input = Arc::clone(&self.midi_input);
        callback.midi_capture_ring = Arc::clone(&self.midi_capture_ring);
        callback.transport.playing.store(
            self.transport
                .playing
                .load(std::sync::atomic::Ordering::Relaxed),
            std::sync::atomic::Ordering::Relaxed,
        );
        let mut buf = vec![0.0_f32; buffer_size * 2];
        let mut left = total_samples;
        while left > 0 {
            let frames = left.min(buffer_size as u64) as usize;
            buf.fill(0.0);
            callback.process(&mut buf, frames, 2);
            if !on_block(&buf[..frames * 2]) {
                break;
            }
            left -= frames as u64;
        }
        Ok(())
    }

    /// A private callback over a snapshot of the project, for renders
    /// that happen beside playback rather than through the sound card.
    #[allow(clippy::too_many_arguments)]
    fn offline_callback(
        &self,
        sample_rate: u32,
        buffer_size: usize,
        start_samples: u64,
        bake_span: Option<(u64, u64)>,
        instantiate: Option<OfflineInsertFactory<'_>>,
        prepare: impl FnOnce(&mut Project),
        worker_pool: Arc<Mutex<Option<Arc<crate::parallel::WorkerPool>>>>,
    ) -> EngineCallback {
        // Bake stretch variants before rendering. The offline callback's
        // rebuild only looks them up, so without this an export would fall
        // back to varispeed and not match playback. Safe here — an export
        // runs on the caller's thread, not the audio callback. Warped clips
        // are baked at the render's own rate, which may not be the
        // device's. Only the clips the render plays need theirs.
        self.offline_bakes(sample_rate, bake_span)
            .run(&std::sync::atomic::AtomicBool::new(false), |_, _| {});

        // Harvest LIVE plug-in state so a bounce reflects the knobs the user
        // is actually hearing. `hydrate_offline_inserts` replays whatever is
        // stored on the project, and that is only ever written by
        // `save_project` — so without this an unsaved tweak renders stale, and
        // a slot added since the last save renders at plug-in DEFAULTS.
        //
        // Harvested before the project lock is taken: the audio thread has to
        // service the request, and it can't while we hold that lock. Returns
        // None when the engine isn't started (offline tests, headless renders),
        // in which case the stored state is still the best available.
        let live_plugin_states = self.snapshot_plugin_states(std::time::Duration::from_millis(500));

        let mut project_snapshot = self.project.lock().clone();
        if let Some(states) = live_plugin_states {
            for ((_track_id, slot_id), bytes) in states {
                project_snapshot.set_plugin_state(slot_id, "unknown", bytes);
            }
        }
        prepare(&mut project_snapshot);
        let initial_bpm = project_snapshot
            .tempo_map
            .entries
            .first()
            .map(|e| e.bpm)
            .unwrap_or(140.0);
        let project_arc = Arc::new(Mutex::new(project_snapshot));

        let transport = TransportState::default();
        use std::sync::atomic::Ordering;
        transport
            .sample_rate
            .store(sample_rate as u64, Ordering::Relaxed);
        transport.bpm.store(initial_bpm, Ordering::Relaxed);
        transport.master_volume_db.store(
            self.transport.master_volume_db.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        transport.set_position(start_samples);
        transport.playing.store(true, Ordering::Relaxed);
        // A render in pattern mode is the pattern, as in FL: the step
        // sequencer's notes, looped at the pattern's length.
        transport.pattern_mode.store(
            self.transport.pattern_mode.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        transport.pattern_len_ticks.store(
            self.transport.pattern_len_ticks.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (_command_tx, command_rx) = bounded::<EngineCommand>(4);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(None));

        // Offline render — use a private, discardable tap so we don't mix
        // offline samples into the live UI visualization stream.
        let offline_tap = master_tap::new_shared();
        let offline_latency = Arc::new(std::sync::atomic::AtomicU32::new(0));
        // Offline render gets its own throwaway insert-chain channels.
        // Plug-in commands aren't fired during offline render anyway —
        // the snapshot project is frozen — but EngineCallback's
        // signature requires them. Capacities are tiny because nothing
        // ever pushes here during offline.
        let (_offline_cmd_tx, offline_cmd_rx) = crate::insert_chain::command_channel(8);
        let (offline_grave_tx, _offline_grave_rx) = crate::insert_chain::graveyard_channel(8);

        let mut callback = EngineCallback::new(
            transport,
            project_arc,
            meter_producer,
            command_rx,
            self.audio_pool.clone(),
            track_meters,
            input_consumer,
            // Offline render doesn't capture anything — the recording tap
            // only matters for live audio. Pass a fresh detached tap so the
            // signature stays uniform without polluting the live capture.
            crate::input_node::capture_set(),
            offline_tap,
            offline_latency,
            offline_cmd_rx,
            offline_grave_tx,
            // Offline render doesn't need snapshot capability — the
            // export pipeline never asks for it. Pass a parked Arc that
            // can never be filled.
            Arc::new(Mutex::new(None)),
            // A MIDI input and capture ring of its own, never wired to a
            // device. It shared the live ones, and a render runs many times
            // faster than playback, so a key or controller touched while an
            // export ran landed in the file at whatever point the render had
            // reached, never reached the instrument being played, and went
            // into the capture ring at the render's positions.
            // [`DawEngine::play_live_without_device`] swaps the live ones in.
            Arc::new(Mutex::new(MidiInputManager::new())),
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            sample_rate,
            buffer_size as u32,
            LoadCounters::new(),
            // A bounce is the music, not the guide track.
            crate::metronome::MetronomeSettings::silent(),
            // Its own request slot, never wired to the UI: an export cannot
            // pick up whatever someone is auditioning in the browser.
            crate::preview_player::PreviewRequest::new(),
            // A bounce should sound like playback, so it reads the same
            // switches the settings panel sets.
            self.audio_prefs.clone(),
            Arc::clone(&self.record_pass),
            Arc::clone(&self.record_last_pos),
            worker_pool,
            Arc::clone(&self.reference),
            // A render is the song. The reference switch, left on from
            // comparing, made the master play the reference record in place
            // of the mix, so an export (or a freeze, a bounce, a stem) was
            // somebody else's record.
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::clone(&self.reference_gain_centi_db),
            // An export plays the song as written, not whatever was
            // launched when the button was pressed, so the render gets
            // a grid of its own with nothing in it.
            Arc::new(crate::session::SessionState::new()),
        );

        // Populate insert chains from the project metadata so the export
        // applies the same FX as live playback. No-op when no factory is
        // supplied (e.g. engine unit tests that don't host plug-ins).
        if let Some(inst) = instantiate {
            callback.hydrate_offline_inserts(inst);
        }
        callback
    }
}

/// Reads a project's audio into the shared pool without the engine; see
/// [`DawEngine::audio_source_loader`].
pub struct AudioSourceLoader {
    pool: AudioPool,
    target_sr: u32,
    project_dir: Option<std::path::PathBuf>,
    trusted: Vec<String>,
}

impl AudioSourceLoader {
    /// Load every audio file the project's clips play that the pool does
    /// not hold yet. `progress(done, total)` follows along. Returns the
    /// files that could not be loaded.
    pub fn load_sources_of(
        &self,
        project: &Project,
        mut progress: impl FnMut(usize, usize),
    ) -> Vec<String> {
        use hardwave_project::clip::ClipContent;
        // (pool id, file on disk). The id is what clips play through; the
        // file is where the audio has to be read from. They are different
        // strings: feeding the pool id to the filesystem made every source
        // "go missing" on reload.
        let mut set = std::collections::BTreeSet::new();
        let mut collect = |content: &ClipContent| {
            if let ClipContent::Audio(ac) = content {
                // Projects written before `source_file` existed carry the
                // path in `source_path`, so fall back to it.
                let file = if ac.source_file.is_empty() {
                    ac.source_path.clone()
                } else {
                    ac.source_file.clone()
                };
                set.insert((ac.source_path.clone(), file));
            }
        };
        for track in &project.tracks {
            for clip in &track.clips {
                collect(&clip.content);
            }
        }
        for arrangement in &project.arrangements {
            for timeline in arrangement.timelines.values() {
                for clip in &timeline.clips {
                    collect(&clip.content);
                }
            }
        }
        // Sources already resident (a sample shared with the song open
        // before) are not read again.
        let wanted: Vec<(String, String)> = set
            .into_iter()
            .filter(|(id, _)| self.pool.get(id).is_none())
            .collect();
        let total = wanted.len();
        let mut missing = Vec::new();
        for (done, (source_id, file)) in wanted.into_iter().enumerate() {
            progress(done, total);
            let loaded =
                crate::source_paths::check(&file, self.project_dir.as_deref(), &self.trusted)
                    .map_err(|why| why.to_string())
                    .and_then(|path| {
                        hardwave_dsp::AudioFileReader::read_resampled(&path, Some(self.target_sr))
                            .map_err(|e| e.to_string())
                    });
            match loaded {
                Ok((info, channels)) => {
                    let num_frames = channels.first().map(|c| c.len()).unwrap_or(0);
                    self.pool.insert(
                        source_id,
                        AudioBuffer {
                            channels,
                            sample_rate: info.sample_rate,
                            num_frames,
                        },
                    );
                }
                Err(e) => {
                    log::warn!("audio source '{file}' was not loaded: {e}");
                    missing.push(file);
                }
            }
        }
        progress(total, total);
        missing
    }
}

/// A render prepared by [`DawEngine::prepare_offline_render`], holding its
/// own copy of everything, so it runs without the engine.
pub struct OfflineRender {
    callback: EngineCallback,
    buffer_size: usize,
    /// Put on every frame handed out; see [`Self::for_a_track`].
    output_gain: f32,
}

impl OfflineRender {
    /// Make this a render whose audio goes back onto a track: a bounce, a
    /// freeze, a consolidate. That audio plays through a centred pan and the
    /// master fader again, so the render leaves both out: the master fader
    /// stands at 0 dB, and the level a centred pan takes off (3 dB with the
    /// usual law) is given back. Without this a bounced, frozen or
    /// consolidated part came back 3 dB quieter than the part it replaced,
    /// and lower again by wherever the master fader sat.
    ///
    /// It is also the track's part on the playlist. A render takes the
    /// transport's mode, so freezing, bouncing or consolidating while the
    /// transport was in pattern mode rendered the channel rack's pattern,
    /// looped, in place of the part.
    pub fn for_a_track(mut self) -> Self {
        self.callback
            .transport
            .master_volume_db
            .store(0.0, std::sync::atomic::Ordering::Relaxed);
        let (centre, _) = crate::pan::gains(0.0, self.callback.audio_prefs.pan_law());
        self.output_gain = if centre > 1e-6 { 1.0 / centre } else { 1.0 };
        self.play_mode(false);
        self
    }

    /// Make this a render of the channel rack's pattern, played once: the
    /// export's Pattern range. It rendered the song's first bars unless the
    /// transport happened to be in pattern mode, and then looped the
    /// pattern into the tail. The pattern plays from its start and is not
    /// started again at its end, so what follows is only what still sounds.
    pub fn the_pattern_once(mut self) -> Self {
        self.play_mode(true);
        self.callback
            .transport
            .pattern_wraps
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// Render in pattern mode or out of it, whatever the transport is in.
    /// The graph was built in the transport's mode, so it is built again
    /// before the first block (the plug-ins carry over).
    fn play_mode(&mut self, pattern: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        let transport = &self.callback.transport;
        if transport.pattern_mode.swap(pattern, Relaxed) != pattern {
            self.callback.needs_rebuild = true;
        }
    }

    /// Render `total_samples` frames of the song from where the render was
    /// prepared, handing each interleaved stereo block to `on_block`; it
    /// returns false to stop early.
    ///
    /// The graph's plug-ins hold the sound back by their latency (a
    /// limiter's lookahead, an oversampler), and compensation only lines
    /// the tracks up with each other: the whole mix still comes out that
    /// much late. The first frames out of the graph are dropped and as many
    /// more are rendered at the end, so the song's first sample is the
    /// file's first sample. A bounce used to start with that much silence
    /// and land late by it.
    pub fn run(self, total_samples: u64, on_block: impl FnMut(&[f32]) -> bool) {
        self.run_swapping(total_samples, None, on_block);
    }

    /// `run`, with the graph built on a builder thread and a new one asked
    /// for every `swap_every` blocks, swapped in whenever it is ready, the
    /// way playback gets one after every edit. Returns how many were
    /// swapped in. A check that playback through swaps sounds exactly like
    /// an export.
    #[doc(hidden)]
    pub fn run_with_graph_swaps(
        mut self,
        total_samples: u64,
        swap_every: usize,
        on_block: impl FnMut(&[f32]) -> bool,
    ) -> u64 {
        self.callback.attach_builder_thread();
        self.run_swapping(total_samples, Some(swap_every.max(1)), on_block)
    }

    fn run_swapping(
        mut self,
        total_samples: u64,
        swap_every: Option<usize>,
        mut on_block: impl FnMut(&[f32]) -> bool,
    ) -> u64 {
        let first_graph = self.callback.graph_seq;
        let mut blocks = 0usize;
        // A render runs plug-ins and tails like the audio thread does, on
        // whatever thread asked for it: flush denormals for the length of
        // it, then leave that thread as it was. Every offline render (an
        // export, a freeze, a bounce) comes through here.
        let _denormals = hardwave_audio_io::denormals::flush_denormals_while();
        let mut buf = vec![0.0_f32; self.buffer_size * 2];
        let mut remaining = total_samples;
        // The frames still to drop: known once the first block has built the
        // graph and measured its latency.
        let mut to_drop: Option<u64> = None;
        while remaining > 0 {
            let ahead = to_drop.unwrap_or(self.buffer_size as u64);
            let frames = (remaining + ahead).min(self.buffer_size as u64) as usize;
            let slice = &mut buf[..frames * 2];
            slice.fill(0.0);
            if swap_every.is_some_and(|every| blocks % every == every - 1) {
                self.callback.needs_rebuild = true;
            }
            blocks += 1;
            self.callback.process(slice, frames, 2);
            let pending = *to_drop.get_or_insert_with(|| {
                self.callback
                    .graph_latency_samples
                    .load(std::sync::atomic::Ordering::Relaxed) as u64
            });
            let dropped = pending.min(frames as u64) as usize;
            to_drop = Some(pending - dropped as u64);
            let kept = ((frames - dropped) as u64).min(remaining) as usize;
            if kept == 0 {
                continue;
            }
            let out = &mut slice[dropped * 2..(dropped + kept) * 2];
            if self.output_gain != 1.0 {
                for s in out.iter_mut() {
                    *s *= self.output_gain;
                }
            }
            if !on_block(out) {
                break;
            }
            remaining -= kept as u64;
        }
        self.callback.graph_seq - first_graph
    }
}

/// Decibels to a linear gain, as the track nodes use it.
fn db_to_linear_f32(db: f64) -> f32 {
    10.0_f64.powf(db / 20.0) as f32
}

impl Default for DawEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple hash for generating source IDs from file paths.
fn md5_hash(data: &[u8]) -> u64 {
    // FNV-1a 64-bit hash (fast, no crypto needed — just a unique key)
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The pool key `load_audio_file` derives for a path — kept in one place so
/// `rehydrate_audio_pool`'s already-resident check can never drift from it.
pub fn source_id_for_path(path: &str) -> String {
    format!("{:x}", md5_hash(path.as_bytes()))
}

// ---------------------------------------------------------------------------
// Audio callback (runs on the real-time audio thread)
// ---------------------------------------------------------------------------

struct EngineCallback {
    transport: TransportState,
    project: Arc<Mutex<Project>>,
    meter_producer: rtrb::Producer<MeterSnapshot>,
    command_rx: Receiver<EngineCommand>,
    audio_pool: AudioPool,
    track_meters: TrackMeterMap,
    master_tap: SharedMasterTap,
    graph: AudioGraph,
    meter: ChannelMeter,
    /// Pre-sized deinterleave scratch for the meter path. Reused every
    /// block so the audio thread never allocates to feed the meter.
    meter_left: Vec<f32>,
    meter_right: Vec<f32>,
    sample_rate: u32,
    needs_rebuild: bool,
    /// Explicit id of the master node so we don't depend on its position in
    /// the node vector — the input node is added after master when armed
    /// tracks exist, so `node_count - 1` no longer identifies the master.
    master_id: Option<crate::graph::NodeId>,
    /// True when at least one track is armed with input monitoring enabled.
    /// When false AND the transport isn't playing, we can short-circuit the
    /// graph and emit silence to save CPU.
    has_monitored_input: bool,
    /// The meters were set to silence after the transport stopped. The
    /// graph does not run while stopped, so without this every meter kept
    /// showing the last block it played.
    meters_parked: bool,
    /// Keeps the playhead on a Link session's beat.
    link_follow: crate::link::PhaseFollower,
    /// The MIDI tracks' nodes in the current graph, so picking another track
    /// for the keyboard is set on them without a rebuild.
    midi_track_nodes: Vec<(String, crate::graph::NodeId)>,
    graph_latency_samples: Arc<std::sync::atomic::AtomicU32>,
    /// Per-track plug-in command receiver. Drained at the start of every
    /// audio block; commands route to TrackNodes by track_id.
    insert_command_rx: crate::insert_chain::InsertCommandReceiver,
    /// Audio→drop-thread channel for vacated plug-in instances. The
    /// audio thread never frees plug-ins directly; it pushes them here
    /// and the UI thread (or the engine on shutdown) drops them.
    insert_graveyard_tx: crate::insert_chain::PluginGraveyardSender,
    /// Stable track-id → graph-NodeId map maintained across rebuilds so
    /// per-track commands resolve to the right TrackNode in O(1).
    track_id_to_node: HashMap<String, crate::graph::NodeId>,
    /// UI-thread snapshot request slot. Cloned from `DawEngine`. When a
    /// `SyncSender` lands here the audio thread harvests `get_state()`
    /// from every loaded plug-in and ships the resulting map back.
    pending_state_snapshot: PluginStateSnapshotSlot,
    /// Shared handle to the live MIDI input manager. Drained at the
    /// start of every audio block via `try_drain_events_into` (non-
    /// blocking — if the mutex is contended this block we just skip
    /// drain and pick up the events on the next block, which adds at
    /// most one block of latency). Cloned from
    /// [`DawEngine::midi_input`] so injected events (from the on-screen
    /// keyboard and Tauri `inject_midi_event` command) and external
    /// hardware events flow through the exact same path.
    midi_input: Arc<Mutex<MidiInputManager>>,
    /// Reusable per-block scratch for live MIDI events. Pre-sized so
    /// the audio thread doesn't allocate when the controller is busy.
    midi_event_scratch: Vec<hardwave_midi::MidiEvent>,
    /// Drained events with how late each one is, in samples. Pre-sized so
    /// the audio thread never allocates for it.
    midi_aged_scratch: Vec<(hardwave_midi::MidiEvent, u64)>,
    /// Shared rolling capture for "dump last N seconds to pattern".
    /// Pushed-into per block on the audio thread.
    midi_capture_ring: Arc<Mutex<MidiCaptureRing>>,
    /// Insert commands whose target TrackNode doesn't exist yet because
    /// a graph rebuild was deferred by lock contention (see
    /// `rebuild_graph`'s try_lock). Retried at the start of the next
    /// dispatch pass instead of being dropped — otherwise a plug-in
    /// added to a just-created track could silently vanish. Pre-sized
    /// so pushes within capacity never allocate on the audio thread.
    deferred_insert_commands: Vec<crate::insert_chain::InsertCommand>,
    /// Shared with `DawEngine` so the UI can read real audio load. Written
    /// only here, on the audio thread.
    audio_load_permille: Arc<std::sync::atomic::AtomicU32>,
    audio_xruns: Arc<std::sync::atomic::AtomicU32>,
    /// Renders the click for this block. The offline renderer is built with
    /// permanently silent settings, so a bounce cannot contain it.
    metronome: crate::metronome::Metronome,
    /// Plays browser auditions. Like the click, mixed in after the tap, so an
    /// audition cannot end up in a recording or a bounce.
    preview: crate::preview_player::PreviewPlayer,
    /// The test tone, mixed in with the auditions. Built unwired; only the
    /// live callback is handed the request the window starts it with.
    test_tone: crate::test_tone::TestTone,
    /// The first run's check voice, the same way.
    check_voice: crate::check_voice::CheckVoice,
    /// The Audio settings panel's behaviour switches, shared with the engine
    /// so a change applies to the next block.
    audio_prefs: crate::audio_prefs::AudioPrefs,
    /// Which loop pass the recording is on, shared with the engine so a MIDI
    /// take can be split per pass the way an audio take is.
    record_pass: Arc<std::sync::atomic::AtomicU32>,
    record_last_pos: Arc<std::sync::atomic::AtomicU64>,
    /// Frames run since the engine started, playing or not: the clock the
    /// MIDI capture keeps so notes played with the song stopped can still be
    /// laid out by when they were played.
    capture_clock: u64,
    /// Frozen tracks' plug-in chains, kept while the track plays its render:
    /// not run (that is what freezing saves), not dropped either, so
    /// unfreezing puts them back with their settings. They were dropped.
    parked_chains: HashMap<String, crate::insert_chain::InsertChain>,
    /// Frames left to keep processing with the song stopped, after the last
    /// live MIDI event (see `LIVE_MIDI_HOLD_SECONDS`).
    live_midi_hold: u64,
    /// Live keys down, one bit per note for each channel, and the sustain
    /// pedal per channel: while any is held the engine keeps running.
    live_keys_down: [u128; 16],
    live_sustain: [bool; 16],
    /// Which graph edges carry each send, filled at rebuild so a SendLevel
    /// automation lane can move a send's gain per block.
    send_edges: Vec<SendEdges>,
    /// Modulation's state between blocks: each route's envelope follower,
    /// where every MIDI controller was last moved, each route's value this
    /// block (NaN when off), and the level and pan it gave each node this
    /// block and the last, so a node whose route went away is put back.
    mod_env: Vec<f32>,
    mod_cc: [f32; 128],
    mod_values: Vec<f64>,
    mod_mix: Vec<(usize, f32, f32)>,
    mod_mix_prev: Vec<(usize, f32, f32)>,
    /// The edges that carry live input to a monitoring track, switched per
    /// block for tracks whose monitoring follows the transport.
    monitor_edges: Vec<MonitorEdges>,
    /// This block's clicks. A fixed-size array rather than a Vec because it
    /// is filled on the audio thread, which must not allocate.
    click_events: [crate::metronome::ClickEvent; crate::metronome::MAX_CLICKS_PER_BLOCK],
    /// Where graphs are built: on the calling thread (an offline render,
    /// and until playback starts the builder thread), or on the builder
    /// thread, which hands them over finished.
    builder: BuildMode,
    /// Per node of the graph: where its chain's delay and size are
    /// published for the next build, and the delay the compensation was
    /// worked out with.
    chain_boards: Vec<Option<Arc<ChainBoard>>>,
    assumed_latency: Vec<u32>,
    /// The build request the graph in use answers.
    graph_seq: u64,
    /// The tempo map the graph in use was built with, for a block in which
    /// the project is busy.
    tempo_map: hardwave_project::TempoMap,
}

/// What one graph cost (see `DawEngine::graph_build_times`).
/// The kinds of work [`LiveProfile::kinds`] counts per block, in order:
/// audio tracks and buses, instrument tracks, the master, gathering the
/// inputs of every node (sums, sends and compensation lines), and nodes of
/// any other kind.
#[doc(hidden)]
pub const LIVE_PROFILE_KINDS: [&str; 5] = ["track", "instrument", "master", "gather", "other"];

/// What [`DawEngine::profile_live_blocks`] measured.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct LiveProfile {
    /// The whole callback, per device block.
    pub blocks: Vec<std::time::Duration>,
    /// Per block, nanoseconds per kind of work ([`LIVE_PROFILE_KINDS`]).
    pub kinds: Vec<[u64; 5]>,
    /// Per node, over the run.
    pub nodes: Vec<NodeCost>,
    /// How many graphs were swapped in during the run.
    pub swaps: usize,
}

/// One node's time over a [`LiveProfile`] run.
#[doc(hidden)]
#[derive(Debug, Default, Clone)]
pub struct NodeCost {
    pub name: String,
    pub kind: &'static str,
    /// Gathering its inputs.
    pub gather_ns: u64,
    /// Running it.
    pub run_ns: u64,
    /// The blocks it ran in.
    pub blocks: u64,
    /// Its slowest block, both together.
    pub max_ns: u64,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct GraphBuildTime {
    /// Building it, off the audio thread.
    pub build: std::time::Duration,
    /// Swapping it in, on the audio thread.
    pub swap: std::time::Duration,
}

/// Audio-thread load counters shared with the UI: how much of each block's
/// budget was used, and how many blocks overran it.
#[derive(Clone)]
pub struct LoadCounters {
    pub load_permille: Arc<std::sync::atomic::AtomicU32>,
    pub xruns: Arc<std::sync::atomic::AtomicU32>,
}

impl LoadCounters {
    pub fn new() -> Self {
        Self {
            load_permille: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            xruns: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }
}

impl Default for LoadCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Share of a block's deadline that was spent working, in per mille.
///
/// The deadline is wall-clock: a 256-frame block at 48 kHz must be finished
/// within 5.33 ms or the device gets no audio and the user hears a click.
/// Over 1000 means the block missed it.
fn audio_load_reading(used_secs: f64, budget_secs: f64) -> u32 {
    if budget_secs <= 0.0 {
        return 0;
    }
    ((used_secs / budget_secs) * 1000.0)
        .round()
        .clamp(0.0, 10_000.0) as u32
}

/// Move the published value towards a new reading.
///
/// Rising fast and falling slowly on purpose: a climb towards the deadline is
/// what a producer needs to see before it starts clicking, while a dip is not
/// worth redrawing the bar for.
fn smooth_load(previous: u32, reading: u32) -> u32 {
    if reading > previous {
        (previous + (reading - previous) / 2).min(reading)
    } else {
        previous - (previous - reading) / 8
    }
}

impl EngineCallback {
    /// Show silence on every meter once the transport stops. Tries again
    /// on the next block when the UI holds the meter map.
    fn park_meters(&mut self) {
        use std::sync::atomic::Ordering;
        if self.meters_parked {
            return;
        }
        let Some(meters) = self.track_meters.try_lock() else {
            return;
        };
        for m in meters.values() {
            m.peak_db_l.store(-120.0, Ordering::Relaxed);
            m.peak_db_r.store(-120.0, Ordering::Relaxed);
            m.rms_db.store(-120.0, Ordering::Relaxed);
            m.pre_fader_peak_db.store(-120.0, Ordering::Relaxed);
        }
        drop(meters);
        let silent = MeterSnapshot {
            peak_db: -120.0,
            peak_hold_db: self.meter.peak_hold_db(),
            true_peak_db: -120.0,
            rms_db: -120.0,
            lufs_m: None,
            lufs_s: None,
            lufs_i: self.meter.lufs_i(),
            clipped: self.meter.clipped(),
            correlation: self.meter.correlation(),
        };
        self.meters_parked = self.meter_producer.push(silent).is_ok();
    }

    /// Publish how much of this block's budget the engine used.
    ///
    /// The budget is the wall-clock time the block represents: at 48 kHz a
    /// 256-frame block must be finished within 5.33 ms or the device gets no
    /// audio and the user hears a click. Spending 2.6 ms of that is 50%.
    ///
    /// Smoothed towards the new value so the meter is readable, except
    /// upwards past 100%, which is reported immediately: a spike that drops
    /// audio matters more than a tidy average. Overruns are counted
    /// separately because an average of 40% with regular xruns still
    /// crackles, and only the count reveals it.
    fn publish_audio_load(&self, started: std::time::Instant, num_frames: usize) {
        if num_frames == 0 || self.sample_rate == 0 {
            return;
        }
        let budget = num_frames as f64 / self.sample_rate as f64;
        let reading = audio_load_reading(started.elapsed().as_secs_f64(), budget);

        use std::sync::atomic::Ordering::Relaxed;
        if reading > 1000 {
            // Reported as-is, not smoothed: a spike that drops audio is
            // exactly what must not be averaged into a calm-looking number.
            self.audio_xruns.fetch_add(1, Relaxed);
            self.audio_load_permille.store(reading, Relaxed);
            return;
        }
        let previous = self.audio_load_permille.load(Relaxed);
        self.audio_load_permille
            .store(smooth_load(previous, reading), Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        transport: TransportState,
        project: Arc<Mutex<Project>>,
        meter_producer: rtrb::Producer<MeterSnapshot>,
        command_rx: Receiver<EngineCommand>,
        audio_pool: AudioPool,
        track_meters: TrackMeterMap,
        input_consumer: SharedInputConsumer,
        captures: Arc<[Arc<CaptureTap>]>,
        master_tap: SharedMasterTap,
        graph_latency_samples: Arc<std::sync::atomic::AtomicU32>,
        insert_command_rx: crate::insert_chain::InsertCommandReceiver,
        insert_graveyard_tx: crate::insert_chain::PluginGraveyardSender,
        pending_state_snapshot: PluginStateSnapshotSlot,
        midi_input: Arc<Mutex<MidiInputManager>>,
        midi_capture_ring: Arc<Mutex<MidiCaptureRing>>,
        sample_rate: u32,
        buffer_size: u32,
        load: LoadCounters,
        metronome: crate::metronome::MetronomeSettings,
        preview: crate::preview_player::PreviewRequest,
        audio_prefs: crate::audio_prefs::AudioPrefs,
        record_pass: Arc<std::sync::atomic::AtomicU32>,
        record_last_pos: Arc<std::sync::atomic::AtomicU64>,
        worker_pool: Arc<Mutex<Option<Arc<crate::parallel::WorkerPool>>>>,
        reference: Arc<Mutex<Option<Arc<crate::audio_pool::AudioBuffer>>>>,
        reference_on: Arc<std::sync::atomic::AtomicBool>,
        reference_gain_centi_db: Arc<std::sync::atomic::AtomicI64>,
        session: Arc<crate::session::SessionState>,
    ) -> Self {
        let input_block = InputBlock::shared(buffer_size as usize);
        let builder = GraphBuilder {
            project: Arc::clone(&project),
            track_meters: Arc::clone(&track_meters),
            transport: transport.clone(),
            audio_pool: audio_pool.clone(),
            audio_prefs: audio_prefs.clone(),
            session: Arc::clone(&session),
            worker_pool: Arc::clone(&worker_pool),
            reference: Arc::clone(&reference),
            reference_on: Arc::clone(&reference_on),
            reference_gain_centi_db: Arc::clone(&reference_gain_centi_db),
            input_consumer: Arc::clone(&input_consumer),
            input_block: Arc::clone(&input_block),
            captures: Arc::clone(&captures),
            sample_rate,
            buffer_size: buffer_size as usize,
            boards: HashMap::new(),
        };
        let mut cb = Self {
            builder: BuildMode::Inline(Box::new(builder)),
            chain_boards: Vec::new(),
            assumed_latency: Vec::new(),
            graph_seq: 0,
            tempo_map: hardwave_project::TempoMap::default(),
            preview: crate::preview_player::PreviewPlayer::new(preview),
            test_tone: crate::test_tone::TestTone::unwired(),
            check_voice: crate::check_voice::CheckVoice::unwired(),
            audio_prefs,
            record_pass,
            record_last_pos,
            capture_clock: 0,
            parked_chains: HashMap::new(),
            live_midi_hold: 0,
            live_keys_down: [0; 16],
            live_sustain: [false; 16],
            send_edges: Vec::with_capacity(64),
            mod_env: vec![0.0; MAX_MOD_ROUTES],
            mod_cc: [0.0; 128],
            mod_values: Vec::with_capacity(MAX_MOD_ROUTES),
            mod_mix: Vec::with_capacity(MAX_MOD_ROUTES),
            mod_mix_prev: Vec::with_capacity(MAX_MOD_ROUTES),
            monitor_edges: Vec::with_capacity(8),
            click_events: [crate::metronome::ClickEvent {
                frame_offset: 0,
                downbeat: false,
            }; crate::metronome::MAX_CLICKS_PER_BLOCK],
            audio_load_permille: load.load_permille,
            audio_xruns: load.xruns,
            metronome: crate::metronome::Metronome::new(metronome),
            transport,
            project,
            meter_producer,
            command_rx,
            audio_pool,
            track_meters,
            master_tap,
            graph: AudioGraph::new(buffer_size as usize),
            meter: ChannelMeter::new(sample_rate as f64),
            meter_left: Vec::with_capacity(buffer_size as usize),
            meter_right: Vec::with_capacity(buffer_size as usize),
            sample_rate,
            needs_rebuild: true,
            master_id: None,
            has_monitored_input: false,
            meters_parked: false,
            link_follow: crate::link::PhaseFollower::default(),
            midi_track_nodes: Vec::new(),
            graph_latency_samples,
            insert_command_rx,
            insert_graveyard_tx,
            track_id_to_node: HashMap::new(),
            pending_state_snapshot,
            midi_input,
            // 256 fits well above the typical few-events-per-block load
            // even from a chord-spamming controller (~32 events). Sized
            // up-front so the audio thread doesn't grow the Vec on its
            // first busy block.
            midi_event_scratch: Vec::with_capacity(256),
            midi_aged_scratch: Vec::with_capacity(256),
            midi_capture_ring,
            // 64 comfortably exceeds the commands one UI tick can emit
            // for tracks that don't have nodes yet (a track add plus a
            // handful of chain edits).
            deferred_insert_commands: Vec::with_capacity(64),
        };
        cb.rebuild_graph();
        cb
    }

    /// Honour a one-shot plug-in state snapshot request from the UI
    /// thread, if any. Called once per audio block. If the UI placed a
    /// SyncSender into `pending_state_snapshot`, we drain it here, walk
    /// every track in `track_id_to_node`, call `get_state()` on each
    /// loaded plug-in slot, and ship the map back via the channel.
    ///
    /// `get_state` is allowed on the audio thread for the formats we
    /// host today: native plug-ins return tiny JSON blobs (single-digit
    /// kilobytes), CLAP / VST3 chunks are also small and the
    /// implementations don't allocate beyond a `Vec<u8>`. A future
    /// commit can move this to a worker thread if a third-party plug-in
    /// turns out to misbehave.
    /// Hand every live plug-in the parameter changes still waiting for a
    /// block (see `HostedPlugin::flush_parameters`). Does not allocate.
    fn flush_plugin_parameters(&mut self) {
        for node in self.graph.iter_nodes_mut() {
            node.flush_plugin_parameters();
        }
        for chain in self.parked_chains.values_mut() {
            chain.flush_parameters();
        }
    }

    fn service_snapshot_request(&mut self) {
        // RT-safety: try_lock — if the UI thread is placing a request
        // into the slot right now, we simply pick it up next block
        // instead of blocking the audio callback on its mutex.
        let tx = {
            let Some(mut slot) = self.pending_state_snapshot.try_lock() else {
                return;
            };
            slot.take()
        };
        let Some(tx) = tx else { return };
        // The state read back has every value set so far, not only the
        // ones a block has run with.
        self.flush_plugin_parameters();

        let mut map: std::collections::HashMap<(String, String), Vec<u8>> =
            std::collections::HashMap::new();
        // Clone the (track_id, node_id) pairs first so the iteration
        // borrow doesn't conflict with the mutable graph access below.
        let pairs: Vec<(String, crate::graph::NodeId)> = self
            .track_id_to_node
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        for (track_id, node_id) in pairs {
            if let Some(node) = self.graph.node_mut(node_id) {
                for (slot_id, bytes) in node.snapshot_plugin_states() {
                    map.insert((track_id.clone(), slot_id), bytes);
                }
            }
        }
        // A frozen track's plug-ins are parked, not in the graph; a save
        // keeps their settings too.
        for (track_id, chain) in &self.parked_chains {
            for slot in &chain.slots {
                map.insert(
                    (track_id.clone(), slot.slot_id.clone()),
                    slot.plugin.get_state(),
                );
            }
        }
        // Best-effort send — if the UI thread dropped the receiver
        // (e.g. on save abort) we silently discard.
        let _ = tx.try_send(map);
    }

    /// Drain pending insert-chain commands and dispatch each to the
    /// TrackNode whose `track_id` matches. Called on the audio thread
    /// at the start of every block, BEFORE the graph runs, so the
    /// chain reflects the latest UI state for this block.
    fn dispatch_insert_commands(&mut self) {
        // Retry commands parked by an earlier block whose target node
        // didn't exist yet (rebuild deferred by lock contention). Only
        // replay once the rebuild has actually landed — while it is still
        // pending the targets can't exist, and replaying would re-park
        // into the freshly-taken (capacity-0) Vec, allocating on the
        // audio thread. With the rebuild done nothing re-parks, so the
        // original allocation is retained via the put-back below.
        if !self.graph_pending() && !self.deferred_insert_commands.is_empty() {
            let mut parked = std::mem::take(&mut self.deferred_insert_commands);
            for cmd in parked.drain(..) {
                self.dispatch_one_insert_command(cmd, true);
            }
            // Put the (now empty) original allocation back if nothing
            // re-parked in the meantime; otherwise keep the new pushes.
            if self.deferred_insert_commands.is_empty() {
                self.deferred_insert_commands = parked;
            }
        }
        while let Some(cmd) = self.insert_command_rx.try_recv() {
            self.dispatch_one_insert_command(cmd, false);
        }
    }

    /// `replay` is a command parked by an earlier block coming round again.
    fn dispatch_one_insert_command(
        &mut self,
        cmd: crate::insert_chain::InsertCommand,
        replay: bool,
    ) {
        let sample_rate = self.sample_rate as f64;
        let buffer_size = self.graph.buffer_size_hint();
        // A new song: every node's plug-ins go, and so does whatever was
        // parked for the old song's tracks.
        let Some(target) = cmd.target_track_id() else {
            for parked in self.deferred_insert_commands.drain(..) {
                if let crate::insert_chain::InsertCommand::Add { mut slot, .. } = parked {
                    slot.plugin.deactivate();
                    if let Err(slot) = self.insert_graveyard_tx.try_bury(slot) {
                        std::mem::forget(slot);
                    }
                }
            }
            for node in self.graph.iter_nodes_mut() {
                node.apply_insert_command(
                    crate::insert_chain::InsertCommand::ClearAll,
                    &mut self.insert_graveyard_tx,
                    sample_rate,
                    buffer_size,
                );
            }
            // And the frozen tracks' chains: the new song has its own.
            for (_, mut chain) in self.parked_chains.drain() {
                for slot in chain.slots.drain(..) {
                    if let Err(slot) = self.insert_graveyard_tx.try_bury(slot) {
                        std::mem::forget(slot);
                    }
                }
            }
            return;
        };
        // Borrow the target id just long enough to look it up; after
        // `.copied()` the NodeId is owned and the borrow on `cmd`
        // ends, freeing it to move into apply_insert_command below.
        let node_id = self.track_id_to_node.get(target).copied();
        let Some(node_id) = node_id else {
            // No node for this track yet: a rebuild still pending (deferred
            // by try_lock contention), or an empty insert, which has no node
            // until something is put on it. A plug-in added to one asks for
            // the rebuild that gives it a node, and waits for it, parked.
            // Once round is enough: a command still without a node after the
            // rebuild has no track to go to.
            let adds = matches!(cmd, crate::insert_chain::InsertCommand::Add { .. });
            if adds && !replay {
                self.needs_rebuild = true;
            }
            if self.graph_pending()
                && !replay
                && self.deferred_insert_commands.len() < self.deferred_insert_commands.capacity()
            {
                self.deferred_insert_commands.push(cmd);
            } else if let crate::insert_chain::InsertCommand::Add { mut slot, .. } = cmd {
                // Never dropped here: a plug-in is let go off this thread.
                slot.plugin.deactivate();
                if let Err(slot) = self.insert_graveyard_tx.try_bury(slot) {
                    std::mem::forget(slot);
                }
            } else {
                log::warn!(
                    "insert chain: no TrackNode for track_id {}; dropping command",
                    cmd.target_track_id().unwrap_or_default()
                );
            }
            return;
        };
        // Adding, removing or switching a plug-in changes how late its track
        // is. The rebuild works out the compensation again so the other
        // tracks wait for it; until now that waited for the next edit of the
        // song, and a plug-in added while playing was heard out of time.
        let moves_latency = matches!(
            cmd,
            crate::insert_chain::InsertCommand::Add { .. }
                | crate::insert_chain::InsertCommand::Remove { .. }
                | crate::insert_chain::InsertCommand::SetEnabled { .. }
                | crate::insert_chain::InsertCommand::ClearAll
        );
        if let Some(node) = self.graph.node_mut(node_id) {
            node.apply_insert_command(cmd, &mut self.insert_graveyard_tx, sample_rate, buffer_size);
            if moves_latency {
                self.needs_rebuild = true;
            }
        }
        // The next graph is built knowing what the chain holds now.
        self.publish_chain(node_id);
    }

    /// Populate every track's insert chain from the project's saved
    /// `inserts` metadata, for offline render. The live path builds chains
    /// via the `InsertCommand` queue / `hydrate_chains_from_project`, but a
    /// fresh offline callback fires no commands — without this, exports
    /// would render dry (no EQ/comp/reverb/synth FX). `instantiate` is the
    /// command layer's native/VST3/CLAP factory keyed by plugin id; saved
    /// plug-in state is replayed so the bounce matches the mix.
    fn hydrate_offline_inserts(&mut self, instantiate: OfflineInsertFactory<'_>) {
        let buffer_size = self.graph.buffer_size_hint();
        let sr = self.sample_rate as f64;
        // Snapshot the insert plan under the project lock, then instantiate
        // outside it (plug-in loads can be slow and must not hold the lock).
        // (track_id, slot_id, plugin_id, enabled, wet, sidechained, saved_state)
        let mut hydrated_any = false;
        let plan = {
            let project = self.project.lock();
            let mut acc = Vec::new();
            // A frozen track plays a render that already went through its
            // plug-ins, and playback parks them (see `rebuild_graph`). They
            // were loaded for every export all the same, so its effects ran
            // twice and an instrument on it played over the render with no
            // notes to play.
            for t in project.tracks.iter().filter(|t| !t.frozen) {
                for s in &t.inserts {
                    let state = project.plugin_state(&s.id).map(|e| e.chunk.clone());
                    acc.push((
                        t.id.clone(),
                        s.id.clone(),
                        s.plugin_id.clone(),
                        s.enabled,
                        s.wet,
                        s.sidechain_source.is_some(),
                        state,
                    ));
                }
            }
            acc
        };
        for (track_id, slot_id, plugin_id, enabled, wet, sidechained, state) in plan {
            let Some(node_id) = self.track_id_to_node.get(&track_id).copied() else {
                continue;
            };
            let Some(mut plugin) = instantiate(&plugin_id) else {
                continue;
            };
            if let Some(bytes) = state {
                let _ = plugin.set_state(&bytes);
            }
            hydrated_any = true;
            if let Some(node) = self.graph.node_mut(node_id) {
                let slot = crate::insert_chain::LiveSlot {
                    param_ranges: crate::insert_chain::LiveSlot::ranges_of(plugin.as_ref()),
                    levels: Default::default(),
                    slot_id,
                    plugin,
                    enabled,
                    wet,
                    // Honour the slot's sidechain routing offline too. The
                    // graph edges that feed a source track into this node's
                    // input ports 2/3 are built by `rebuild_graph` from the
                    // same project metadata, so the bus is already present in
                    // an offline graph — only this per-slot flag was missing,
                    // which silently dropped ducking from every export while
                    // playback ducked correctly.
                    sidechain_active: sidechained,
                    // Nothing reads a meter during an offline render.
                    gain_reduction_db: crate::insert_chain::LiveSlot::new_gain_reduction(),
                };
                node.push_offline_slot(slot, sr, buffer_size);
            }
        }

        // Recompute PDC now that plug-ins are on the graph. `rebuild_graph`
        // already ran `finalize_pdc` from `EngineCallback::new`, but that was
        // before any insert existed and `needs_rebuild` is already false, so
        // `process()` never runs it again — an export would otherwise align a
        // graph in which every node still claimed zero latency, while live
        // playback re-runs the rebuild (and therefore PDC) on every mutation.
        if hydrated_any {
            self.graph.finalize_pdc(
                self.transport
                    .pdc_enabled
                    .load(std::sync::atomic::Ordering::Relaxed),
            );
            self.graph.index_delays();
            // A graph built again for this render (`OfflineRender::play_mode`)
            // knows the chains' delays, and this one was worked out with them.
            self.publish_chains();
            for (node_id, assumed) in self.assumed_latency.iter_mut().enumerate() {
                if let Some(node) = self.graph.node_mut(node_id) {
                    *assumed = node.latency_samples();
                }
            }
            self.graph_latency_samples.store(
                self.graph.total_latency_samples(),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    fn process_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                EngineCommand::Transport(tcmd) => self.process_transport(tcmd),
                EngineCommand::RebuildGraph => {
                    self.needs_rebuild = true;
                }
                EngineCommand::RefreshLiveMidi => self.refresh_live_midi(),
            }
        }
    }

    fn process_transport(&mut self, cmd: TransportCommand) {
        use std::sync::atomic::Ordering::Relaxed;
        match cmd {
            TransportCommand::Play => {
                if self.transport.wait_for_input.load(Relaxed)
                    && !self.transport.playing.load(Relaxed)
                {
                    // Park: process() starts playback on the first MIDI event.
                    self.transport.wait_pending.store(true, Relaxed);
                } else {
                    self.transport.playing.store(true, Relaxed);
                }
            }
            TransportCommand::SetWaitForInput(on) => {
                self.transport.wait_for_input.store(on, Relaxed);
                // Disabling while parked honours the earlier Play press.
                if !on && self.transport.wait_pending.swap(false, Relaxed) {
                    self.transport.playing.store(true, Relaxed);
                }
            }
            TransportCommand::Stop => {
                self.transport.wait_pending.store(false, Relaxed);
                if self.audio_prefs.reset_on_transport() {
                    self.reset_nodes();
                }
                let was_playing = self
                    .transport
                    .playing
                    .swap(false, std::sync::atomic::Ordering::Relaxed);
                // Double-stop behavior: if already stopped, reset to loop start (or 0).
                if !was_playing {
                    let loop_start = if self
                        .transport
                        .looping
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        self.transport
                            .loop_start
                            .load(std::sync::atomic::Ordering::Relaxed)
                    } else {
                        0
                    };
                    self.transport.set_position(loop_start);
                }
                self.transport
                    .recording
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::Record => {
                self.transport
                    .recording
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if self.transport.wait_for_input.load(Relaxed)
                    && !self.transport.playing.load(Relaxed)
                {
                    // Armed-and-parked: recording light on, playhead waits
                    // for the performer's first note.
                    self.transport.wait_pending.store(true, Relaxed);
                } else {
                    self.transport.playing.store(true, Relaxed);
                }
            }
            TransportCommand::SetMasterVolume(db) => {
                self.transport
                    .master_volume_db
                    .store(db, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::SetTimeSignature(num, den) => {
                self.transport.time_sig.store(
                    crate::transport::pack_time_sig(num, den),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            TransportCommand::SetPatternMode(on) => {
                self.transport
                    .pattern_mode
                    .store(on, std::sync::atomic::Ordering::Relaxed);
                // The instrument tracks swap between the playlist's notes
                // and the pattern's.
                self.needs_rebuild = true;
            }
            TransportCommand::SetPosition(pos) => {
                self.transport.set_position(pos);
                // A jump leaves whatever was sounding at the old position
                // behind: held instrument voices carry on at the new one,
                // which is the "note stuck on after I moved the playhead"
                // report. The settings panel offers this as a switch and
                // nothing read it, so it never happened either way.
                if self.audio_prefs.reset_on_transport() {
                    self.reset_nodes();
                }
            }
            TransportCommand::SetBpm(bpm) => {
                self.transport
                    .bpm
                    .store(bpm, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::SetLoop(start, end) => {
                self.transport
                    .loop_start
                    .store(start, std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .loop_end
                    .store(end, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::ToggleLoop => {
                let current = self
                    .transport
                    .looping
                    .load(std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .looping
                    .store(!current, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Ask for a graph of the project as it is now.
    ///
    /// With the builder thread running this only asks: the graph is built
    /// there and swapped in by `take_ready_graph` a block or two later.
    /// Without it (an offline render, the first graph) it is built here.
    /// The project and the meter map are only tried, never waited for: a
    /// busy lock leaves `needs_rebuild` set and the next block tries again.
    fn rebuild_graph(&mut self) {
        match &mut self.builder {
            BuildMode::Thread(link) => {
                link.request();
                self.needs_rebuild = false;
            }
            BuildMode::Inline(builder) => {
                let Some(project) = self.project.try_lock() else {
                    self.needs_rebuild = true;
                    return;
                };
                let Some(mut meters) = self.track_meters.try_lock() else {
                    self.needs_rebuild = true;
                    return;
                };
                let seq = self.graph_seq + 1;
                let prepared = builder.build(&project, &mut meters, seq);
                drop(meters);
                drop(project);
                self.needs_rebuild = false;
                let old = self.install_graph(prepared);
                drop(old);
            }
        }
    }

    /// Swap in the newest graph the builder thread finished, if there is
    /// one, and send the one it replaces back to be freed there.
    fn take_ready_graph(&mut self) {
        let BuildMode::Thread(link) = &mut self.builder else {
            return;
        };
        let Some(prepared) = link.take_ready() else {
            return;
        };
        let old = self.install_graph(prepared);
        if let BuildMode::Thread(link) = &mut self.builder {
            link.throw_away(old);
        }
    }

    /// Whether a graph that was asked for has not been swapped in yet. A
    /// plug-in for a track with no node waits for it.
    fn graph_pending(&self) -> bool {
        self.needs_rebuild
            || matches!(&self.builder, BuildMode::Thread(link) if link.requested() > self.graph_seq)
    }

    /// Put a finished graph in place of the current one. Runs on the audio
    /// thread, so it only moves things: the plug-in chains, the keys held
    /// and the sound of each node (`AudioNode::carry_on_from`), and the
    /// audio in the compensation lines go over to the new graph; what was
    /// built for it already is all there. Nothing is allocated or freed:
    /// the returned box holds the old graph, to be freed elsewhere.
    fn install_graph(&mut self, mut p: Box<PreparedGraph>) -> Box<PreparedGraph> {
        std::mem::swap(&mut self.graph, &mut p.graph);
        std::mem::swap(&mut self.master_id, &mut p.master_id);
        std::mem::swap(&mut self.track_id_to_node, &mut p.track_id_to_node);
        std::mem::swap(&mut self.send_edges, &mut p.send_edges);
        std::mem::swap(&mut self.monitor_edges, &mut p.monitor_edges);
        std::mem::swap(&mut self.midi_track_nodes, &mut p.midi_track_nodes);
        std::mem::swap(&mut self.parked_chains, &mut p.parked_chains);
        std::mem::swap(&mut self.chain_boards, &mut p.boards);
        std::mem::swap(&mut self.assumed_latency, &mut p.assumed_latency);
        std::mem::swap(&mut self.tempo_map, &mut p.tempo_map);
        self.has_monitored_input = p.has_monitored_input;
        self.graph_seq = self.graph_seq.max(p.seq);
        // From here `self` has the new graph and `p` the old one, apart from
        // the lists that describe the new one (`carries`, `sidechain_flags`).
        if !p.pool_read {
            self.graph.keep_pool_of(&mut p.graph);
        }
        // The new nodes start with their own level and pan.
        self.mod_mix.clear();
        self.mod_mix_prev.clear();

        let old = &mut *p;
        for carry in &old.carries {
            let old_node = old.track_id_to_node.get(carry.track_id.as_str()).copied();
            // The chain: from the node this one replaces, or parked while
            // the track was frozen.
            let mut chain = old_node
                .and_then(|id| old.graph.node_mut(id))
                .and_then(|node| node.take_chain());
            if chain.is_none() {
                chain = old
                    .parked_chains
                    .get_mut(carry.track_id.as_str())
                    .filter(|c| !c.slots.is_empty())
                    .map(std::mem::take);
            }
            if let Some(chain) = chain {
                if carry.kind == (CarryKind::Audio { frozen: true }) {
                    match self.parked_chains.get_mut(carry.track_id.as_str()) {
                        Some(parked) => *parked = chain,
                        // Not expected: the build parks every frozen track.
                        // The chain stays with the old graph and is let go.
                        None => {
                            if let Some(node) = old_node.and_then(|id| old.graph.node_mut(id)) {
                                node.restore_chain(chain);
                            }
                        }
                    }
                } else if let Some(node) = self.graph.node_mut(carry.node) {
                    node.restore_chain(chain);
                }
            }
            // The sound: voices, tails, where the playhead was.
            if let (Some(new_node), Some(old_node)) = (
                self.graph.node_mut(carry.node),
                old_node.and_then(|id| old.graph.node_mut(id)),
            ) {
                new_node.carry_on_from(old_node.as_mut());
            }
        }
        for (node, slot_id, active) in &old.sidechain_flags {
            if let Some(node) = self.graph.node_mut(*node) {
                node.set_slot_sidechain(slot_id, *active);
            }
        }
        self.graph.carry_delays_from(&mut old.graph);

        // Chains left in the old graph belong to tracks that are gone. Their
        // plug-ins go to the graveyard, to be let go on the UI thread; when
        // it is full, what is left goes with the old graph.
        for node in old.graph.iter_nodes_mut() {
            if node.chain_len() == 0 {
                continue;
            }
            if let Some(mut chain) = node.take_chain() {
                bury_chain(&mut self.insert_graveyard_tx, &mut chain);
                // Back in the old node even when empty: its list is freed
                // with the old graph, not here.
                node.restore_chain(chain);
            }
        }
        for chain in old.parked_chains.values_mut() {
            bury_chain(&mut self.insert_graveyard_tx, chain);
        }

        if self.publish_chains() {
            // A chain's delay is not the one the compensation was worked out
            // with (a plug-in changed its latency, or one was added since):
            // the next graph gets it right.
            self.needs_rebuild = true;
        }
        self.graph_latency_samples.store(
            self.graph.planned_latency_samples(),
            std::sync::atomic::Ordering::Relaxed,
        );
        p
    }

    /// Publish each chain's delay and size for the next build. True when a
    /// delay differs from the one the graph's compensation assumed.
    fn publish_chains(&mut self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        let mut differs = false;
        for node_id in 0..self.chain_boards.len() {
            if self.chain_boards[node_id].is_none() {
                continue;
            }
            let Some(node) = self.graph.node_mut(node_id) else {
                continue;
            };
            let latency = node.latency_samples();
            let mut slots = node.chain_len();
            if let Some(parked) = node.track_id().and_then(|id| self.parked_chains.get(id)) {
                slots += parked.slots.len();
            }
            if let Some(board) = &self.chain_boards[node_id] {
                board.latency.store(latency, Relaxed);
                board.slots.store(slots as u32, Relaxed);
            }
            if self.assumed_latency.get(node_id).copied() != Some(latency) {
                differs = true;
            }
        }
        differs
    }

    /// Publish one node's chain after a plug-in command changed it.
    fn publish_chain(&mut self, node_id: crate::graph::NodeId) {
        use std::sync::atomic::Ordering::Relaxed;
        let Some(Some(board)) = self.chain_boards.get(node_id) else {
            return;
        };
        if let Some(node) = self.graph.node_mut(node_id) {
            board.latency.store(node.latency_samples(), Relaxed);
            board.slots.store(node.chain_len() as u32, Relaxed);
        }
    }

    /// Start building graphs on a thread of their own. Playback calls this
    /// once the first graph is in; until then, and when the thread cannot
    /// be started, graphs are built where they are asked for, as before.
    fn attach_builder_thread(&mut self) {
        let BuildMode::Inline(builder) = &self.builder else {
            return;
        };
        match Box::new(builder.as_ref().clone()).spawn(self.graph_seq) {
            Ok(link) => self.builder = BuildMode::Thread(link),
            Err(e) => {
                log::warn!(
                    "graph builder thread did not start ({e}); building on the audio thread"
                );
            }
        }
    }

    /// Set which MIDI tracks hear the keyboard, on the nodes already built.
    ///
    /// Picking a track moves the keyboard to it, and that used to rebuild
    /// the whole graph on this thread: about 14 ms for a new song's 500
    /// tracks, longer than a 512-frame block, so every click on a track was
    /// a click in the sound. This changes one flag per MIDI track.
    fn refresh_live_midi(&mut self) {
        let Some(project) = self.project.try_lock() else {
            self.needs_rebuild = true;
            return;
        };
        let armed_for_input = project.tracks.iter().any(|t| {
            t.armed && t.monitor_input && !matches!(t.kind, hardwave_project::TrackKind::Master)
        });
        let keyboard_track = keyboard_track_ref(&project);
        for (id, node) in &self.midi_track_nodes {
            if let Some(track) = project.tracks.iter().find(|t| &t.id == id) {
                let accepts = hears_keyboard(track, armed_for_input, keyboard_track);
                self.graph.set_accepts_live_midi(*node, accepts);
            }
        }
    }

    /// Reset every node in the graph.
    ///
    /// Clears held instrument voices and the sample-buffer caches, so nothing
    /// from before a stop or a jump is still sounding afterwards. Cheap and
    /// allocation-free: each node's reset only touches state it already owns.
    fn reset_nodes(&mut self) {
        for node in self.graph.iter_nodes_mut() {
            node.reset();
        }
    }

    /// Work out which beats fall inside this block, and fill `click_events`.
    ///
    /// Returns how many events were filled. The scheduling itself lives in
    /// `metronome::schedule_block_clicks`; this is the audio-thread wrapper
    /// around it: it decides whether the click is audible at all and takes the
    /// project without waiting for it.
    fn schedule_clicks(&mut self, num_frames: usize) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        let recording = self.transport.recording.load(Relaxed);
        if !self.metronome.settings().is_audible(recording) {
            return 0;
        }
        // try_lock, never lock: the UI holds the project for a moment when it
        // edits it, and waiting here would be a dropout. Losing a block's
        // clicks costs at most one beat.
        // When it is busy, the tempo map the graph was built with stands in.
        let project = self.project.try_lock();
        let tempo_map = project.as_ref().map_or(&self.tempo_map, |p| &p.tempo_map);
        crate::metronome::schedule_block_clicks(
            tempo_map,
            self.transport.position_samples.load(Relaxed),
            num_frames,
            self.sample_rate as f64,
            &mut self.click_events,
        )
    }
}

/// How long the engine keeps processing with the song stopped after the
/// last note or controller played live: long enough for a release or a
/// reverb tail to sound out.
const LIVE_MIDI_HOLD_SECONDS: u64 = 10;

impl AudioCallback for EngineCallback {
    /// Some drivers hand over a bigger block than the one asked for. The
    /// graph is built for the size asked for, so everything past it came out
    /// as silence, a gap in every block. Such a block is now played as
    /// pieces of the size the graph was built for.
    ///
    /// A block that crosses the loop end is split there too, so the jump
    /// back lands on the exact sample. The whole block used to play on past
    /// the loop end and the next one started at the loop start, so every
    /// pass ran up to a block long, the audio after the loop end was heard,
    /// and a loop drifted against a clock or a Link session.
    fn process(&mut self, output: &mut [f32], num_frames: usize, _num_channels: u16) {
        self.run(output, None, num_frames);
    }

    /// The click goes to `click`, on an output pair of its own, instead of
    /// into the mix.
    fn process_split(&mut self, main: &mut [f32], click: &mut [f32], num_frames: usize) {
        self.run(main, Some(click), num_frames);
    }
}

impl EngineCallback {
    fn run(&mut self, output: &mut [f32], mut click: Option<&mut [f32]>, num_frames: usize) {
        let most = (self.graph.buffer_size_hint() as usize).max(1);
        let mut num_frames = num_frames.min(output.len() / 2);
        if let Some(c) = click.as_deref() {
            num_frames = num_frames.min(c.len() / 2);
        }
        let mut done = 0;
        while done < num_frames {
            let mut n = (num_frames - done).min(most);
            if let Some(to_end) = self.frames_to_loop_end(n) {
                n = to_end;
            }
            let piece = click.as_deref_mut().map(|c| {
                let c = &mut c[done * 2..(done + n) * 2];
                c.fill(0.0);
                c
            });
            let block = &mut output[done * 2..(done + n) * 2];
            self.process_block(block, piece, n);
            self.render_auditions(block, n);
            done += n;
        }
    }

    /// What is played straight to the sound card on top of the song: the
    /// browser's auditions, the test tone and the first run's check voice.
    /// After the master, its tap and the capture path, so none of it prints
    /// into a recording or a bounce, and a muted master or a plug-in on it
    /// cannot hide it. On every block, the stopped and the counting-in ones
    /// too: they are mostly listened to with the song stopped.
    fn render_auditions(&mut self, block: &mut [f32], num_frames: usize) {
        let rate = self.sample_rate as f64;
        self.preview
            .render(block, num_frames, &self.audio_pool, rate);
        self.test_tone.render(block, num_frames, rate);
        self.check_voice.render(block, num_frames, rate);
        // The last thing before the sound card: nothing broken reaches the
        // speakers, whatever produced it (a plug-in, a script, a value from
        // a file or the network).
        let written = (num_frames * 2).min(block.len());
        guard_output(&mut block[..written]);
    }
}

impl EngineCallback {
    /// Frames left before the loop end, when it falls inside the next `n`.
    fn frames_to_loop_end(&self, n: usize) -> Option<usize> {
        use std::sync::atomic::Ordering::Relaxed;
        let t = &self.transport;
        if !t.playing.load(Relaxed) || !t.looping.load(Relaxed) || t.pattern_mode.load(Relaxed) {
            return None;
        }
        let (start, end) = (t.loop_start.load(Relaxed), t.loop_end.load(Relaxed));
        let pos = t.position();
        if end <= start || pos >= end {
            return None;
        }
        let left = (end - pos) as usize;
        (left > 0 && left < n).then_some(left)
    }
}

impl EngineCallback {
    /// One block. With `click`, the metronome and the count-in are rendered
    /// there, for an output pair of their own, and not into the mix.
    fn process_block(
        &mut self,
        output: &mut [f32],
        mut click: Option<&mut [f32]>,
        num_frames: usize,
    ) {
        // Real audio load starts here and is published at the end of the
        // block. Instant::now does not allocate or lock, so it is safe on
        // this thread.
        let block_started = std::time::Instant::now();

        self.process_commands();

        // Count-in. While it runs the song is silent and the playhead does
        // not move: the count happens before the take, not over it. Playback
        // starts from here, on the sample the count ends, rather than from a
        // timeout in the UI that guessed when the clicks had finished.
        {
            use std::sync::atomic::Ordering::Relaxed;
            let remaining = self.transport.count_in_remaining.load(Relaxed);
            if remaining > 0 {
                output[..num_frames * 2].fill(0.0);
                let target: &mut [f32] = match click.as_deref_mut() {
                    Some(c) => c,
                    None => &mut output[..],
                };
                let total = self.transport.count_in_total.load(Relaxed);
                let elapsed = total.saturating_sub(remaining);
                let bpm = self.transport.bpm.load(Relaxed).max(1.0);
                let (beats_per_bar, den) =
                    crate::transport::unpack_time_sig(self.transport.time_sig.load(Relaxed));
                // A beat is a note value, so the denominator sets its length:
                // in 7/8 the count-in clicks eighths.
                let samples_per_beat =
                    60.0 / bpm * self.sample_rate as f64 * 4.0 / den.max(1) as f64;
                self.metronome.render_count_in(
                    target,
                    num_frames,
                    elapsed,
                    samples_per_beat,
                    beats_per_bar,
                    self.sample_rate as f64,
                );

                let left = remaining.saturating_sub(num_frames as u64);
                self.transport.count_in_remaining.store(left, Relaxed);
                if left == 0 {
                    self.transport.count_in_total.store(0, Relaxed);
                    if self.transport.count_in_then_play.swap(false, Relaxed) {
                        self.transport.playing.store(true, Relaxed);
                    }
                }
                self.publish_audio_load(block_started, num_frames);
                return;
            }
        }

        if self.needs_rebuild {
            self.rebuild_graph();
        }
        // A graph the builder thread finished goes in before the plug-in
        // commands, so a plug-in for a new track finds its node.
        self.take_ready_graph();

        // Drain pending plug-in chain commands (Add/Remove/SetParameter
        // /etc) and dispatch each to its owning TrackNode. Done AFTER
        // any pending rebuild so newly-created TrackNodes are reachable
        // and BEFORE graph.process so the chain reflects the latest
        // requested state for this block.
        self.dispatch_insert_commands();

        // Honour any pending plug-in state snapshot request from the UI
        // thread. Done AFTER dispatch so newly-added plug-ins from the
        // same UI tick are captured, and BEFORE graph.process so the
        // returned bytes reflect end-of-prior-block state instead of
        // a half-written block.
        self.service_snapshot_request();

        // Wait-for-input: while parked we must still drain MIDI (the
        // early-silence return below would otherwise starve the check
        // forever). The first event un-parks, starts playback THIS block,
        // and the drained events flow to the graph below so the note that
        // started the transport is heard.
        let parked = self
            .transport
            .wait_pending
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut drained_while_parked = false;
        if parked {
            self.midi_event_scratch.clear();
            if let Some(mgr) = self.midi_input.try_lock() {
                mgr.try_drain_events_into(&mut self.midi_event_scratch);
            }
            drained_while_parked = true;
            if !self.midi_event_scratch.is_empty() {
                self.transport
                    .wait_pending
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .playing
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let mut playing = self.transport.is_playing();
        // Link: with others in the session, play waits for the session to
        // reach the playhead's place in the bar, and the playhead is kept on
        // the session's beat after that. Not during a count-in, which is
        // ours alone.
        {
            use std::sync::atomic::Ordering::Relaxed;
            let counting_in = self.transport.count_in_remaining.load(Relaxed) > 0;
            let (num, den) =
                crate::transport::unpack_time_sig(self.transport.time_sig.load(Relaxed));
            let quantum = num.max(1) as f64 * 4.0 / den.max(1) as f64;
            let sr = self.sample_rate as f64;
            let position = self.transport.position();
            let project = &self.project;
            let action = self.link_follow.block(
                &self.transport.link,
                playing && !counting_in,
                || {
                    project.try_lock().map(|p| {
                        p.tempo_map.samples_to_tick(position, sr) as f64 / hardwave_midi::PPQ as f64
                    })
                },
                quantum,
                self.transport.bpm.load(Relaxed),
                sr,
                num_frames,
            );
            self.transport
                .link_holding
                .store(matches!(action, crate::link::LinkAction::Hold), Relaxed);
            match action {
                crate::link::LinkAction::Keep => {}
                crate::link::LinkAction::Hold => playing = false,
                crate::link::LinkAction::Shift(samples) => {
                    let moved = if samples >= 0 {
                        position.saturating_add(samples as u64)
                    } else {
                        position.saturating_sub(samples.unsigned_abs())
                    };
                    self.transport.set_position(moved);
                }
            }
        }
        // Drain external/injected MIDI events into the per-block scratch.
        // try_lock keeps the audio thread non-blocking — on contention we
        // simply pick up the events on the next block (~5ms later, well
        // below human perception). The graph then forwards this slice
        // only to nodes whose `accepts_live_midi` flag is set during
        // rebuild (armed+monitor_input tracks + MIDI tracks).
        // (Skipped when the wait-for-input check above already drained —
        // clearing here would eat the very note that started playback.)
        if !drained_while_parked {
            self.midi_event_scratch.clear();
            self.midi_aged_scratch.clear();
            if let Some(mgr) = self.midi_input.try_lock() {
                // With ages, so a recorded note goes back to when it was
                // played rather than to the start of the block that noticed
                // it. The graph still gets the plain events.
                mgr.try_drain_events_with_age_into(
                    &mut self.midi_aged_scratch,
                    self.sample_rate as f64,
                );
            }
            for (ev, _) in &self.midi_aged_scratch {
                self.midi_event_scratch.push(*ev);
                // Where each controller was left, for modulation from it.
                if let hardwave_midi::MidiEvent::ControlChange { cc, value, .. } = ev {
                    self.mod_cc[(*cc & 0x7F) as usize] = value.clamp(0.0, 1.0);
                }
            }
        }
        // The first run's check voice hears the keys too (it is silent
        // unless its step is open).
        self.check_voice
            .take_events(&self.midi_event_scratch, self.sample_rate as f64);

        // Push drained events into the rolling capture ring. try_lock
        // keeps the audio thread non-blocking — if the UI is mid-dump
        // we skip this block (events still feed the graph below).
        {
            use std::sync::atomic::Ordering::Relaxed;
            // Notice a loop jump while recording, so captured MIDI can be
            // split per pass the way audio is. Inside a loop the positions
            // repeat, so without this a second pass reads as more notes in
            // the first one.
            let abs = self.transport.position();
            // Only blocks that play count: while Link holds the start, the
            // playhead sits still and then moves back a little to land on
            // the beat, which is not a loop pass.
            if self.transport.recording.load(Relaxed) && playing {
                let previous = self.record_last_pos.swap(abs, Relaxed);
                if previous != u64::MAX && abs < previous {
                    self.record_pass.fetch_add(1, Relaxed);
                }
            }
            if !self.midi_event_scratch.is_empty() {
                if let Some(mut ring) = self.midi_capture_ring.try_lock() {
                    let pass = self.record_pass.load(Relaxed);
                    if self.midi_aged_scratch.len() == self.midi_event_scratch.len() {
                        for (ev, age) in &self.midi_aged_scratch {
                            // Back off the lateness, but never past the block
                            // before this one: a stalled UI thread must not
                            // throw a note into an earlier bar.
                            let late = (*age).min(num_frames as u64);
                            ring.push_timed(
                                abs.saturating_sub(late),
                                self.capture_clock.saturating_sub(late),
                                pass,
                                *ev,
                            );
                        }
                    } else {
                        // Parked-drain path, which has no ages.
                        for ev in &self.midi_event_scratch {
                            ring.push_timed(abs, self.capture_clock, pass, *ev);
                        }
                    }
                }
            }
            self.capture_clock = self.capture_clock.wrapping_add(num_frames as u64);
        }

        // Live MIDI keeps the engine running a while after the last event,
        // so a note played with the song stopped sounds out with its tail.
        // The input used to be read only after the stopped-song shortcut
        // below, so with no track armed a keyboard played nothing while
        // stopped, its notes waited and all sounded at once on play, and
        // capture had nothing to capture.
        // A key still down (or the sustain pedal) keeps it running too: the
        // hold counted from the last event, so a pad held longer than that
        // with the song stopped went silent while the key was down.
        for ev in &self.midi_event_scratch {
            match *ev {
                hardwave_midi::MidiEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                    ..
                } if velocity > 0.0 => {
                    self.live_keys_down[(channel & 0x0F) as usize] |= 1u128 << (note & 0x7F);
                }
                hardwave_midi::MidiEvent::NoteOn { channel, note, .. }
                | hardwave_midi::MidiEvent::NoteOff { channel, note, .. } => {
                    self.live_keys_down[(channel & 0x0F) as usize] &= !(1u128 << (note & 0x7F));
                }
                hardwave_midi::MidiEvent::ControlChange {
                    channel,
                    cc: 64,
                    value,
                    ..
                } => self.live_sustain[(channel & 0x0F) as usize] = value >= 0.5,
                _ => {}
            }
        }
        let held = self.live_keys_down.iter().any(|row| *row != 0)
            || self.live_sustain.iter().any(|down| *down);
        if self.midi_event_scratch.is_empty() && !held {
            self.live_midi_hold = self.live_midi_hold.saturating_sub(num_frames as u64);
        } else {
            self.live_midi_hold = self.sample_rate as u64 * LIVE_MIDI_HOLD_SECONDS;
        }

        // When transport is stopped AND nothing is monitoring live input
        // or being played live, the graph can only produce silence — skip
        // processing to save CPU.
        if !playing && !self.has_monitored_input && self.live_midi_hold == 0 {
            // No block runs, so VST3 and CLAP plug-ins get the values set
            // meanwhile here: a knob turned with the song stopped took
            // effect (and read back) only once play started.
            self.flush_plugin_parameters();
            output.fill(0.0);
            self.park_meters();
            return;
        }
        self.meters_parked = false;

        // Tempo-map following: when the project has multi-entry tempo automation,
        // look up the current BPM for the playhead and push it into the transport
        // atomic so plugins see the right tempo in ProcessContext. try_lock so the
        // audio thread never blocks on a mutating UI command — stale BPM for one
        // block is fine.
        if playing {
            // When the project is busy, the tempo map the graph was built
            // with stands in.
            let project = self.project.try_lock();
            let tempo_map = project.as_ref().map_or(&self.tempo_map, |p| &p.tempo_map);
            if tempo_map.entries.len() > 1 {
                let sr = self.sample_rate as f64;
                let pos_samples = self.transport.position();
                let cur_tick = tempo_map.samples_to_tick(pos_samples, sr);
                let cur_bpm = tempo_map.bpm_at(cur_tick);
                self.transport
                    .bpm
                    .store(cur_bpm, std::sync::atomic::Ordering::Relaxed);
                // The signature followed the same way. Only the tempo was
                // pushed here, so a signature change part-way through a
                // song was stored in the project and then ignored: the
                // click kept accenting the old bar length and plug-ins
                // kept being told the old meter.
                let (num, den) = tempo_map.time_sig_at(cur_tick);
                self.transport.time_sig.store(
                    crate::transport::pack_time_sig(num, den),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
        }

        // The block's position in ticks, for everything downstream that thinks
        // musically: automation curves, and anything that follows. Taken from
        // the project's tempo map when the lock is free, so it is right under
        // a tempo change or a ramp, and from the map the graph was built with
        // otherwise. That was worked out from the current tempo, which came
        // out a hair off even in a one-tempo song: an automated level moved
        // by a rounding step whenever an edit held the project.
        let position_samples = self.transport.position();
        let tempo_now = self
            .transport
            .bpm
            .load(std::sync::atomic::Ordering::Relaxed);
        let position_ticks = match self.project.try_lock() {
            Some(project) => project
                .tempo_map
                .samples_to_tick(position_samples, self.sample_rate as f64),
            None => self
                .tempo_map
                .samples_to_tick(position_samples, self.sample_rate as f64),
        };

        // Monitoring that follows the transport, as on a tape machine: the
        // input while stopped or recording, the track's own audio while
        // playing back.
        if !self.monitor_edges.is_empty() {
            let input_heard = !playing
                || self
                    .transport
                    .recording
                    .load(std::sync::atomic::Ordering::Relaxed);
            for entry in &self.monitor_edges {
                let gain = if !entry.auto || input_heard { 1.0 } else { 0.0 };
                self.graph.set_edge_gain(entry.left_edge, gain);
                self.graph.set_edge_gain(entry.right_edge, gain);
            }
        }

        // Send levels and modulation. Both live in the project, which the
        // audio thread must not wait on, so this runs only when the lock is
        // free, and a busy block leaves every send and knob where it was.
        if let Some(project) = self.project.try_lock() {
            use hardwave_project::automation::AutomationTarget;
            use hardwave_project::modulation::{ModSource, ModTarget};

            // Every route's value this block. A source that keeps running,
            // wired to a knob: automation draws a shape once, this one never
            // stops. A track's level is read from its meter, so it is the
            // level of the block before: a few milliseconds.
            self.mod_values.clear();
            let routes = &project.modulations[..project.modulations.len().min(MAX_MOD_ROUTES)];
            let meters = if routes
                .iter()
                .any(|r| matches!(r.source, ModSource::Envelope { .. }))
            {
                self.track_meters.try_lock()
            } else {
                None
            };
            let block_secs = num_frames as f32 / self.sample_rate as f32;
            for (i, route) in routes.iter().enumerate() {
                let swing = match &route.source {
                    ModSource::Lfo { .. } => route
                        .lfo_swing(position_ticks, tempo_now, hardwave_midi::PPQ)
                        .unwrap_or(0.0),
                    ModSource::Envelope {
                        track_id,
                        release_ms,
                    } => {
                        let env = &mut self.mod_env[i];
                        if let Some(meter) = meters.as_ref().and_then(|m| m.get(track_id)) {
                            use std::sync::atomic::Ordering::Relaxed;
                            let db = meter
                                .peak_db_l
                                .load(Relaxed)
                                .max(meter.peak_db_r.load(Relaxed));
                            let level = 10.0_f32.powf(db / 20.0).min(1.0);
                            // Up at once with the track, down over the
                            // release: a follower, as a compressor has.
                            let fall = (-block_secs * 1000.0 / release_ms.max(1.0) as f32).exp();
                            *env = level.max(*env * fall);
                        }
                        *env as f64 * 2.0 - 1.0
                    }
                    ModSource::MidiCc { cc } => {
                        self.mod_cc[(*cc & 0x7F) as usize] as f64 * 2.0 - 1.0
                    }
                };
                self.mod_values.push(if route.enabled {
                    route.value_with(swing)
                } else {
                    f64::NAN
                });
            }
            drop(meters);

            // Sends are graph edges with a gain baked at rebuild, so a
            // SendLevel lane had nowhere to land: it was stored, drawn in
            // the lane editor, and never applied. A lane sets the level each
            // block, and a route multiplies whatever the lane or the mixer
            // says.
            for entry in &mut self.send_edges {
                let Some(track) = project.tracks.iter().find(|t| t.id == entry.track_id) else {
                    continue;
                };
                // A lane without points drives nothing, as on the fader: it
                // read as half-way, so adding an empty send lane dropped
                // the send to -27 dB.
                let lane = track.automation_lanes.iter().find(|l| {
                    l.visible
                        && !l.points.is_empty()
                        && matches!(
                            l.target,
                            AutomationTarget::SendLevel { send_index } if send_index == entry.send_index
                        )
                });
                let mut gain = match lane {
                    // Lane values are 0..1 across the send's own range, the
                    // same shape the mixer's send control uses.
                    Some(lane) => {
                        let db = lane.denormalized_value_at(position_ticks, -60.0, 6.0);
                        if db <= -60.0 {
                            0.0
                        } else {
                            10.0_f64.powf(db / 20.0) as f32
                        }
                    }
                    None => entry.base_gain,
                };
                let mut modulated = false;
                for (route, &value) in routes.iter().zip(&self.mod_values) {
                    if !value.is_nan()
                        && route.track_id == entry.track_id
                        && matches!(
                            route.target,
                            ModTarget::SendLevel { send_index } if send_index == entry.send_index
                        )
                    {
                        gain *= value as f32;
                        modulated = true;
                    }
                }
                if lane.is_none() && !modulated && !entry.modulated {
                    continue;
                }
                entry.modulated = modulated;
                self.graph.set_edge_gain(entry.left_edge, gain);
                self.graph.set_edge_gain(entry.right_edge, gain);
            }

            // Plug-in knobs are set to the value; a track's level and pan
            // are moved on top of the fader and pan knob, which their
            // automation writes every block. All routes onto one node are
            // added up first: levels multiply, pans add.
            std::mem::swap(&mut self.mod_mix, &mut self.mod_mix_prev);
            self.mod_mix.clear();
            for (route, &value) in routes.iter().zip(&self.mod_values) {
                if value.is_nan() {
                    continue;
                }
                let Some(&node_id) = self.track_id_to_node.get(&route.track_id) else {
                    continue;
                };
                let (gain, pan) = match &route.target {
                    ModTarget::PluginParam { slot_id, param_id } => {
                        if let Some(node) = self.graph.node_mut(node_id) {
                            node.set_chain_parameter(slot_id, *param_id, value);
                        }
                        continue;
                    }
                    ModTarget::SendLevel { .. } => continue,
                    ModTarget::TrackVolume => (value as f32, 0.0),
                    ModTarget::TrackPan => (1.0, (value as f32 - 0.5) * 2.0),
                };
                match self.mod_mix.iter_mut().find(|m| m.0 == node_id) {
                    Some(m) => {
                        m.1 *= gain;
                        m.2 += pan;
                    }
                    None => self.mod_mix.push((node_id, gain, pan)),
                }
            }
            for &(node_id, _, _) in &self.mod_mix_prev {
                if !self.mod_mix.iter().any(|m| m.0 == node_id) {
                    if let Some(node) = self.graph.node_mut(node_id) {
                        node.set_mod_mix(1.0, 0.0);
                    }
                }
            }
            for &(node_id, gain, pan) in &self.mod_mix {
                if let Some(node) = self.graph.node_mut(node_id) {
                    node.set_mod_mix(gain, pan);
                }
            }
        }

        let ctx = ProcessContext {
            sample_rate: self.sample_rate as f64,
            buffer_size: num_frames as u32,
            position_ticks,
            tempo: self
                .transport
                .bpm
                .load(std::sync::atomic::Ordering::Relaxed),
            time_sig: crate::transport::unpack_time_sig(
                self.transport
                    .time_sig
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            position_samples: self.transport.position(),
            playing,
        };

        // Process the audio graph
        self.graph.process(&ctx, &self.midi_event_scratch);

        // Pre-zero the output so any early-return / missing master silences
        // the speakers instead of leaking last block's samples.
        output.fill(0.0);

        // Get master output
        if let Some(master_id) = self.master_id {
            if let Some(master_out) = self.graph.node_output(master_id) {
                // Copy to interleaved output
                for frame in 0..num_frames {
                    let l = master_out
                        .first()
                        .and_then(|ch| ch.get(frame))
                        .copied()
                        .unwrap_or(0.0);
                    let r = master_out
                        .get(1)
                        .and_then(|ch| ch.get(frame))
                        .copied()
                        .unwrap_or(0.0);
                    output[frame * 2] = l;
                    output[frame * 2 + 1] = r;
                }

                // Update meters. Deinterleave into pre-sized scratch —
                // clear() keeps capacity, so no allocation after block 1.
                self.meter_left.clear();
                self.meter_right.clear();
                for i in 0..num_frames {
                    self.meter_left.push(output[i * 2]);
                    self.meter_right.push(output[i * 2 + 1]);
                }
                self.meter
                    .process_block(&self.meter_left, &self.meter_right);

                let snapshot = MeterSnapshot {
                    peak_db: self.meter.peak_db(),
                    peak_hold_db: self.meter.peak_hold_db(),
                    true_peak_db: self.meter.true_peak_db(),
                    rms_db: self.meter.rms_db(),
                    lufs_m: self.meter.lufs_m(num_frames),
                    lufs_s: self.meter.lufs_s(num_frames),
                    lufs_i: self.meter.lufs_i(),
                    clipped: self.meter.clipped(),
                    correlation: self.meter.correlation(),
                };
                // Lock-free push; if the consumer is behind, drop the sample.
                let _ = self.meter_producer.push(snapshot);

                // Best-effort push of master samples for UI visualizations.
                // try_lock keeps the audio thread non-blocking — if the UI
                // is mid-snapshot we just drop this block.
                if let Some(mut tap) = self.master_tap.try_lock() {
                    tap.push_block(&output[..num_frames * 2]);
                }
            }
        }

        // The click, last of all.
        //
        // After the master tap and the capture path on purpose: the tap feeds
        // the UI's meters and visualisations and the capture feeds recording,
        // so mixing the click in before them would print the guide track into
        // recordings and bounces. Each beat is placed on its own sample, from
        // the project's tempo map, so the click follows the song's tempo and
        // signature rather than one number that was right at bar 1.
        {
            use std::sync::atomic::Ordering::Relaxed;
            let recording = self.transport.recording.load(Relaxed);
            let events = if playing {
                self.schedule_clicks(num_frames)
            } else {
                // Not playing: no new beats, but a click already sounding is
                // allowed to finish rather than being cut off.
                0
            };
            let target: &mut [f32] = match click {
                Some(c) => c,
                None => &mut output[..],
            };
            self.metronome.render_scheduled(
                target,
                num_frames,
                &self.click_events[..events],
                recording,
                self.sample_rate as f64,
            );
        }

        // Browser auditions and the output guard follow in `run`, for every
        // block: the stopped-song shortcut above returned before them, so a
        // sample clicked in the browser was silent with the song stopped.

        // Advance transport only when playing — input-monitoring alone must
        // not move the playhead.
        if playing {
            self.transport.advance(num_frames as u64);
        }

        self.publish_audio_load(block_started, num_frames);
    }
}

/// Past this the block is not music but a fault: +18 dBFS.
const OUTPUT_CEILING: f32 = 8.0;

/// Make a block safe to send to speakers and headphones.
///
/// A sample that is not a number, or infinite, silences the whole block:
/// some drivers turn NaN into full-scale noise, which is a hearing risk
/// and can damage speakers. Finite samples beyond +18 dBFS are held there;
/// a mix that hot is a fault, and a float export a little over 0 dBFS
/// passes untouched. Returns whether anything had to be changed.
pub fn guard_output(block: &mut [f32]) -> bool {
    if block.iter().any(|s| !s.is_finite()) {
        block.fill(0.0);
        return true;
    }
    let mut changed = false;
    for s in block.iter_mut() {
        if s.abs() > OUTPUT_CEILING {
            *s = s.signum() * OUTPUT_CEILING;
            changed = true;
        }
    }
    changed
}

/// The MIDI track a keyboard plays when nothing is armed: the one picked
/// last, if it is still a MIDI track, else the first MIDI track.
pub fn keyboard_track_of(project: &hardwave_project::Project) -> Option<String> {
    keyboard_track_ref(project).map(str::to_string)
}

/// A track with nothing in it and nothing reaching it: an empty insert of
/// a new song, or a bus or return nothing plays into (an FL insert that
/// has a name and no effects). It makes no sound, so the graph leaves it
/// out.
/// Send a chain's plug-ins to the graveyard, to be let go of off the audio
/// thread. What the graveyard has no room for stays in the chain.
fn bury_chain(
    graveyard: &mut crate::insert_chain::PluginGraveyardSender,
    chain: &mut crate::insert_chain::InsertChain,
) {
    while let Some(slot) = chain.slots.pop() {
        if let Err(slot) = graveyard.try_bury(slot) {
            chain.slots.push(slot);
            log::warn!(
                "plug-in graveyard full; the rest of a deleted track's chain is let go later"
            );
            break;
        }
    }
}

fn is_placeholder(
    track: &hardwave_project::Track,
    reached: &std::collections::HashSet<&str>,
) -> bool {
    matches!(
        track.kind,
        hardwave_project::TrackKind::Audio
            | hardwave_project::TrackKind::Bus
            | hardwave_project::TrackKind::Return
    ) && track.clips.is_empty()
        && track.inserts.is_empty()
        && track.automation_lanes.is_empty()
        && track.automation_clips.is_empty()
        && track.session_slots.iter().all(Option::is_none)
        && !track.armed
        && !track.frozen
        && !reached.contains(track.id.as_str())
}

/// `keyboard_track_of` without making a String, for the audio thread.
fn keyboard_track_ref(project: &hardwave_project::Project) -> Option<&str> {
    let is_midi = |id: &str| {
        project
            .tracks
            .iter()
            .any(|t| t.id == id && matches!(t.kind, hardwave_project::TrackKind::Midi))
    };
    project
        .keyboard_track
        .as_deref()
        .filter(|id| is_midi(id))
        .or_else(|| {
            project
                .tracks
                .iter()
                .find(|t| matches!(t.kind, hardwave_project::TrackKind::Midi))
                .map(|t| t.id.as_str())
        })
}

/// Whether a MIDI track's node hears the keyboard: armed with its input on,
/// or, with no track armed, the track picked last.
fn hears_keyboard(
    track: &hardwave_project::Track,
    armed_for_input: bool,
    keyboard_track: Option<&str>,
) -> bool {
    (track.armed && track.monitor_input)
        || (!armed_for_input && keyboard_track == Some(track.id.as_str()))
}

#[cfg(test)]
mod keyboard_track_tests {
    use super::keyboard_track_of;
    use hardwave_project::{Project, Track, TrackKind};

    fn song() -> Project {
        let mut p = Project::default();
        let mut a = Track::new_audio("synth-a".into(), "Lead".into());
        a.kind = TrackKind::Midi;
        let mut b = Track::new_audio("synth-b".into(), "Bass".into());
        b.kind = TrackKind::Midi;
        p.tracks.push(a);
        p.tracks.push(b);
        p
    }

    #[test]
    fn the_keyboard_plays_the_track_picked_last() {
        let mut p = song();
        p.keyboard_track = Some("synth-b".into());
        assert_eq!(keyboard_track_of(&p).as_deref(), Some("synth-b"));
    }

    #[test]
    fn with_nothing_picked_it_plays_the_first_midi_track_only() {
        let p = song();
        assert_eq!(keyboard_track_of(&p).as_deref(), Some("synth-a"));
    }

    #[test]
    fn a_pick_that_is_gone_or_not_midi_falls_back() {
        let mut p = song();
        p.keyboard_track = Some("insert-001".into());
        assert_eq!(keyboard_track_of(&p).as_deref(), Some("synth-a"));
        p.keyboard_track = Some("deleted".into());
        assert_eq!(keyboard_track_of(&p).as_deref(), Some("synth-a"));
    }
}

#[cfg(test)]
mod output_guard_tests {
    use super::guard_output;

    #[test]
    fn a_broken_sample_silences_the_block_instead_of_reaching_the_speakers() {
        let mut block = vec![0.5, -0.5, f32::NAN, 0.1];
        assert!(guard_output(&mut block));
        assert!(block.iter().all(|s| *s == 0.0));
        let mut block = vec![0.5, f32::INFINITY];
        assert!(guard_output(&mut block));
        assert!(block.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_runaway_level_is_held_and_ordinary_audio_is_untouched() {
        let mut block = vec![1.2, -0.9, 1e30, -1e30];
        assert!(guard_output(&mut block));
        assert_eq!(block, vec![1.2, -0.9, 8.0, -8.0]);
        let mut fine = vec![0.99, -1.05, 0.0];
        assert!(!guard_output(&mut fine));
        assert_eq!(fine, vec![0.99, -1.05, 0.0]);
    }
}

#[cfg(test)]
mod offline_insert_tests {
    //! Proves the export path applies a track's insert FX. Before the
    //! offline-chain-hydration fix, `render_offline_with` rendered dry
    //! (no inserts). The A/B here: a track that makes sound (KickSynth)
    //! plus a "silencer" insert renders SILENT when a plugin factory is
    //! supplied, and AUDIBLE when it isn't.
    use super::*;
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{
        HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
    };
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};
    use hardwave_project::track::PluginSlot;

    /// Minimal insert that zeroes its output.
    struct Silencer {
        desc: PluginDescriptor,
    }
    impl HostedPlugin for Silencer {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            for o in outputs.iter_mut() {
                o.clear();
                o.resize(num_samples, 0.0);
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            0.0
        }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    fn silencer_desc() -> PluginDescriptor {
        PluginDescriptor {
            id: "test.silencer".into(),
            name: "Silencer".into(),
            vendor: "t".into(),
            version: "1".into(),
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from("<native>"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    fn build_engine_with_silenced_sine() -> DawEngine {
        let engine = DawEngine::new();
        let sr = 48_000u32;
        // A 1s stereo sine in the audio pool.
        let frames = sr as usize;
        let mut ch = Vec::with_capacity(frames);
        for n in 0..frames {
            let t = n as f32 / sr as f32;
            ch.push((std::f32::consts::TAU * 220.0 * t).sin() * 0.5);
        }
        let buffer = crate::AudioBuffer {
            channels: vec![ch.clone(), ch],
            sample_rate: sr,
            num_frames: frames,
        };
        engine.audio_pool.insert("sine".to_string(), buffer);

        let mut project = engine.project.lock();
        let id = project.add_audio_track("Sine".into());
        if let Some(t) = project.track_mut(&id) {
            t.clips.push(ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-sine".into(),
                    name: "sine".into(),
                    source_path: "sine".into(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: frames as u64,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.0,
                    warp_markers: Vec::new(),
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                    // Legacy shape on purpose: empty means the engine falls back
                    // to reading source_path as the file location.
                    source_file: String::new(),
                    treatment: Default::default(),
                    tuned_notes: Vec::new(),
                }),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
                color: None,
                row: None,
            });
            t.inserts.push(PluginSlot {
                id: "slot1".into(),
                plugin_id: "test.silencer".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
        drop(project);
        engine
    }

    /// Delays its input by LAT frames and says so, as a lookahead limiter
    /// does; at half level, so the render shows it was in the path.
    const LAT: usize = 512;
    struct Latent {
        desc: PluginDescriptor,
        hist: [Vec<f32>; 2],
    }
    impl HostedPlugin for Latent {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            for ch in 0..2 {
                let inp: &[f32] = inputs.get(ch).copied().unwrap_or(&[]);
                let hist = &mut self.hist[ch];
                let Some(out) = outputs.get_mut(ch) else {
                    continue;
                };
                out.clear();
                for i in 0..num_samples {
                    hist.push(inp.get(i).copied().unwrap_or(0.0));
                    let n = hist.len();
                    out.push(if n > LAT {
                        hist[n - 1 - LAT] * 0.5
                    } else {
                        0.0
                    });
                }
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            0.0
        }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            LAT as u32
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    #[test]
    fn an_audio_clip_on_a_midi_channel_is_heard() {
        // A consolidated kick lands on its MIDI channel as an audio clip; it
        // played nothing (2026-10-14, the starter song's Kick Intro). It
        // plays now, level with a clip on an audio track: the sine at half
        // level through the track's pan law, nothing more.
        use hardwave_project::track::NativeInstrument;
        let engine = DawEngine::new();
        let sr = 48_000u32;
        let frames = sr as usize;
        let ch: Vec<f32> = (0..frames)
            .map(|n| (std::f32::consts::TAU * 220.0 * n as f32 / sr as f32).sin() * 0.5)
            .collect();
        engine.audio_pool.insert(
            "sine".to_string(),
            crate::AudioBuffer {
                channels: vec![ch.clone(), ch],
                sample_rate: sr,
                num_frames: frames,
            },
        );
        {
            let mut project = engine.project.lock();
            let id = project.add_midi_track("Kick".into());
            if let Some(t) = project.track_mut(&id) {
                t.instrument = NativeInstrument::KickSynth;
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(AudioClip {
                        id: "part".into(),
                        name: "Kick consolidated".into(),
                        source_path: "sine".into(),
                        source_hash: String::new(),
                        source_start: 0,
                        source_end: frames as u64,
                        gain_db: 0.0,
                        fade_in_ticks: 0,
                        fade_out_ticks: 0,
                        muted: false,
                        reversed: false,
                        pitch_semitones: 0.0,
                        stretch_ratio: 1.0,
                        warp_markers: Vec::new(),
                        fade_in_curve: FadeCurve::Linear,
                        fade_out_curve: FadeCurve::Linear,
                        source_file: String::new(),
                        treatment: Default::default(),
                        tuned_notes: Vec::new(),
                    }),
                    track_id: id.clone(),
                    position_ticks: 0,
                    length_ticks: 1920,
                    lane: 0,
                    color: None,
                    row: None,
                });
            }
        }
        let mut out: Vec<f32> = Vec::new();
        engine
            .render_offline_with(
                48_000,
                12_000,
                0,
                None,
                |_| {},
                |b| {
                    out.extend_from_slice(b);
                    true
                },
            )
            .expect("it renders");
        let peak = out[2048..].iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(
            (0.33..0.38).contains(&peak),
            "the clip at half level through one pan law, peak {peak}"
        );
    }

    #[test]
    fn a_fade_after_a_tempo_change_lasts_its_beats_at_that_tempo() {
        use hardwave_project::tempo::{TempoEntry, TempoRamp};
        let engine = DawEngine::new();
        let sr = 48_000u32;
        let frames = 4 * sr as usize;
        let ch: Vec<f32> = (0..frames)
            .map(|n| (std::f32::consts::TAU * 1_000.0 * n as f32 / sr as f32).sin() * 0.5)
            .collect();
        engine.audio_pool.insert(
            "sine".to_string(),
            crate::AudioBuffer {
                channels: vec![ch.clone(), ch],
                sample_rate: sr,
                num_frames: frames,
            },
        );
        let beat = hardwave_midi::PPQ;
        let clip_start = {
            let mut project = engine.project.lock();
            // 140 at the start, half that from the second bar on.
            project.tempo_map.entries[0].bpm = 140.0;
            project.tempo_map.entries.push(TempoEntry {
                tick: 4 * beat,
                bpm: 70.0,
                time_sig_num: 4,
                time_sig_den: 4,
                ramp: TempoRamp::Instant,
            });
            let id = project.add_audio_track("Pad".into());
            if let Some(t) = project.track_mut(&id) {
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(AudioClip {
                        id: "pad".into(),
                        name: "pad".into(),
                        source_path: "sine".into(),
                        source_hash: String::new(),
                        source_start: 0,
                        source_end: frames as u64,
                        gain_db: 0.0,
                        // One beat: 60 / 70 s here, not 60 / 140 s.
                        fade_in_ticks: beat,
                        fade_out_ticks: 0,
                        muted: false,
                        reversed: false,
                        pitch_semitones: 0.0,
                        stretch_ratio: 1.0,
                        warp_markers: Vec::new(),
                        fade_in_curve: FadeCurve::Linear,
                        fade_out_curve: FadeCurve::Linear,
                        source_file: String::new(),
                        treatment: Default::default(),
                        tuned_notes: Vec::new(),
                    }),
                    track_id: id.clone(),
                    position_ticks: 8 * beat,
                    length_ticks: 4 * beat,
                    lane: 0,
                    color: None,
                    row: None,
                });
            }
            project.tempo_map.tick_to_samples(8 * beat, sr as f64)
        };
        let mut out: Vec<f32> = Vec::new();
        engine
            .render_offline_with(
                sr,
                sr as u64 * 2,
                clip_start,
                None,
                |_| {},
                |b| {
                    out.extend_from_slice(b);
                    true
                },
            )
            .expect("it renders");
        // Interleaved stereo: the peak of the left channel over a stretch.
        let peak = |from: usize, to: usize| {
            out[from * 2..to * 2]
                .iter()
                .step_by(2)
                .fold(0.0f32, |m, &v| m.max(v.abs()))
        };
        let beat_here = 60.0 / 70.0 * sr as f64;
        let full = peak(60_000, 70_000);
        // Half a beat at 70 into the clip: halfway up the fade. Counted at
        // 140 the fade was over by then and this was at full level.
        let mid = (beat_here / 2.0) as usize;
        let halfway = peak(mid - 480, mid + 480);
        assert!(full > 0.2, "the clip plays, peak {full}");
        assert!(
            (0.4..0.6).contains(&(halfway / full)),
            "halfway up the fade: {halfway} of {full}"
        );
    }

    /// Peak of a render, in windows of `window` frames, for a kick track
    /// holding one MIDI clip at the song start.
    fn kick_clip_peaks(notes: &[u64], clip_len: u64, frames: u64, window: usize) -> Vec<f32> {
        clip_peaks(
            NativeInstrumentChoice::Kick,
            notes,
            240,
            (clip_len, clip_len),
            frames,
            window,
        )
    }

    #[derive(Clone, Copy)]
    enum NativeInstrumentChoice {
        Kick,
        Sine,
    }

    /// Peak of a render in windows of `window` frames: one MIDI track, one
    /// clip at the song start holding `notes` of `note_len` ticks, with the
    /// clip's own length and its length on the playlist given separately
    /// (`lengths` = (content, playlist)).
    fn clip_peaks(
        instrument: NativeInstrumentChoice,
        notes: &[u64],
        note_len: u64,
        lengths: (u64, u64),
        frames: u64,
        window: usize,
    ) -> Vec<f32> {
        use hardwave_midi::{MidiClip, MidiNote};
        use hardwave_project::clip::MidiClipRef;
        use hardwave_project::track::NativeInstrument;
        let engine = DawEngine::new();
        {
            let mut project = engine.project.lock();
            let id = project.add_midi_track("Notes".into());
            let mut clip = MidiClip::new("c".into(), "notes".into(), lengths.0);
            for &start in notes {
                clip.notes.push(MidiNote {
                    start_tick: start,
                    duration_ticks: note_len,
                    pitch: 36,
                    velocity: 1.0,
                    ..Default::default()
                });
            }
            if let Some(t) = project.track_mut(&id) {
                t.instrument = match instrument {
                    NativeInstrumentChoice::Kick => NativeInstrument::KickSynth,
                    NativeInstrumentChoice::Sine => NativeInstrument::BuiltinSine,
                };
                t.clips.push(ClipPlacement {
                    content: ClipContent::Midi(MidiClipRef {
                        id: "c".into(),
                        clip,
                    }),
                    track_id: id.clone(),
                    position_ticks: 0,
                    length_ticks: lengths.1,
                    lane: 0,
                    color: None,
                    row: None,
                });
            }
        }
        let mut out: Vec<f32> = Vec::new();
        engine
            .render_offline_with(
                48_000,
                frames,
                0,
                None,
                |_| {},
                |b| {
                    out.extend_from_slice(b);
                    true
                },
            )
            .expect("it renders");
        out.chunks(window * 2)
            .map(|w| w.iter().fold(0.0f32, |m, v| m.max(v.abs())))
            .collect()
    }

    #[test]
    fn a_note_after_the_end_of_a_shortened_midi_clip_is_not_heard() {
        // A bar of clip shrunk on the playlist to one beat; kicks at beat
        // one and beat three. The second kick is past the end of the clip
        // and has to be cut, as it is for audio.
        let beat = hardwave_project::TempoMap::default().tick_to_samples(960, 48_000.0) as usize;
        let peaks = clip_peaks(
            NativeInstrumentChoice::Kick,
            &[0, 1920],
            240,
            (3840, 960),
            (4 * beat) as u64,
            beat,
        );
        assert!(peaks[0] > 0.1, "the first kick sounds: {peaks:?}");
        assert!(peaks[2] < 0.01, "the kick past the end is cut: {peaks:?}");
    }

    #[test]
    fn a_note_held_over_the_end_of_a_shortened_midi_clip_is_let_go_there() {
        // A sine held for a whole bar in a clip shrunk to one beat.
        let beat = hardwave_project::TempoMap::default().tick_to_samples(960, 48_000.0) as usize;
        let window = beat / 8;
        let peaks = clip_peaks(
            NativeInstrumentChoice::Sine,
            &[0],
            3840,
            (3840, 960),
            (2 * beat) as u64,
            window,
        );
        assert!(peaks[4] > 0.05, "it sounds inside the clip: {peaks:?}");
        // A beat is eight windows. Held, the sine stays at its level; let go
        // at the clip's end, it fades out with its release.
        let held = peaks[6];
        assert!(
            peaks[12] < held / 3.0 && peaks[15] < held / 10.0,
            "it is let go where the clip ends: {peaks:?}"
        );
    }

    #[test]
    fn a_clip_as_long_as_its_content_still_plays_notes_drawn_past_its_end() {
        // Songs made before clips were cut at their end: a note drawn past
        // the end of an untouched clip played, and still does.
        let beat = hardwave_project::TempoMap::default().tick_to_samples(960, 48_000.0) as usize;
        let peaks = kick_clip_peaks(&[0, 1920], 960, (4 * beat) as u64, beat);
        assert!(
            peaks[2] > 0.1,
            "the kick past the end still sounds: {peaks:?}"
        );
    }

    #[test]
    fn a_kick_at_the_first_tick_sounds_from_the_first_frames() {
        // The starter song's kick sits on the first tick. Consolidated on
        // the laptop (2026-10-14) its file opened with 21 ms of nothing:
        // one 1024-frame block. A render has to carry the kick from its
        // first frames.
        use hardwave_midi::{MidiClip, MidiNote};
        use hardwave_project::clip::MidiClipRef;
        use hardwave_project::track::NativeInstrument;
        let engine = DawEngine::new();
        {
            let mut project = engine.project.lock();
            let id = project.add_midi_track("Kick".into());
            if let Some(t) = project.track_mut(&id) {
                t.instrument = NativeInstrument::KickSynth;
            }
            let mut clip = MidiClip::new("c".into(), "kick".into(), 1920);
            clip.notes.push(MidiNote {
                start_tick: 0,
                duration_ticks: 480,
                pitch: 36,
                velocity: 1.0,
                channel: 0,
                muted: false,
                ..Default::default()
            });
            if let Some(t) = project.track_mut(&id) {
                t.clips.push(ClipPlacement {
                    content: ClipContent::Midi(MidiClipRef {
                        id: "c".into(),
                        clip,
                    }),
                    track_id: id.clone(),
                    position_ticks: 0,
                    length_ticks: 1920,
                    lane: 0,
                    color: None,
                    row: None,
                });
            }
        }
        let mut out: Vec<f32> = Vec::new();
        engine
            .render_offline_with(
                48_000,
                24_000,
                0,
                None,
                |_| {},
                |b| {
                    out.extend_from_slice(b);
                    true
                },
            )
            .expect("it renders");
        let first = out.iter().position(|v| v.abs() > 0.02).map(|i| i / 2);
        assert!(
            matches!(first, Some(i) if i < 64),
            "the kick sounds within the first frames; first loud frame {first:?}"
        );
    }

    #[test]
    fn a_render_starts_where_the_song_does_behind_a_plug_in_s_latency() {
        // The sine through a plug-in that holds it back LAT frames. The
        // render drops that much and renders that much more: the first
        // frames of the file carry the sine, and the file is as long as
        // asked. It used to open with LAT frames of silence.
        let engine = build_engine_with_silenced_sine();
        {
            let mut project = engine.project.lock();
            for track in &mut project.tracks {
                for slot in &mut track.inserts {
                    slot.plugin_id = "test.latent".into();
                }
            }
        }
        let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
            (id == "test.latent").then(|| {
                Box::new(Latent {
                    desc: PluginDescriptor {
                        id: "test.latent".into(),
                        name: "Latent".into(),
                        vendor: "t".into(),
                        version: "1".into(),
                        format: PluginFormat::Clap,
                        path: std::path::PathBuf::from("<native>"),
                        category: PluginCategory::Effect,
                        num_inputs: 2,
                        num_outputs: 2,
                        has_midi_input: false,
                        has_editor: false,
                    },
                    hist: [Vec::new(), Vec::new()],
                }) as Box<dyn HostedPlugin>
            })
        };
        let total = 24_000u64;
        let mut out: Vec<f32> = Vec::new();
        engine
            .render_offline_with(
                48_000,
                total,
                0,
                Some(&factory),
                |_| {},
                |block| {
                    out.extend_from_slice(block);
                    true
                },
            )
            .expect("it renders");
        assert_eq!(out.len() as u64, total * 2, "as many frames as asked");
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        let head = peak(&out[..LAT * 2]);
        let all = peak(&out);
        // At half level (and -3 dB from the pan law) the plug-in was in the
        // path; from the first frames, its latency was taken out.
        assert!(
            (0.15..0.2).contains(&all),
            "the sine at half level through the plug-in, peak {all}"
        );
        assert!(
            head > 0.15,
            "the sine is heard from the first frames, peak {head}"
        );
    }

    /// The loudest sample of the silenced sine's song rendered dry (no
    /// plug-in factory), after `change` is done to its clip.
    fn dry_peak(change: impl Fn(&mut AudioClip)) -> f32 {
        let engine = build_engine_with_silenced_sine();
        {
            let mut project = engine.project.lock();
            for track in &mut project.tracks {
                for placement in &mut track.clips {
                    if let ClipContent::Audio(clip) = &mut placement.content {
                        change(clip);
                    }
                }
                track.inserts.clear();
            }
        }
        let mut peak = 0.0f32;
        engine
            .render_offline_with(
                48_000,
                24_000,
                0,
                None,
                |_| {},
                |block| {
                    for &s in block {
                        peak = peak.max(s.abs());
                    }
                    true
                },
            )
            .expect("it renders");
        peak
    }
    #[test]
    fn a_normalised_clip_plays_its_peak_at_full_scale() {
        // The sine peaks at 0.5: normalised, it plays twice as loud.
        let plain = dry_peak(|_| {});
        let normalised = dry_peak(|c| c.treatment.normalize = true);
        assert!(plain > 0.1);
        assert!(
            (normalised / plain - 2.0).abs() < 0.02,
            "{plain} -> {normalised}"
        );
        // Turned down 6 dB first, it still comes up to the same place.
        let quieter = dry_peak(|c| {
            c.treatment.normalize = true;
            c.gain_db = -6.0;
        });
        assert!((quieter / normalised - 0.501).abs() < 0.01);
    }

    #[test]
    fn a_resampled_clip_is_not_baked() {
        let mut clip = AudioClip {
            id: "c".into(),
            name: "c".into(),
            source_path: "s".into(),
            source_hash: String::new(),
            source_start: 0,
            source_end: 10,
            gain_db: 0.0,
            fade_in_ticks: 0,
            fade_out_ticks: 0,
            muted: false,
            reversed: false,
            pitch_semitones: 0.0,
            stretch_ratio: 2.0,
            warp_markers: Vec::new(),
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            source_file: String::new(),
            treatment: Default::default(),
            tuned_notes: Vec::new(),
        };
        assert_eq!(bake_params(&clip), Some((2.0, 0.0)), "stretched: baked");
        clip.treatment.resample = true;
        assert_eq!(bake_params(&clip), None, "like a tape: played at speed");
    }

    /// The CPU meter is only worth having if the number is real, so these
    /// pin the maths it reports.
    #[test]
    fn load_is_the_share_of_the_block_deadline_that_was_used() {
        // 256 frames at 48 kHz is a 5.33 ms deadline.
        let budget = 256.0 / 48_000.0;
        assert_eq!(audio_load_reading(budget / 2.0, budget), 500, "half = 50%");
        assert_eq!(audio_load_reading(budget, budget), 1000, "all of it = 100%");
        assert_eq!(audio_load_reading(0.0, budget), 0);
        // A bigger buffer is a longer deadline, so the same work reads lower:
        // this is why raising the buffer size fixes crackle.
        let roomier = 1024.0 / 48_000.0;
        assert_eq!(audio_load_reading(budget / 2.0, roomier), 125);
        // Never divide by a deadline that does not exist.
        assert_eq!(audio_load_reading(0.01, 0.0), 0);
    }

    #[test]
    fn an_overrun_reads_above_one_hundred_percent_so_it_can_be_counted() {
        let budget = 256.0 / 48_000.0;
        assert!(audio_load_reading(budget * 2.0, budget) > 1000);
    }

    #[test]
    fn the_meter_climbs_quickly_and_settles_slowly() {
        // Rising: within two samples it is most of the way to a spike.
        let first = smooth_load(0, 800);
        let second = smooth_load(first, 800);
        assert!(second >= 600, "too slow to warn: {second}");
        // Falling: it eases down instead of snapping to zero.
        let after = smooth_load(800, 0);
        assert!(after > 600 && after < 800, "got {after}");
    }

    /// Regression for the shape the APP writes, which the test below never
    /// exercised: `import_audio_file` puts the pool id in `source_path` and
    /// the file in `source_file`. Rehydrate used to feed `source_path` to the
    /// filesystem, so it tried to open a file named after a hash and reported
    /// every source missing, leaving reopened projects silent on the very
    /// machine that made them.
    #[test]
    fn a_project_saved_the_way_the_app_writes_it_reloads_its_audio() {
        let dir = std::env::temp_dir().join(format!("hw-import-shape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("snare.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..4_800 {
            w.write_sample(((i as f32 / 8.0).sin() * 6_000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();

        // Exactly what import does: load the file, then store the returned
        // pool id in source_path and the path in source_file.
        let engine = DawEngine::new();
        let (pool_id, _) = engine.load_audio_file(&wav).expect("import");
        {
            let mut project = engine.project.lock();
            let track = project.add_audio_track("Snare".into());
            if let Some(t) = project.track_mut(&track) {
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(AudioClip {
                        id: "clip-1".into(),
                        name: "snare".into(),
                        source_path: pool_id.clone(),
                        source_hash: String::new(),
                        source_start: 0,
                        source_end: 4_800,
                        gain_db: 0.0,
                        fade_in_ticks: 0,
                        fade_out_ticks: 0,
                        muted: false,
                        reversed: false,
                        pitch_semitones: 0.0,
                        stretch_ratio: 1.0,
                        warp_markers: Vec::new(),
                        fade_in_curve: FadeCurve::Linear,
                        fade_out_curve: FadeCurve::Linear,
                        source_file: wav.to_string_lossy().into_owned(),
                        treatment: Default::default(),
                        tuned_notes: Vec::new(),
                    }),
                    track_id: track.clone(),
                    position_ticks: 0,
                    length_ticks: 1920,
                    lane: 0,
                    color: None,
                    row: None,
                });
            }
            let path = dir.join("song.hwp");
            project.save(&path).expect("save");
        }

        // Reopen in a fresh engine, as a restart does.
        let reopened = DawEngine::new();
        {
            let loaded = Project::load(&dir.join("song.hwp")).expect("load");
            *reopened.project.lock() = loaded;
        }
        assert!(
            reopened.audio_pool.get(&pool_id).is_none(),
            "pool starts empty before rehydrate"
        );

        let missing = reopened.rehydrate_audio_pool();

        assert!(missing.is_empty(), "nothing should be missing: {missing:?}");
        // Under the clip's OWN id, which is what playback looks up. Loading it
        // under an id derived from the path would leave the clip silent.
        let buffer = reopened
            .audio_pool
            .get(&pool_id)
            .expect("audio is back under the id the clip references");
        assert_eq!(buffer.num_frames, 4_800);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Regression: `load_project` swaps the Project in but the audio pool
    /// only fills at import time — before `rehydrate_audio_pool`, every
    /// project reopened after an app restart had SILENT audio clips.
    #[test]
    fn rehydrate_audio_pool_reloads_project_sources() {
        // A real WAV on disk, like a user's sample.
        let dir = std::env::temp_dir().join(format!("hw-rehydrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("kick.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..4_800 {
            w.write_sample(((i as f32 / 10.0).sin() * 8_000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
        let path_str = wav.to_string_lossy().to_string();

        // A project referencing that file — one clip on a live track, one in
        // a stored arrangement timeline (both walks must find sources).
        let engine = DawEngine::new();
        {
            let mut project = engine.project.lock();
            let id = project.add_audio_track("Kick".into());
            let clip = |cid: &str| ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: cid.into(),
                    name: "kick".into(),
                    source_path: path_str.clone(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: 4_800,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.0,
                    warp_markers: Vec::new(),
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                    // Legacy shape on purpose: empty means the engine falls back
                    // to reading source_path as the file location.
                    source_file: String::new(),
                    treatment: Default::default(),
                    tuned_notes: Vec::new(),
                }),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
                color: None,
                row: None,
            };
            if let Some(t) = project.track_mut(&id) {
                t.clips.push(clip("clip-live"));
            }
            let mut arrangement = hardwave_project::arrangement::Arrangement {
                id: "arr-b".into(),
                name: "B".into(),
                timelines: Default::default(),
                rows: Vec::new(),
            };
            arrangement.timelines.insert(
                id.clone(),
                hardwave_project::arrangement::TrackTimeline {
                    clips: vec![clip("clip-arr")],
                    automation_clips: vec![],
                },
            );
            project.arrangements.push(arrangement);
        }

        // Pool is empty (fresh engine) — exactly the post-restart state.
        //
        // These clips carry the path in `source_path`, the pre-`source_file`
        // shape, and playback looks a buffer up by `source_path` verbatim. So
        // the pool key that matters here is the path itself. This used to
        // assert `source_id_for_path(path)` instead, a key playback never
        // reads: the audio was loaded under a name no clip could find, so even
        // this shape played silence.
        let source_id = path_str.clone();
        assert!(engine.audio_pool.get(&source_id).is_none());

        let missing = engine.rehydrate_audio_pool();
        assert!(
            missing.is_empty(),
            "unexpected missing sources: {missing:?}"
        );
        let buf = engine
            .audio_pool
            .get(&source_id)
            .expect("source should be back in the pool after rehydrate");
        assert!(buf.num_frames > 0);

        // Missing files are reported, not fatal.
        {
            let mut project = engine.project.lock();
            let id2 = project.add_audio_track("Ghost".into());
            let mut ghost = ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-ghost".into(),
                    name: "ghost".into(),
                    source_path: dir.join("does-not-exist.wav").to_string_lossy().to_string(),
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
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                    // Legacy shape on purpose: empty means the engine falls back
                    // to reading source_path as the file location.
                    source_file: String::new(),
                    treatment: Default::default(),
                    tuned_notes: Vec::new(),
                }),
                track_id: id2.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
                color: None,
                row: None,
            };
            ghost.track_id = id2.clone();
            if let Some(t) = project.track_mut(&id2) {
                t.clips.push(ghost);
            }
        }
        let missing = engine.rehydrate_audio_pool();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("does-not-exist"));

        std::fs::remove_dir_all(&dir).ok();
    }

    type TestPluginFactory<'a> = &'a dyn Fn(&str) -> Option<Box<dyn HostedPlugin>>;

    fn render_peak(engine: &DawEngine, factory: Option<TestPluginFactory<'_>>) -> f32 {
        let sr = 48_000u32;
        let mut peak = 0.0f32;
        engine
            .render_offline_with(
                sr,
                sr as u64,
                0,
                factory,
                |_| {},
                |block| {
                    for &s in block {
                        peak = peak.max(s.abs());
                    }
                    true
                },
            )
            .unwrap();
        peak
    }

    #[test]
    fn offline_render_applies_track_inserts() {
        let engine = build_engine_with_silenced_sine();
        let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
            if id == "test.silencer" {
                Some(Box::new(Silencer {
                    desc: silencer_desc(),
                }))
            } else {
                None
            }
        };
        // With the factory, the silencer insert zeroes the kick.
        let peak_with = render_peak(&engine, Some(&factory));
        assert!(
            peak_with < 1e-6,
            "insert chain must apply offline; silencer should zero output, got peak={peak_with}"
        );
        // Without a factory (old behavior), the insert is skipped and the
        // kick is audible — confirming the difference is the hydration.
        let peak_without = render_peak(&engine, None);
        assert!(
            peak_without > 1e-3,
            "without the factory the source should still sound, got peak={peak_without}"
        );
    }

    /// A key played while an export runs is not in the export, and still
    /// reaches the track being played.
    #[test]
    fn a_key_played_during_an_export_stays_out_of_it() {
        let engine = DawEngine::new();
        engine.project.lock().add_midi_track("Lead".into());
        engine.midi_input.lock().inject(MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 1.0,
        });
        let peak = render_peak(&engine, None);
        assert!(
            peak < 1e-6,
            "the render is the song, not the keyboard: peak {peak}"
        );
        let live = engine.midi_input.lock().drain_events();
        assert_eq!(live.len(), 1, "the key is still there for playback");
    }

    /// A frozen track's plug-ins stay out of an export, as they stay out
    /// of playback: the render it plays already went through them.
    #[test]
    fn an_export_leaves_a_frozen_track_s_plug_ins_out() {
        let engine = build_engine_with_silenced_sine();
        {
            let mut project = engine.project.lock();
            for track in project.tracks.iter_mut().filter(|t| !t.inserts.is_empty()) {
                for clip in track.clips.iter_mut() {
                    clip.lane = hardwave_project::clip::FREEZE_LANE;
                }
                track.frozen = true;
            }
        }
        let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
            (id == "test.silencer").then(|| {
                Box::new(Silencer {
                    desc: silencer_desc(),
                }) as Box<dyn HostedPlugin>
            })
        };
        let peak = render_peak(&engine, Some(&factory));
        assert!(
            peak > 0.1,
            "the frozen render plays, not run through the silencer again: peak {peak}"
        );
    }
}

#[cfg(test)]
mod vca_mix_tests {
    use super::*;

    /// Moving a VCA's fader reaches its members' live mix straight away, as
    /// a track fader does, with no rebuild of the graph; the group's level
    /// is kept apart too, for a member whose own level is automated.
    #[test]
    fn a_vca_move_reaches_its_members_without_a_rebuild() {
        let engine = DawEngine::new();
        {
            let mut project = engine.project.lock();
            let mut kick = hardwave_project::Track::new_audio("kick".into(), "Kick".into());
            kick.volume_db = -6.0;
            project.tracks.push(kick);
            let mut drums = hardwave_project::vca::Vca::new("drums".into(), "Drums".into());
            drums.members.push("kick".into());
            drums.gain_db = -6.0;
            project.vcas.push(drums);
        }
        engine.sync_track_meters();
        engine.apply_vca_mix("drums");
        let meter = engine.track_meters.lock().get("kick").cloned().unwrap();
        let (volume, _, _) = meter.live_mix().unwrap();
        assert!((volume - db_to_linear_f32(-12.0)).abs() < 1e-4, "{volume}");
        assert!((meter.vca_gain() - db_to_linear_f32(-6.0)).abs() < 1e-4);
    }
}

#[cfg(test)]
mod history_group_tests {
    use super::*;

    /// One drag of the paint tool places many clips, each through its own
    /// command. Without grouping that is one undo step per clip, so undoing
    /// the drag means pressing undo as many times as it placed.
    #[test]
    fn a_group_is_one_undo_step() {
        let engine = DawEngine::new();
        assert_eq!(engine.history_sizes().0, 0);

        engine.begin_history_group();
        for _ in 0..20 {
            engine.snapshot_before_mutation();
        }
        engine.end_history_group();

        assert_eq!(engine.history_sizes().0, 1, "twenty clips, one undo step");
    }

    #[test]
    fn mutations_outside_a_group_keep_their_own_steps() {
        let engine = DawEngine::new();
        engine.snapshot_before_mutation();
        engine.snapshot_before_mutation();
        assert_eq!(engine.history_sizes().0, 2);
    }

    #[test]
    fn a_second_group_is_its_own_step() {
        let engine = DawEngine::new();
        for _ in 0..2 {
            engine.begin_history_group();
            engine.snapshot_before_mutation();
            engine.snapshot_before_mutation();
            engine.end_history_group();
        }
        assert_eq!(engine.history_sizes().0, 2, "two drags, two undo steps");
    }

    #[test]
    fn a_repeated_begin_does_not_open_a_second_group() {
        let engine = DawEngine::new();
        engine.begin_history_group();
        engine.snapshot_before_mutation();
        // A stray second begin would otherwise reset the group and let the
        // next mutation snapshot again.
        engine.begin_history_group();
        engine.snapshot_before_mutation();
        engine.end_history_group();
        assert_eq!(engine.history_sizes().0, 1);
        assert!(!engine.history_group_open());
    }

    #[test]
    fn the_group_ends_even_if_nothing_was_placed() {
        let engine = DawEngine::new();
        engine.begin_history_group();
        engine.end_history_group();
        assert_eq!(engine.history_sizes().0, 0, "an empty drag is not history");
        engine.snapshot_before_mutation();
        assert_eq!(engine.history_sizes().0, 1);
    }
}

/// The device audio moved to after a restart, when it moved at all.
fn moved_to(before: Option<String>, now: Option<String>) -> Option<String> {
    if now == before {
        None
    } else {
        Some(now.unwrap_or_else(|| "system default output".into()))
    }
}

#[cfg(test)]
mod device_restart_tests {
    use super::moved_to;

    /// After a sound card would not open, picking another tries to start
    /// audio; it only stored the choice. The outputs asked for here exist
    /// on no machine, so every start fails, and the second error is the
    /// proof that it tried.
    #[test]
    fn a_new_choice_after_a_failed_start_starts_audio() {
        let mut engine = super::DawEngine::new();
        engine.set_channel_offsets(0, 1000);
        assert!(engine.start().is_err(), "no device has a thousand outputs");
        assert!(!engine.is_running());
        assert!(
            engine.set_audio_config(None, 48_000, 256).is_err(),
            "the new choice was tried, not only stored"
        );
    }

    #[test]
    fn a_restart_on_the_same_device_says_nothing() {
        let cable = Some("CABLE Input".to_string());
        assert_eq!(moved_to(cable.clone(), cable), None);
        assert_eq!(moved_to(None, None), None);
    }

    #[test]
    fn a_move_to_another_device_names_it() {
        assert_eq!(
            moved_to(Some("Focusrite USB".into()), Some("Speakers".into())),
            Some("Speakers".to_string())
        );
        assert_eq!(
            moved_to(Some("Focusrite USB".into()), None),
            Some("system default output".to_string())
        );
    }
}

#[cfg(test)]
mod rt_safety_tests {
    //! Proves the audio callback never blocks on UI-held locks.
    //!
    //! Before the try_lock fix, `rebuild_graph()` took a blocking
    //! `project.lock()` on the audio thread — any UI mutation holding
    //! that mutex stalled the callback (audible dropout). Now a
    //! contended rebuild defers to the next block, and insert commands
    //! that target not-yet-built nodes are parked instead of dropped.
    use super::*;

    /// Build a minimal EngineCallback around the given shared project.
    fn make_callback(
        project_arc: Arc<Mutex<Project>>,
    ) -> (EngineCallback, crate::insert_chain::InsertCommandSender) {
        let transport = TransportState::default();
        transport
            .sample_rate
            .store(48_000, std::sync::atomic::Ordering::Relaxed);
        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (_command_tx, command_rx) = bounded::<EngineCommand>(4);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(None));
        let (cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
        let (grave_tx, _grave_rx) = crate::insert_chain::graveyard_channel(8);
        let cb = EngineCallback::new(
            transport,
            project_arc,
            meter_producer,
            command_rx,
            AudioPool::new(),
            track_meters,
            input_consumer,
            crate::input_node::capture_set(),
            master_tap::new_shared(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cmd_rx,
            grave_tx,
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(MidiInputManager::new())),
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            48_000,
            256,
            LoadCounters::new(),
            crate::metronome::MetronomeSettings::silent(),
            crate::preview_player::PreviewRequest::new(),
            crate::audio_prefs::AudioPrefs::new(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            // One thread: these tests are about the callback, not about
            // sharing the work.
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(None)),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(std::sync::atomic::AtomicI64::new(0)),
            Arc::new(crate::session::SessionState::new()),
        );
        (cb, cmd_tx)
    }

    /// A sample clicked in the browser is heard with the song stopped. The
    /// stopped-song shortcut sent silence and returned before the audition
    /// was mixed in, so a preview was only heard while the song played.
    #[test]
    fn a_browser_preview_plays_while_the_song_is_stopped() {
        let (mut cb, _cmd_tx) = make_callback(Arc::new(Mutex::new(Project::default())));
        let request = crate::preview_player::PreviewRequest::new();
        cb.preview = crate::preview_player::PreviewPlayer::new(request.clone());
        cb.audio_pool.insert(
            "kick".into(),
            AudioBuffer {
                channels: vec![vec![0.5; 4_800]; 2],
                sample_rate: 48_000,
                num_frames: 4_800,
            },
        );
        assert!(!cb.transport.is_playing());
        request.play("kick");
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(
            out.iter().any(|s| s.abs() > 0.1),
            "the audition is heard with the song stopped"
        );
    }

    /// The test tone after Apply is heard with the song stopped, on the left
    /// first, straight on the output: nothing reaches the master's tap, so
    /// the meters and scopes (and anything recording from them) never see it.
    #[test]
    fn the_test_tone_plays_on_the_output_and_not_through_the_master() {
        let (mut cb, _cmd_tx) = make_callback(Arc::new(Mutex::new(Project::default())));
        let request = crate::test_tone::TestToneRequest::new();
        cb.test_tone = crate::test_tone::TestTone::new(request.clone());
        // Playing, so the graph runs and the master's tap is fed.
        cb.transport
            .playing
            .store(true, std::sync::atomic::Ordering::Relaxed);
        request.play();
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        let left = out.iter().step_by(2).fold(0.0f32, |m, s| m.max(s.abs()));
        let right = out
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(left > 0.0 && right == 0.0, "left first: {left} {right}");
        let tapped = cb.master_tap.lock().snapshot_interleaved(256);
        assert!(
            tapped.iter().all(|s| *s == 0.0),
            "the master's tap does not have it"
        );

        // With the song stopped as well.
        cb.transport
            .playing
            .store(false, std::sync::atomic::Ordering::Relaxed);
        request.play();
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(out.iter().any(|s| s.abs() > 0.0));
    }

    /// The first run's check voice plays a key with the song stopped and no
    /// instrument in it, and only while its step is open.
    #[test]
    fn the_check_voice_plays_a_key_while_its_step_is_open() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, _cmd_tx) = make_callback(Arc::new(Mutex::new(Project::default())));
        cb.midi_input = Arc::clone(&midi);
        let switch = crate::check_voice::CheckVoiceSwitch::new();
        cb.check_voice = crate::check_voice::CheckVoice::new(switch.clone());
        let key = hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.8,
        };

        midi.lock().inject(key);
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(out.iter().all(|s| *s == 0.0), "off: the key plays nothing");

        switch.set(true);
        midi.lock().inject(key);
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(out.iter().any(|s| s.abs() > 0.01), "on: the key is heard");
    }

    /// An export never has the test tone or the check voice in it, even
    /// while the window has them playing.
    #[test]
    fn an_export_never_has_the_test_tone_in_it() {
        let engine = DawEngine::new();
        engine.check_voice().set(true);
        engine.test_tone().play();
        let mut peak = 0.0f32;
        engine
            .render_offline(48_000, 48_000, |block| {
                peak = block.iter().fold(peak, |m, s| m.max(s.abs()));
                true
            })
            .unwrap();
        assert_eq!(peak, 0.0, "an empty song bounces to silence");
        assert!(engine.test_tone().is_playing(), "still asked for, live");
    }

    #[test]
    fn contended_rebuild_defers_instead_of_blocking() {
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        assert!(!cb.needs_rebuild, "constructor rebuild should have run");

        // UI thread holds the project lock across a callback.
        let guard = project_arc.lock();
        cb.needs_rebuild = true;
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(
            cb.needs_rebuild,
            "rebuild must defer while the project lock is contended"
        );
        drop(guard);

        // Lock released — next block completes the rebuild.
        cb.process(&mut out, 256, 2);
        assert!(!cb.needs_rebuild, "rebuild should run once the lock frees");
    }

    #[test]
    fn meters_fall_to_silence_when_the_transport_stops() {
        use std::sync::atomic::Ordering;
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        let m = Arc::new(crate::track_node::TrackMeterState::default());
        m.peak_db_l.store(-3.0, Ordering::Relaxed);
        m.peak_db_r.store(-1.0, Ordering::Relaxed);
        cb.track_meters.lock().insert("t".into(), Arc::clone(&m));
        assert!(!cb.transport.is_playing());
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert_eq!(m.peak_db_l.load(Ordering::Relaxed), -120.0);
        assert_eq!(m.peak_db_r.load(Ordering::Relaxed), -120.0);
    }

    #[test]
    fn contended_meter_map_also_defers() {
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        let meters = Arc::clone(&cb.track_meters);
        let guard = meters.lock();
        cb.needs_rebuild = true;
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(cb.needs_rebuild, "meter-map contention must defer rebuild");
        drop(guard);
        cb.process(&mut out, 256, 2);
        assert!(!cb.needs_rebuild);
    }

    #[test]
    fn snapshot_request_survives_slot_contention() {
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut out = vec![0.0f32; 512];

        // Block 1: UI holds the slot mutex mid-placement — callback must
        // skip, not block, and the request must NOT be consumed.
        {
            let slot = Arc::clone(&cb.pending_state_snapshot);
            let guard = slot.lock();
            cb.process(&mut out, 256, 2);
            drop(guard);
        }
        *cb.pending_state_snapshot.lock() = Some(tx);
        // Block 2: uncontended — request is serviced.
        cb.process(&mut out, 256, 2);
        assert!(
            rx.try_recv().is_ok(),
            "snapshot request must be serviced on the first uncontended block"
        );
    }

    /// Like a VST3 or CLAP plug-in: a value set is only taken in at the
    /// start of a block (or a flush), and the state reports what was taken.
    struct BlockFed {
        desc: hardwave_plugin_host::types::PluginDescriptor,
        waiting: Option<f64>,
        taken: Arc<std::sync::atomic::AtomicU64>,
    }
    impl BlockFed {
        fn take(&mut self) {
            if let Some(v) = self.waiting.take() {
                self.taken
                    .store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    impl hardwave_plugin_host::types::HostedPlugin for BlockFed {
        fn descriptor(&self) -> &hardwave_plugin_host::types::PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            num_samples: usize,
        ) {
            self.take();
            for (o, i) in outputs.iter_mut().zip(inputs) {
                o.clear();
                o.extend_from_slice(&i[..num_samples.min(i.len())]);
            }
        }
        fn flush_parameters(&mut self) {
            self.take();
        }
        fn get_parameter_count(&self) -> u32 {
            1
        }
        fn get_parameter_info(
            &self,
            _i: u32,
        ) -> Option<hardwave_plugin_host::types::ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            f64::from_bits(self.taken.load(std::sync::atomic::Ordering::Relaxed))
        }
        fn set_parameter_value(&mut self, _i: u32, v: f64) {
            self.waiting = Some(v);
        }
        fn get_state(&self) -> Vec<u8> {
            self.get_parameter_value(0).to_le_bytes().to_vec()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    /// Bug D from the test machine: Drive set to 0.73 on the master's
    /// plug-in with the song stopped read back 0.333 until play. No block
    /// runs while stopped, and VST3 and CLAP plug-ins only take a value in
    /// a block, so the plug-in, and the state read back, kept the old one.
    #[test]
    fn a_parameter_set_with_the_song_stopped_takes_effect_and_reads_back_at_once() {
        use crate::insert_chain::{InsertCommand, LiveSlot};
        use hardwave_plugin_host::types::{
            HostedPlugin, PluginCategory, PluginDescriptor, PluginFormat,
        };
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));
        assert!(!cb.transport.is_playing());
        let taken = Arc::new(std::sync::atomic::AtomicU64::new(0.333f64.to_bits()));
        let plugin: Box<dyn HostedPlugin> = Box::new(BlockFed {
            desc: PluginDescriptor {
                id: "test.block-fed".into(),
                name: "Block Fed".into(),
                vendor: "Hardwave Tests".into(),
                version: "1".into(),
                format: PluginFormat::Vst3,
                path: std::path::PathBuf::from("<test>"),
                category: PluginCategory::Effect,
                num_inputs: 2,
                num_outputs: 2,
                has_midi_input: false,
                has_editor: false,
            },
            waiting: None,
            taken: Arc::clone(&taken),
        });
        let slot = LiveSlot {
            param_ranges: LiveSlot::ranges_of(plugin.as_ref()),
            levels: Default::default(),
            slot_id: "shape".into(),
            plugin,
            enabled: true,
            wet: 1.0,
            sidechain_active: false,
            gain_reduction_db: LiveSlot::new_gain_reduction(),
        };
        let set = |value: f64| InsertCommand::SetParameter {
            track_id: "master".into(),
            slot_id: "shape".into(),
            param_id: 0,
            value,
        };
        cmd_tx
            .try_send(InsertCommand::Add {
                track_id: "master".into(),
                slot,
            })
            .ok()
            .unwrap();
        cmd_tx.try_send(set(0.73)).ok().unwrap();
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        let now = || f64::from_bits(taken.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(
            now(),
            0.73,
            "the plug-in has the value with the song stopped"
        );

        // A read-back asked in the same block as the change has it too.
        cmd_tx.try_send(set(0.5)).ok().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        *cb.pending_state_snapshot.lock() = Some(tx);
        cb.process(&mut out, 256, 2);
        let states = rx.try_recv().expect("the read-back was answered");
        let bytes = states
            .get(&("master".to_string(), "shape".to_string()))
            .expect("the master's plug-in is in it");
        assert_eq!(f64::from_le_bytes(bytes[..8].try_into().unwrap()), 0.5);
    }

    #[test]
    fn a_new_song_takes_the_old_songs_plug_ins_off_shared_track_ids() {
        use crate::insert_chain::{tests::make_gain_slot, InsertCommand};
        // Two songs both have the mixer insert "insert-001" (a fixed id),
        // with a plug-in on it, as the song lists every plug-in it has.
        let with_plugin = |name: &str| {
            let mut p = Project::default();
            p.track_mut("insert-001")
                .unwrap()
                .inserts
                .push(hardwave_project::track::PluginSlot {
                    id: name.into(),
                    plugin_id: "test.gain".into(),
                    enabled: true,
                    state: None,
                    sidechain_source: None,
                    wet: 1.0,
                });
            p
        };
        let project_arc = Arc::new(Mutex::new(with_plugin("old-sampler")));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));
        let shared = "insert-001".to_string();
        assert!(cb.track_id_to_node.contains_key(&shared));
        let slots_on = |cb: &mut EngineCallback, id: &str| -> Vec<String> {
            let node_id = cb.track_id_to_node[id];
            let node = cb.graph.node_mut(node_id).unwrap();
            let chain = node.take_chain();
            let ids = chain
                .as_ref()
                .map(|c| c.slots.iter().map(|s| s.slot_id.clone()).collect())
                .unwrap_or_default();
            if let Some(chain) = chain {
                node.restore_chain(chain);
            }
            ids
        };
        let mut out = vec![0.0f32; 512];

        // The old song's sampler on the shared insert.
        let (old, _) = make_gain_slot("old-sampler", 1.0, true);
        cmd_tx
            .try_send(InsertCommand::Add {
                track_id: shared.clone(),
                slot: old,
            })
            .ok()
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert_eq!(slots_on(&mut cb, &shared), vec!["old-sampler".to_string()]);

        // Opening the new song: clear, swap the song, rebuild, then the new
        // song's own plug-in for the same track id.
        cmd_tx.try_send(InsertCommand::ClearAll).ok().unwrap();
        *project_arc.lock() = with_plugin("new-synth");
        cb.needs_rebuild = true;
        let (new, _) = make_gain_slot("new-synth", 1.0, true);
        cmd_tx
            .try_send(InsertCommand::Add {
                track_id: shared.clone(),
                slot: new,
            })
            .ok()
            .unwrap();
        cb.process(&mut out, 256, 2);
        cb.process(&mut out, 256, 2);
        assert_eq!(
            slots_on(&mut cb, &shared),
            vec!["new-synth".to_string()],
            "the old song's plug-in must not survive into the new song"
        );
    }

    /// Freezing a track parks its plug-ins (they do not run while it plays
    /// the render); unfreezing gives them back. They were dropped, so an
    /// unfrozen track came back without its plug-ins.
    #[test]
    fn unfreezing_a_track_gives_its_plug_ins_back() {
        use crate::insert_chain::{tests::make_gain_slot, InsertCommand};
        let mut project = Project::default();
        let id = project.add_audio_track("Vox".into());
        // In the song too, as adding a plug-in in the app does.
        project
            .track_mut(&id)
            .unwrap()
            .inserts
            .push(hardwave_project::track::PluginSlot {
                id: "fx".into(),
                plugin_id: "test.gain".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        let project_arc = Arc::new(Mutex::new(project));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));
        let mut out = vec![0.0f32; 512];
        let slots_on = |cb: &mut EngineCallback, id: &str| -> Vec<String> {
            let Some(&node_id) = cb.track_id_to_node.get(id) else {
                return Vec::new();
            };
            let node = cb.graph.node_mut(node_id).unwrap();
            let chain = node.take_chain();
            let ids = chain
                .as_ref()
                .map(|c| c.slots.iter().map(|s| s.slot_id.clone()).collect())
                .unwrap_or_default();
            if let Some(chain) = chain {
                node.restore_chain(chain);
            }
            ids
        };
        cb.process(&mut out, 256, 2);
        let (fx, _) = make_gain_slot("fx", 1.0, true);
        cmd_tx
            .try_send(InsertCommand::Add {
                track_id: id.clone(),
                slot: fx,
            })
            .ok()
            .unwrap();
        // An empty track has no node yet: the plug-in asks for one and lands
        // the block after.
        cb.process(&mut out, 256, 2);
        cb.process(&mut out, 256, 2);
        assert_eq!(slots_on(&mut cb, &id), vec!["fx".to_string()]);

        project_arc.lock().track_mut(&id).unwrap().frozen = true;
        cb.needs_rebuild = true;
        cb.process(&mut out, 256, 2);
        assert!(
            slots_on(&mut cb, &id).is_empty(),
            "a frozen track runs no plug-ins"
        );
        assert!(cb.parked_chains.contains_key(&id), "they are kept");

        project_arc.lock().track_mut(&id).unwrap().frozen = false;
        cb.needs_rebuild = true;
        cb.process(&mut out, 256, 2);
        assert_eq!(
            slots_on(&mut cb, &id),
            vec!["fx".to_string()],
            "back after unfreezing"
        );
        assert!(cb.parked_chains.is_empty());
    }

    /// A plug-in on a MIDI track (an instrument, or an effect after the
    /// synth) stays there when the graph is rebuilt, as on an audio track.
    #[test]
    fn a_midi_track_keeps_its_plug_ins_through_a_rebuild() {
        use crate::insert_chain::{tests::make_gain_slot, InsertCommand};
        let mut project = Project::default();
        let id = project.add_midi_track("Lead".into());
        let project_arc = Arc::new(Mutex::new(project));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));
        let mut out = vec![0.0f32; 512];
        let slots_on = |cb: &mut EngineCallback, id: &str| -> Vec<String> {
            let node_id = cb.track_id_to_node[id];
            let node = cb.graph.node_mut(node_id).unwrap();
            let chain = node.take_chain();
            let ids = chain
                .as_ref()
                .map(|c| c.slots.iter().map(|s| s.slot_id.clone()).collect())
                .unwrap_or_default();
            if let Some(chain) = chain {
                node.restore_chain(chain);
            }
            ids
        };
        cb.process(&mut out, 256, 2);
        let (fx, _) = make_gain_slot("fx", 1.0, true);
        cmd_tx
            .try_send(InsertCommand::Add {
                track_id: id.clone(),
                slot: fx,
            })
            .ok()
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert_eq!(slots_on(&mut cb, &id), vec!["fx".to_string()]);

        // Anything that rebuilds the graph: a note, a new track, a route.
        cb.needs_rebuild = true;
        cb.process(&mut out, 256, 2);
        cb.process(&mut out, 256, 2);
        assert_eq!(
            slots_on(&mut cb, &id),
            vec!["fx".to_string()],
            "the MIDI track's chain must come through the rebuild"
        );
    }

    /// A new song's empty inserts have no node; a plug-in put on one asks
    /// for the rebuild that gives it a node, waits for it, and plays.
    #[test]
    fn a_plug_in_put_on_an_empty_insert_gets_a_node_and_plays() {
        use crate::insert_chain::{tests::make_gain_slot, InsertCommand};
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));
        let id = "insert-007".to_string();
        assert!(
            !cb.track_id_to_node.contains_key(&id),
            "an empty insert has no node"
        );
        // The app lists the plug-in in the song, then hands it to the
        // audio thread.
        project_arc.lock().track_mut(&id).unwrap().inserts.push(
            hardwave_project::track::PluginSlot {
                id: "eq".into(),
                plugin_id: "test.gain".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            },
        );
        let (slot, _) = make_gain_slot("eq", 1.0, true);
        cmd_tx
            .try_send(InsertCommand::Add {
                track_id: id.clone(),
                slot,
            })
            .ok()
            .unwrap();
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        cb.process(&mut out, 256, 2);
        let node_id = *cb.track_id_to_node.get(&id).expect("the insert got a node");
        let node = cb.graph.node_mut(node_id).unwrap();
        let chain = node.take_chain().expect("a chain");
        assert_eq!(chain.slots.len(), 1, "the plug-in is on it");
        assert!(cb.deferred_insert_commands.is_empty());
    }

    /// An insert nothing plays into and with no effects (FL's named but
    /// unused inserts) builds no node, bus or return; one something
    /// reaches, or with an effect on it, does.
    #[test]
    fn an_empty_bus_builds_no_node() {
        use hardwave_project::track::{PluginSlot, Track};
        let mut project = Project::default();
        project.tracks.truncate(1);
        project
            .tracks
            .push(Track::new_bus("empty".into(), "KICK REVERB".into()));
        let mut ret = Track::new_bus("ret".into(), "Return".into());
        ret.kind = hardwave_project::TrackKind::Return;
        project.tracks.push(ret);
        project
            .tracks
            .push(Track::new_bus("fed".into(), "Fed".into()));
        let mut fx = Track::new_bus("fx".into(), "With effect".into());
        fx.inserts.push(PluginSlot {
            id: "s".into(),
            plugin_id: "test.gain".into(),
            enabled: true,
            state: None,
            sidechain_source: None,
            wet: 1.0,
        });
        project.tracks.push(fx);
        let mut source = Track::new_audio("src".into(), "Source".into());
        source.output_bus = Some("fed".into());
        project.tracks.push(source);
        let (cb, _cmd_tx) = make_callback(Arc::new(Mutex::new(project)));
        assert!(!cb.track_id_to_node.contains_key("empty"));
        assert!(!cb.track_id_to_node.contains_key("ret"));
        assert!(
            cb.track_id_to_node.contains_key("fed"),
            "something plays into it"
        );
        assert!(cb.track_id_to_node.contains_key("fx"), "it has an effect");
    }

    #[test]
    fn insert_command_for_pending_track_is_parked_not_dropped() {
        let mut project = Project::default();
        let track_id = project.add_audio_track("new track".into());
        // A track with a plug-in on it, as one getting insert commands has;
        // an empty one has no node at all.
        project
            .track_mut(&track_id)
            .unwrap()
            .inserts
            .push(hardwave_project::track::PluginSlot {
                id: "slot-x".into(),
                plugin_id: "test.gain".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        let project_arc = Arc::new(Mutex::new(project));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));

        // Simulate "track just added, rebuild deferred": wipe the node
        // map the command would resolve against and mark rebuild pending
        // while the UI holds the project lock.
        cb.track_id_to_node.clear();
        cb.needs_rebuild = true;

        assert!(
            cmd_tx
                .try_send(crate::insert_chain::InsertCommand::SetWet {
                    track_id: track_id.clone(),
                    slot_id: "slot-x".into(),
                    wet: 0.5,
                })
                .is_ok(),
            "queue insert command"
        );

        let guard = project_arc.lock();
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        drop(guard);
        assert_eq!(
            cb.deferred_insert_commands.len(),
            1,
            "command for a not-yet-built node must be parked while rebuild is pending"
        );

        // Rebuild completes on the next block; the parked command now
        // resolves to the rebuilt TrackNode and the parking lot drains.
        cb.process(&mut out, 256, 2);
        assert!(!cb.needs_rebuild);
        assert!(
            cb.deferred_insert_commands.is_empty(),
            "parked commands must be retried after the rebuild lands"
        );
        assert!(
            cb.track_id_to_node.contains_key(&track_id),
            "rebuilt graph must contain the new track's node"
        );
    }
}

#[cfg(test)]
mod wait_for_input_tests {
    //! FL-style "wait for input": Play/Record park the transport; the
    //! first MIDI event starts playback and is NOT swallowed (it must
    //! reach the graph in the same block so the triggering note sounds).
    use super::*;
    use std::sync::atomic::Ordering::Relaxed;

    fn make_callback(
        midi: Arc<Mutex<MidiInputManager>>,
    ) -> (EngineCallback, crossbeam_channel::Sender<EngineCommand>) {
        make_callback_with(midi, Project::default())
    }

    fn make_callback_with(
        midi: Arc<Mutex<MidiInputManager>>,
        project: Project,
    ) -> (EngineCallback, crossbeam_channel::Sender<EngineCommand>) {
        let transport = TransportState::default();
        transport.sample_rate.store(48_000, Relaxed);
        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (command_tx, command_rx) = bounded::<EngineCommand>(8);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(None));
        let (_cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
        let (grave_tx, _grave_rx) = crate::insert_chain::graveyard_channel(8);
        let cb = EngineCallback::new(
            transport,
            Arc::new(Mutex::new(project)),
            meter_producer,
            command_rx,
            AudioPool::new(),
            track_meters,
            input_consumer,
            crate::input_node::capture_set(),
            master_tap::new_shared(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cmd_rx,
            grave_tx,
            Arc::new(Mutex::new(None)),
            midi,
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            48_000,
            256,
            LoadCounters::new(),
            crate::metronome::MetronomeSettings::silent(),
            crate::preview_player::PreviewRequest::new(),
            crate::audio_prefs::AudioPrefs::new(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            // One thread: these tests are about the callback, not about
            // sharing the work.
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(None)),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(std::sync::atomic::AtomicI64::new(0)),
            Arc::new(crate::session::SessionState::new()),
        );
        (cb, command_tx)
    }

    #[test]
    fn play_parks_until_first_midi_event() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];

        cb.transport.wait_for_input.store(true, Relaxed);
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();

        // Blocks pass with no MIDI: parked, position frozen.
        for _ in 0..3 {
            cb.process(&mut out, 256, 2);
        }
        assert!(!cb.transport.is_playing(), "must stay parked with no input");
        assert!(cb.transport.wait_pending.load(Relaxed));
        assert_eq!(
            cb.transport.position(),
            0,
            "playhead must not advance while parked"
        );

        // First MIDI event: playback starts, and the event stays in the
        // scratch so the triggering note reaches the graph THIS block.
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        cb.process(&mut out, 256, 2);
        assert!(
            cb.transport.is_playing(),
            "first MIDI event starts playback"
        );
        assert!(!cb.transport.wait_pending.load(Relaxed));
        assert!(
            !cb.midi_event_scratch.is_empty(),
            "the triggering event must not be swallowed"
        );
    }

    /// With the song stopped and no track armed, notes played live are
    /// still read, captured with the engine's clock, and keep the engine
    /// running for their tail; they used to wait unread until play.
    #[test]
    fn live_notes_with_the_song_stopped_are_read_and_captured() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, _tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];
        assert!(!cb.transport.is_playing());
        cb.process(&mut out, 256, 2);
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        cb.process(&mut out, 256, 2);
        assert!(
            cb.live_midi_hold > 0,
            "the engine keeps running for the tail"
        );
        for _ in 0..4 {
            cb.process(&mut out, 256, 2);
        }
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.0,
        });
        cb.process(&mut out, 256, 2);
        let captured = cb.midi_capture_ring.lock().captured_in_order();
        assert_eq!(captured.len(), 2, "both events captured");
        assert_eq!(
            captured[0].sample_pos, captured[1].sample_pos,
            "the playhead stood still"
        );
        // How long each event waited for its block is measured on the wall
        // clock, so the two can differ by a sample or so (a busy Windows
        // runner put them 1281 apart); a block's worth would be a real fault.
        let apart = captured[1].clock - captured[0].clock;
        assert!(
            apart.abs_diff(5 * 256) <= 48,
            "the clock says how far apart they were played: {apart}"
        );
    }

    /// A synth played on the keyboard with the song stopped, sent pre-fader
    /// to a bus, with its own fader all the way down: the bus carries it,
    /// stopped and playing. A bus ran only while the song played, so a
    /// keyboard reached no bus or reverb return until Play.
    #[test]
    fn a_bus_carries_a_synth_played_live_with_the_song_stopped() {
        let mut song = Project::default();
        let mut synth = hardwave_project::Track::new_audio("synth".into(), "Synth".into());
        synth.kind = hardwave_project::TrackKind::Midi;
        synth.volume_db = -200.0;
        synth.sends.push(hardwave_project::mixer::Send {
            target: "bus".into(),
            gain_db: 0.0,
            pre_fader: true,
            enabled: true,
        });
        song.tracks.push(synth);
        song.tracks.push(hardwave_project::Track::new_audio(
            "bus".into(),
            "Bus".into(),
        ));
        song.keyboard_track = Some("synth".into());
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback_with(Arc::clone(&midi), song);
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        let mut loudest = |cb: &mut EngineCallback| {
            let mut peak = 0.0_f32;
            for _ in 0..8 {
                cb.process(&mut out, 256, 2);
                peak = out.iter().fold(peak, |m, s| m.max(s.abs()));
            }
            peak
        };
        assert!(!cb.transport.is_playing());
        let stopped = loudest(&mut cb);
        assert!(
            stopped > 0.05,
            "stopped, the bus carries the synth: {stopped}"
        );
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();
        let playing = loudest(&mut cb);
        assert!(playing > 0.05, "playing, the bus carries it too: {playing}");
    }

    /// A key held longer than the hold with the song stopped keeps the
    /// engine running: it went silent ten seconds after the press while
    /// the key was still down.
    #[test]
    fn a_key_held_with_the_song_stopped_keeps_sounding() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, _tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        let blocks = (cb.sample_rate as u64 * (LIVE_MIDI_HOLD_SECONDS + 2)) / 256 + 1;
        for _ in 0..blocks {
            cb.process(&mut out, 256, 2);
        }
        assert!(cb.live_midi_hold > 0, "still running with the key down");
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.0,
        });
        for _ in 0..blocks {
            cb.process(&mut out, 256, 2);
        }
        assert_eq!(cb.live_midi_hold, 0, "and stops once it is let go");
    }

    #[test]
    fn stop_cancels_a_parked_transport() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];

        cb.transport.wait_for_input.store(true, Relaxed);
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(cb.transport.wait_pending.load(Relaxed));

        tx.send(EngineCommand::Transport(TransportCommand::Stop))
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(
            !cb.transport.wait_pending.load(Relaxed),
            "Stop clears the park"
        );
        assert!(!cb.transport.is_playing());

        // Late MIDI after Stop must NOT start playback.
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        cb.process(&mut out, 256, 2);
        assert!(
            !cb.transport.is_playing(),
            "events after Stop must not un-park"
        );
    }

    #[test]
    fn disabling_wait_honours_pending_play() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];

        cb.transport.wait_for_input.store(true, Relaxed);
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(cb.transport.wait_pending.load(Relaxed));

        tx.send(EngineCommand::Transport(TransportCommand::SetWaitForInput(
            false,
        )))
        .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(
            cb.transport.is_playing(),
            "turning the pref off honours the earlier Play press"
        );
    }
}

#[cfg(test)]
mod recording_latency_tests {
    use super::*;

    #[test]
    fn the_manual_offset_moves_the_compensation_and_never_below_zero() {
        let engine = DawEngine::new();
        let sr = engine.current_sample_rate() as u64;
        let base = engine.recording_latency().total_frames;

        engine.audio_prefs().set_record_offset_ms(10);
        assert_eq!(engine.recording_latency().total_frames, base + sr / 100);

        engine.audio_prefs().set_record_offset_ms(-500);
        assert_eq!(
            engine.recording_latency().total_frames,
            0,
            "a large negative offset clamps at zero rather than moving takes later than played"
        );
    }

    /// Cancelling a take removes its file and leaves nothing to finish.
    #[test]
    fn a_discarded_take_leaves_no_file() {
        let engine = DawEngine::new();
        let dir = std::env::temp_dir().join(format!("hw-discard-{}", std::process::id()));
        engine.set_capture_dir(dir.clone());
        engine.start_capture();
        let block = vec![0.5_f32; 256];
        engine.capture.write_test_block_at(&block, &block, 256, 0);
        engine.discard_capture();
        let left: Vec<_> = std::fs::read_dir(&dir)
            .map(|d| d.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "no take file left behind: {left:?}");
        assert!(
            engine.stop_capture_passes().is_empty(),
            "nothing to finish after"
        );
    }

    /// A jump back with no loop (a timecode master locating, a Link session
    /// jumping) starts a new pass where the playhead went. Every pass after
    /// the first was put at the loop start, loop or not, so the second half
    /// of such a take landed at bar 1.
    #[test]
    fn a_pass_after_a_jump_lands_where_it_was_played() {
        let engine = DawEngine::new();
        engine.audio_prefs().set_record_offset_ms(-1000);
        assert_eq!(engine.recording_latency().total_frames, 0);
        engine
            .transport
            .loop_start
            .store(0, std::sync::atomic::Ordering::Relaxed);
        engine.transport.set_position(96_000);
        engine.start_capture();
        let block = vec![0.5_f32; 256];
        engine
            .capture
            .write_test_block_at(&block, &block, 256, 96_000);
        engine
            .capture
            .write_test_block_at(&block, &block, 256, 96_256);
        // The master located back to 48 000.
        engine
            .capture
            .write_test_block_at(&block, &block, 256, 48_000);
        let passes = engine.stop_capture_passes();
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[0].0, 96_000);
        assert_eq!(passes[1].0, 48_000, "not the loop start (0)");
    }

    #[test]
    fn a_take_starts_where_its_first_sample_was_played() {
        let engine = DawEngine::new();
        engine.audio_prefs().set_record_offset_ms(1); // 48 frames at 48 kHz
        let latency = engine.recording_latency().total_frames;
        assert!(latency > 0);
        engine.transport.set_position(10_000);
        engine.start_capture();
        let block = vec![0.5_f32; 256];
        engine
            .capture
            .write_test_block_at(&block, &block, 256, 10_000);
        let passes = engine.stop_capture_passes();
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].0, 10_000, "the take starts at the record start");
        assert_eq!(
            passes[0].1.len() as u64,
            (256 - latency) * 2,
            "the frames played before recording began are not kept"
        );
    }
}

#[cfg(test)]
mod monitoring_tests {
    //! What an armed track lets you hear of the live input, stopped,
    //! playing and recording.
    use super::*;
    use std::sync::atomic::Ordering::Relaxed;

    struct Rig {
        cb: EngineCallback,
        input: rtrb::Producer<f32>,
        commands: crossbeam_channel::Sender<EngineCommand>,
        capture: Arc<CaptureTap>,
        captures: Arc<[Arc<CaptureTap>]>,
    }

    fn rig(project: Project) -> Rig {
        rig_with(project, AudioPool::new())
    }

    fn rig_with(project: Project, pool: AudioPool) -> Rig {
        rig_wide(project, pool, 2)
    }

    /// A rig whose interface has `width` inputs.
    fn rig_wide(project: Project, pool: AudioPool, width: u16) -> Rig {
        let transport = TransportState::default();
        transport.sample_rate.store(48_000, Relaxed);
        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (command_tx, command_rx) = bounded::<EngineCommand>(8);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let (input, consumer) = RingBuffer::<f32>::new(48_000 * 2);
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(Some(InputRing {
            consumer,
            width,
            default: InputChannel {
                first: 0,
                mono: false,
            },
        })));
        let (_cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
        let (grave_tx, _grave_rx) = crate::insert_chain::graveyard_channel(8);
        let captures = crate::input_node::capture_set();
        let capture = Arc::clone(&captures[0]);
        let cb = EngineCallback::new(
            transport,
            Arc::new(Mutex::new(project)),
            meter_producer,
            command_rx,
            pool,
            track_meters,
            input_consumer,
            Arc::clone(&captures),
            master_tap::new_shared(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cmd_rx,
            grave_tx,
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(MidiInputManager::new())),
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            48_000,
            256,
            LoadCounters::new(),
            crate::metronome::MetronomeSettings::silent(),
            crate::preview_player::PreviewRequest::new(),
            crate::audio_prefs::AudioPrefs::new(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(None)),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(std::sync::atomic::AtomicI64::new(0)),
            Arc::new(crate::session::SessionState::new()),
        );
        Rig {
            cb,
            input,
            commands: command_tx,
            capture,
            captures,
        }
    }

    fn armed_song() -> Project {
        let mut p = Project::default();
        let mut t = hardwave_project::Track::new_audio("vox".into(), "Vocal".into());
        t.armed = true;
        t.monitor_input = true;
        p.tracks.push(t);
        p
    }

    /// Feed live input at 0.5 and return the loudest output sample of the
    /// next few blocks: about 0.35 when heard, through the centre pan law.
    fn heard(rig: &mut Rig) -> f32 {
        let mut loudest = 0.0_f32;
        let mut out = vec![0.0_f32; 512];
        for _ in 0..4 {
            for _ in 0..512 {
                let _ = rig.input.push(0.5);
            }
            rig.cb.process(&mut out, 256, 2);
            loudest = out.iter().fold(loudest, |m, s| m.max(s.abs()));
        }
        loudest
    }

    fn play(rig: &Rig) {
        rig.commands
            .send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();
    }

    /// Monitoring through a track went silent unless the track had a
    /// plug-in, and a stopped track returned silence before reading its
    /// input, so the IN button let a singer hear nothing.
    #[test]
    fn an_armed_track_with_nothing_on_it_is_heard_stopped_and_playing() {
        let mut r = rig(armed_song());
        assert!(heard(&mut r) > 0.3, "stopped, the input is heard");
        play(&r);
        assert!(heard(&mut r) > 0.3, "playing, the input is heard");
    }

    #[test]
    fn monitoring_off_is_silent() {
        let mut song = armed_song();
        song.tracks.last_mut().unwrap().monitor_input = false;
        let mut r = rig(song);
        assert_eq!(heard(&mut r), 0.0);
        play(&r);
        assert_eq!(heard(&mut r), 0.0);
    }

    #[test]
    fn auto_monitoring_hears_the_input_stopped_and_recording_but_not_playing_back() {
        let mut song = armed_song();
        song.tracks.last_mut().unwrap().monitor_auto = true;
        let mut r = rig(song);
        assert!(heard(&mut r) > 0.3, "stopped: the input");
        play(&r);
        assert_eq!(heard(&mut r), 0.0, "playing back: not the input");
        r.cb.transport.recording.store(true, Relaxed);
        assert!(heard(&mut r) > 0.3, "recording: the input");
    }

    #[test]
    fn a_bus_the_monitored_track_feeds_passes_the_input_while_stopped() {
        let mut song = armed_song();
        song.tracks.push(hardwave_project::Track::new_audio(
            "bus".into(),
            "Vocal bus".into(),
        ));
        song.tracks
            .iter_mut()
            .find(|t| t.id == "vox")
            .unwrap()
            .output_bus = Some("bus".into());
        let mut r = rig(song);
        // Through the pan law twice, the track's and the bus's: about 0.25.
        let level = heard(&mut r);
        assert!(
            level > 0.2,
            "the bus carries the input to master, got {level}"
        );
    }

    /// A send-level lane with no points yet leaves the send where the mixer
    /// has it. It read as half-way, -27 dB, from the moment it was added.
    #[test]
    fn an_empty_send_lane_leaves_the_send_alone() {
        let song = |with_lane: bool| {
            let mut song = armed_song();
            song.tracks.push(hardwave_project::Track::new_audio(
                "fx".into(),
                "Reverb".into(),
            ));
            let vox = song.tracks.iter_mut().find(|t| t.id == "vox").unwrap();
            vox.sends.push(hardwave_project::mixer::Send {
                target: "fx".into(),
                gain_db: 0.0,
                pre_fader: false,
                enabled: true,
            });
            if with_lane {
                vox.automation_lanes
                    .push(hardwave_project::automation::AutomationLane {
                        id: "send".into(),
                        target: hardwave_project::automation::AutomationTarget::SendLevel {
                            send_index: 0,
                        },
                        points: Vec::new(),
                        visible: true,
                        row: None,
                    });
            }
            song
        };
        let plain = heard(&mut rig(song(false)));
        let with_lane = heard(&mut rig(song(true)));
        assert!(plain > 0.5, "the dry vocal and the send: {plain}");
        assert!(
            (plain - with_lane).abs() < 1e-3,
            "the lane changed the send: {plain} then {with_lane}"
        );
    }

    /// The vocal playing into its bus, and the bus pointed back at the
    /// vocal: by a send, or as the key of a compressor on the vocal.
    fn song_with_loop_back(by_send: bool) -> Project {
        let mut song = armed_song();
        let mut bus = hardwave_project::Track::new_audio("bus".into(), "Vocal bus".into());
        let vox = song.tracks.iter_mut().find(|t| t.id == "vox").unwrap();
        vox.output_bus = Some("bus".into());
        if by_send {
            bus.sends.push(hardwave_project::mixer::Send {
                target: "vox".into(),
                gain_db: 0.0,
                pre_fader: false,
                enabled: true,
            });
        } else {
            vox.inserts.push(hardwave_project::track::PluginSlot {
                id: "comp".into(),
                plugin_id: "compressor".into(),
                enabled: true,
                state: None,
                sidechain_source: Some("bus".into()),
                wet: 1.0,
            });
        }
        song.tracks.push(bus);
        song
    }

    /// A send or a sidechain key that loops back made the graph skip the
    /// loop and everything after it, master included: one click in a send
    /// or sidechain menu and the whole song went silent. The loop is left
    /// out and the rest plays.
    #[test]
    fn a_send_or_key_that_loops_back_does_not_silence_the_song() {
        for by_send in [true, false] {
            let mut r = rig(song_with_loop_back(by_send));
            let level = heard(&mut r);
            assert!(
                level > 0.2,
                "by send: {by_send}, the vocal still reaches the master, got {level}"
            );
        }
    }

    /// A song with one track playing a steady 0.5 for a second, and an
    /// empty track armed to record the master.
    fn resampling_song() -> (Project, AudioPool) {
        use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};
        let frames = 48_000;
        let pool = AudioPool::new();
        pool.insert(
            "tone".into(),
            crate::AudioBuffer {
                channels: vec![vec![0.5; frames]; 2],
                sample_rate: 48_000,
                num_frames: frames,
            },
        );
        let mut p = Project::default();
        let mut beat = hardwave_project::Track::new_audio("beat".into(), "Beat".into());
        beat.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "clip-tone".into(),
                name: "tone".into(),
                source_path: "tone".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: frames as u64,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: String::new(),
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            }),
            track_id: "beat".into(),
            position_ticks: 0,
            length_ticks: 1920,
            lane: 0,
            color: None,
            row: None,
        });
        p.tracks.push(beat);
        let mut rec = hardwave_project::Track::new_audio("rec".into(), "Resample".into());
        rec.armed = true;
        rec.input_source = Some("master".into());
        p.tracks.push(rec);
        (p, pool)
    }

    /// Recording the song's own output, as resampling: the take is what
    /// the master played, not what came in at the interface.
    #[test]
    fn an_armed_track_records_the_master_not_the_interface() {
        let (song, pool) = resampling_song();
        let mut r = rig_with(song, pool);
        r.capture.arm(48_000, 1);
        r.capture.recording.store(true, Relaxed);
        play(&r);
        let mut out = vec![0.0_f32; 512];
        for _ in 0..8 {
            for _ in 0..512 {
                // A loud interface input that must not end up in the take.
                let _ = r.input.push(0.9);
            }
            r.cb.process(&mut out, 256, 2);
        }
        r.capture.recording.store(false, Relaxed);
        let take = r.capture.take();
        assert!(!take.is_empty(), "the master was recorded");
        let peak = take.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.2, "the tone through the master, got {peak}");
        assert!(peak < 0.6, "not the interface input, got {peak}");
        let heard = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            heard < 0.6,
            "the interface input is not heard on the armed track"
        );
    }

    /// Some drivers hand over a bigger block than the one asked for. The
    /// graph is built for the size asked for, so everything past it came
    /// out as silence: a gap in every block.
    #[test]
    fn a_block_bigger_than_asked_is_played_whole() {
        let (mut song, pool) = resampling_song();
        song.tracks.retain(|t| t.id != "rec");
        let mut r = rig_with(song, pool);
        play(&r);
        // The rig asks for 256; the driver gives 1000.
        let mut out = vec![0.0_f32; 2000];
        r.cb.process(&mut out, 1000, 2);
        let quiet = out
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .skip(64)
            .find(|(_, f)| f[0].abs() < 0.1)
            .map(|(i, _)| i);
        assert_eq!(
            quiet, None,
            "the whole block plays, not the first 256 frames"
        );
        assert_eq!(
            r.cb.transport.position(),
            1000,
            "and the playhead moves by all of it"
        );
    }

    /// Previewing a loop at the song's tempo moved its pitch like a
    /// turntable. The stretched preview keeps the length the song's tempo
    /// asks for.
    #[test]
    fn a_preview_stretch_takes_the_songs_length() {
        let pool = AudioPool::new();
        let frames = 48_000;
        let tone: Vec<f32> = (0..frames)
            .map(|n| (std::f32::consts::TAU * 440.0 * n as f32 / 48_000.0).sin() * 0.5)
            .collect();
        pool.insert(
            "loop".into(),
            crate::AudioBuffer {
                channels: vec![tone.clone(), tone],
                sample_rate: 48_000,
                num_frames: frames,
            },
        );
        // A 128 BPM loop in a 160 BPM song plays 1.25 times as fast.
        let key = DawEngine::bake_preview_stretch(&pool, "loop", 1.25).expect("baked");
        let baked = pool.get(&key).unwrap();
        let expected = (frames as f64 / 1.25) as i64;
        assert!(
            (baked.num_frames as i64 - expected).abs() < 4_800,
            "{} frames, expected about {expected}",
            baked.num_frames
        );
        assert_ne!(key, "loop");
    }

    /// A singer on input 1 and a guitar on input 2, on two armed tracks:
    /// each track records and hears its own input. Every armed track got the
    /// same stereo input before, so the two could not be recorded at once.
    #[test]
    fn two_armed_tracks_record_two_inputs_at_once() {
        let mut song = Project::default();
        for (id, first) in [("vox", 0), ("gtr", 1)] {
            let mut t = hardwave_project::Track::new_audio(id.into(), id.into());
            t.armed = true;
            t.monitor_input = true;
            t.input_channel = Some(InputChannel { first, mono: true });
            song.tracks.push(t);
        }
        let mut r = rig_wide(song, AudioPool::new(), 4);
        // Armed as start_capture arms them: one capture per input, in the
        // order of the tracks.
        for tap in &r.captures[..2] {
            tap.arm(48_000, 1);
            tap.recording.store(true, Relaxed);
        }
        play(&r);
        let mut out = vec![0.0_f32; 512];
        for _ in 0..8 {
            for _ in 0..256 {
                for v in [0.3, 0.6, 0.9, 0.9] {
                    let _ = r.input.push(v);
                }
            }
            r.cb.process(&mut out, 256, 2);
        }
        for tap in r.captures.iter() {
            tap.recording.store(false, Relaxed);
        }
        let vox = r.captures[0].take();
        let gtr = r.captures[1].take();
        assert!(
            !vox.is_empty() && !gtr.is_empty(),
            "both inputs were recorded"
        );
        assert!(
            vox.iter().all(|&s| s == 0.3),
            "the singer's take is input 1 only"
        );
        assert!(
            gtr.iter().all(|&s| s == 0.6),
            "the guitar's take is input 2 only"
        );
        assert!(r.captures[2].take().is_empty(), "nothing else recorded");
        let heard = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            heard > 0.4,
            "both inputs are heard through their tracks, got {heard}"
        );
    }

    /// How long one rebuild of a new song's graph takes on the audio thread:
    /// a figure to read, not a pass/fail. Run with --ignored --nocapture.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn measure_a_new_song_rebuild() {
        let mut r = rig(Project::default());
        let mut times = Vec::new();
        for _ in 0..20 {
            let t = std::time::Instant::now();
            r.cb.rebuild_graph();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!(
            "REBUILD ms: median {:.2}, max {:.2} (a 512-frame block at 48 kHz is 10.7 ms)",
            times[10], times[19]
        );

        // The same song with 60 of its tracks in use, a clip on each.
        let (mut song, pool) = resampling_song();
        let clip = song.tracks.iter().find(|t| t.id == "beat").unwrap().clips[0].clone();
        for t in song
            .tracks
            .iter_mut()
            .filter(|t| t.name.starts_with("Insert "))
            .take(60)
        {
            let mut c = clip.clone();
            c.track_id = t.id.clone();
            t.clips.push(c);
        }
        let mut r = rig_with(song, pool);
        let mut times = Vec::new();
        for _ in 0..20 {
            let t = std::time::Instant::now();
            r.cb.rebuild_graph();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!(
            "REBUILD 60 used ms: median {:.2}, max {:.2}",
            times[10], times[19]
        );
    }

    /// A block crossing the loop end is split there: the playhead goes back
    /// to the loop start and carries on from there, nothing past the end
    /// is played, and each pass is exactly the loop long.
    #[test]
    fn the_loop_jumps_back_on_the_exact_sample() {
        let mut r = rig(Project::default());
        r.cb.transport.looping.store(true, Relaxed);
        r.cb.transport.loop_start.store(1_000, Relaxed);
        r.cb.transport.loop_end.store(2_000, Relaxed);
        play(&r);
        let mut out = vec![0.0_f32; 512];
        // Playing already, as it is by the time a loop comes round.
        r.cb.process(&mut out, 256, 2);
        r.cb.transport.set_position(1_900);
        r.cb.process(&mut out, 256, 2);
        assert_eq!(
            r.cb.transport.position(),
            1_000 + 156,
            "100 frames to the end, then 156 from the loop start"
        );
    }

    /// With the click on an output pair of its own, the beat sounds there
    /// and the mix stays silent; without one, it is in the mix as before.
    #[test]
    fn the_click_goes_to_its_own_pair_when_there_is_one() {
        let peak = |b: &[f32]| b.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        let mut r = rig(Project::default());
        r.cb.metronome.settings().enabled.store(true, Relaxed);
        play(&r);
        let (mut main_peak, mut click_peak) = (0.0_f32, 0.0_f32);
        for _ in 0..4 {
            let mut main = vec![0.0_f32; 512];
            let mut click = vec![0.0_f32; 512];
            r.cb.process_split(&mut main, &mut click, 256);
            main_peak = main_peak.max(peak(&main));
            click_peak = click_peak.max(peak(&click));
        }
        assert!(
            click_peak > 0.01,
            "the beat on the click pair: {click_peak}"
        );
        assert_eq!(main_peak, 0.0, "nothing in the mix");

        let mut r = rig(Project::default());
        r.cb.metronome.settings().enabled.store(true, Relaxed);
        play(&r);
        let mut in_mix = 0.0_f32;
        for _ in 0..4 {
            let mut out = vec![0.0_f32; 512];
            r.cb.process(&mut out, 256, 2);
            in_mix = in_mix.max(peak(&out));
        }
        assert!(
            in_mix > 0.01,
            "without a pair of its own the click is in the mix"
        );
    }
}

#[cfg(test)]
mod track_render_tests {
    //! Renders whose audio goes back onto a track (a bounce, a freeze, a
    //! consolidate), played back the way the app places them.
    use super::*;
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};

    const SR: u32 = 48_000;
    /// Two beats at the default 140 BPM.
    const LEN_TICKS: u64 = 1920;

    fn clip(id: &str, source: &str, frames: u64, track_id: &str) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: id.into(),
                name: id.into(),
                source_path: source.into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: frames,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: String::new(),
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            }),
            track_id: track_id.into(),
            position_ticks: 0,
            length_ticks: LEN_TICKS,
            lane: 0,
            color: None,
            row: None,
        }
    }

    /// A song with one track playing a sine, a different one on each side,
    /// and the track's id.
    fn sine_song() -> (DawEngine, String, u64) {
        let engine = DawEngine::new();
        let frames = SR as u64;
        let sine = |hz: f32| -> Vec<f32> {
            (0..frames)
                .map(|n| (std::f32::consts::TAU * hz * n as f32 / SR as f32).sin() * 0.5)
                .collect()
        };
        engine.audio_pool.insert(
            "sine".to_string(),
            crate::AudioBuffer {
                channels: vec![sine(220.0), sine(330.0)],
                sample_rate: SR,
                num_frames: frames as usize,
            },
        );
        let id = {
            let mut project = engine.project.lock();
            let id = project.add_audio_track("Sine".into());
            let placed = clip("c", "sine", frames, &id);
            project.track_mut(&id).unwrap().clips.push(placed);
            id
        };
        let len = engine
            .project
            .lock()
            .tempo_map
            .tick_to_samples(LEN_TICKS, SR as f64);
        (engine, id, len)
    }

    fn render(engine: &DawEngine, len: u64, prepare: impl FnOnce(&mut Project)) -> Vec<f32> {
        let mut out = Vec::new();
        engine
            .render_offline_with(SR, len, 0, None, prepare, |b| {
                out.extend_from_slice(b);
                true
            })
            .expect("it renders");
        out
    }

    /// The track alone, as the app narrows a render for a bounce, with
    /// `change` made to the render's copy of the song.
    fn render_for_a_track(
        engine: &DawEngine,
        target: &str,
        len: u64,
        change: impl FnOnce(&mut Project),
    ) -> Vec<f32> {
        let target = target.to_string();
        let mut out = Vec::new();
        engine
            .prepare_offline_render(SR, 0, None, move |p: &mut Project| {
                for t in p.tracks.iter_mut() {
                    t.stem_excluded = t.id != target;
                }
                change(p);
            })
            .for_a_track()
            .run(len, |b| {
                out.extend_from_slice(b);
                true
            });
        out
    }

    /// An interleaved render in the audio pool under `source`, and how many
    /// frames it has.
    fn pool_render(engine: &DawEngine, source: &str, rendered: &[f32]) -> u64 {
        let frames = rendered.len() / 2;
        let left: Vec<f32> = rendered.iter().step_by(2).copied().collect();
        let right: Vec<f32> = rendered.iter().skip(1).step_by(2).copied().collect();
        engine.audio_pool.insert(
            source.to_string(),
            crate::AudioBuffer {
                channels: vec![left, right],
                sample_rate: SR,
                num_frames: frames,
            },
        );
        frames as u64
    }

    /// Place an interleaved render on a new track, as a bounce does, and
    /// mute the original.
    fn place_bounce(engine: &DawEngine, original: &str, rendered: &[f32]) {
        let frames = pool_render(engine, "bounce", rendered);
        let mut project = engine.project.lock();
        let id = project.add_audio_track("Bounced".into());
        let placed = clip("b", "bounce", frames, &id);
        project.track_mut(&id).unwrap().clips.push(placed);
        project.track_mut(original).unwrap().muted = true;
    }

    /// Place a render on the track's freeze lane and freeze it, as a freeze
    /// does.
    fn place_freeze(engine: &DawEngine, track: &str, rendered: &[f32]) {
        let frames = pool_render(engine, "freeze", rendered);
        let mut project = engine.project.lock();
        let mut placed = clip("f", "freeze", frames, track);
        placed.lane = hardwave_project::clip::FREEZE_LANE;
        let t = project.track_mut(track).unwrap();
        t.clips.push(placed);
        t.frozen = true;
    }

    /// An instrument track with one note on the playlist, and a channel
    /// rack whose pattern plays nothing on it.
    fn note_and_empty_pattern() -> (DawEngine, String) {
        let engine = DawEngine::new();
        let id = {
            let mut p = engine.project.lock();
            let id = p.add_midi_track("Lead".into());
            let mut midi = hardwave_midi::MidiClip::new("m".into(), "m".into(), LEN_TICKS);
            midi.notes.push(hardwave_midi::MidiNote {
                start_tick: 0,
                duration_ticks: LEN_TICKS,
                pitch: 69,
                velocity: 0.8,
                ..Default::default()
            });
            p.track_mut(&id).unwrap().clips.push(ClipPlacement {
                content: ClipContent::Midi(hardwave_project::clip::MidiClipRef {
                    id: "m".into(),
                    clip: midi,
                }),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: LEN_TICKS,
                lane: 0,
                color: None,
                row: None,
            });
            p.channel_rack_state = Some(r#"{"patterns":[{"id":"p"}],"activeId":"p"}"#.into());
            p.refresh_step_rack();
            id
        };
        (engine, id)
    }

    /// Freezing, bouncing or consolidating with the transport in pattern
    /// mode renders the track's part on the playlist. It rendered the
    /// pattern, here nothing at all.
    #[test]
    fn a_render_for_a_track_is_the_playlist_in_pattern_mode() {
        let (engine, id) = note_and_empty_pattern();
        let len = engine
            .project
            .lock()
            .tempo_map
            .tick_to_samples(LEN_TICKS, SR as f64);
        let song = render_for_a_track(&engine, &id, len, |_| {});
        assert!(peak(&song) > 0.05, "the note plays: {}", peak(&song));
        engine
            .transport
            .pattern_mode
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let in_pattern_mode = render_for_a_track(&engine, &id, len, |_| {});
        let diff = worst_difference(&song, &in_pattern_mode);
        assert!(diff < 1e-6, "the part, not the pattern: off by {diff}");
    }

    /// An export of the pattern plays it once, from its start, whatever mode
    /// the transport is in, and its tail is what still sounds, not the
    /// pattern again.
    #[test]
    fn a_pattern_export_plays_the_pattern_once() {
        let engine = DawEngine::new();
        {
            let mut p = engine.project.lock();
            let id = p.add_midi_track("Hat".into());
            // A step on the second sixteenth, so a pattern started again
            // plays it again even when the loop lands inside the bar.
            let mut row = vec![0.0; 16];
            row[1] = 1.0;
            let steps = serde_json::json!({ id.clone(): row });
            let rack = serde_json::json!({
                "patterns": [{ "id": "p", "steps": steps }],
                "activeId": "p",
            });
            p.channel_rack_state = Some(rack.to_string());
            p.refresh_step_rack();
        }
        // One bar of pattern and one of tail.
        let bar = engine
            .project
            .lock()
            .tempo_map
            .tick_to_samples(4 * hardwave_midi::PPQ, SR as f64) as usize;
        let render = |once: bool| -> Vec<f32> {
            let mut out = Vec::new();
            let r = engine.prepare_offline_render(SR, 0, None, |_| {});
            let r = if once { r.the_pattern_once() } else { r };
            r.run(2 * bar as u64, |b| {
                out.extend_from_slice(b);
                true
            });
            out
        };
        // A fifth of a second from a frame, interleaved: the step is a
        // sixteenth in, a tenth of a second at 140 BPM.
        let from =
            |s: &[f32], frame: usize| -> f32 { peak(&s[2 * frame..2 * (frame + SR as usize / 5)]) };
        let once = render(true);
        assert!(from(&once, 0) > 0.05, "the step plays near the start");
        assert!(from(&once, bar) < 1e-4, "and not again in the tail");
        // What it rendered before, with the transport in song mode: the
        // song's first bar, here nothing.
        let song = render(false);
        assert!(from(&song, 0) < 1e-4);
    }

    /// The loudest difference between two renders, away from the first and
    /// last few milliseconds where each clip's own declick fades sit.
    fn worst_difference(a: &[f32], b: &[f32]) -> f32 {
        let edge = (SR as usize / 50) * 2;
        a.iter()
            .zip(b)
            .skip(edge)
            .take(a.len().min(b.len()).saturating_sub(2 * edge))
            .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
    }

    /// A render is the song even with the reference switched on for
    /// comparing: that made the export the reference record.
    #[test]
    fn a_render_is_the_mix_with_the_reference_switched_on() {
        let (engine, _id, len) = sine_song();
        let heard = render(&engine, len, |_| {});
        engine.audio_pool.insert(
            "record".to_string(),
            crate::AudioBuffer {
                channels: vec![vec![0.9; len as usize], vec![0.9; len as usize]],
                sample_rate: SR,
                num_frames: len as usize,
            },
        );
        engine.set_reference_source(Some("record"));
        engine.set_reference_on(true);
        let exported = render(&engine, len, |_| {});
        let diff = worst_difference(&heard, &exported);
        assert!(
            diff < 1e-6,
            "the export is the mix, not the record: worst difference {diff}"
        );
    }

    /// A bounced part plays back as loud as the part it replaced. It came
    /// back 3 dB down (the centred pan, once in the render and again on the
    /// new track) and lower again by the master fader, which the render
    /// went through and the new track plays through again.
    #[test]
    fn a_bounce_plays_back_as_loud_as_the_part_it_replaced() {
        let (engine, id, len) = sine_song();
        engine
            .transport
            .master_volume_db
            .store(-6.0, std::sync::atomic::Ordering::Relaxed);
        let heard = render(&engine, len, |_| {});
        assert!(peak(&heard) > 0.1, "the song plays: {}", peak(&heard));

        let bounced = render_for_a_track(&engine, &id, len, |_| {});
        place_bounce(&engine, &id, &bounced);
        let played = render(&engine, len, |_| {});
        let diff = worst_difference(&heard, &played);
        assert!(
            diff < 1e-3,
            "the bounce plays as the part did: worst difference {diff}, peaks {} and {}",
            peak(&heard),
            peak(&played)
        );
    }

    /// A frozen track plays as it did live. The freeze render goes up to
    /// the fader (the app leaves the fader and pan out of it), and the
    /// frozen track plays it through its fader and pan only. It also ran
    /// the render through its delay, swap, polarity, width, filter and pitch
    /// a second time: a flipped polarity flipped back, swapped sides swapped
    /// back, the filter cut twice as steep and the part played transposed
    /// twice, and faster.
    #[test]
    fn a_frozen_track_plays_as_it_did_before_it_was_frozen() {
        let (engine, id, len) = sine_song();
        {
            let mut project = engine.project.lock();
            let t = project.track_mut(&id).unwrap();
            t.volume_db = -6.0;
            t.pan = 0.4;
            t.phase_invert = true;
            t.swap_lr = true;
            t.stereo_separation = 0.5;
            t.filter_type = "lp".into();
            t.filter_cutoff_hz = 300.0;
            t.filter_resonance = 0.707;
            t.pitch_semitones = 2;
        }
        let heard = render(&engine, len, |_| {});
        assert!(peak(&heard) > 0.05, "the song plays: {}", peak(&heard));

        let target = id.clone();
        let rendered = render_for_a_track(&engine, &id, len, move |p| {
            let t = p.track_mut(&target).unwrap();
            t.volume_db = 0.0;
            t.pan = 0.0;
        });
        place_freeze(&engine, &id, &rendered);
        let played = render(&engine, len, |_| {});
        let diff = worst_difference(&heard, &played);
        assert!(
            diff < 1e-3,
            "the frozen track plays as it did: worst difference {diff}, peaks {} and {}",
            peak(&heard),
            peak(&played)
        );
    }
}

#[cfg(test)]
mod tempo_follow_tests {
    //! A song whose tempo changes, played back and exported: audio that
    //! follows the tempo lands on the beats the tempo map puts it on, audio
    //! that keeps its speed plays as recorded, and the two renders agree.
    use super::*;
    use hardwave_midi::PPQ;
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, SampleTreatment};
    use hardwave_project::tempo::{TempoEntry, TempoMap, TempoRamp};
    use std::sync::atomic::Ordering::Relaxed;

    const RATE: u32 = 48_000;
    /// A beat of the loop: it is in 150 BPM.
    const LOOP_BEAT: usize = 19_200;
    const BEATS: usize = 16;

    fn entry(tick: u64, bpm: f64, ramp: TempoRamp) -> TempoEntry {
        TempoEntry {
            tick,
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            ramp,
        }
    }

    /// 150 for two bars, then a ramp up to 175 over the next bar, held.
    fn ramp_up() -> TempoMap {
        TempoMap {
            entries: vec![
                entry(0, 150.0, TempoRamp::Instant),
                entry(PPQ * 8, 150.0, TempoRamp::Linear),
                entry(PPQ * 12, 175.0, TempoRamp::Instant),
            ],
        }
    }

    /// A click on every beat of a 150 BPM loop, sixteen beats long.
    fn click_loop() -> AudioBuffer {
        let mut ch = vec![0.0f32; LOOP_BEAT * BEATS];
        for beat in 0..BEATS {
            ch[beat * LOOP_BEAT] = 1.0;
        }
        AudioBuffer {
            channels: vec![ch.clone(), ch],
            sample_rate: RATE,
            num_frames: LOOP_BEAT * BEATS,
        }
    }

    /// The song: the click loop at the start of an audio track, following
    /// the tempo or keeping its speed. Played like a tape, so the test
    /// hears where each click lands without a phase vocoder's smear.
    fn song(map: TempoMap, follow: bool) -> Project {
        let mut project = Project {
            tempo_map: map,
            ..Default::default()
        };
        let id = project.add_audio_track("Loop".into());
        let track = project.track_mut(&id).unwrap();
        track.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "loop".into(),
                name: "Loop 150 BPM".into(),
                source_path: "loop".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: (LOOP_BEAT * BEATS) as u64,
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
                source_file: String::new(),
                treatment: SampleTreatment {
                    resample: true,
                    follow_bpm: follow.then_some(150.0),
                    ..Default::default()
                },
                tuned_notes: Vec::new(),
            }),
            track_id: id.clone(),
            position_ticks: 0,
            length_ticks: PPQ * 20,
            lane: 0,
            color: None,
            row: None,
        });
        project
    }

    fn exported(project: Project, frames: u64) -> Vec<f32> {
        let engine = DawEngine::new();
        engine.audio_pool.insert("loop".to_string(), click_loop());
        *engine.project.lock() = project;
        let mut out = Vec::new();
        engine
            .render_offline_with(
                RATE,
                frames,
                0,
                None,
                |_| {},
                |b| {
                    out.extend(b.iter().step_by(2));
                    true
                },
            )
            .expect("it renders");
        out
    }

    /// The same song through the callback the sound card drives, a small
    /// block at a time.
    fn played(project: Project, frames: usize) -> Vec<f32> {
        let transport = TransportState::default();
        transport.sample_rate.store(RATE as u64, Relaxed);
        transport
            .bpm
            .store(project.tempo_map.entries[0].bpm, Relaxed);
        transport.playing.store(true, Relaxed);
        let pool = AudioPool::new();
        pool.insert("loop".to_string(), click_loop());
        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (_command_tx, command_rx) = bounded::<EngineCommand>(4);
        let (_cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
        let (grave_tx, _grave_rx) = crate::insert_chain::graveyard_channel(8);
        let mut cb = EngineCallback::new(
            transport,
            Arc::new(Mutex::new(project)),
            meter_producer,
            command_rx,
            pool,
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(None)),
            crate::input_node::capture_set(),
            master_tap::new_shared(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cmd_rx,
            grave_tx,
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(MidiInputManager::new())),
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            RATE,
            256,
            LoadCounters::new(),
            crate::metronome::MetronomeSettings::silent(),
            crate::preview_player::PreviewRequest::new(),
            crate::audio_prefs::AudioPrefs::new(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(None)),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(std::sync::atomic::AtomicI64::new(0)),
            Arc::new(crate::session::SessionState::new()),
        );
        let mut out = Vec::with_capacity(frames);
        let mut block = vec![0.0f32; 512];
        while out.len() < frames {
            block.fill(0.0);
            cb.process(&mut block, 256, 2);
            out.extend(block.iter().step_by(2));
        }
        out.truncate(frames);
        out
    }

    /// Where each click is heard: the loudest sample near where it should be.
    fn clicks_near(out: &[f32], expected: &[u64]) -> Vec<u64> {
        expected
            .iter()
            .map(|&at| {
                let from = (at as usize).saturating_sub(400);
                let to = (at as usize + 400).min(out.len());
                let (i, _) = out[from..to]
                    .iter()
                    .enumerate()
                    .fold((0, 0.0f32), |best, (i, v)| {
                        if v.abs() > best.1 {
                            (i, v.abs())
                        } else {
                            best
                        }
                    });
                (from + i) as u64
            })
            .collect()
    }

    fn beats_of(map: &TempoMap) -> Vec<u64> {
        (0..BEATS as u64)
            .map(|b| map.tick_to_samples(b * PPQ, RATE as f64))
            .collect()
    }

    /// The loop follows the tempo: through the steady start, up the ramp
    /// and on at 175, every click is on the beat the tempo map has there.
    /// It used to play on at 150, so by the end it was a beat and a half
    /// behind the grid it was drawn on.
    #[test]
    fn a_loop_that_follows_the_tempo_stays_on_the_beat() {
        let map = ramp_up();
        let beats = beats_of(&map);
        let out = exported(song(map, true), beats[BEATS - 1] + 4_800);
        for (k, (heard, want)) in clicks_near(&out, &beats)
            .into_iter()
            .zip(&beats)
            .enumerate()
        {
            assert!(
                heard.abs_diff(*want) <= 3,
                "beat {k}: the click is at {heard}, the beat at {want}"
            );
        }
    }

    /// A loop that keeps its speed plays as recorded: its clicks keep the
    /// loop's own spacing whatever the tempo does around them.
    #[test]
    fn a_clip_that_keeps_its_speed_plays_as_recorded() {
        let map = ramp_up();
        let recorded: Vec<u64> = (0..BEATS as u64).map(|b| b * LOOP_BEAT as u64).collect();
        let out = exported(song(map.clone(), false), recorded[BEATS - 1] + 4_800);
        for (heard, want) in clicks_near(&out, &recorded).into_iter().zip(&recorded) {
            assert!(heard.abs_diff(*want) <= 1, "{heard} against {want}");
        }
        // And that is off the song's beats once the ramp has begun.
        let beats = beats_of(&map);
        assert!(recorded[15].abs_diff(beats[15]) > 4_800);
    }

    /// Playback and the export are one song: the clicks land on the same
    /// samples whether the song is played a block at a time or bounced.
    #[test]
    fn playback_and_the_export_agree() {
        let map = ramp_up();
        let beats = beats_of(&map);
        let frames = beats[BEATS - 1] + 4_800;
        let live = played(song(map.clone(), true), frames as usize);
        let bounced = exported(song(map, true), frames);
        let heard_live = clicks_near(&live, &beats);
        let heard_bounced = clicks_near(&bounced, &beats);
        assert_eq!(heard_live, heard_bounced);
        let peak_live = live.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let peak_bounced = bounced.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak_live > 0.1, "the live render is heard");
        assert!(
            (peak_live - peak_bounced).abs() < 1e-3,
            "{peak_live} against {peak_bounced}"
        );
    }
}
