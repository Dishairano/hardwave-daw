//! Audio graph — DAG of processing nodes, topologically sorted for real-time execution.

pub type NodeId = usize;

/// Context passed to each node during processing.
pub struct ProcessContext {
    pub sample_rate: f64,
    pub buffer_size: u32,
    pub tempo: f64,
    pub time_sig: (u32, u32),
    pub position_samples: u64,
    /// The same position in ticks, through the project's tempo map.
    ///
    /// Computed once per block by the engine, because a node cannot take the
    /// project lock on the audio thread. Automation used to work this out
    /// itself as `samples / rate * tempo`, which is only right for a song at
    /// one tempo: with a tempo change or a ramp the curve slid off the music.
    pub position_ticks: u64,
    pub playing: bool,
    /// Smart disable (Settings > Audio): plug-ins with nothing to do are
    /// left out, and nodes with nothing to play skip their work.
    pub smart_disable: bool,
}

/// Trait implemented by all nodes in the audio graph.
pub trait AudioNode: Send {
    fn name(&self) -> &str;

    /// Process audio. Reads from `inputs`, writes to `outputs`.
    /// Each slice is one channel of audio (non-interleaved).
    ///
    /// The block is `ctx.buffer_size` frames, which can be fewer than the
    /// graph's block size (the rest of a bigger driver block, or a block cut
    /// at the loop end). Every input slice is exactly that long. The output
    /// buffers are the graph's size, holding an older block past the frames
    /// of this one: a node writes the block's frames (it may cut the buffers
    /// to them) and must not read or pass on anything past them.
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[hardwave_midi::MidiEvent],
        midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    );

    fn latency_samples(&self) -> u32 {
        0
    }
    fn reset(&mut self) {}

    /// Whether this block leaves the node silent with nothing worth doing:
    /// it reads no input and has nothing of its own to play. The graph
    /// skips such a node, and the zeroing around it, for as long as its
    /// output is already silent. A node that says so must, when processed,
    /// write silence to every output channel. Default: never idle.
    fn idle(&self, _ctx: &ProcessContext) -> bool {
        false
    }

    /// Tell a node that another track routes into it, so a bus with nothing
    /// of its own on it still mixes what reaches it. Only track nodes care.
    fn set_receives_input(&mut self, _receives: bool) {}

    /// Tell a node that live input passes through it, so it keeps running
    /// while the song is stopped. Only track nodes care.
    fn set_live_input(&mut self, _live: bool) {}

    /// The keyboard moved to another track: let go of every note it is
    /// holding here, since the releases will not arrive any more.
    fn release_live_notes(&mut self) {}

    /// Move one knob of one plug-in in this node's chain.
    ///
    /// Borrowed rather than a command: modulation sets a knob every
    /// block, and building a command would allocate on the audio thread
    /// once per route per block.
    fn set_chain_parameter(&mut self, _slot_id: &str, _param_id: u32, _value: f64) {}

    /// Modulation's share of a track's level and pan, on top of the fader,
    /// the pan knob and their automation: the level multiplied by `gain`,
    /// the pan moved by `pan`. (1, 0) changes nothing.
    fn set_mod_mix(&mut self, _gain: f32, _pan: f32) {}

    /// Stable project-side identifier for the track this node represents,
    /// if any. Returns `None` for non-track nodes (master, input bus,
    /// etc.). Used by the engine to route per-track plug-in commands.
    fn track_id(&self) -> Option<&str> {
        None
    }

    /// Apply a per-track plug-in insert command to this node. Default
    /// no-op so non-track nodes silently ignore. Track nodes override
    /// to forward the command into their own [`crate::insert_chain::InsertChain`].
    /// Removed slots are buried via the supplied graveyard so the audio
    /// thread never frees plug-ins directly.
    fn apply_insert_command(
        &mut self,
        cmd: crate::insert_chain::InsertCommand,
        graveyard: &mut crate::insert_chain::PluginGraveyardSender,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        let _ = (cmd, graveyard, sample_rate, max_block_size);
    }

    /// Snapshot the current state of every loaded plug-in slot on this
    /// node as `(slot_id, state_bytes)`. Default empty so non-track
    /// nodes contribute nothing. TrackNode overrides to walk its
    /// `chain.slots` and call `plugin.get_state()` per entry.
    fn snapshot_plugin_states(&self) -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }

    /// Hand this node's plug-ins the parameter changes still waiting for
    /// a block. Default nothing, for nodes without plug-ins.
    fn flush_plugin_parameters(&mut self) {}

    /// Hand off this node's plug-in chain so it can be reattached to a
    /// freshly-built TrackNode after a graph rebuild. Default `None`.
    /// TrackNode overrides to swap in an empty chain and return the
    /// previous one.
    fn take_chain(&mut self) -> Option<crate::insert_chain::InsertChain> {
        None
    }

    /// Reattach a previously-stashed plug-in chain after a graph rebuild.
    /// No-op for non-track nodes; TrackNode replaces its empty chain
    /// with the supplied one.
    fn restore_chain(&mut self, chain: crate::insert_chain::InsertChain) {
        let _ = chain;
    }

    /// Carry on the sound of `old`, the node this one replaces when a new
    /// graph is swapped in: the voices still ringing, the filter and delay
    /// state, the gain ramp, where the playhead was. Without it every edit
    /// during playback cut every synth note and filter tail. Runs on the
    /// audio thread: it swaps state, never allocates or frees. Default:
    /// nothing to carry.
    fn carry_on_from(&mut self, old: &mut dyn AudioNode) {
        let _ = old;
    }

    /// The node as `Any`, so `carry_on_from` can see whether the node it
    /// replaces is of its own kind. `None` for nodes that carry nothing.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }

    /// How long this node's work took in one block, in nanoseconds, for
    /// its load read-out in the mixer. The graph times each node on one
    /// block in [`CPU_STRIDE`], not on every block. Default: not shown.
    fn report_cpu(&self, nanos: u64) {
        let _ = nanos;
    }

    /// Whether the block just processed left every output channel silent.
    /// The graph then leaves the node out of the sums it feeds, as it does
    /// an idle one. Default: never said.
    fn output_is_silent(&self) -> bool {
        false
    }

    /// How many plug-ins the node's chain holds.
    fn chain_len(&self) -> usize {
        0
    }

    /// Build-time insert population for offline render: push a pre-built
    /// `LiveSlot` onto this node's chain. Default no-op for non-track
    /// nodes; TrackNode overrides to append to its insert chain so an
    /// export applies the same FX as live playback.
    fn push_offline_slot(
        &mut self,
        slot: crate::insert_chain::LiveSlot,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        let _ = (slot, sample_rate, max_block_size);
    }

    /// Mark whether the insert slot `slot_id` has a sidechain source
    /// routed to it, so the chain feeds it the track sidechain bus
    /// (this node's input ports 2/3). Default no-op for non-track nodes;
    /// track nodes forward to their insert chain. Synced on every graph
    /// rebuild from the project's `PluginSlot::sidechain_source`.
    fn set_slot_sidechain(&mut self, slot_id: &str, active: bool) {
        let _ = (slot_id, active);
    }

    /// Allow or forbid smart disable on one plug-in slot. Synced on every
    /// graph rebuild from the project's `PluginSlot::allow_smart_disable`.
    fn set_slot_smart_disable(&mut self, slot_id: &str, allowed: Option<bool>) {
        let _ = (slot_id, allowed);
    }
}

/// Each node's work is timed on one block in this many, for the mixer's
/// per-track load read-out. Reading the clock is not free: on a Windows
/// virtual machine one read took 14 microseconds, and two per track on
/// every block came to a fifth of a 512-frame block on a 127-track song.
/// This way a block reads the clock for a handful of nodes at most.
pub const CPU_STRIDE: usize = 32;

/// How many frames this block has, at most the graph's block size.
fn block_frames(ctx: &ProcessContext, buffer_size: usize) -> usize {
    (ctx.buffer_size as usize).min(buffer_size)
}

/// The first `frames` of a buffer (all of it when shorter).
fn block_part(buffer: &[f32], frames: usize) -> &[f32] {
    &buffer[..frames.min(buffer.len())]
}

/// Number of output channels every node gets. Tracks use 0/1 for post-fader
/// stereo and 2/3 for the pre-fader tap that feeds pre-fader sends; other
/// nodes simply leave 2/3 at zero.
pub const NODE_CHANNELS: usize = 4;

/// Edge in the audio graph.
#[derive(Debug, Clone)]
struct Edge {
    source: NodeId,
    source_port: usize,
    dest: NodeId,
    dest_port: usize,
    /// Linear gain multiplier applied when mixing this edge into the dest buffer.
    /// 1.0 is unity; used to implement send amounts without a per-send node.
    gain: f32,
    /// Plugin-delay compensation: samples to delay source before mixing into
    /// dest. Computed from per-node accumulated latency so parallel branches
    /// line up against the slowest branch.
    compensation_samples: u32,
}

/// Where a compensation line sits, by the tracks and ports at each end of
/// its edge (source track, source port, destination track, destination
/// port), and the edge's index. A graph built in place of another carries
/// on with the lines of the edges they share (`carry_delays_from`).
type DelayKey = (String, usize, String, usize, usize);

/// Delay line for one edge's PDC compensation. `ring` holds `capacity`
/// samples; `write_pos` is the next index to write to.
#[derive(Debug, Clone)]
struct EdgeDelayLine {
    samples: u32,
    ring: Vec<f32>,
    write_pos: usize,
}

impl EdgeDelayLine {
    fn new(samples: u32, buffer_size: usize) -> Self {
        // ring must hold (samples + buffer_size) so a full block's worth of
        // reads can pull fully-delayed samples without stepping on writes.
        let capacity = samples as usize + buffer_size;
        Self {
            samples,
            ring: vec![0.0; capacity.max(1)],
            write_pos: 0,
        }
    }

    /// Advance the line by one block: write `src` into the ring and read the
    /// same number of samples, delayed by `self.samples`, into `dst`.
    fn process(&mut self, src: &[f32], dst: &mut [f32]) {
        let n = src.len().min(dst.len());
        if self.samples == 0 {
            // Zero-delay edges bypass the ring entirely and act as an
            // identity copy. finalize_pdc wouldn't normally build a line
            // with samples == 0, but keep this defensive path explicit.
            dst[..n].copy_from_slice(&src[..n]);
            return;
        }
        let cap = self.ring.len();
        for i in 0..n {
            // Read delayed sample first, then overwrite it. With non-zero
            // `samples`, read_pos != write_pos so this is safe.
            let read_pos = (self.write_pos + cap - self.samples as usize) % cap;
            dst[i] = self.ring[read_pos];
            self.ring[self.write_pos] = src[i];
            self.write_pos = (self.write_pos + 1) % cap;
        }
    }
}

/// The audio graph. Nodes are stored in topological order for lock-free processing.
/// Per-node scratch space, pre-allocated once at `add_node` time so the
/// audio thread never asks the allocator during `process`. Indexed 1:1
/// with `nodes` / `buffers` / `scratch`.
///
/// Without this struct, `AudioGraph::process` would `vec![vec![0.0; buf];
/// NODE_CHANNELS]` for `ch_bufs` AND for `outputs` every node every
/// block, plus a `Vec<f32>` for `delay_scratch` and a `Vec::new()` for
/// `midi_out`. At 48 kHz / 256 samples / 500 nodes that's roughly half
/// a million allocations per second on the real-time thread — the
/// number one finding from the May 2026 perf audit.
struct NodeScratch {
    /// Gathered inputs after edge accumulation. `[channel][samples]`,
    /// zeroed each block via `.fill(0.0)` rather than re-allocated.
    ch_bufs: Vec<Vec<f32>>,
    /// Node output buffer that is swapped with `buffers[node_id]` after
    /// `process` so the next block reads it back as a source — zero
    /// copy.
    outputs: Vec<Vec<f32>>,
    /// Workspace for `EdgeDelayLine::process`. Lives here so each edge
    /// loop iteration doesn't allocate a fresh scratch vec.
    delay_scratch: Vec<f32>,
    /// MIDI output the node emits. Cleared (not re-allocated) per call.
    midi_out: Vec<hardwave_midi::MidiEvent>,
}

impl NodeScratch {
    fn new(buffer_size: usize) -> Self {
        Self {
            ch_bufs: vec![vec![0.0; buffer_size]; NODE_CHANNELS],
            outputs: vec![vec![0.0; buffer_size]; NODE_CHANNELS],
            delay_scratch: vec![0.0; buffer_size],
            midi_out: Vec::with_capacity(64),
        }
    }
}

pub struct AudioGraph {
    nodes: Vec<Box<dyn AudioNode>>,
    edges: Vec<Edge>,
    /// Per node, the indices of the edges that end at it, in edge order.
    /// A node gathers its inputs from these; scanning every edge for
    /// every node made a block cost grow with the square of the track
    /// count (a new song has 500 inserts). Rebuilt with the order.
    incoming: Vec<Vec<usize>>,
    /// Per node, whether its output buffer is known to be silent: it was
    /// idle last block and wrote zeros. An idle node with a silent output
    /// is skipped, and an edge from a silent node adds nothing. A new song
    /// has 500 empty inserts; zeroing and summing all of them every block
    /// cost more than the audio deadline on a small machine.
    silent: Vec<bool>,
    /// Per-edge PDC delay lines, indexed 1:1 with `edges`.
    edge_delays: Vec<Option<EdgeDelayLine>>,
    processing_order: Vec<NodeId>,
    /// Nodes or edges were added since the order was worked out. Adding
    /// them only marks this; the order is worked out once, when the graph
    /// is finished (`finalize_pdc`) or first run. Working it out on every
    /// add cost a 500-track song a tenth of a second per rebuild.
    order_dirty: bool,
    /// Pre-allocated buffers for intermediate data.
    buffers: Vec<Vec<Vec<f32>>>, // [node][channel][samples]
    /// Per-node scratch space — eliminates 5 allocations per node per
    /// block. Sized 1:1 with `nodes` / `buffers`.
    scratch: Vec<NodeScratch>,
    /// Per-node flag: when true, the live-MIDI slice passed to
    /// `AudioGraph::process` is forwarded to this node. When false, the
    /// node receives an empty slice. Set during `rebuild_graph` for
    /// tracks where `armed && monitor_input` is true (and for every MIDI
    /// track as a usability default — see `set_accepts_live_midi`).
    accepts_live_midi: Vec<bool>,
    buffer_size: usize,
    /// Nodes grouped so that everything in a group can run at the same
    /// time. Rebuilt with the order, not per block.
    levels: crate::parallel::Levels,
    /// The threads that share the work, when more than one core is
    /// being used. `None` is the plain one-thread path, which is what
    /// every build did until this existed.
    pool: Option<std::sync::Arc<crate::parallel::WorkerPool>>,
    /// The edges with a compensation line between two tracks, sorted, so a
    /// graph swapped in for this one finds the lines it shares with it in
    /// one pass (`index_delays`, `carry_delays_from`).
    delay_keys: Vec<DelayKey>,
    /// The critical-path latency `finalize_pdc` worked out, with the chain
    /// delays it was told.
    planned_latency: u32,
    /// Per node, the nanoseconds spent gathering its inputs and running
    /// it, added up over the blocks since timing was switched on. `None`
    /// (the default) times nothing. For measurements only.
    node_nanos: Option<Vec<[u64; 2]>>,
    /// Counts the blocks, to pick the nodes timed for their load read-out
    /// this block (see [`CPU_STRIDE`]).
    cpu_turn: usize,
}

impl AudioGraph {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            incoming: Vec::new(),
            silent: Vec::new(),
            edge_delays: Vec::new(),
            processing_order: Vec::new(),
            order_dirty: false,
            buffers: Vec::new(),
            scratch: Vec::new(),
            accepts_live_midi: Vec::new(),
            buffer_size,
            levels: crate::parallel::Levels::default(),
            pool: None,
            delay_keys: Vec::new(),
            planned_latency: 0,
            node_nanos: None,
            cpu_turn: 0,
        }
    }

    /// Start timing each node on the one-thread path (see
    /// [`Self::node_nanos`]), from zero, or stop. For measurements only.
    #[doc(hidden)]
    pub fn time_nodes(&mut self, on: bool) {
        self.node_nanos = on.then(|| vec![[0, 0]; self.nodes.len()]);
    }

    /// Per node, the nanoseconds spent gathering its inputs (sums and
    /// compensation lines) and running it since [`Self::time_nodes`],
    /// when timing is on.
    #[doc(hidden)]
    pub fn node_nanos(&mut self) -> Option<&mut [[u64; 2]]> {
        self.node_nanos.as_deref_mut()
    }

    /// The name of a node, and whether it is an audio track, an
    /// instrument track or the master. For measurements.
    #[doc(hidden)]
    pub fn node_label(&mut self, node_id: NodeId) -> Option<(String, &'static str)> {
        let node = self.nodes.get_mut(node_id)?;
        let name = node.name().to_string();
        let kind = match node.as_any_mut() {
            Some(any) if any.is::<crate::track_node::TrackNode>() => "track",
            Some(any) if any.is::<crate::midi_track_node::MidiTrackNode>() => "instrument",
            _ if name == "Master" => "master",
            _ => "other",
        };
        Some((name, kind))
    }

    /// Mark a node as accepting external/live MIDI for the next call to
    /// [`AudioGraph::process`]. The engine sets this during graph rebuild
    /// based on `track.armed && track.monitor_input` (and as a default
    /// for MIDI tracks). Out-of-range ids silently no-op so the engine's
    /// rebuild ordering — `add_node` then `set_accepts_live_midi` — is
    /// guaranteed safe.
    pub fn set_accepts_live_midi(&mut self, node_id: NodeId, accepts: bool) {
        if let Some(slot) = self.accepts_live_midi.get_mut(node_id) {
            let was = std::mem::replace(slot, accepts);
            // A node that stops hearing the keyboard would never get the
            // releases of the notes held on it.
            if was && !accepts {
                if let Some(node) = self.nodes.get_mut(node_id) {
                    node.release_live_notes();
                }
            }
        }
    }

    /// Add a node, returns its ID. Pre-allocates the buffer and scratch
    /// space for the node — none of that allocation happens on the
    /// audio thread.
    pub fn add_node(&mut self, node: Box<dyn AudioNode>) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        self.buffers
            .push(vec![vec![0.0; self.buffer_size]; NODE_CHANNELS]);
        self.scratch.push(NodeScratch::new(self.buffer_size));
        self.accepts_live_midi.push(false);
        // Its buffer starts at zero, but nothing has run it yet: the first
        // block processes it either way.
        self.silent.push(false);
        self.order_dirty = true;
        id
    }

    /// Connect source node's output to dest node's input with unity gain.
    pub fn connect(&mut self, source: NodeId, source_port: usize, dest: NodeId, dest_port: usize) {
        self.connect_with_gain(source, source_port, dest, dest_port, 1.0);
    }

    /// Connect with a linear gain multiplier. Used for sends.
    /// Index of the edge a `connect_with_gain` call created, so its gain can
    /// be changed later without rebuilding the graph. Sends are edges with a
    /// gain, and a send level baked at rebuild time cannot be automated.
    pub fn last_edge_index(&self) -> usize {
        self.edges.len().saturating_sub(1)
    }

    /// Change one edge's gain. Called from the audio thread, once per block
    /// per automated send, so it does no work beyond the write.
    pub fn set_edge_gain(&mut self, edge: usize, gain: f32) {
        if let Some(e) = self.edges.get_mut(edge) {
            e.gain = gain;
        }
    }

    pub fn connect_with_gain(
        &mut self,
        source: NodeId,
        source_port: usize,
        dest: NodeId,
        dest_port: usize,
        gain: f32,
    ) {
        self.edges.push(Edge {
            source,
            source_port,
            dest,
            dest_port,
            gain,
            compensation_samples: 0,
        });
        self.edge_delays.push(None);
        self.order_dirty = true;
    }

    /// Note where each compensation line sits, for the graph built in this
    /// one's place (`carry_delays_from`). Called once the graph is finished,
    /// off the audio thread: it allocates.
    pub fn index_delays(&mut self) {
        let mut keys = Vec::new();
        for (index, edge) in self.edges.iter().enumerate() {
            if !matches!(self.edge_delays.get(index), Some(Some(_))) {
                continue;
            }
            let (Some(source), Some(dest)) = (
                self.nodes.get(edge.source).and_then(|n| n.track_id()),
                self.nodes.get(edge.dest).and_then(|n| n.track_id()),
            ) else {
                continue;
            };
            keys.push((
                source.to_string(),
                edge.source_port,
                dest.to_string(),
                edge.dest_port,
                index,
            ));
        }
        keys.sort();
        self.delay_keys = keys;
    }

    /// Carry on with the compensation lines of `old`, the graph this one
    /// replaces: an edge between the same tracks and ports, delayed by the
    /// same amount, takes that edge's line with the audio already in it,
    /// and `old` gets this one's empty line to be freed with it. A new line
    /// starts empty, so every rebuild (a mute, a solo, a send) cut that much
    /// out of every compensated track while the song played: a click with a
    /// short lookahead, a gap with a long one.
    ///
    /// Runs on the audio thread when a graph is swapped in: one pass over
    /// both sorted lists, swapping lines, never allocating.
    pub fn carry_delays_from(&mut self, old: &mut AudioGraph) {
        let (mut i, mut j) = (0, 0);
        while i < self.delay_keys.len() && j < old.delay_keys.len() {
            let (new_key, old_key) = (&self.delay_keys[i], &old.delay_keys[j]);
            let order = (&new_key.0, new_key.1, &new_key.2, new_key.3)
                .cmp(&(&old_key.0, old_key.1, &old_key.2, old_key.3));
            match order {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    let (new_edge, old_edge) = (new_key.4, old_key.4);
                    if let (Some(Some(new_line)), Some(Some(old_line))) = (
                        self.edge_delays.get_mut(new_edge),
                        old.edge_delays.get_mut(old_edge),
                    ) {
                        if new_line.samples == old_line.samples
                            && new_line.ring.len() == old_line.ring.len()
                        {
                            std::mem::swap(new_line, old_line);
                        }
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
    }

    /// Keep the worker pool of `old` when this graph was built without
    /// reading the setting (its lock was busy). A swap of two handles.
    pub fn keep_pool_of(&mut self, old: &mut AudioGraph) {
        std::mem::swap(&mut self.pool, &mut old.pool);
    }

    /// Whether sound leaving `from` already arrives at `to` through the
    /// edges added so far. An edge from `to` back to `from` would close a
    /// loop, and a node in a loop is never put in the order: it, and every
    /// node after it (the master among them), would stop running.
    pub fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = vec![from];
        while let Some(node) = stack.pop() {
            if node == to {
                return true;
            }
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            stack.extend(
                self.edges
                    .iter()
                    .filter(|e| e.source == node && !seen[e.dest])
                    .map(|e| e.dest),
            );
        }
        false
    }

    /// Remove all nodes and edges.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.incoming.clear();
        self.silent.clear();
        self.edge_delays.clear();
        self.processing_order.clear();
        self.order_dirty = false;
        self.buffers.clear();
        self.scratch.clear();
        self.accepts_live_midi.clear();
    }

    /// Compute per-edge PDC compensation delays from each node's accumulated
    /// latency. Must be called after all nodes/edges are added — typically at
    /// the end of `EngineCallback::rebuild_graph`. For each edge, the delay
    /// is `acc[dest] - acc[source] - dest.latency_samples()`, which pads the
    /// source's arrival so parallel branches reach the dest sample-aligned
    /// against the slowest incoming path.
    ///
    /// With `enabled` false nothing is padded: the switch in the Options
    /// menu, for someone who wants the lowest latency while playing live
    /// and accepts plug-ins that are late. It used to change nothing.
    pub fn finalize_pdc(&mut self, enabled: bool) {
        self.finalize_pdc_with(enabled, &[]);
    }

    /// `finalize_pdc` for a graph whose plug-in chains are not in it yet:
    /// `chain_latency[node]` is added to each node's own latency, for the
    /// chain that will be moved into it when the graph is swapped in.
    pub fn finalize_pdc_with(&mut self, enabled: bool, chain_latency: &[u32]) {
        self.ensure_order();
        let n = self.nodes.len();
        self.delay_keys.clear();
        if n == 0 {
            self.edge_delays.clear();
            self.planned_latency = 0;
            return;
        }
        let own = |node: usize, nodes: &[Box<dyn AudioNode>]| {
            nodes[node]
                .latency_samples()
                .saturating_add(chain_latency.get(node).copied().unwrap_or(0))
        };
        // Accumulated latency per node (critical path into the node +
        // node's own latency) — same DP as `total_latency_samples`.
        let mut acc = vec![0u32; n];
        for &node_id in &self.processing_order {
            let incoming_max = self.incoming[node_id]
                .iter()
                .map(|&e| acc[self.edges[e].source])
                .max()
                .unwrap_or(0);
            acc[node_id] = incoming_max.saturating_add(own(node_id, &self.nodes));
        }
        self.planned_latency = acc.iter().copied().max().unwrap_or(0);
        // For each edge, compute the pad samples needed so the source arrives
        // aligned with the slowest incoming branch of the same dest. The
        // slowest branch into each node is found in one pass over the
        // edges; looking it up again for every edge made this grow with the
        // square of the edge count (a 500-track song has over 1000 edges).
        let mut dest_incoming_max = vec![0u32; n];
        for e in &self.edges {
            dest_incoming_max[e.dest] = dest_incoming_max[e.dest].max(acc[e.source]);
        }
        let pads: Vec<u32> = self
            .edges
            .iter()
            .map(|edge| {
                if enabled {
                    dest_incoming_max[edge.dest].saturating_sub(acc[edge.source])
                } else {
                    0
                }
            })
            .collect();
        self.edge_delays.clear();
        self.edge_delays.reserve(self.edges.len());
        for (edge, &pad) in self.edges.iter_mut().zip(pads.iter()) {
            edge.compensation_samples = pad;
            self.edge_delays.push(if pad > 0 {
                Some(EdgeDelayLine::new(pad, self.buffer_size))
            } else {
                None
            });
        }
    }

    /// Rebuild topological order using Kahn's algorithm.
    fn rebuild_order(&mut self) {
        let (order, incoming) = self.order_and_incoming();
        let n = self.nodes.len();
        // Which nodes can run beside each other: a node sits one level
        // past the highest level of anything feeding it, so two nodes
        // in a level never have an edge between them.
        let edges = &self.edges;
        self.levels = crate::parallel::Levels::build(&order, n, |node| {
            incoming[node].iter().map(|&e| edges[e].source).collect()
        });
        self.processing_order = order;
        self.incoming = incoming;
        self.order_dirty = false;
    }

    /// Work the order out now if anything was added since.
    fn ensure_order(&mut self) {
        if self.order_dirty {
            self.rebuild_order();
        }
    }

    /// The topological order (Kahn's algorithm) and, per node, the edges
    /// that end at it.
    fn order_and_incoming(&self) -> (Vec<NodeId>, Vec<Vec<usize>>) {
        let n = self.nodes.len();
        let mut in_degree = vec![0u32; n];
        let mut adjacency: Vec<Vec<NodeId>> = vec![Vec::new(); n];
        let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];

        for (index, edge) in self.edges.iter().enumerate() {
            in_degree[edge.dest] += 1;
            adjacency[edge.source].push(edge.dest);
            incoming[edge.dest].push(index);
        }

        let mut queue: Vec<NodeId> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);

        while let Some(node) = queue.pop() {
            order.push(node);
            for &next in &adjacency[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push(next);
                }
            }
        }
        (order, incoming)
    }

    /// Share the per-block work across a pool of threads.
    ///
    /// `None` puts it back on the audio thread alone, which is what
    /// every build did before this and what a machine with one core
    /// should keep doing.
    pub fn set_worker_pool(&mut self, pool: Option<std::sync::Arc<crate::parallel::WorkerPool>>) {
        self.pool = pool;
    }

    /// How many threads are helping, zero when none are.
    pub fn worker_threads(&self) -> usize {
        self.pool.as_ref().map(|p| p.threads()).unwrap_or(0)
    }

    /// Process the entire graph for one buffer. Called on the audio
    /// thread.
    ///
    /// `midi_in` is the buffer of live external MIDI events drained from
    /// the input manager at the start of this block. The graph forwards
    /// it only to nodes whose `accepts_live_midi[node_id]` flag is true
    /// (set during `rebuild_graph` based on `track.armed && track.monitor_input`).
    /// Every other node receives an empty slice, matching the
    /// pre-live-MIDI behaviour.
    ///
    /// **No-alloc on the hot path.** All per-node temporaries live in
    /// `self.scratch[node_id]` (see [`NodeScratch`]) and are reused
    /// across blocks. The per-edge source read is now a borrow (disjoint
    /// field borrows of `buffers` / `scratch` / `edge_delays`), so the
    /// old per-block `src_vec` clone is gone — the audio thread does zero
    /// heap allocation in steady state.
    pub fn process(&mut self, ctx: &ProcessContext, midi_in: &[hardwave_midi::MidiEvent]) {
        // Finished graphs have their order already (`finalize_pdc`); this
        // only catches one run without it.
        self.ensure_order();
        // More than one core, when a pool is attached and the graph is
        // wide enough to be worth it.
        let cpu_turn = self.cpu_turn % CPU_STRIDE;
        self.cpu_turn = self.cpu_turn.wrapping_add(1);
        if self.pool.is_some() && !self.levels.is_empty() {
            self.process_in_parallel(ctx, midi_in, cpu_turn);
            return;
        }
        // Iterate by index — `processing_order` mutates inside the loop
        // body via no path, so a snapshot clone is unnecessary. (The
        // previous code's `.clone()` was defensive but never load-bearing.)
        let midi_empty: [hardwave_midi::MidiEvent; 0] = [];
        let order_len = self.processing_order.len();
        let frames = block_frames(ctx, self.buffer_size);

        for order_idx in 0..order_len {
            let node_id = self.processing_order[order_idx];

            // An idle node whose output is already silent has nothing to
            // do this block. Not when a delay line feeds it: that line
            // has to keep moving so it holds the right audio later.
            // Nor when the keyboard plays into it this block: a node left
            // out would never hear those notes.
            let hears_keys = !midi_in.is_empty()
                && self
                    .accepts_live_midi
                    .get(node_id)
                    .copied()
                    .unwrap_or(false);
            // Nor when sound reaches its sidechain ports (2 and up): a
            // plug-in keyed from another track has to hear it.
            let keyed = self.incoming[node_id].iter().any(|&e| {
                let edge = &self.edges[e];
                edge.dest_port >= 2 && !self.silent.get(edge.source).copied().unwrap_or(false)
            });
            let idle = !hears_keys
                && !keyed
                && self.nodes[node_id].idle(ctx)
                && self.incoming[node_id]
                    .iter()
                    .all(|&e| matches!(self.edge_delays.get(e), None | Some(None)));
            if idle && self.silent[node_id] {
                continue;
            }
            let timed = self.node_nanos.is_some().then(std::time::Instant::now);

            // Zero the scratch's input buffers in place (no realloc),
            // and only the frames this block has: a short block (the rest
            // of a driver block cut into graph-size pieces) used to zero
            // four whole graph-size buffers per node all the same.
            for ch in &mut self.scratch[node_id].ch_bufs {
                let n = frames.min(ch.len());
                ch[..n].fill(0.0);
            }

            // Gather inputs from edges that target this node. Disjoint
            // field borrows (buffers read-only; scratch + edge_delays
            // mutable — all separate fields of `self`) let us read the
            // source channel by reference instead of cloning it. This
            // removes the last per-block heap allocation from the audio
            // thread (was `ch.clone()` per edge per block → allocator
            // jitter risk). The source is another, already-processed
            // node's output buffer, so it never aliases this node's
            // scratch.
            {
                let buffers = &self.buffers;
                let edges = &self.edges;
                let silent = &self.silent;
                let edge_delays = &mut self.edge_delays;
                let scratch = &mut self.scratch[node_id];
                for &edge_idx in &self.incoming[node_id] {
                    let e = &edges[edge_idx];
                    // A silent source adds nothing, unless its edge delays
                    // it: the line still has to take the silence in.
                    let delayed = matches!(edge_delays.get(edge_idx), Some(Some(_)));
                    if silent.get(e.source).copied().unwrap_or(false) && !delayed {
                        continue;
                    }
                    let src: &[f32] = match buffers.get(e.source).and_then(|b| b.get(e.source_port))
                    {
                        Some(ch) => ch.as_slice(),
                        None => continue,
                    };
                    let n = src.len().min(frames);
                    let src_slice: &[f32] = if let Some(Some(dl)) = edge_delays.get_mut(edge_idx) {
                        dl.process(&src[..n], &mut scratch.delay_scratch[..n]);
                        &scratch.delay_scratch[..n]
                    } else {
                        &src[..n]
                    };
                    if e.dest_port < scratch.ch_bufs.len() {
                        let dest = &mut scratch.ch_bufs[e.dest_port];
                        for (i, s) in src_slice.iter().enumerate() {
                            if i < dest.len() {
                                dest[i] += s * e.gain;
                            }
                        }
                    }
                }
            }

            let gathered = timed.map(|t| (t.elapsed(), std::time::Instant::now()));

            // Run the node. Stack-allocated input_refs avoids a
            // per-iteration `Vec::collect`. NODE_CHANNELS is small + const
            // so the array stays cheap.
            let scratch = &mut self.scratch[node_id];
            scratch.midi_out.clear();
            // The block's frames only: past them the buffers hold an older
            // block's sums, no longer zeroed.
            let input_refs: [&[f32]; NODE_CHANNELS] = [
                block_part(&scratch.ch_bufs[0], frames),
                block_part(&scratch.ch_bufs[1], frames),
                block_part(&scratch.ch_bufs[2], frames),
                block_part(&scratch.ch_bufs[3], frames),
            ];
            let live: &[hardwave_midi::MidiEvent] = if self
                .accepts_live_midi
                .get(node_id)
                .copied()
                .unwrap_or(false)
            {
                midi_in
            } else {
                &midi_empty
            };
            let clocked = (order_idx % CPU_STRIDE == cpu_turn).then(std::time::Instant::now);
            self.nodes[node_id].process(
                &input_refs,
                &mut scratch.outputs,
                live,
                &mut scratch.midi_out,
                ctx,
            );
            if let Some(started) = clocked {
                self.nodes[node_id].report_cpu(started.elapsed().as_nanos() as u64);
            }

            // Swap the freshly-written output with the persistent buffer.
            // Zero copy — the persistent buffer becomes the next-block
            // scratch.outputs, which we'll .fill(0.0) on next call.
            std::mem::swap(&mut scratch.outputs, &mut self.buffers[node_id]);
            // An idle node wrote silence (the trait's promise), and a node
            // may say it did.
            self.silent[node_id] = idle || self.nodes[node_id].output_is_silent();
            if let (Some(nanos), Some((gather, ran))) = (&mut self.node_nanos, gathered) {
                if let Some(slot) = nanos.get_mut(node_id) {
                    slot[0] += gather.as_nanos() as u64;
                    slot[1] += ran.elapsed().as_nanos() as u64;
                }
            }
        }
    }

    /// One block, with the levels shared across the pool.
    ///
    /// Each node touches only its own scratch, its own output buffer
    /// and the delay lines of the edges that end at it, and reads the
    /// output buffers of nodes in earlier levels, which are finished
    /// before this level starts. That is what makes handing out
    /// pointers to separate nodes sound.
    fn process_in_parallel(
        &mut self,
        ctx: &ProcessContext,
        midi_in: &[hardwave_midi::MidiEvent],
        cpu_turn: usize,
    ) {
        let Some(pool) = self.pool.clone() else {
            return;
        };
        let shared = NodeWork {
            nodes: self.nodes.as_mut_ptr(),
            node_count: self.nodes.len(),
            scratch: self.scratch.as_mut_ptr(),
            buffers: self.buffers.as_mut_ptr(),
            edge_delays: self.edge_delays.as_mut_ptr(),
            edge_delay_count: self.edge_delays.len(),
            silent: self.silent.as_mut_ptr(),
            edges: &self.edges,
            incoming: &self.incoming,
            accepts_live_midi: &self.accepts_live_midi,
            buffer_size: self.buffer_size,
            midi_in,
            ctx,
            cpu_turn,
        };
        let run = |node_id: usize| {
            // SAFETY: every node id in a level is distinct, and a node
            // only writes its own slots.
            unsafe { shared.process_node(node_id) };
        };
        for level in &self.levels.levels {
            pool.run_level(level, &run);
        }
    }

    /// Get the output buffer of a specific node (e.g., the master node).
    pub fn node_output(&self, node_id: NodeId) -> Option<&[Vec<f32>]> {
        self.buffers.get(node_id).map(|v| v.as_slice())
    }

    /// Mutable access to a node by id — needed so the engine can route
    /// per-track plug-in commands directly to the right TrackNode
    /// without iterating every node in the graph.
    pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut Box<dyn AudioNode>> {
        self.nodes.get_mut(node_id)
    }

    /// Iterate every node in the graph mutably. Used during graph
    /// rebuild to extract plug-in chains from the *outgoing* TrackNodes
    /// before they're dropped, so plug-in instances survive the
    /// rebuild instead of being freed on the audio thread.
    pub fn iter_nodes_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn AudioNode>> {
        self.nodes.iter_mut()
    }

    /// Maximum block size the graph was built for. Plug-ins activate
    /// against this so they pre-size internal buffers correctly.
    pub fn buffer_size_hint(&self) -> u32 {
        self.buffer_size as u32
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Maximum per-node `latency_samples` in the graph. Kept for callers that
    /// only want a quick lower bound — see [`Self::total_latency_samples`]
    /// for the real critical-path figure.
    pub fn max_latency_samples(&self) -> u32 {
        self.nodes
            .iter()
            .map(|n| n.latency_samples())
            .max()
            .unwrap_or(0)
    }

    /// Critical-path latency in samples: the maximum, over every signal path
    /// in the graph, of the sum of `latency_samples()` along that path.
    /// Computed by DP over the topologically-sorted node list — each node's
    /// accumulated latency is `self_latency + max(incoming source latencies)`.
    /// This is the number plugin delay compensation would use to align
    /// parallel paths against the slowest branch.
    /// The critical-path latency the last `finalize_pdc` worked out, with
    /// the chain delays it was given. Cheap, for the audio thread.
    pub fn planned_latency_samples(&self) -> u32 {
        self.planned_latency
    }

    pub fn total_latency_samples(&self) -> u32 {
        let n = self.nodes.len();
        if n == 0 {
            return 0;
        }
        // Read before the graph is finished as well, so work the order out
        // here when it is stale rather than read an old one.
        let fresh = self.order_dirty.then(|| self.order_and_incoming());
        let (order, incoming) = match &fresh {
            Some((order, incoming)) => (order, incoming),
            None => (&self.processing_order, &self.incoming),
        };
        let mut acc = vec![0u32; n];
        for &node_id in order {
            let incoming_max = incoming[node_id]
                .iter()
                .map(|&e| acc[self.edges[e].source])
                .max()
                .unwrap_or(0);
            acc[node_id] = incoming_max.saturating_add(self.nodes[node_id].latency_samples());
        }
        acc.into_iter().max().unwrap_or(0)
    }
}

/// One block's worth of shared state, handed to the worker threads.
///
/// Raw pointers rather than references because several threads work on
/// different nodes of the same arrays at once. What keeps that sound is
/// the level grouping in [`crate::parallel::Levels`]: two nodes in a
/// level have no edge between them, so no two threads ever touch the
/// same slot.
struct NodeWork<'a> {
    nodes: *mut Box<dyn AudioNode>,
    node_count: usize,
    scratch: *mut NodeScratch,
    buffers: *mut Vec<Vec<f32>>,
    edge_delays: *mut Option<EdgeDelayLine>,
    edge_delay_count: usize,
    silent: *mut bool,
    edges: &'a [Edge],
    incoming: &'a [Vec<usize>],
    accepts_live_midi: &'a [bool],
    buffer_size: usize,
    midi_in: &'a [hardwave_midi::MidiEvent],
    ctx: &'a ProcessContext,
    /// The nodes whose id falls on this in [`CPU_STRIDE`] are timed.
    cpu_turn: usize,
}

// SAFETY: the nodes handed to different threads are distinct, and the
// batch does not outlive the block that published it.
unsafe impl Sync for NodeWork<'_> {}
unsafe impl Send for NodeWork<'_> {}

impl NodeWork<'_> {
    /// Gather one node's inputs, run it, and publish its output. The
    /// same work the single-threaded loop does, written so it can be
    /// called for several nodes at once.
    ///
    /// # Safety
    ///
    /// `node_id` must be a node of the level currently being processed,
    /// and no other thread may be processing the same id.
    unsafe fn process_node(&self, node_id: usize) {
        if node_id >= self.node_count {
            return;
        }
        // The edge's delay line, if it has one. Bounded: the lines are
        // made once the graph is finished, and a graph used before that
        // has none.
        let delay_of = |edge_idx: usize| -> Option<&mut Option<EdgeDelayLine>> {
            (edge_idx < self.edge_delay_count)
                .then(|| unsafe { &mut *self.edge_delays.add(edge_idx) })
        };
        // Same skip as the one-thread loop: an idle node with a silent
        // output and no delay line feeding it has nothing to do.
        let node = unsafe { &mut *self.nodes.add(node_id) };
        let hears_keys = !self.midi_in.is_empty()
            && self
                .accepts_live_midi
                .get(node_id)
                .copied()
                .unwrap_or(false);
        // Sources are in earlier levels, finished, so their silent flags
        // are this block's.
        let keyed = self.incoming[node_id].iter().any(|&e| {
            let edge = &self.edges[e];
            edge.dest_port >= 2 && !unsafe { *self.silent.add(edge.source) }
        });
        let idle = !hears_keys
            && !keyed
            && node.idle(self.ctx)
            && self.incoming[node_id]
                .iter()
                .all(|&e| delay_of(e).is_none_or(|d| d.is_none()));
        let silent = unsafe { &mut *self.silent.add(node_id) };
        if idle && *silent {
            return;
        }
        let scratch = unsafe { &mut *self.scratch.add(node_id) };
        // Only the block's frames, as in the one-thread loop.
        let frames = block_frames(self.ctx, self.buffer_size);
        for ch in &mut scratch.ch_bufs {
            let n = frames.min(ch.len());
            ch[..n].fill(0.0);
        }

        for &edge_idx in &self.incoming[node_id] {
            let e = &self.edges[edge_idx];
            if e.source >= self.node_count {
                continue;
            }
            let delay = delay_of(edge_idx).and_then(|d| d.as_mut());
            // A silent source adds nothing, unless a delay line has to
            // take the silence in. Its flag was set in an earlier level.
            if unsafe { *self.silent.add(e.source) } && delay.is_none() {
                continue;
            }
            // The source is a node of an earlier level, which finished
            // before this level started.
            let source_bufs = unsafe { &*self.buffers.add(e.source) };
            let Some(src) = source_bufs.get(e.source_port) else {
                continue;
            };
            let n = src.len().min(frames);
            let src_slice: &[f32] = if let Some(dl) = delay {
                dl.process(&src[..n], &mut scratch.delay_scratch[..n]);
                &scratch.delay_scratch[..n]
            } else {
                &src[..n]
            };
            if e.dest_port < scratch.ch_bufs.len() {
                let dest = &mut scratch.ch_bufs[e.dest_port];
                for (i, s) in src_slice.iter().enumerate() {
                    if i < dest.len() {
                        dest[i] += s * e.gain;
                    }
                }
            }
        }

        scratch.midi_out.clear();
        let input_refs: [&[f32]; NODE_CHANNELS] = [
            block_part(&scratch.ch_bufs[0], frames),
            block_part(&scratch.ch_bufs[1], frames),
            block_part(&scratch.ch_bufs[2], frames),
            block_part(&scratch.ch_bufs[3], frames),
        ];
        let midi_empty: [hardwave_midi::MidiEvent; 0] = [];
        let live: &[hardwave_midi::MidiEvent] = if self
            .accepts_live_midi
            .get(node_id)
            .copied()
            .unwrap_or(false)
        {
            self.midi_in
        } else {
            &midi_empty
        };
        let clocked = (node_id % CPU_STRIDE == self.cpu_turn).then(std::time::Instant::now);
        node.process(
            &input_refs,
            &mut scratch.outputs,
            live,
            &mut scratch.midi_out,
            self.ctx,
        );
        if let Some(started) = clocked {
            node.report_cpu(started.elapsed().as_nanos() as u64);
        }

        let buffers = unsafe { &mut *self.buffers.add(node_id) };
        std::mem::swap(&mut scratch.outputs, buffers);
        *silent = idle || node.output_is_silent();
    }
}

/// The short-block contract (see [`AudioNode::process`]) for a node's own
/// tests: a full block, then a `short` one with inputs exactly that long
/// and the graph-size outputs holding the full block, as the graph hands
/// them over. A read past the block's inputs panics. Returns the outputs.
#[cfg(test)]
pub(crate) fn run_short_block_after_a_full_one(
    node: &mut dyn AudioNode,
    graph_size: usize,
    short: usize,
) -> Vec<Vec<f32>> {
    let at = |position_samples: u64, frames: usize| ProcessContext {
        sample_rate: 48_000.0,
        buffer_size: frames as u32,
        tempo: 120.0,
        time_sig: (4, 4),
        position_samples,
        position_ticks: 0,
        playing: true,
        smart_disable: false,
    };
    let mut outputs = vec![vec![0.0f32; graph_size]; NODE_CHANNELS];
    let full = vec![0.5f32; graph_size];
    let inputs = [&full[..], &full[..], &full[..], &full[..]];
    node.process(
        &inputs,
        &mut outputs,
        &[],
        &mut Vec::new(),
        &at(0, graph_size),
    );
    let part = vec![0.25f32; short];
    let inputs = [&part[..], &part[..], &part[..], &part[..]];
    node.process(
        &inputs,
        &mut outputs,
        &[],
        &mut Vec::new(),
        &at(graph_size as u64, short),
    );
    for ch in outputs.iter().take(2) {
        assert!(ch.len() >= short, "the block's frames are written");
        assert!(ch[..short].iter().all(|s| s.is_finite()));
    }
    outputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct FakeNode {
        name: &'static str,
        latency: u32,
    }

    impl AudioNode for FakeNode {
        fn name(&self) -> &str {
            self.name
        }
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            _outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            _ctx: &ProcessContext,
        ) {
        }
        fn latency_samples(&self) -> u32 {
            self.latency
        }
    }

    fn node(n: &'static str, l: u32) -> Box<dyn AudioNode> {
        Box::new(FakeNode {
            name: n,
            latency: l,
        })
    }

    /// A source that writes a constant, or silence while idle, and counts
    /// the blocks it was run; and a sink that sums its input.
    struct Source {
        level: f32,
        idle: std::sync::Arc<std::sync::atomic::AtomicBool>,
        runs: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AudioNode for Source {
        fn name(&self) -> &str {
            "source"
        }
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            _ctx: &ProcessContext,
        ) {
            use std::sync::atomic::Ordering::Relaxed;
            self.runs.fetch_add(1, Relaxed);
            let v = if self.idle.load(Relaxed) {
                0.0
            } else {
                self.level
            };
            for ch in outputs.iter_mut() {
                ch.fill(v);
            }
        }
        fn idle(&self, _ctx: &ProcessContext) -> bool {
            self.idle.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    struct Sum;

    impl AudioNode for Sum {
        fn name(&self) -> &str {
            "sum"
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            _ctx: &ProcessContext,
        ) {
            outputs[0].copy_from_slice(inputs[0]);
        }
    }

    fn ctx() -> ProcessContext {
        ProcessContext {
            smart_disable: true,
            sample_rate: 48_000.0,
            buffer_size: 16,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: 0,
            position_ticks: 0,
            playing: true,
        }
    }

    #[test]
    fn an_idle_silent_node_is_skipped_and_the_sum_stays_right() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
        let mut g = AudioGraph::new(16);
        let idle = Arc::new(AtomicBool::new(true));
        let quiet_runs = Arc::new(AtomicUsize::new(0));
        let loud_runs = Arc::new(AtomicUsize::new(0));
        let quiet = g.add_node(Box::new(Source {
            level: 0.25,
            idle: idle.clone(),
            runs: quiet_runs.clone(),
        }));
        let loud = g.add_node(Box::new(Source {
            level: 0.5,
            idle: Arc::new(AtomicBool::new(false)),
            runs: loud_runs.clone(),
        }));
        let sink = g.add_node(Box::new(Sum));
        g.connect(quiet, 0, sink, 0);
        g.connect(loud, 0, sink, 0);
        g.finalize_pdc(true);

        for _ in 0..4 {
            g.process(&ctx(), &[]);
        }
        // Run once to write its silence, then left alone.
        assert_eq!(quiet_runs.load(Relaxed), 1);
        assert_eq!(loud_runs.load(Relaxed), 4);
        assert!(g.node_output(sink).unwrap()[0].iter().all(|v| *v == 0.5));

        // Once it has something to play it runs again, and is heard.
        idle.store(false, Relaxed);
        g.process(&ctx(), &[]);
        assert_eq!(quiet_runs.load(Relaxed), 2);
        assert!(g.node_output(sink).unwrap()[0].iter().all(|v| *v == 0.75));

        // And idle again: one block to fall silent, then skipped.
        idle.store(true, Relaxed);
        g.process(&ctx(), &[]);
        g.process(&ctx(), &[]);
        assert_eq!(quiet_runs.load(Relaxed), 3);
        assert!(g.node_output(sink).unwrap()[0].iter().all(|v| *v == 0.5));
    }

    /// A node fed sound on its sidechain ports (2 and 3) is run, even when
    /// it says it is idle: what it keys off is only there while it runs.
    #[test]
    fn a_node_with_sound_on_its_sidechain_is_not_left_out() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
        use std::sync::Arc;
        let mut g = AudioGraph::new(16);
        let keyed_runs = Arc::new(AtomicUsize::new(0));
        let key = g.add_node(Box::new(Source {
            level: 0.5,
            idle: Arc::new(AtomicBool::new(false)),
            runs: Arc::new(AtomicUsize::new(0)),
        }));
        let keyed = g.add_node(Box::new(Source {
            level: 0.25,
            idle: Arc::new(AtomicBool::new(true)),
            runs: keyed_runs.clone(),
        }));
        g.connect(key, 0, keyed, 2);
        g.finalize_pdc(true);
        for _ in 0..4 {
            g.process(&ctx(), &[]);
        }
        assert_eq!(keyed_runs.load(Relaxed), 4);
    }

    /// A node that says it wrote silence, whatever it wrote.
    struct SaysSilent;

    impl AudioNode for SaysSilent {
        fn name(&self) -> &str {
            "says silent"
        }
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            _ctx: &ProcessContext,
        ) {
            outputs[0].fill(0.25);
        }
        fn output_is_silent(&self) -> bool {
            true
        }
    }

    /// A node that says its block was silent is left out of the sums it
    /// feeds, on one thread and on several, and is still run every block.
    #[test]
    fn a_node_that_says_it_wrote_silence_is_left_out_of_the_sums() {
        for threads in [0, 2] {
            let mut g = AudioGraph::new(16);
            if threads > 0 {
                g.set_worker_pool(Some(std::sync::Arc::new(crate::parallel::WorkerPool::new(
                    threads,
                ))));
            }
            let quiet = g.add_node(Box::new(SaysSilent));
            let loud = g.add_node(Box::new(Source {
                level: 0.5,
                idle: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                runs: Default::default(),
            }));
            let sink = g.add_node(Box::new(Sum));
            g.connect(quiet, 0, sink, 0);
            g.connect(loud, 0, sink, 0);
            g.finalize_pdc(true);
            for _ in 0..3 {
                g.process(&ctx(), &[]);
                assert!(g.node_output(sink).unwrap()[0].iter().all(|v| *v == 0.5));
            }
        }
    }

    /// A node that counts the times it was timed for its load read-out.
    struct Clocked {
        reports: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AudioNode for Clocked {
        fn name(&self) -> &str {
            "clocked"
        }
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            _outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            _ctx: &ProcessContext,
        ) {
        }
        fn report_cpu(&self, _nanos: u64) {
            self.reports
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Each node is timed for its load read-out on one block in
    /// `CPU_STRIDE`, and a block times a few nodes, not all of them: every
    /// node read the clock twice on every block, which on a Windows virtual
    /// machine (14 microseconds a read) took a fifth of the block on a big
    /// song.
    #[test]
    fn a_block_times_a_few_nodes_and_every_node_gets_its_turn() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let mut g = AudioGraph::new(16);
        let counts: Vec<Arc<AtomicUsize>> =
            (0..80).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        for c in &counts {
            g.add_node(Box::new(Clocked { reports: c.clone() }));
        }
        g.finalize_pdc(true);
        let total =
            |counts: &[Arc<AtomicUsize>]| counts.iter().map(|c| c.load(Relaxed)).sum::<usize>();
        let mut most_in_a_block = 0;
        for _ in 0..3 * CPU_STRIDE {
            let before = total(&counts);
            g.process(&ctx(), &[]);
            most_in_a_block = most_in_a_block.max(total(&counts) - before);
        }
        assert!(
            most_in_a_block <= 80usize.div_ceil(CPU_STRIDE),
            "{most_in_a_block} nodes timed in one block"
        );
        for c in &counts {
            assert_eq!(c.load(Relaxed), 3, "each node timed once a round");
        }
    }

    /// A track's node for the rebuild test: a steady level, a latency, and
    /// the track id that names its edges across a rebuild. With no level it
    /// passes its input on, as the master does.
    struct Tracked {
        id: &'static str,
        latency: u32,
        level: Option<f32>,
    }

    impl AudioNode for Tracked {
        fn name(&self) -> &str {
            self.id
        }
        fn track_id(&self) -> Option<&str> {
            Some(self.id)
        }
        fn latency_samples(&self) -> u32 {
            self.latency
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            _ctx: &ProcessContext,
        ) {
            match self.level {
                Some(level) => outputs[0].fill(level),
                None => outputs[0].copy_from_slice(inputs[0]),
            }
        }
    }

    /// Counts up with the song: sample `p` of the song is `p + 1`. Only the
    /// block's frames are written. With `trims` it cuts its buffers to the
    /// block, as a track does; without, what is past the block keeps a
    /// stale value, as a node that leaves its buffers whole does.
    struct Ramp {
        latency: u32,
        trims: bool,
    }

    impl AudioNode for Ramp {
        fn name(&self) -> &str {
            "ramp"
        }
        fn latency_samples(&self) -> u32 {
            self.latency
        }
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            ctx: &ProcessContext,
        ) {
            let frames = ctx.buffer_size as usize;
            for ch in outputs.iter_mut() {
                if self.trims {
                    ch.resize(frames, 0.0);
                } else {
                    ch.fill(1000.0);
                }
                for (i, s) in ch.iter_mut().take(frames).enumerate() {
                    *s = (ctx.position_samples + i as u64 + 1) as f32;
                }
            }
        }
    }

    /// Per block: how long the inputs were, and the first two inputs.
    type Heard = Arc<parking_lot::Mutex<Vec<(usize, Vec<f32>, Vec<f32>)>>>;

    /// Keeps what reached its first two inputs, and how long they were.
    struct Probe(Heard);

    impl AudioNode for Probe {
        fn name(&self) -> &str {
            "probe"
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            _outputs: &mut [Vec<f32>],
            _midi_in: &[hardwave_midi::MidiEvent],
            _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
            ctx: &ProcessContext,
        ) {
            let frames = ctx.buffer_size as usize;
            self.0.lock().push((
                inputs[0].len().max(inputs[1].len()),
                inputs[0][..frames.min(inputs[0].len())].to_vec(),
                inputs[1][..frames.min(inputs[1].len())].to_vec(),
            ));
        }
    }

    /// Short blocks (the rest of a driver block cut into graph-size
    /// pieces) zero and sum only their own frames now. What a node hears
    /// must not depend on how the song was cut into blocks: the same as a
    /// render in whole blocks, through a compensation line too, and
    /// nothing from past the block's end.
    #[test]
    fn short_blocks_hear_the_same_as_whole_blocks() {
        const SIZE: usize = 16;
        const PAD: u64 = 5;
        let ramp = |p: u64| if p >= PAD { (p - PAD + 1) as f32 } else { 0.0 };
        for threads in [0, 2] {
            let mut g = AudioGraph::new(SIZE);
            if threads > 0 {
                g.set_worker_pool(Some(Arc::new(crate::parallel::WorkerPool::new(threads))));
            }
            // Two counters held back behind a slower third, all into one
            // node: their edges get a compensation line of PAD samples.
            let trimmed = g.add_node(Box::new(Ramp {
                latency: 0,
                trims: true,
            }));
            let whole = g.add_node(Box::new(Ramp {
                latency: 0,
                trims: false,
            }));
            let slow = g.add_node(Box::new(Ramp {
                latency: PAD as u32,
                trims: true,
            }));
            let heard = Arc::new(parking_lot::Mutex::new(Vec::new()));
            let probe = g.add_node(Box::new(Probe(Arc::clone(&heard))));
            g.connect(trimmed, 0, probe, 0);
            g.connect(slow, 0, probe, 0);
            g.connect(whole, 0, probe, 1);
            g.finalize_pdc(true);

            let blocks = [16, 3, 16, 16, 1, 9, 16, 5, 16, 2];
            let mut at = 0u64;
            for &n in &blocks {
                let mut c = ctx();
                c.buffer_size = n as u32;
                c.position_samples = at;
                g.process(&c, &[]);
                at += n as u64;
            }
            let heard = heard.lock();
            let mut first = Vec::new();
            let mut second = Vec::new();
            for (&n, (len, a, b)) in blocks.iter().zip(heard.iter()) {
                assert_eq!(
                    *len, n,
                    "{threads} threads: a node is handed the block's frames"
                );
                first.extend_from_slice(a);
                second.extend_from_slice(b);
            }
            let want_first: Vec<f32> = (0..at).map(|p| ramp(p) + (p + 1) as f32).collect();
            let want_second: Vec<f32> = (0..at).map(ramp).collect();
            assert_eq!(first, want_first, "{threads} threads");
            assert_eq!(second, want_second, "{threads} threads");
        }
    }

    /// A fast track lined up behind a slow one, both into the master.
    fn padded_song(g: &mut AudioGraph) -> NodeId {
        let fast = g.add_node(Box::new(Tracked {
            id: "fast",
            latency: 0,
            level: Some(0.5),
        }));
        let slow = g.add_node(Box::new(Tracked {
            id: "slow",
            latency: 8,
            level: Some(0.0),
        }));
        let master = g.add_node(Box::new(Tracked {
            id: "master",
            latency: 0,
            level: None,
        }));
        g.connect(fast, 0, master, 0);
        g.connect(slow, 0, master, 0);
        g.finalize_pdc(true);
        master
    }

    /// A rebuild (a mute, a solo, a send) keeps the audio waiting in the
    /// compensation delay lines. The new lines started empty, so a track
    /// held back behind a slower one dropped out for that long.
    #[test]
    fn a_rebuild_keeps_the_audio_in_the_compensation_lines() {
        let mut g = AudioGraph::new(16);
        padded_song(&mut g);
        g.process(&ctx(), &[]);
        g.process(&ctx(), &[]);
        g.index_delays();
        let mut next = AudioGraph::new(16);
        let master = padded_song(&mut next);
        next.index_delays();
        next.carry_delays_from(&mut g);
        let mut g = next;
        g.process(&ctx(), &[]);
        let out = &g.node_output(master).unwrap()[0];
        assert!(
            out.iter().all(|v| *v == 0.5),
            "no gap after the rebuild: {out:?}"
        );
    }

    #[test]
    fn total_latency_is_zero_for_empty_graph() {
        let g = AudioGraph::new(64);
        assert_eq!(g.total_latency_samples(), 0);
    }

    #[test]
    fn total_latency_accumulates_along_chain() {
        let mut g = AudioGraph::new(64);
        let a = g.add_node(node("A", 64));
        let b = g.add_node(node("B", 128));
        let c = g.add_node(node("C", 256));
        g.connect(a, 0, b, 0);
        g.connect(b, 0, c, 0);
        // critical path = 64 + 128 + 256 = 448
        assert_eq!(g.total_latency_samples(), 448);
    }

    #[test]
    fn total_latency_takes_max_of_parallel_branches() {
        let mut g = AudioGraph::new(64);
        let src = g.add_node(node("src", 0));
        let slow = g.add_node(node("slow", 1024));
        let fast = g.add_node(node("fast", 64));
        let sink = g.add_node(node("sink", 0));
        g.connect(src, 0, slow, 0);
        g.connect(src, 0, fast, 0);
        g.connect(slow, 0, sink, 0);
        g.connect(fast, 0, sink, 0);
        // Critical path runs through the slow branch: 0 + 1024 + 0 = 1024
        assert_eq!(g.total_latency_samples(), 1024);
    }

    #[test]
    fn max_latency_differs_from_total_on_chains() {
        let mut g = AudioGraph::new(64);
        let a = g.add_node(node("A", 100));
        let b = g.add_node(node("B", 100));
        g.connect(a, 0, b, 0);
        // Per-node max is 100, but the correct critical-path figure is 200.
        assert_eq!(g.max_latency_samples(), 100);
        assert_eq!(g.total_latency_samples(), 200);
    }

    #[test]
    fn finalize_pdc_leaves_single_chain_uncompensated() {
        let mut g = AudioGraph::new(64);
        let a = g.add_node(node("A", 0));
        let b = g.add_node(node("B", 100));
        let c = g.add_node(node("C", 50));
        g.connect(a, 0, b, 0);
        g.connect(b, 0, c, 0);
        g.finalize_pdc(true);
        for e in &g.edges {
            assert_eq!(e.compensation_samples, 0, "single chain needs no PDC");
        }
    }

    #[test]
    fn with_compensation_off_no_branch_is_padded() {
        // The same slow and fast branches as the test below.
        let mut g = AudioGraph::new(64);
        let src = g.add_node(node("src", 0));
        let slow = g.add_node(node("slow", 1000));
        let fast = g.add_node(node("fast", 100));
        let sink = g.add_node(node("sink", 0));
        g.connect(src, 0, slow, 0);
        g.connect(src, 0, fast, 0);
        g.connect(slow, 0, sink, 0);
        g.connect(fast, 0, sink, 0);
        g.finalize_pdc(false);
        assert!(g.edges.iter().all(|e| e.compensation_samples == 0));
        g.finalize_pdc(true);
        assert!(g.edges.iter().any(|e| e.compensation_samples > 0));
    }

    #[test]
    fn finalize_pdc_pads_fast_branch_to_match_slow() {
        // src feeds both `slow` (latency 1000) and `fast` (latency 100);
        // both feed `sink`. `fast → sink` needs 900 samples of delay so it
        // lines up with `slow → sink` at the sink's input.
        let mut g = AudioGraph::new(64);
        let src = g.add_node(node("src", 0));
        let slow = g.add_node(node("slow", 1000));
        let fast = g.add_node(node("fast", 100));
        let sink = g.add_node(node("sink", 0));
        g.connect(src, 0, slow, 0);
        g.connect(src, 0, fast, 0);
        g.connect(slow, 0, sink, 0);
        g.connect(fast, 0, sink, 0);
        g.finalize_pdc(true);
        let mut slow_to_sink = None;
        let mut fast_to_sink = None;
        for e in &g.edges {
            if e.source == slow && e.dest == sink {
                slow_to_sink = Some(e.compensation_samples);
            }
            if e.source == fast && e.dest == sink {
                fast_to_sink = Some(e.compensation_samples);
            }
        }
        assert_eq!(slow_to_sink, Some(0), "slowest branch already aligns");
        assert_eq!(fast_to_sink, Some(900), "fast branch padded to 1000");
    }

    #[test]
    fn edge_delay_line_delays_samples_by_configured_amount() {
        let mut dl = EdgeDelayLine::new(3, 8);
        let mut out = [0.0_f32; 8];
        let input: [f32; 8] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        dl.process(&input, &mut out);
        // First 3 samples were pre-filled zeros; then 1, 2, 3, 4, 5.
        assert_eq!(out, [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn edge_delay_line_with_zero_samples_is_identity() {
        let mut dl = EdgeDelayLine::new(0, 4);
        let mut out = [0.0_f32; 4];
        let input: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
        dl.process(&input, &mut out);
        assert_eq!(out, input);
    }
}
