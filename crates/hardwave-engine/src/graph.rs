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
}

/// The audio graph. Nodes are stored in topological order for lock-free processing.
pub struct AudioGraph {
    nodes: Vec<Box<dyn AudioNode>>,
    edges: Vec<Edge>,
    processing_order: Vec<NodeId>,
    /// Pre-allocated buffers for intermediate data.
    buffers: Vec<Vec<Vec<f32>>>, // [node][channel][samples]
    buffer_size: usize,
}

impl AudioGraph {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            processing_order: Vec::new(),
            buffers: Vec::new(),
            buffer_size,
        }
    }

    /// Add a node, returns its ID.
    pub fn add_node(&mut self, node: Box<dyn AudioNode>) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        self.buffers
            .push(vec![vec![0.0; self.buffer_size]; NODE_CHANNELS]);
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
        });
        self.rebuild_order();
    }

    /// Remove all nodes and edges.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.processing_order.clear();
        self.buffers.clear();
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

    /// Process the entire graph for one buffer. Called on the audio thread.
    pub fn process(&mut self, ctx: &ProcessContext) {
        let order = self.processing_order.clone();
        let midi_empty: Vec<hardwave_midi::MidiEvent> = Vec::new();

        for &node_id in &order {
            // Gather inputs from connected source nodes
            let inputs: Vec<Vec<f32>> = {
                let mut ch_bufs: Vec<Vec<f32>> = vec![vec![0.0; self.buffer_size]; NODE_CHANNELS];
                for edge in &self.edges {
                    if edge.dest == node_id {
                        if let Some(src_buf) = self.buffers.get(edge.source) {
                            if let Some(ch) = src_buf.get(edge.source_port) {
                                if edge.dest_port < ch_bufs.len() {
                                    let g = edge.gain;
                                    for (i, s) in ch.iter().enumerate() {
                                        if i < ch_bufs[edge.dest_port].len() {
                                            ch_bufs[edge.dest_port][i] += s * g;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                ch_bufs
            };

            let input_refs: Vec<&[f32]> = inputs.iter().map(|v| v.as_slice()).collect();
            let mut outputs = vec![vec![0.0f32; self.buffer_size]; NODE_CHANNELS];
            let mut midi_out = Vec::new();

            self.nodes[node_id].process(&input_refs, &mut outputs, &midi_empty, &mut midi_out, ctx);

            self.buffers[node_id] = outputs;
        }
    }

    /// Get the output buffer of a specific node (e.g., the master node).
    pub fn node_output(&self, node_id: NodeId) -> Option<&[Vec<f32>]> {
        self.buffers.get(node_id).map(|v| v.as_slice())
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
}
