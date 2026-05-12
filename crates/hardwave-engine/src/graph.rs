//! Audio graph — DAG of processing nodes, topologically sorted for real-time execution.

pub type NodeId = usize;

/// Context passed to each node during processing.
pub struct ProcessContext {
    pub sample_rate: f64,
    pub buffer_size: u32,
    pub tempo: f64,
    pub time_sig: (u32, u32),
    pub position_samples: u64,
    pub playing: bool,
}

/// Trait implemented by all nodes in the audio graph.
pub trait AudioNode: Send {
    fn name(&self) -> &str;

    /// Process audio. Reads from `inputs`, writes to `outputs`.
    /// Each slice is one channel of audio (non-interleaved).
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
    /// Per-edge PDC delay lines, indexed 1:1 with `edges`.
    edge_delays: Vec<Option<EdgeDelayLine>>,
    processing_order: Vec<NodeId>,
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
}

impl AudioGraph {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            edge_delays: Vec::new(),
            processing_order: Vec::new(),
            buffers: Vec::new(),
            scratch: Vec::new(),
            accepts_live_midi: Vec::new(),
            buffer_size,
        }
    }

    /// Mark a node as accepting external/live MIDI for the next call to
    /// [`AudioGraph::process`]. The engine sets this during graph rebuild
    /// based on `track.armed && track.monitor_input` (and as a default
    /// for MIDI tracks). Out-of-range ids silently no-op so the engine's
    /// rebuild ordering — `add_node` then `set_accepts_live_midi` — is
    /// guaranteed safe.
    pub fn set_accepts_live_midi(&mut self, node_id: NodeId, accepts: bool) {
        if let Some(slot) = self.accepts_live_midi.get_mut(node_id) {
            *slot = accepts;
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
        self.rebuild_order();
        id
    }

    /// Connect source node's output to dest node's input with unity gain.
    pub fn connect(&mut self, source: NodeId, source_port: usize, dest: NodeId, dest_port: usize) {
        self.connect_with_gain(source, source_port, dest, dest_port, 1.0);
    }

    /// Connect with a linear gain multiplier. Used for sends.
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
        self.rebuild_order();
    }

    /// Remove all nodes and edges.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.edge_delays.clear();
        self.processing_order.clear();
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
    pub fn finalize_pdc(&mut self) {
        let n = self.nodes.len();
        if n == 0 {
            self.edge_delays.clear();
            return;
        }
        // Accumulated latency per node (critical path into the node +
        // node's own latency) — same DP as `total_latency_samples`.
        let mut acc = vec![0u32; n];
        for &node_id in &self.processing_order {
            let mut incoming_max = 0u32;
            for edge in &self.edges {
                if edge.dest == node_id {
                    incoming_max = incoming_max.max(acc[edge.source]);
                }
            }
            acc[node_id] = incoming_max.saturating_add(self.nodes[node_id].latency_samples());
        }
        // For each edge, compute the pad samples needed so the source arrives
        // aligned with the slowest incoming branch of the same dest.
        // Two passes: write compensation_samples on edges, then build delay
        // lines — separate borrows keep the borrow checker happy.
        let pads: Vec<u32> = self
            .edges
            .iter()
            .map(|edge| {
                let dest_incoming_max = self
                    .edges
                    .iter()
                    .filter(|e| e.dest == edge.dest)
                    .map(|e| acc[e.source])
                    .max()
                    .unwrap_or(0);
                dest_incoming_max.saturating_sub(acc[edge.source])
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
        let n = self.nodes.len();
        let mut in_degree = vec![0u32; n];
        let mut adjacency: Vec<Vec<NodeId>> = vec![Vec::new(); n];

        for edge in &self.edges {
            in_degree[edge.dest] += 1;
            adjacency[edge.source].push(edge.dest);
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

        self.processing_order = order;
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
    /// across blocks. The per-edge `src_vec` clone at line ~360 is the
    /// only remaining allocation — it survives this refactor because
    /// removing it requires extracting `edge_delays` out of `self` for
    /// split-borrow reasons (planned follow-up).
    pub fn process(&mut self, ctx: &ProcessContext, midi_in: &[hardwave_midi::MidiEvent]) {
        // Iterate by index — `processing_order` mutates inside the loop
        // body via no path, so a snapshot clone is unnecessary. (The
        // previous code's `.clone()` was defensive but never load-bearing.)
        let midi_empty: [hardwave_midi::MidiEvent; 0] = [];
        let order_len = self.processing_order.len();

        for order_idx in 0..order_len {
            let node_id = self.processing_order[order_idx];

            // Zero the scratch's input buffers in place — no realloc.
            for ch in &mut self.scratch[node_id].ch_bufs {
                ch.fill(0.0);
            }

            // Gather inputs from edges that target this node.
            for edge_idx in 0..self.edges.len() {
                let (edge_source, edge_source_port, edge_dest, edge_dest_port, edge_gain) = {
                    let e = &self.edges[edge_idx];
                    (e.source, e.source_port, e.dest, e.dest_port, e.gain)
                };
                if edge_dest != node_id {
                    continue;
                }
                // Snapshot the source channel so we can release the
                // immutable borrow on `self.buffers` and then access
                // `self.edge_delays` + `self.scratch` mutably. The
                // clone here is the last per-block alloc; see process
                // doc comment.
                let src_vec: Vec<f32> = match self
                    .buffers
                    .get(edge_source)
                    .and_then(|b| b.get(edge_source_port))
                {
                    Some(ch) => ch.clone(),
                    None => continue,
                };
                let n = src_vec.len().min(self.buffer_size);
                let scratch = &mut self.scratch[node_id];
                let src_slice: &[f32] =
                    if let Some(Some(dl)) = self.edge_delays.get_mut(edge_idx) {
                        dl.process(&src_vec[..n], &mut scratch.delay_scratch[..n]);
                        &scratch.delay_scratch[..n]
                    } else {
                        &src_vec[..n]
                    };
                if edge_dest_port < scratch.ch_bufs.len() {
                    let dest = &mut scratch.ch_bufs[edge_dest_port];
                    for (i, s) in src_slice.iter().enumerate() {
                        if i < dest.len() {
                            dest[i] += s * edge_gain;
                        }
                    }
                }
            }

            // Run the node. Stack-allocated input_refs avoids a
            // per-iteration `Vec::collect`. NODE_CHANNELS is small + const
            // so the array stays cheap.
            let scratch = &mut self.scratch[node_id];
            scratch.midi_out.clear();
            let input_refs: [&[f32]; NODE_CHANNELS] = [
                scratch.ch_bufs[0].as_slice(),
                scratch.ch_bufs[1].as_slice(),
                scratch.ch_bufs[2].as_slice(),
                scratch.ch_bufs[3].as_slice(),
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
            self.nodes[node_id].process(
                &input_refs,
                &mut scratch.outputs,
                live,
                &mut scratch.midi_out,
                ctx,
            );

            // Swap the freshly-written output with the persistent buffer.
            // Zero copy — the persistent buffer becomes the next-block
            // scratch.outputs, which we'll .fill(0.0) on next call.
            std::mem::swap(&mut scratch.outputs, &mut self.buffers[node_id]);
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
    pub fn total_latency_samples(&self) -> u32 {
        let n = self.nodes.len();
        if n == 0 {
            return 0;
        }
        let mut acc = vec![0u32; n];
        for &node_id in &self.processing_order {
            let mut incoming_max = 0u32;
            for edge in &self.edges {
                if edge.dest == node_id {
                    incoming_max = incoming_max.max(acc[edge.source]);
                }
            }
            acc[node_id] = incoming_max.saturating_add(self.nodes[node_id].latency_samples());
        }
        acc.into_iter().max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        g.finalize_pdc();
        for e in &g.edges {
            assert_eq!(e.compensation_samples, 0, "single chain needs no PDC");
        }
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
        g.finalize_pdc();
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
