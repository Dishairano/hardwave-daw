//! Building the audio graph away from the audio thread.
//!
//! Every edit asks for a new graph: the clips as sample regions, the notes
//! through the tempo map, the automation, the routing, the compensation.
//! For a big song that took 20 to 30 ms, and it ran on the audio thread,
//! where a 512-frame block has 10.7 ms: every edit in a big song dropped
//! audio. It is now built here, on a thread of its own (or on the calling
//! thread for an offline render), and handed to the audio thread finished.
//! The audio thread only swaps it in (`EngineCallback::install_graph`):
//! it moves the plug-ins, the held keys, the voices and the audio in the
//! compensation lines across, and hands the old graph back to be freed
//! here.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// The pair of graph edges that carry one send, so its gain can be moved
/// without rebuilding the graph.
pub(crate) struct SendEdges {
    pub(crate) track_id: String,
    pub(crate) send_index: usize,
    pub(crate) left_edge: usize,
    pub(crate) right_edge: usize,
    /// The send's level as the mixer has it, for a block with no lane.
    pub(crate) base_gain: f32,
    /// A route moved it last block, so it goes back to its level when
    /// the route is taken away.
    pub(crate) modulated: bool,
}

/// The two edges that carry live input to one monitoring track (or to the
/// master, with direct monitoring), and whether that monitoring follows the
/// transport: input while stopped or recording, the track's own audio while
/// playing back.
pub(crate) struct MonitorEdges {
    pub(crate) auto: bool,
    pub(crate) left_edge: usize,
    pub(crate) right_edge: usize,
}

/// What one track's plug-in chain is, as the audio thread last saw it.
///
/// The chains live on the audio thread (plug-ins are added and removed
/// there), so a graph built elsewhere cannot look at them. The audio thread
/// writes these after every swap and every plug-in change, and the next
/// build reads them: a track holding plug-ins keeps its node, and the
/// compensation is worked out with each chain's delay.
#[derive(Default)]
pub(crate) struct ChainBoard {
    pub(crate) latency: AtomicU32,
    pub(crate) slots: AtomicU32,
}

/// What a node in a new graph takes over from the graph it replaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CarryKind {
    /// An audio track's node: its chain, unless the track is frozen, in
    /// which case the chain is parked.
    Audio { frozen: bool },
    /// An instrument track's node: its chain and the keys held on it.
    Midi,
    /// The master: its chain.
    Master,
}

/// One node that carries on from the node with the same track id.
pub(crate) struct Carry {
    pub(crate) track_id: String,
    pub(crate) node: crate::graph::NodeId,
    pub(crate) kind: CarryKind,
}

/// A graph built and ready to be swapped in, with everything the audio
/// thread keeps beside it. After the swap the same box holds the graph it
/// replaced, and goes back to be freed.
pub(crate) struct PreparedGraph {
    /// Which request this answers (see `BuildRequests`).
    pub(crate) seq: u64,
    pub(crate) graph: AudioGraph,
    pub(crate) master_id: Option<crate::graph::NodeId>,
    pub(crate) track_id_to_node: HashMap<String, crate::graph::NodeId>,
    pub(crate) send_edges: Vec<SendEdges>,
    pub(crate) monitor_edges: Vec<MonitorEdges>,
    pub(crate) midi_track_nodes: Vec<(String, crate::graph::NodeId)>,
    pub(crate) has_monitored_input: bool,
    /// The nodes that take a chain, held keys and sound over, in node
    /// order.
    pub(crate) carries: Vec<Carry>,
    /// The frozen tracks, each with an empty chain to park its plug-ins in.
    /// Made here so parking one on the audio thread never grows the map.
    pub(crate) parked_chains: HashMap<String, crate::insert_chain::InsertChain>,
    /// Which plug-in slots have a sidechain routed to them, set on the
    /// chains once they are in their new nodes.
    pub(crate) sidechain_flags: Vec<(crate::graph::NodeId, String, bool)>,
    /// Per node: where its chain's delay and size are published.
    pub(crate) boards: Vec<Option<Arc<ChainBoard>>>,
    /// Per node: the chain delay the compensation was worked out with.
    pub(crate) assumed_latency: Vec<u32>,
    /// The worker pool setting was read; when it was busy the graph keeps
    /// the pool the old one had.
    pub(crate) pool_read: bool,
    /// The tempo map the graph was built with: where the audio thread
    /// finds ticks, tempo and clicks for a block in which the project is
    /// busy (being edited, or read for the next graph).
    pub(crate) tempo_map: hardwave_project::TempoMap,
}

impl PreparedGraph {
    fn new(seq: u64, buffer_size: usize) -> Self {
        Self {
            seq,
            graph: AudioGraph::new(buffer_size),
            master_id: None,
            track_id_to_node: HashMap::new(),
            send_edges: Vec::new(),
            monitor_edges: Vec::new(),
            midi_track_nodes: Vec::new(),
            has_monitored_input: false,
            carries: Vec::new(),
            parked_chains: HashMap::new(),
            sidechain_flags: Vec::new(),
            boards: Vec::new(),
            assumed_latency: Vec::new(),
            pool_read: false,
            tempo_map: hardwave_project::TempoMap::default(),
        }
    }
}

/// Everything a graph is built from. Shared handles only, so the builder
/// thread holds a copy and nothing it reads belongs to the audio thread.
#[derive(Clone)]
pub(crate) struct GraphBuilder {
    pub(crate) project: Arc<Mutex<Project>>,
    pub(crate) track_meters: TrackMeterMap,
    pub(crate) transport: TransportState,
    pub(crate) audio_pool: AudioPool,
    pub(crate) audio_prefs: crate::audio_prefs::AudioPrefs,
    /// The launcher grid, shared with the app so a clip pressed in the
    /// UI reaches the track node that plays it.
    pub(crate) session: Arc<crate::session::SessionState>,
    /// The threads that share the per-block work, when the setting asks
    /// for them. Picked up at rebuild, never started on the audio thread.
    pub(crate) worker_pool: Arc<Mutex<Option<Arc<crate::parallel::WorkerPool>>>>,
    /// The reference track handles, handed to the master node at every
    /// rebuild so comparing against a record survives one.
    #[allow(clippy::type_complexity)]
    pub(crate) reference: Arc<Mutex<Option<Arc<crate::audio_pool::AudioBuffer>>>>,
    pub(crate) reference_on: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) reference_gain_centi_db: Arc<std::sync::atomic::AtomicI64>,
    pub(crate) input_consumer: SharedInputConsumer,
    /// One block of every interface input, drained by the input hub and
    /// read by each input node. Made once, at the largest block size, so
    /// the audio thread never grows it.
    pub(crate) input_block: SharedInputBlock,
    /// Shared with `DawEngine.captures`. Forwarded into the input nodes and
    /// record taps each rebuild so a recording survives graph rebuilds.
    pub(crate) captures: Arc<[Arc<CaptureTap>]>,
    pub(crate) sample_rate: u32,
    pub(crate) buffer_size: usize,
    /// One board per track (and the master), kept from build to build.
    pub(crate) boards: HashMap<String, Arc<ChainBoard>>,
}

impl GraphBuilder {
    /// The board a track's chain is published on, made the first time.
    fn board(&mut self, track_id: &str) -> Arc<ChainBoard> {
        if let Some(board) = self.boards.get(track_id) {
            return Arc::clone(board);
        }
        let board = Arc::new(ChainBoard::default());
        self.boards.insert(track_id.to_string(), Arc::clone(&board));
        board
    }

    /// Whether the audio thread holds plug-ins for this track.
    fn holds_chain(&self, track_id: &str) -> bool {
        self.boards
            .get(track_id)
            .is_some_and(|b| b.slots.load(Ordering::Relaxed) > 0)
    }

    /// The delay of a track's chain as the audio thread last published it.
    fn chain_latency(&self, track_id: &str) -> u32 {
        self.boards
            .get(track_id)
            .map(|b| b.latency.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Build the graph for `project`, with the meter map held.
    ///
    /// Everything that costs time happens here: every node made, every
    /// clip and note placed in samples, every edge and compensation line.
    /// What is left for the audio thread is moving the plug-ins and the
    /// sound over (`EngineCallback::install_graph`).
    pub(crate) fn build(
        &mut self,
        project: &Project,
        meters: &mut HashMap<String, Arc<TrackMeterState>>,
        seq: u64,
    ) -> Box<PreparedGraph> {
        let mut p = Box::new(PreparedGraph::new(seq, self.buffer_size));
        // Whether the work is shared across cores is a setting, and a
        // rebuild is where it is picked up. A busy lock keeps the pool the
        // graph had, and the next rebuild gets it.
        if let Some(pool) = self.worker_pool.try_lock() {
            p.graph.set_worker_pool(pool.clone());
            p.pool_read = true;
        }
        // Boards for tracks that are gone go too.
        self.boards
            .retain(|id, _| project.tracks.iter().any(|t| &t.id == id));

        let sample_rate = self.sample_rate as f64;
        let tempo_map = &project.tempo_map;
        p.tempo_map = tempo_map.clone();

        // Parallel map from project track id to graph node id, needed to
        // connect send edges once every track node has been inserted.
        let mut track_id_to_node: HashMap<String, crate::graph::NodeId> = HashMap::new();

        // While any track is soloed, the tracks not heard are muted: the
        // soloed ones are heard with their way out and what feeds them.
        let solo_audible = hardwave_project::mixer::solo_audible(&project.tracks);
        let silenced_by_solo = |track: &hardwave_project::track::Track| {
            solo_audible
                .as_ref()
                .is_some_and(|heard| !heard.contains(&track.id) && !track.solo_safe)
        };
        // Playlist rows switched off, or not soloed while one is: what sits
        // on them (clips, notes, automation) is left out below. Rows never
        // route sound, so the channels and their effects keep running.
        let silent_rows = hardwave_project::layout::SilentRows::of(project);
        let audible_lanes = |track: &hardwave_project::track::Track| {
            track
                .automation_lanes
                .iter()
                .filter(|lane| !silent_rows.lane(lane))
                .cloned()
                .collect::<Vec<_>>()
        };
        // Automation clips that move another track's knob (FL's automation
        // clip channels), gathered once the way MIDI routes are: the target
        // track's node plays them as its own. A clip with no target moves
        // the track it sits on, as every clip did before.
        let mut handed: HashMap<&str, Vec<&hardwave_project::automation_clip::AutomationClip>> =
            HashMap::new();
        for source in &project.tracks {
            for clip in &source.automation_clips {
                if let Some(target) = clip.target_track.as_deref() {
                    if target != source.id && !silent_rows.automation_clip(clip) {
                        handed.entry(target).or_default().push(clip);
                    }
                }
            }
        }
        let audible_automation_clips = |track: &hardwave_project::track::Track| {
            let own = track.automation_clips.iter().filter(|clip| {
                clip.target_track.as_deref().is_none_or(|t| t == track.id)
                    && !silent_rows.automation_clip(clip)
            });
            let theirs = handed.get(track.id.as_str()).into_iter().flatten().copied();
            own.chain(theirs).cloned().collect::<Vec<_>>()
        };

        // Reconcile the per-track meter map with the current tracks: reuse existing
        // Arcs where possible, create new ones, drop meters for removed tracks.
        // (`meters` guard acquired via try_lock at the top of this fn.)
        let live_ids: std::collections::HashSet<String> = project
            .tracks
            .iter()
            .filter(|t| t.kind.is_audio_bearing())
            .map(|t| t.id.clone())
            .collect();
        meters.retain(|id, _| live_ids.contains(id));

        // MIDI routes, gathered once: for each target track, the tracks
        // whose notes it also plays. A route to a track that has been
        // deleted simply never appears here.
        let mut midi_routes: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, source) in project.tracks.iter().enumerate() {
            if !matches!(source.kind, hardwave_project::TrackKind::Midi) {
                continue;
            }
            for target in &source.midi_route_to {
                if target == &source.id {
                    continue;
                }
                midi_routes.entry(target.clone()).or_default().push(index);
            }
        }

        // Who hears a MIDI keyboard: the armed tracks when any are armed,
        // otherwise the one MIDI track picked last (the first one in a new
        // song). Every MIDI track used to take it, so a keyboard played all
        // of them at once.
        let armed_for_input = project.tracks.iter().any(|t| {
            t.armed && t.monitor_input && !matches!(t.kind, hardwave_project::TrackKind::Master)
        });
        let keyboard_track = keyboard_track_of(project);
        // Tracks something reaches: another track's output, a send, a MIDI
        // route. A track nothing reaches and that has nothing of its own is
        // an empty insert, and gets no node (see `is_placeholder`).
        let mut reached: std::collections::HashSet<&str> = std::collections::HashSet::new();
        // Of those, the ones sound plays into: in a song with FL's inserts
        // they pass a centred sound at 0 dB (`Project::inserts_unity_at_centre`).
        let mut inserts: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for t in &project.tracks {
            if let Some(bus) = t.output_bus.as_deref() {
                reached.insert(bus);
                inserts.insert(bus);
            }
            for send in t.sends.iter().filter(|s| s.enabled) {
                reached.insert(send.target.as_str());
                inserts.insert(send.target.as_str());
            }
            for target in &t.midi_route_to {
                reached.insert(target.as_str());
            }
        }

        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }

            let meter = meters
                .entry(track.id.clone())
                .or_insert_with(|| Arc::new(TrackMeterState::default()))
                .clone();

            // An empty insert makes no sound, and building its node was most
            // of a rebuild: a new song has 500 of them, and building them all
            // took about 14 ms on the audio thread, longer than a block, so
            // every rebuild was a click. Its meter shows silence.
            if is_placeholder(track, &reached) && !self.holds_chain(&track.id) {
                meter.park();
                continue;
            }

            // MIDI tracks get an entirely separate node type — a built-in
            // monosynth that converts pre-resolved note schedules into
            // audio. The TrackNode path below is for audio-bearing tracks
            // that play sample clips, which can't service MIDI content.
            // A frozen MIDI track is audio now: it falls through to the
            // TrackNode path below, which plays the render. Building the
            // instrument node as well would play the part twice.
            if matches!(track.kind, hardwave_project::TrackKind::Midi) && !track.frozen {
                let mut midi_node = crate::midi_track_node::MidiTrackNode::new(
                    track.id.clone(),
                    track.name.clone(),
                    meter.clone(),
                );
                midi_node.set_prefs(self.audio_prefs.clone());
                let (vca_gain_db, vca_muted) =
                    hardwave_project::vca::offset_for(&project.vcas, &track.id);
                midi_node.set_volume_db(track.volume_db + vca_gain_db);
                midi_node.set_pan(track.pan);
                midi_node.set_automation_lanes(audible_lanes(track));
                midi_node.set_automation_clips(audible_automation_clips(track));
                midi_node.set_stereo_separation(track.stereo_separation);
                // The mixer strip's polarity, swap and delay and the rack's
                // filter, as an audio track has them.
                midi_node.set_phase_invert(track.phase_invert);
                midi_node.set_swap_lr(track.swap_lr);
                midi_node.set_delay_samples(track.delay_samples);
                midi_node.set_filter(
                    crate::track_node::TrackFilterType::parse(&track.filter_type),
                    track.filter_cutoff_hz,
                    track.filter_resonance,
                    self.sample_rate as f32,
                );
                // The live mix starts from what the project says.
                meter.set_vca_gain(db_to_linear_f32(vca_gain_db));
                meter.set_mix(
                    db_to_linear_f32(track.volume_db + vca_gain_db),
                    track.pan as f32,
                    track.stereo_separation as f32,
                );
                let effective_mute_midi = track.muted || vca_muted || silenced_by_solo(track);
                midi_node.set_muted(effective_mute_midi);
                midi_node.set_soloed(track.soloed);
                // Map the project's NativeInstrument enum to the
                // engine's Instrument enum (they're separate so the
                // engine doesn't take a project-crate dep cycle).
                use crate::midi_track_node::Instrument as EngInstr;
                use crate::midi_track_node::Waveform;
                use hardwave_project::track::NativeInstrument as ProjInstr;
                let (eng_instr, waveform) = match track.instrument {
                    ProjInstr::BuiltinSine => (EngInstr::BuiltinSine, Waveform::Sine),
                    ProjInstr::BuiltinSaw => (EngInstr::BuiltinSine, Waveform::Saw),
                    ProjInstr::BuiltinSquare => (EngInstr::BuiltinSine, Waveform::Square),
                    ProjInstr::BuiltinTriangle => (EngInstr::BuiltinSine, Waveform::Triangle),
                    ProjInstr::KickSynth => (EngInstr::KickSynth, Waveform::Sine),
                };
                midi_node.set_instrument(eng_instr, self.sample_rate as f32);
                midi_node.reserve_block(self.buffer_size);
                midi_node.set_waveform(waveform);
                if matches!(track.instrument, ProjInstr::KickSynth)
                    && track.kick_patch.has_overrides()
                {
                    midi_node.apply_kick_patch(&track.kick_patch);
                }

                // Walk the clip placements and turn each MIDI note into a
                // sample-positioned MidiNoteRegion. Tempo-map lookups happen
                // here, off the audio thread, so the audio thread sees a flat
                // sorted list.
                let mut note_regions: Vec<crate::midi_track_node::MidiNoteRegion> = Vec::new();
                let mut control_regions: Vec<crate::midi_track_node::MidiControlRegion> =
                    Vec::new();
                // The track's own clips, plus the clips of every track
                // routed into it. A routed clip plays this track's
                // instrument through this track's chain, which is what
                // layering two synths off one part means.
                let routed = midi_routes.get(&track.id).cloned().unwrap_or_default();
                // In pattern mode the channel plays the channel rack's
                // active pattern instead of the playlist, as in FL. The
                // steps were saved and never read, so the step sequencer
                // made no sound at all.
                let pattern_mode = self
                    .transport
                    .pattern_mode
                    .load(std::sync::atomic::Ordering::Relaxed);
                if let Some(rack) = &project.step_rack {
                    self.transport
                        .pattern_len_ticks
                        .store(rack.length_ticks(), std::sync::atomic::Ordering::Relaxed);
                }
                let pattern_clip = if pattern_mode {
                    project.step_rack.as_ref().map(|rack| {
                        let notes: Vec<hardwave_midi::MidiNote> = rack
                            .notes_for(&track.id)
                            .into_iter()
                            .map(|n| hardwave_midi::MidiNote {
                                start_tick: n.tick,
                                duration_ticks: n.duration_ticks,
                                pitch: n.pitch,
                                velocity: n.velocity,
                                pan: n.pan,
                                fine_cents: n.fine_cents,
                                release_velocity: n.release_velocity,
                                ..Default::default()
                            })
                            .collect();
                        hardwave_project::clip::ClipPlacement {
                            content: hardwave_project::clip::ClipContent::Midi(
                                hardwave_project::clip::MidiClipRef {
                                    id: "pattern".into(),
                                    clip: hardwave_midi::MidiClip {
                                        notes,
                                        length_ticks: rack.length_ticks(),
                                        ..hardwave_midi::MidiClip::new(
                                            "pattern".into(),
                                            "Pattern".into(),
                                            rack.length_ticks(),
                                        )
                                    },
                                },
                            ),
                            track_id: track.id.clone(),
                            position_ticks: 0,
                            length_ticks: rack.length_ticks(),
                            lane: 0,
                            color: None,
                            row: None,
                        }
                    })
                } else {
                    None
                };
                let pattern_clips = pattern_clip.map(|c| vec![c]);
                let clip_sources: Box<
                    dyn Iterator<Item = &Vec<hardwave_project::clip::ClipPlacement>>,
                > = match &pattern_clips {
                    Some(p) => Box::new(std::iter::once(p)),
                    None => Box::new(
                        std::iter::once(&track.clips)
                            .chain(routed.iter().map(|index| &project.tracks[*index].clips)),
                    ),
                };
                for clip in clip_sources.flatten() {
                    let hardwave_project::clip::ClipContent::Midi(midi_ref) = &clip.content else {
                        continue;
                    };
                    // A muted clip contributes nothing, the same way a muted
                    // audio clip is skipped; nor does one on a silent row.
                    if midi_ref.clip.muted || silent_rows.clip(clip) {
                        continue;
                    }
                    // MIDI effects sit between the clip and the
                    // instrument. With an empty chain this is the clip's
                    // own notes, which is every project written before
                    // the chain existed.
                    let heard = if track.midi_fx.is_empty() {
                        std::borrow::Cow::Borrowed(&midi_ref.clip.notes)
                    } else {
                        std::borrow::Cow::Owned(hardwave_midi::midi_fx::apply_chain(
                            &midi_ref.clip.notes,
                            &track.midi_fx,
                        ))
                    };
                    // Where the clip on the playlist ends, once it has been
                    // made shorter than what it holds (shrunk from the right,
                    // or the left half of a split): nothing after it plays
                    // and a note held over it is let go there. A clip as long
                    // as its content or longer plays all of it, so a song
                    // with notes drawn past a clip's end sounds as it did.
                    let window_end = (clip.length_ticks < midi_ref.clip.length_ticks)
                        .then_some(clip.length_ticks);
                    for note in heard.iter() {
                        let mut duration = note.duration_ticks.max(1);
                        if let Some(end) = window_end {
                            if note.start_tick >= end {
                                continue;
                            }
                            duration = duration.min(end - note.start_tick);
                        }
                        let on_tick = clip.position_ticks + note.start_tick;
                        let off_tick = on_tick + duration;
                        let note_on = tempo_map.tick_to_samples(on_tick, sample_rate);
                        let note_off = tempo_map.tick_to_samples(off_tick, sample_rate);
                        note_regions.push(crate::midi_track_node::MidiNoteRegion {
                            note_on_sample: note_on,
                            note_off_sample: note_off,
                            pitch: note.pitch,
                            velocity: note.velocity,
                            muted: note.muted,
                            pan: note.pan,
                            fine_cents: note.fine_cents,
                            release_velocity: note.release_velocity,
                            channel: note.channel & 0x0F,
                        });
                    }
                    // Controller movements: mod wheel, sustain, bend. Mapped
                    // through the tempo map here, like the notes, and kept
                    // inside the clip so a clip moved on the playlist takes
                    // them with it.
                    for c in &midi_ref.clip.controls {
                        if window_end.is_some_and(|end| c.tick >= end) {
                            continue;
                        }
                        let tick = clip.position_ticks + c.tick.min(midi_ref.clip.length_ticks);
                        control_regions.push(crate::midi_track_node::MidiControlRegion {
                            sample: tempo_map.tick_to_samples(tick, sample_rate),
                            channel: c.channel,
                            kind: c.kind,
                            value: c.value,
                        });
                    }
                }
                // Live input goes through the same chain the clips did.
                midi_node.set_midi_fx(track.midi_fx.clone());
                // A tuning other than twelve equal, worked out here so
                // the audio thread only ever reads a table.
                midi_node.set_tuning(track.tuning.as_ref().map(|t| Box::new(t.table())));
                midi_node.set_mpe(track.mpe);
                // MPE out: each note a member channel of its own, so its
                // detune can be its own bend.
                if track.mpe {
                    crate::midi_track_node::assign_mpe_channels(&mut note_regions);
                }
                midi_node.set_notes(note_regions);
                // The channel's audio clips: a consolidated part, a sample
                // dropped on it. They used to be silent here, so a
                // consolidated kick on a MIDI channel played nothing.
                let midi_pitch_factor = 2.0_f64.powf(
                    (track.pitch_semitones as f64) / 12.0 + (track.fine_tune_cents as f64) / 1200.0,
                );
                let audio = self.audio_regions(
                    track,
                    tempo_map,
                    sample_rate,
                    midi_pitch_factor,
                    &silent_rows,
                );
                if !audio.is_empty() {
                    midi_node.set_audio_clips(self.audio_pool.clone(), audio);
                }
                midi_node.set_controls(control_regions);
                // This track's row of the launcher grid: its MIDI loops, in
                // samples from each loop's start. Lengths are in ticks in
                // the project so they follow the tempo.
                if !track.session_slots.is_empty() {
                    let samples_per_tick =
                        tempo_map.tick_to_samples(hardwave_midi::PPQ, sample_rate) as f64
                            / hardwave_midi::PPQ as f64;
                    let at = |tick: u64| (tick as f64 * samples_per_tick).round() as u64;
                    let slots = track
                        .session_slots
                        .iter()
                        .map(|slot| {
                            let slot = slot.as_ref()?;
                            let notes = slot.notes.as_ref()?;
                            let length = slot.length_ticks.max(1);
                            let mut regions: Vec<crate::midi_track_node::MidiNoteRegion> = notes
                                .iter()
                                .filter(|n| n.start_tick < length)
                                .map(|n| {
                                    // A note held past the loop's end is let go there.
                                    let off = (n.start_tick + n.duration_ticks.max(1)).min(length);
                                    crate::midi_track_node::MidiNoteRegion {
                                        note_on_sample: at(n.start_tick),
                                        note_off_sample: at(off),
                                        pitch: n.pitch,
                                        velocity: n.velocity,
                                        muted: n.muted,
                                        pan: n.pan,
                                        fine_cents: n.fine_cents,
                                        release_velocity: n.release_velocity,
                                        channel: n.channel & 0x0F,
                                    }
                                })
                                .collect();
                            regions.sort_by_key(|r| r.note_on_sample);
                            Some(crate::midi_track_node::SessionMidiClip {
                                length_samples: at(length),
                                notes: regions,
                            })
                        })
                        .collect();
                    midi_node.set_session_midi(
                        slots,
                        self.session.track(&track.id),
                        Arc::clone(&self.session.quantise_beats),
                    );
                }

                // Its plug-ins, carried over from the node this rebuild
                // replaces, as an audio track's are below. They were not:
                // every rebuild after a plug-in went on an instrument track
                // (a note drawn, a track added, a plug-in added, which itself
                // rebuilds) sent the chain to the graveyard, so instruments
                // and effects on MIDI tracks went silent and the built-in
                // synth played in their place.
                // The keys held on a live keyboard go with them, so the new
                // node lets them go and an arpeggio goes on. Both are moved
                // over when the graph is swapped in.
                let node_id = p.graph.add_node(Box::new(midi_node));
                p.carries.push(Carry {
                    track_id: track.id.clone(),
                    node: node_id,
                    kind: CarryKind::Midi,
                });
                // A track armed for input hears the keyboard; with none
                // armed, the keyboard plays the track picked last, so a new
                // MIDI track still answers a controller without arming.
                let accepts = hears_keyboard(track, armed_for_input, keyboard_track.as_deref());
                p.graph.set_accepts_live_midi(node_id, accepts);
                p.midi_track_nodes.push((track.id.clone(), node_id));
                track_id_to_node.insert(track.id.clone(), node_id);
                continue;
            }

            let mut node = TrackNode::new(
                track.id.clone(),
                track.name.clone(),
                self.audio_pool.clone(),
                meter,
            );
            // Snapshot the project's automation lanes for this track
            // onto the audio-thread node. Cloned here, off the audio thread,
            // so process() never has to walk the project tree under a
            // lock. Empty list is the no-automation steady state.
            node.set_prefs(self.audio_prefs.clone());
            node.set_unity_centre(
                project.inserts_unity_at_centre
                    && (matches!(
                        track.kind,
                        hardwave_project::TrackKind::Bus | hardwave_project::TrackKind::Return
                    ) || inserts.contains(track.id.as_str())),
            );
            node.set_automation_lanes(audible_lanes(track));
            node.set_automation_clips(audible_automation_clips(track));
            // The plug-in chain is moved over from the node this one
            // replaces when the graph is swapped in. Tracks that didn't
            // exist before keep the default-empty chain TrackNode::new gave
            // them.
            //
            // A frozen track is the exception: the render already went
            // through the chain, so running it again would apply every
            // plug-in twice and cost the CPU freezing was meant to give
            // back. The chain is parked, not dropped, so unfreezing puts it
            // back untouched.
            if track.frozen {
                p.parked_chains
                    .insert(track.id.clone(), crate::insert_chain::InsertChain::new());
            }
            // A VCA group adds its level to the member's own fader and can
            // silence it, without moving anything in the signal path.
            let (vca_gain_db, vca_muted) =
                hardwave_project::vca::offset_for(&project.vcas, &track.id);
            node.set_volume_db(track.volume_db + vca_gain_db);
            node.set_pan(track.pan);
            let effective_mute = track.muted || vca_muted || silenced_by_solo(track);
            node.set_muted(effective_mute);
            node.set_soloed(track.soloed);
            node.set_phase_invert(track.phase_invert);
            node.set_swap_lr(track.swap_lr);
            node.set_stereo_separation(track.stereo_separation);
            // The live mix starts from what the project says.
            if let Some(meter) = meters.get(&track.id) {
                meter.set_vca_gain(db_to_linear_f32(vca_gain_db));
                meter.set_mix(
                    db_to_linear_f32(track.volume_db + vca_gain_db),
                    track.pan as f32,
                    track.stereo_separation as f32,
                );
            }
            node.set_delay_samples(track.delay_samples);
            node.set_frozen(track.frozen);
            let filter_kind = crate::track_node::TrackFilterType::parse(&track.filter_type);
            node.set_filter(
                filter_kind,
                track.filter_cutoff_hz,
                track.filter_resonance,
                sample_rate as f32,
            );

            // Per-track coarse + fine pitch offset. Combined as a single resample
            // factor that's folded into each clip's source_step below. A
            // freeze render already has it: played through it again, a
            // frozen track came out transposed twice and out of time.
            let track_pitch_offset =
                (track.pitch_semitones as f64) / 12.0 + (track.fine_tune_cents as f64) / 1200.0;
            let track_pitch_factor = if track.frozen {
                1.0
            } else {
                2.0_f64.powf(track_pitch_offset)
            };

            let regions = self.audio_regions(
                track,
                tempo_map,
                sample_rate,
                track_pitch_factor,
                &silent_rows,
            );
            node.set_clips(regions);

            // This track's row of the launcher grid. Lengths are in
            // ticks in the project so they follow the tempo, and in
            // samples here because the audio thread counts samples.
            if !track.session_slots.is_empty() {
                let tempo_map = &project.tempo_map;
                let samples_per_tick = tempo_map.tick_to_samples(hardwave_midi::PPQ, sample_rate)
                    as f64
                    / hardwave_midi::PPQ as f64;
                let slots: Vec<Option<crate::track_node::SessionClip>> = track
                    .session_slots
                    .iter()
                    .map(|slot| {
                        // A MIDI loop has no audio to play on an audio track.
                        slot.as_ref().filter(|s| s.notes.is_none()).map(|s| {
                            crate::track_node::SessionClip {
                                source_id: s.source_path.clone(),
                                source_offset: s.source_start,
                                length_samples: (s.length_ticks as f64 * samples_per_tick) as u64,
                                gain: if s.gain_db <= -100.0 {
                                    0.0
                                } else {
                                    10f64.powf(s.gain_db / 20.0) as f32
                                },
                            }
                        })
                    })
                    .collect();
                node.set_session_slots(slots);
                node.set_session(
                    self.session.track(&track.id),
                    Arc::clone(&self.session.quantise_beats),
                );
            }

            let node_id = p.graph.add_node(Box::new(node));
            // Audio tracks accept live MIDI only when explicitly armed
            // for input — common pattern for routing a controller into
            // a synth plug-in loaded on an audio-bearing track. Hardware
            // events flow through the insert chain to that plug-in.
            p.graph
                .set_accepts_live_midi(node_id, track.armed && track.monitor_input);
            track_id_to_node.insert(track.id.clone(), node_id);
            p.carries.push(Carry {
                track_id: track.id.clone(),
                node: node_id,
                kind: CarryKind::Audio {
                    frozen: track.frozen,
                },
            });
        }

        // Add master node (reads volume from shared transport atomic — no rebuild on change)
        let master_track_id = project
            .tracks
            .iter()
            .find(|t| matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .map(|t| t.id.clone());
        let master_node = MasterNode::new_with_reference(
            Arc::clone(&self.transport.master_volume_db),
            master_track_id.clone(),
            Arc::clone(&self.reference),
            Arc::clone(&self.reference_on),
            Arc::clone(&self.reference_gain_centi_db),
        );
        let master_id = p.graph.add_node(Box::new(master_node));
        p.master_id = Some(master_id);

        // Point the Master track's id at the master node so its insert chain is
        // reachable, and carry the live chain across the rebuild the same way
        // track chains are carried.
        //
        // Master isn't audio-bearing, so it gets no `TrackNode` and never
        // landed in this map — every `InsertCommand` aimed at the master strip
        // resolved to no node and was dropped, and `hydrate_offline_inserts`
        // skipped it on export too. A plug-in added to the master was persisted
        // in the project and then silently processed nothing, anywhere.
        if let Some(mid) = &master_track_id {
            track_id_to_node.insert(mid.clone(), master_id);
            p.carries.push(Carry {
                track_id: mid.clone(),
                node: master_id,
                kind: CarryKind::Master,
            });
        }

        // Connect each track either to its configured output_bus (another
        // track) or to master. Invalid targets (self-routing, unknown id,
        // cycles) silently fall back to master — the command layer already
        // rejects bad values, but the engine is defensive to keep audio
        // flowing even if a legacy project carries stale routing.
        // Tracks another track's output lands on. They are buses, and are
        // told so below together with the send targets.
        let mut bus_targets: Vec<String> = Vec::new();
        // Where each track's output goes, looked up by id while walking a
        // chain of buses. Searching the track list for every step made the
        // walk grow with the square of the track count. The first track
        // with an id wins, as the search did.
        let mut output_of: HashMap<&str, Option<&str>> =
            HashMap::with_capacity(project.tracks.len());
        for t in &project.tracks {
            output_of
                .entry(t.id.as_str())
                .or_insert(t.output_bus.as_deref());
        }
        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }
            // Stem renders exclude every track but the target from the mix.
            // The node still exists and still processes, so it can key a
            // sidechain — it just doesn't reach the output.
            if track.stem_excluded {
                continue;
            }
            let Some(&src_node) = track_id_to_node.get(&track.id) else {
                continue;
            };
            let dst_node = match track.output_bus.as_ref() {
                Some(bus_id) if bus_id != &track.id => {
                    // Walk the chain from the bus target; if the chain comes
                    // back to the current track it's a cycle, fall through to
                    // master. Otherwise use the configured bus when it maps to
                    // a real graph node.
                    let mut visited: std::collections::HashSet<&str> =
                        std::collections::HashSet::new();
                    let mut cursor: Option<&str> = Some(bus_id.as_str());
                    let mut cycle = false;
                    while let Some(next) = cursor {
                        if next == track.id {
                            cycle = true;
                            break;
                        }
                        if !visited.insert(next) {
                            break;
                        }
                        cursor = output_of.get(next).copied().flatten();
                    }
                    match track_id_to_node.get(bus_id).copied() {
                        Some(node) if !cycle => {
                            bus_targets.push(bus_id.clone());
                            node
                        }
                        _ => master_id,
                    }
                }
                _ => master_id,
            };
            p.graph.connect(src_node, 0, dst_node, 0);
            p.graph.connect(src_node, 1, dst_node, 1);
        }

        // Wire the live input to a single InputNode that drains the input ring
        // buffer. Two separate questions decide what happens:
        //
        //   armed          -> the input has to reach the recording tap
        //   monitor_input  -> the input also has to be audible
        //
        // The node used to be built only for armed AND monitoring tracks, and
        // the recording tap lives on that node, so arming a track without
        // switching monitoring on recorded silence: press record, play, get
        // an empty take and no error. Arming alone is now enough to capture.
        //
        // Each armed track records its own input: a channel or a pair of the
        // interface, or another track's output. Every source gets a capture
        // of its own, in the order `record_sources` gives, which is the order
        // `start_capture` armed them in.
        let default_input = self
            .input_consumer
            .try_lock()
            .and_then(|g| g.as_ref().map(|r| r.default))
            .unwrap_or(InputChannel {
                first: 0,
                mono: false,
            });
        let mut sources = project.record_sources(default_input);
        sources.truncate(MAX_RECORD_SOURCES);
        // Each monitoring track with its input and whether it follows the
        // transport (`monitor_auto`), and every track the live input passes
        // through.
        let mut monitor_routes: Vec<(crate::graph::NodeId, InputChannel, bool)> = Vec::new();
        let mut live_tracks: Vec<&str> = Vec::new();
        // An armed instrument track plays the keyboard live, so any armed
        // track keeps the graph running while stopped; only audio tracks
        // record and monitor the interface.
        let any_armed = project
            .tracks
            .iter()
            .any(|t| t.armed && t.kind.is_audio_bearing());
        for track in project.record_armed() {
            // A track recording another track's output takes nothing from
            // the interface, so there is no input to hear on it.
            let RecordSource::Interface(pick) = project.record_source_of(track, default_input)
            else {
                continue;
            };
            if track.monitor_input {
                if let Some(&node_id) = track_id_to_node.get(&track.id) {
                    monitor_routes.push((node_id, pick, track.monitor_auto));
                    live_tracks.push(&track.id);
                }
            }
        }
        p.has_monitored_input = any_armed;
        if any_armed {
            let direct = self
                .transport
                .direct_monitoring
                .load(std::sync::atomic::Ordering::Relaxed);
            // The hub drains the interface's input once per block; each input
            // node reads its channels from what the hub drained, and the
            // edge from the hub makes sure it has run first.
            let hub = p.graph.add_node(Box::new(InputHub::new(
                Arc::clone(&self.input_consumer),
                Arc::clone(&self.input_block),
            )));
            let mut input_nodes: Vec<(InputChannel, crate::graph::NodeId)> = Vec::new();
            for (slot, source) in sources.iter().enumerate() {
                let capture = Arc::clone(&self.captures[slot]);
                match source {
                    RecordSource::Interface(pick) => {
                        let mut node = InputNode::new(Arc::clone(&self.input_block), *pick);
                        node.set_capture(Some(capture));
                        let id = p.graph.add_node(Box::new(node));
                        p.graph.connect(hub, 0, id, 0);
                        input_nodes.push((*pick, id));
                    }
                    // Recording another track's output (resampling): a tap on
                    // that node's output feeds this capture.
                    RecordSource::Track(source_id) => {
                        if let Some(&source_node) = track_id_to_node.get(source_id) {
                            let tap = p.graph.add_node(Box::new(RecordTapNode::new(capture)));
                            p.graph.connect(source_node, 0, tap, 0);
                            p.graph.connect(source_node, 1, tap, 1);
                        }
                    }
                }
            }
            // Past the most inputs that record at once, a track still hears
            // its input; it just has no capture.
            for &(_, pick, _) in &monitor_routes {
                if !input_nodes.iter().any(|(p, _)| *p == pick) {
                    let id = p.graph.add_node(Box::new(InputNode::new(
                        Arc::clone(&self.input_block),
                        pick,
                    )));
                    p.graph.connect(hub, 0, id, 0);
                    input_nodes.push((pick, id));
                }
            }
            let node_of = |pick: InputChannel| {
                input_nodes
                    .iter()
                    .find(|(p, _)| *p == pick)
                    .map(|&(_, id)| id)
            };
            if monitor_routes.is_empty() {
                // Armed but not monitoring: the input nodes exist only to
                // feed the recording, so they connect to nothing. Wiring them
                // to master here would make the input audible when the user
                // switched monitoring off.
            } else if direct {
                // Direct monitoring: bypass the track FX chain and route live
                // input straight to master for minimum latency. One path per
                // input, so it follows the transport only when every track
                // monitoring that input does.
                for &(pick, input_id) in &input_nodes {
                    let on_it: Vec<bool> = monitor_routes
                        .iter()
                        .filter(|r| r.1 == pick)
                        .map(|r| r.2)
                        .collect();
                    if on_it.is_empty() {
                        continue;
                    }
                    let auto = on_it.iter().all(|&auto| auto);
                    p.graph.connect(input_id, 0, master_id, 0);
                    let left_edge = p.graph.last_edge_index();
                    p.graph.connect(input_id, 1, master_id, 1);
                    let right_edge = p.graph.last_edge_index();
                    p.monitor_edges.push(MonitorEdges {
                        auto,
                        left_edge,
                        right_edge,
                    });
                }
            } else {
                for (track_node_id, pick, auto) in monitor_routes {
                    let Some(input_id) = node_of(pick) else {
                        continue;
                    };
                    p.graph.connect(input_id, 0, track_node_id, 0);
                    let left_edge = p.graph.last_edge_index();
                    p.graph.connect(input_id, 1, track_node_id, 1);
                    let right_edge = p.graph.last_edge_index();
                    p.monitor_edges.push(MonitorEdges {
                        auto,
                        left_edge,
                        right_edge,
                    });
                }
                // The tracks the input passes through keep running while
                // stopped: each monitoring track, and every bus it reaches
                // through its output or a send.
                let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
                while let Some(id) = live_tracks.pop() {
                    if !seen.insert(id) {
                        continue;
                    }
                    let Some(track) = project.tracks.iter().find(|t| t.id == id) else {
                        continue;
                    };
                    if let Some(&node_id) = track_id_to_node.get(id) {
                        if let Some(node) = p.graph.node_mut(node_id) {
                            node.set_live_input(true);
                        }
                    }
                    live_tracks.extend(track.output_bus.as_deref());
                    live_tracks.extend(
                        track
                            .sends
                            .iter()
                            .filter(|send| send.enabled)
                            .map(|send| send.target.as_str()),
                    );
                }
            }
        }

        // Wire send routing. Each enabled send contributes an extra edge from
        // the source track's pre-fader tap (ports 2/3) or post-fader output
        // (ports 0/1) into the target track's input (ports 0/1), with the
        // send amount applied as per-edge gain.
        // Tracks that something routes into. A bus with nothing of its own on
        // it still has to mix what reaches it.
        // A track routed to another track's output counts the same way:
        // without it, a bus that only had tracks routed to it (no sends, no
        // clips of its own) took the idle path and played nothing.
        let mut receives_input: std::collections::HashSet<String> =
            bus_targets.into_iter().collect();
        for track in &project.tracks {
            // An excluded track's sends are dropped too, so a stem carries only
            // the target's own send tail rather than everyone else's.
            if track.stem_excluded {
                continue;
            }
            let src_node = match track_id_to_node.get(&track.id) {
                Some(&id) => id,
                None => continue,
            };
            for (send_index, send) in track.sends.iter().enumerate() {
                if !send.enabled {
                    continue;
                }
                let dst_node = match track_id_to_node.get(&send.target) {
                    Some(&id) => id,
                    None => continue,
                };
                if dst_node == src_node {
                    continue;
                }
                // A send into something that already plays into this track
                // (a bus it is routed to, through outputs, other sends or a
                // sidechain) would close a loop, and a loop silenced the
                // whole song: the graph runs nothing in or after it. The
                // send is left out and everything else plays.
                if p.graph.reaches(dst_node, src_node) {
                    log::warn!(
                        "send {send_index} from track {} to {} would loop back; left out",
                        track.id,
                        send.target
                    );
                    continue;
                }
                let gain = if send.gain_db <= -100.0 {
                    0.0
                } else {
                    10.0_f64.powf(send.gain_db / 20.0) as f32
                };
                let (src_l, src_r) = if send.pre_fader { (2, 3) } else { (0, 1) };
                p.graph
                    .connect_with_gain(src_node, src_l, dst_node, 0, gain);
                let left_edge = p.graph.last_edge_index();
                p.graph
                    .connect_with_gain(src_node, src_r, dst_node, 1, gain);
                let right_edge = p.graph.last_edge_index();
                // Remember which edges carry this send, so a SendLevel
                // automation lane can move its gain per block. The level used
                // to be baked here at rebuild time, which is why send
                // automation was stored, drawn and never applied.
                receives_input.insert(send.target.clone());
                p.send_edges.push(SendEdges {
                    track_id: track.id.clone(),
                    send_index,
                    left_edge,
                    right_edge,
                    base_gain: gain,
                    modulated: false,
                });
            }
        }

        // Now that every send is known, tell the receiving nodes they are
        // buses. Without this a bus with no clips, no inserts and no
        // automation took the idle path and every send into it was silent.
        for (track_id, &node_id) in track_id_to_node.iter() {
            if !receives_input.contains(track_id) {
                continue;
            }
            if let Some(node) = p.graph.node_mut(node_id) {
                node.set_receives_input(true);
            }
        }

        // Sidechain routing. A plug-in slot with a `sidechain_source` set
        // gets that source track's post-fader output mixed into THIS
        // track node's input ports 2/3 — the sidechain bus, kept separate
        // from the main 0/1 signal. The node's insert chain forwards
        // those channels to the keyed slot so e.g. a compressor ducks the
        // bass against the kick. The per-slot flag is always synced (so
        // clearing a source disables it); multiple slots on one track
        // share the single bus, and distinct sources sum into it.
        for track in &project.tracks {
            let dst_node = match track_id_to_node.get(&track.id) {
                Some(&id) => id,
                None => continue,
            };
            let mut sources = Vec::new();
            for slot in &track.inserts {
                // Set on the chain once it is in its new node.
                let active = slot.sidechain_source.is_some();
                p.sidechain_flags.push((dst_node, slot.id.clone(), active));
                if let Some(src_id) = &slot.sidechain_source {
                    if let Some(&src_node) = track_id_to_node.get(src_id) {
                        if src_node != dst_node && !sources.contains(&src_node) {
                            sources.push(src_node);
                        }
                    }
                }
            }
            for src_node in sources {
                // Keyed from a track this one plays into (its own bus): the
                // key would need this track's sound before it is made. The
                // loop silenced everything, as with a send; it is left out.
                if p.graph.reaches(dst_node, src_node) {
                    log::warn!(
                        "sidechain into track {} from a track it plays into would loop back; left out",
                        track.id
                    );
                    continue;
                }
                p.graph.connect_with_gain(src_node, 0, dst_node, 2, 1.0);
                p.graph.connect_with_gain(src_node, 1, dst_node, 3, 1.0);
            }
        }

        // Persist the track_id → NodeId map so per-track plug-in
        // commands can resolve in O(1) on the audio thread.
        p.track_id_to_node = track_id_to_node;

        // The compensation, worked out with each chain's delay as the audio
        // thread last published it: the chains themselves are moved in at
        // the swap. A chain that turns out to differ asks for another build
        // (`EngineCallback::install_graph`).
        let nodes = p.graph.node_count();
        p.boards = vec![None; nodes];
        p.assumed_latency = vec![0; nodes];
        for carry in &p.carries {
            let board = self.board(&carry.track_id);
            let frozen = matches!(carry.kind, CarryKind::Audio { frozen: true });
            if !frozen {
                p.assumed_latency[carry.node] = self.chain_latency(&carry.track_id);
            }
            p.boards[carry.node] = Some(board);
        }
        p.graph.finalize_pdc_with(
            self.transport
                .pdc_enabled
                .load(std::sync::atomic::Ordering::Relaxed),
            &p.assumed_latency,
        );
        p.graph.index_delays();
        p
    }

    /// Pool id of an already-baked pitch-preserving variant of `source_id`,
    /// or `None` if it hasn't been baked yet.
    ///
    /// LOOKUP ONLY — deliberately never bakes. A graph is built for every
    /// edit, and the bake is an FFT over the whole source plus
    /// multi-megabyte allocations; doing it here would hold up every edit
    /// for as long as the file is big. Baking is
    /// `DawEngine::prebake_stretch_sources`, driven from the UI thread before
    /// the rebuild is signalled. A miss simply falls back to the varispeed
    /// resample path for this rebuild, and the next one picks the bake up.
    fn stretched_source(&self, source_id: &str, stretch: f64, semitones: f64) -> Option<String> {
        let key = stretch_cache_key(source_id, stretch, semitones);
        self.audio_pool.contains(&key).then_some(key)
    }

    /// The pool id a clip plays from before its stretch: its take with the
    /// notes tuned by hand once that render is in the pool, else the file.
    /// LOOKUP ONLY, like `stretched_source`: the render is baked off this
    /// thread.
    fn tuned_source(&self, audio_clip: &hardwave_project::clip::AudioClip) -> String {
        if audio_clip.has_tuned_notes() {
            let key = tune_cache_key(&audio_clip.source_path, &audio_clip.tuned_notes);
            if self.audio_pool.contains(&key) {
                return key;
            }
        }
        audio_clip.source_path.clone()
    }

    /// Creates one TrackNode per non-master track, a MasterNode, and wires them together.
    /// A track's audio clips as the regions its node plays, with fades,
    /// stretch, warp and treatment worked out.
    /// Audio tracks play these in their own node; a MIDI track plays them
    /// beside its synth, so a consolidated part or a dropped sample on a
    /// channel is heard through the same inserts and fader.
    fn audio_regions(
        &self,
        track: &hardwave_project::track::Track,
        tempo_map: &hardwave_project::TempoMap,
        sample_rate: f64,
        track_pitch_factor: f64,
        silent_rows: &hardwave_project::layout::SilentRows,
    ) -> Vec<ClipRegion> {
        // Convert clip placements to sample-based ClipRegions. Clips that
        // overlap each play in full, each with only its own fades, as
        // separate items do in FL. Every overlap used to be turned into a
        // crossfade as long as the overlap: a clap dropped on a long beat
        // faded the beat out from the clap to the beat's end. A crossfade
        // is the clips' own fades, set where the user wants one; a split or
        // a comp leaves its pieces end to end, so they never needed one.
        // Frozen: the render, and nothing else. Live: everything but
        // the render, which is parked on its own lane.
        // A clip on a playlist row that is switched off (or not soloed
        // while another is) is left out, as FL's track switch does.
        let clips_to_play = track.clips.iter().filter(|clip| {
            let is_freeze = clip.lane == hardwave_project::clip::FREEZE_LANE;
            let on_track = if track.frozen { is_freeze } else { !is_freeze };
            on_track && !silent_rows.clip(clip)
        });
        clips_to_play
            .filter_map(|clip| match &clip.content {
                hardwave_project::clip::ClipContent::Audio(audio_clip) => {
                    // The clip's sample, looked up once: its rate, length
                    // and the step it plays at all come from it. It was
                    // looked up four times over for every clip, which was a
                    // good part of building a song with thousands of them.
                    let source = self.audio_pool.get(&audio_clip.source_path);
                    // A clip that follows the tempo is played as a stretch or
                    // as warp markers at this tempo map (`tempo_follow`), the
                    // same clip the bakes were planned from.
                    let file_rate = source.as_ref().map(|b| b.sample_rate as f64);
                    let followed =
                        crate::tempo_follow::tempo_followed(clip, audio_clip, tempo_map, file_rate);
                    let audio_clip: &hardwave_project::clip::AudioClip = &followed;
                    let timeline_start =
                        tempo_map.tick_to_samples(clip.position_ticks, sample_rate);
                    let timeline_end = tempo_map
                        .tick_to_samples(clip.position_ticks + clip.length_ticks, sample_rate);
                    // Normalising and taking the DC offset off need the
                    // sample's levels, worked out once in the pool.
                    let treatment = audio_clip.treatment;
                    let levels = (treatment.normalize || treatment.remove_dc)
                        .then(|| self.audio_pool.level_stats(&audio_clip.source_path))
                        .flatten();
                    let normalise = match (treatment.normalize, levels) {
                        (true, Some(l)) => {
                            let peak = if treatment.remove_dc {
                                l.peak_without_dc
                            } else {
                                l.peak
                            };
                            if peak > 1e-6 {
                                1.0 / peak
                            } else {
                                1.0
                            }
                        }
                        _ => 1.0,
                    };
                    // The sample's window: its start and length, and
                    // its tail cut where it falls under the trim level.
                    let frames = source
                        .as_ref()
                        .map(|b| b.num_frames as u64)
                        .unwrap_or(u64::MAX);
                    let trim_end = treatment.trim_threshold().and_then(|share| {
                        let peak = self
                            .audio_pool
                            .level_stats(&audio_clip.source_path)
                            .map(|l| l.peak)
                            .unwrap_or(1.0);
                        self.audio_pool
                            .trim_end(&audio_clip.source_path, share as f32 * peak)
                    });
                    let treat = crate::track_node::ClipTreat::new(
                        &treatment,
                        levels.map(|l| l.dc).unwrap_or([0.0; 2]),
                    )
                    .with_window(&treatment, frames, trim_end);
                    let gain = if audio_clip.gain_db <= -100.0 {
                        0.0
                    } else {
                        10.0_f64.powf(audio_clip.gain_db / 20.0) as f32 * normalise
                    };
                    // A clip without fades of its own gets the tiny
                    // ones its declicking mode stands for.
                    let (declick_in, declick_out) = treatment.declick.fades_ms();
                    let micro = |ms: f64| (ms / 1000.0 * sample_rate).round() as u64;
                    // A fade lasts its ticks at the tempo where it is: from
                    // the clip's start, and up to its end. Counted from the
                    // song's start, a fade after a tempo change played at
                    // the opening tempo, longer or shorter than drawn.
                    let clip_end_ticks = clip.position_ticks + clip.length_ticks;
                    let fade_in_samples = if audio_clip.fade_in_ticks > 0 {
                        tempo_map
                            .tick_to_samples(
                                clip.position_ticks + audio_clip.fade_in_ticks,
                                sample_rate,
                            )
                            .saturating_sub(timeline_start)
                    } else {
                        micro(declick_in)
                    };
                    let fade_out_samples = if audio_clip.fade_out_ticks > 0 {
                        timeline_end.saturating_sub(tempo_map.tick_to_samples(
                            clip_end_ticks.saturating_sub(audio_clip.fade_out_ticks),
                            sample_rate,
                        ))
                    } else {
                        micro(declick_out)
                    };
                    let clip_pitch_factor = 2.0_f64.powf(audio_clip.pitch_semitones / 12.0);
                    let pitch_factor = clip_pitch_factor * track_pitch_factor;
                    let stretch = if audio_clip.stretch_ratio <= 0.01 {
                        1.0
                    } else {
                        audio_clip.stretch_ratio
                    };
                    // Pitch-preserving stretch. A plain `source_step` resample
                    // can only change duration and pitch *together* (varispeed:
                    // stretch_ratio 2.0 also dropped an octave), so when a clip
                    // asks for either, bake a stretched / pitch-shifted copy of
                    // the source once and play it back at unity step. That makes
                    // stretch_ratio change duration only and pitch_semitones
                    // change pitch only, as their names promise.
                    //
                    // Warped clips keep the resample path — warp markers define
                    // their own piecewise source map and take precedence below.
                    // The track-level pitch/fine-tune offset stays a resample, so
                    // it remains a varispeed control.
                    // Resample is the varispeed path on purpose: like a
                    // tape, a longer clip plays lower.
                    // A warped clip rendered with its pitch kept (see
                    // `warp_bake_plan`), when the render has landed: played
                    // straight through, its markers already in it.
                    // A take with notes tuned by hand plays its tuned render
                    // once that has landed (see `tune_bake_plan`); until
                    // then, the file. Its stretch and warp are made from it.
                    let base = self.tuned_source(audio_clip);
                    let warp_baked = if audio_clip.warp_markers.is_empty() || treatment.resample {
                        None
                    } else {
                        source
                            .as_ref()
                            .map(|b| b.sample_rate as f64)
                            .and_then(|buffer_rate| {
                                let unity = if buffer_rate > 0.0 {
                                    buffer_rate / sample_rate
                                } else {
                                    1.0
                                };
                                let pieces =
                                    warp_pieces(clip, audio_clip, tempo_map, sample_rate, unity);
                                let key = warp_cache_key(
                                    &base,
                                    &pieces,
                                    audio_clip.pitch_semitones,
                                    sample_rate,
                                );
                                self.audio_pool.contains(&key).then_some(key)
                            })
                    };
                    let needs_bake = audio_clip.warp_markers.is_empty()
                        && !treatment.resample
                        && ((stretch - 1.0).abs() > 1e-4
                            || audio_clip.pitch_semitones.abs() > 1e-4);
                    let baked = if needs_bake {
                        self.stretched_source(&base, stretch, audio_clip.pitch_semitones)
                    } else {
                        None
                    };
                    // Baking stretches the whole source, so an in-source start
                    // offset scales with the same ratio.
                    let (region_source_id, region_source_offset, source_step) =
                        match (baked, &warp_baked) {
                            (_, Some(key)) => (key.clone(), 0, track_pitch_factor),
                            (Some(key), None) => (
                                key,
                                (audio_clip.source_start as f64 * stretch) as u64,
                                track_pitch_factor,
                            ),
                            (None, None) => (base, audio_clip.source_start, pitch_factor / stretch),
                        };
                    // Rate-match the source to the rate we're rendering at.
                    // Pool buffers are normalised to the *device* rate on
                    // import, but an export renders at whatever rate the
                    // user picked — so bouncing at 44.1 k from a 48 k
                    // device read every audio clip 8.8% too slow (flat and
                    // long) while MIDI/synth tracks, which are generated at
                    // the render rate, stayed in tune. A mix would come out
                    // with its samples and its synths in different keys.
                    // Ratio is 1.0 during playback, where the two agree.
                    let other_source;
                    let region_buffer = if region_source_id == audio_clip.source_path {
                        source.as_ref()
                    } else {
                        other_source = self.audio_pool.get(&region_source_id);
                        other_source.as_ref()
                    };
                    let source_step = match region_buffer {
                        Some(buf)
                            if buf.sample_rate != 0
                                && (buf.sample_rate as f64 - sample_rate).abs() > f64::EPSILON =>
                        {
                            source_step * (buf.sample_rate as f64 / sample_rate)
                        }
                        _ => source_step,
                    };
                    // Warp markers → precomputed segments. Marker
                    // ticks are clip-relative; going through the
                    // tempo map (absolute tick → samples, minus the
                    // clip start) keeps anchors honest under tempo
                    // automation.
                    let warp = if audio_clip.warp_markers.is_empty() || warp_baked.is_some() {
                        Vec::new()
                    } else {
                        let anchors: Vec<(u64, f64)> = audio_clip
                            .warp_markers
                            .iter()
                            .map(|m| {
                                let abs = tempo_map.tick_to_samples(
                                    clip.position_ticks + m.clip_tick,
                                    sample_rate,
                                );
                                (abs.saturating_sub(timeline_start), m.source_sample as f64)
                            })
                            .collect();
                        crate::track_node::build_warp_segments(
                            &anchors,
                            source_step,
                            audio_clip.source_start as f64,
                        )
                    };
                    Some(ClipRegion {
                        source_id: region_source_id,
                        timeline_start,
                        timeline_end,
                        source_offset: region_source_offset,
                        gain,
                        muted: audio_clip.muted,
                        fade_in_samples,
                        fade_out_samples,
                        fade_in_curve: audio_clip.fade_in_curve,
                        fade_out_curve: audio_clip.fade_out_curve,
                        reversed: audio_clip.reversed,
                        source_step,
                        warp,
                        treat,
                    })
                }
                _ => None,
            })
            .collect()
    }
}

/// What the audio thread and the builder thread share: the requests made
/// so far, and whether the builder is to stop.
#[derive(Default)]
pub(crate) struct BuildRequests {
    seq: AtomicU64,
    shutdown: AtomicBool,
}

/// How the callback gets its graphs.
pub(crate) enum BuildMode {
    /// Built on the calling thread: an offline render, which is not in a
    /// hurry, and the callback until playback hands it a builder thread.
    Inline(Box<GraphBuilder>),
    /// Built on the builder thread and swapped in when ready.
    Thread(BuilderLink),
}

/// The audio thread's end of the builder thread.
pub(crate) struct BuilderLink {
    requests: Arc<BuildRequests>,
    thread: std::thread::Thread,
    /// Finished graphs, one at a time.
    ready: rtrb::Consumer<Box<PreparedGraph>>,
    /// Replaced graphs on their way back to be freed.
    trash: rtrb::Producer<Box<PreparedGraph>>,
    /// A replaced graph the trash had no room for yet. No new graph is
    /// taken until it has gone.
    pending_trash: Option<Box<PreparedGraph>>,
    /// The last request made.
    requested: u64,
}

impl BuilderLink {
    /// Ask for a graph of the project as it is now. Wakes the builder; no
    /// lock, no allocation.
    pub(crate) fn request(&mut self) {
        self.requested = self.requests.seq.fetch_add(1, Ordering::AcqRel) + 1;
        self.thread.unpark();
    }

    /// The last request made.
    pub(crate) fn requested(&self) -> u64 {
        self.requested
    }

    /// A finished graph is waiting.
    #[cfg(test)]
    pub(crate) fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// The newest finished graph, once the last replaced one is on its way.
    pub(crate) fn take_ready(&mut self) -> Option<Box<PreparedGraph>> {
        if let Some(old) = self.pending_trash.take() {
            self.throw_away(old);
            if self.pending_trash.is_some() {
                return None;
            }
        }
        self.ready.pop().ok()
    }

    /// Send a replaced graph back to be freed on the builder thread.
    pub(crate) fn throw_away(&mut self, old: Box<PreparedGraph>) {
        match self.trash.push(old) {
            Ok(()) => self.thread.unpark(),
            Err(rtrb::PushError::Full(old)) => self.pending_trash = Some(old),
        }
    }
}

impl Drop for BuilderLink {
    fn drop(&mut self) {
        self.requests.shutdown.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

impl GraphBuilder {
    /// Start the builder thread with this builder. Graphs it finishes wait
    /// in the link for the audio thread.
    pub(crate) fn spawn(self: Box<Self>, first_seq: u64) -> std::io::Result<BuilderLink> {
        let requests = Arc::new(BuildRequests::default());
        requests.seq.store(first_seq, Ordering::Release);
        let (ready_tx, ready_rx) = rtrb::RingBuffer::<Box<PreparedGraph>>::new(1);
        let (trash_tx, trash_rx) = rtrb::RingBuffer::<Box<PreparedGraph>>::new(4);
        let shared = Arc::clone(&requests);
        let handle = std::thread::Builder::new()
            .name("hw-graph-builder".into())
            .spawn(move || self.run(shared, first_seq, ready_tx, trash_rx))?;
        Ok(BuilderLink {
            requests,
            thread: handle.thread().clone(),
            ready: ready_rx,
            trash: trash_tx,
            pending_trash: None,
            requested: first_seq,
        })
    }

    /// The builder thread: build when asked, hand the graph over, free the
    /// graphs that come back.
    fn run(
        mut self: Box<Self>,
        requests: Arc<BuildRequests>,
        mut built: u64,
        mut ready: rtrb::Producer<Box<PreparedGraph>>,
        mut trash: rtrb::Consumer<Box<PreparedGraph>>,
    ) {
        let empty_trash = |trash: &mut rtrb::Consumer<Box<PreparedGraph>>| {
            while let Ok(old) = trash.pop() {
                drop(old);
            }
        };
        while !requests.shutdown.load(Ordering::Acquire) {
            empty_trash(&mut trash);
            let wanted = requests.seq.load(Ordering::Acquire);
            if wanted == built {
                std::thread::park_timeout(std::time::Duration::from_millis(200));
                continue;
            }
            // A copy of the song, so the project is held only for as long
            // as copying it takes. The audio thread reads it every block
            // (tempo, sends, modulation) and goes without for a block when
            // it is busy; a whole build is several blocks in a big song.
            let project = self.project.lock().clone();
            let graph = {
                let meters = Arc::clone(&self.track_meters);
                let mut meters = meters.lock();
                self.build(&project, &mut meters, wanted)
            };
            drop(project);
            built = wanted;
            // One graph waits at a time. While the audio thread has not
            // taken the last one, this one waits; a newer request makes it
            // stale, and it is built again instead.
            let mut graph = Some(graph);
            while let Some(g) = graph.take() {
                match ready.push(g) {
                    Ok(()) => {}
                    Err(rtrb::PushError::Full(g)) => {
                        let stale = requests.seq.load(Ordering::Acquire) != wanted;
                        if stale || requests.shutdown.load(Ordering::Acquire) {
                            break;
                        }
                        empty_trash(&mut trash);
                        std::thread::park_timeout(std::time::Duration::from_millis(1));
                        graph = Some(g);
                    }
                }
            }
        }
        empty_trash(&mut trash);
    }
}
