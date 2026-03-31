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

    fn latency_samples(&self) -> u32 { 0 }
    fn reset(&mut self) {}
}

/// Edge in the audio graph.
#[derive(Debug, Clone)]
struct Edge {
    source: NodeId,
    source_port: usize,
    dest: NodeId,
    dest_port: usize,
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
        // Allocate 2-channel buffer for this node
        self.buffers.push(vec![vec![0.0; self.buffer_size]; 2]);
        self.rebuild_order();
        id
    }

    /// Connect source node's output to dest node's input.
    pub fn connect(&mut self, source: NodeId, source_port: usize, dest: NodeId, dest_port: usize) {
        self.edges.push(Edge { source, source_port, dest, dest_port });
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
                let mut ch_bufs: Vec<Vec<f32>> = vec![vec![0.0; self.buffer_size]; 2];
                for edge in &self.edges {
                    if edge.dest == node_id {
                        if let Some(src_buf) = self.buffers.get(edge.source) {
                            if let Some(ch) = src_buf.get(edge.source_port) {
                                if edge.dest_port < ch_bufs.len() {
                                    for (i, s) in ch.iter().enumerate() {
                                        if i < ch_bufs[edge.dest_port].len() {
                                            ch_bufs[edge.dest_port][i] += s;
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
            let mut outputs = vec![vec![0.0f32; self.buffer_size]; 2];
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
}
