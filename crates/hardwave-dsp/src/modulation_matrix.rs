//! Modulation-matrix visualization — the data the FM synth's UI
//! needs to draw per-operator modulation routing as a grid / wiring
//! diagram.

/// One modulation connection — `modulator_operator_index → carrier`
/// with a signed amount in `[-1, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModConnection {
    pub modulator: usize,
    pub carrier: usize,
    pub amount: f32,
}

/// Full modulation-matrix description for an N-operator engine.
/// `connections` are directed edges; `feedback` is the self-loop
/// amount per operator.
#[derive(Debug, Clone)]
pub struct ModulationMatrix {
    pub operator_count: usize,
    pub connections: Vec<ModConnection>,
    pub feedback: Vec<f32>,
}

impl ModulationMatrix {
    pub fn new(operator_count: usize) -> Self {
        Self {
            operator_count,
            connections: Vec::new(),
            feedback: vec![0.0; operator_count],
        }
    }

    /// Build from the canonical FM algorithm index — maps 0..8 to
    /// the classic DX7-style operator routings. Feedback defaults
    /// stay zero; the caller applies per-preset feedback after.
    pub fn from_algorithm(algorithm_index: u8) -> Self {
        // Simple 4-operator algorithm set: op_i modulates op_j per
        // the hand-picked topology list. Carriers (those actually
        // sent to output) are not represented here since the edge
        // list only describes the modulator → carrier chain.
        let edges: Vec<(usize, usize)> = match algorithm_index % 8 {
            // 0: op3→op2→op1→op0 (single stack).
            0 => vec![(3, 2), (2, 1), (1, 0)],
            // 1: op3→op2, op2→op0, op1→op0 (parallel mod of op0).
            1 => vec![(3, 2), (2, 0), (1, 0)],
            // 2: op3→op2, op1→op0 (two parallel pairs).
            2 => vec![(3, 2), (1, 0)],
            // 3: op3→op1, op3→op0, op2→op0 (fan-out from op3).
            3 => vec![(3, 1), (3, 0), (2, 0)],
            // 4: op3→op2→op1, op1→op0 (longer stack + split).
            4 => vec![(3, 2), (2, 1), (1, 0)],
            // 5: op2→op1→op0 (no op3 modulation).
            5 => vec![(2, 1), (1, 0)],
            // 6: op3→op1, op2→op0 (two independent pairs).
            6 => vec![(3, 1), (2, 0)],
            // 7: everything direct (all operators as carriers).
            _ => Vec::new(),
        };
        let connections = edges
            .into_iter()
            .map(|(m, c)| ModConnection {
                modulator: m,
                carrier: c,
                amount: 1.0,
            })
            .collect();
        Self {
            operator_count: 4,
            connections,
            feedback: vec![0.0; 4],
        }
    }

    pub fn set_feedback(&mut self, op: usize, amount: f32) {
        if op < self.feedback.len() {
            self.feedback[op] = amount.clamp(-1.0, 1.0);
        }
    }

    pub fn set_amount(&mut self, modulator: usize, carrier: usize, amount: f32) {
        if let Some(c) = self
            .connections
            .iter_mut()
            .find(|c| c.modulator == modulator && c.carrier == carrier)
        {
            c.amount = amount.clamp(-1.0, 1.0);
        } else {
            self.connections.push(ModConnection {
                modulator,
                carrier,
                amount: amount.clamp(-1.0, 1.0),
            });
        }
    }

    /// Adjacency matrix — `mat[modulator][carrier]` = amount. Useful
    /// for the UI's grid-style display.
    pub fn adjacency(&self) -> Vec<Vec<f32>> {
        let n = self.operator_count;
        let mut mat = vec![vec![0.0_f32; n]; n];
        for c in &self.connections {
            if c.modulator < n && c.carrier < n {
                mat[c.modulator][c.carrier] = c.amount;
            }
        }
        mat
    }

    /// Carriers (operators with nothing modulating further along the
    /// chain) — these get summed into the audio output. Derived from
    /// the connection list as operators that do not appear as the
    /// source of any edge.
    pub fn carriers(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for op in 0..self.operator_count {
            if !self.connections.iter().any(|c| c.modulator == op) {
                out.push(op);
            }
        }
        out
    }

    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_0_is_single_stack() {
        let m = ModulationMatrix::from_algorithm(0);
        assert_eq!(m.operator_count, 4);
        assert_eq!(m.connection_count(), 3);
        // Op3 → Op2 → Op1 → Op0. Carrier is Op0 only.
        assert_eq!(m.carriers(), vec![0]);
    }

    #[test]
    fn algorithm_7_has_no_modulation_all_operators_are_carriers() {
        let m = ModulationMatrix::from_algorithm(7);
        assert_eq!(m.connection_count(), 0);
        assert_eq!(m.carriers(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn algorithm_index_wraps_modulo_8() {
        let m = ModulationMatrix::from_algorithm(15);
        let reference = ModulationMatrix::from_algorithm(7);
        assert_eq!(m.connection_count(), reference.connection_count());
    }

    #[test]
    fn set_amount_inserts_or_updates() {
        let mut m = ModulationMatrix::new(4);
        m.set_amount(3, 2, 0.5);
        m.set_amount(3, 2, 2.0); // clamped to 1.0
        assert_eq!(m.connection_count(), 1);
        assert!((m.connections[0].amount - 1.0).abs() < 1e-6);
    }

    #[test]
    fn feedback_clamps_and_indexes_correctly() {
        let mut m = ModulationMatrix::new(4);
        m.set_feedback(0, 2.0);
        m.set_feedback(9, 0.5); // out-of-range → no-op
        assert_eq!(m.feedback[0], 1.0);
        assert_eq!(m.feedback.len(), 4);
    }

    #[test]
    fn adjacency_reflects_connections() {
        let m = ModulationMatrix::from_algorithm(0);
        let adj = m.adjacency();
        assert_eq!(adj.len(), 4);
        assert_eq!(adj[0].len(), 4);
        assert!((adj[3][2] - 1.0).abs() < 1e-6);
        assert!((adj[2][1] - 1.0).abs() < 1e-6);
        assert!((adj[1][0] - 1.0).abs() < 1e-6);
        // No feedback / diagonal should all be zero.
        for (i, row) in adj.iter().enumerate() {
            assert_eq!(row[i], 0.0);
        }
    }
}
