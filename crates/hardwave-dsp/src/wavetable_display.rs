//! 2D display helpers for the wavetable synth's UI — sample the
//! current wavetable at a dense grid of (position, phase) pairs so
//! the caller can render a 2D heatmap / waveform-per-frame ribbon.

use crate::wavetable::Wavetable;

/// One frame's worth of normalized samples at evenly-spaced phase
/// positions. `frame.len() == phase_steps` with values in
/// approximately `[-1, 1]`.
pub type DisplayFrame = Vec<f32>;

/// The full 2D display matrix — `frames.len()` position slices, each
/// of length `phase_steps`. Rendered as a ribbon of waveforms or a
/// heatmap with color == sample value.
pub struct WavetableDisplay {
    pub frames: Vec<DisplayFrame>,
    pub position_steps: usize,
    pub phase_steps: usize,
}

impl WavetableDisplay {
    /// Build a dense sampling at `position_steps × phase_steps`
    /// resolution. Every column is one wavetable frame position in
    /// `[0, 1]`; every row is one phase in `[0, 1]`.
    pub fn build(wt: &Wavetable, position_steps: usize, phase_steps: usize) -> Self {
        let position_steps = position_steps.max(1);
        let phase_steps = phase_steps.max(1);
        let mut frames = Vec::with_capacity(position_steps);
        for p in 0..position_steps {
            let pos = if position_steps == 1 {
                0.0
            } else {
                p as f32 / (position_steps - 1) as f32
            };
            let mut frame = Vec::with_capacity(phase_steps);
            for i in 0..phase_steps {
                let phase = i as f32 / phase_steps as f32;
                frame.push(wt.sample(phase, pos));
            }
            frames.push(frame);
        }
        Self {
            frames,
            position_steps,
            phase_steps,
        }
    }

    /// Per-frame peak amplitude — useful for normalizing the render
    /// height.
    pub fn peaks(&self) -> Vec<f32> {
        self.frames
            .iter()
            .map(|f| f.iter().fold(0.0_f32, |acc, v| acc.max(v.abs())))
            .collect()
    }

    /// Linearized heatmap — one `f32` per cell, row-major, with cell
    /// `(p, ph)` at index `p * phase_steps + ph`.
    pub fn heatmap_flat(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.position_steps * self.phase_steps);
        for frame in &self.frames {
            out.extend_from_slice(frame);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wavetable::Wavetable;

    #[test]
    fn display_has_requested_resolution() {
        let wt = Wavetable::basic();
        let disp = WavetableDisplay::build(&wt, 16, 128);
        assert_eq!(disp.position_steps, 16);
        assert_eq!(disp.phase_steps, 128);
        assert_eq!(disp.frames.len(), 16);
        assert!(disp.frames.iter().all(|f| f.len() == 128));
    }

    #[test]
    fn basic_wavetable_peak_per_frame_matches_expectation() {
        let wt = Wavetable::basic();
        let disp = WavetableDisplay::build(&wt, 4, 256);
        let peaks = disp.peaks();
        // Sine / triangle / saw / square — each frame's peak should
        // be close to 1.0.
        for (i, p) in peaks.iter().enumerate() {
            assert!(*p > 0.8, "frame {} peak {}", i, p);
        }
    }

    #[test]
    fn heatmap_flat_is_row_major() {
        let wt = Wavetable::basic();
        let disp = WavetableDisplay::build(&wt, 2, 4);
        let flat = disp.heatmap_flat();
        assert_eq!(flat.len(), 2 * 4);
        // Row-major: first four samples should match frames[0].
        assert_eq!(&flat[..4], &disp.frames[0][..]);
        assert_eq!(&flat[4..], &disp.frames[1][..]);
    }

    #[test]
    fn single_position_wavetable_returns_one_frame() {
        let wt = Wavetable::noise();
        let disp = WavetableDisplay::build(&wt, 1, 64);
        assert_eq!(disp.frames.len(), 1);
        assert_eq!(disp.frames[0].len(), 64);
    }
}
