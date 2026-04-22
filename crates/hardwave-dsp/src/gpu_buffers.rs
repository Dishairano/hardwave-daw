//! GPU-ready buffer builders for the waveform, spectrum, and meter
//! displays. The DSP side produces the exact vertex arrays the
//! WebGL / Canvas-GPU frontend uploads — no JS-side recomputation,
//! no per-frame allocation in the audio thread.

/// A 2D vertex with an implicit Z=0 and a single float attribute
/// for shaders to read. Tightly packed so the frontend can
/// `bufferData` the raw slice.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Vertex2D {
    pub x: f32,
    pub y: f32,
    pub attr: f32,
}

/// Build a triangle-strip waveform — each sample contributes two
/// vertices (top + bottom mirrored around `y_baseline`) so the
/// GPU rasterizer can fill the filled waveform in one draw call.
/// Samples are decimated to `target_vertices / 2` if longer.
pub fn waveform_triangle_strip(
    samples: &[f32],
    target_vertex_pairs: usize,
    width: f32,
    amplitude: f32,
    y_baseline: f32,
) -> Vec<Vertex2D> {
    if samples.is_empty() || target_vertex_pairs == 0 {
        return Vec::new();
    }
    let pairs = target_vertex_pairs.max(2);
    let mut out = Vec::with_capacity(pairs * 2);
    let step = samples.len() as f32 / pairs as f32;
    for i in 0..pairs {
        let src_start = (i as f32 * step) as usize;
        let src_end = ((i + 1) as f32 * step).min(samples.len() as f32) as usize;
        let slice = &samples[src_start..src_end.max(src_start + 1).min(samples.len())];
        let peak = slice
            .iter()
            .fold(0.0_f32, |acc, &v| acc.max(v.abs()))
            .min(1.0);
        let x = (i as f32 / (pairs - 1) as f32) * width;
        let amp = peak * amplitude;
        out.push(Vertex2D {
            x,
            y: y_baseline - amp,
            attr: peak,
        });
        out.push(Vertex2D {
            x,
            y: y_baseline + amp,
            attr: peak,
        });
    }
    out
}

/// Build a bar-chart spectrum — one quad per frequency bin, log-
/// spaced on the x axis, with height derived from `spectrum_db`.
/// Returns `6 × bin_count` vertices (two triangles per quad).
#[allow(clippy::too_many_arguments)]
pub fn spectrum_bars_vertices(
    spectrum_db: &[f32],
    sample_rate: f32,
    low_hz: f32,
    high_hz: f32,
    bar_count: usize,
    width: f32,
    height: f32,
    min_db: f32,
    max_db: f32,
) -> Vec<Vertex2D> {
    if spectrum_db.is_empty() || bar_count == 0 || sample_rate <= 0.0 {
        return Vec::new();
    }
    let bin_hz = sample_rate / (2.0 * spectrum_db.len() as f32);
    let log_lo = low_hz.max(1.0).log10();
    let log_hi = high_hz.max(low_hz + 1.0).log10();
    let mut out = Vec::with_capacity(bar_count * 6);
    let bar_width = width / bar_count as f32;
    let db_range = (max_db - min_db).max(1e-3);
    for i in 0..bar_count {
        let t = i as f32 / (bar_count - 1).max(1) as f32;
        let freq = 10_f32.powf(log_lo + (log_hi - log_lo) * t);
        let bin = (freq / bin_hz).round() as usize;
        let bin = bin.min(spectrum_db.len() - 1);
        let db = spectrum_db[bin].clamp(min_db, max_db);
        let normalized = ((db - min_db) / db_range).clamp(0.0, 1.0);
        let x0 = (i as f32) * bar_width;
        let x1 = x0 + bar_width * 0.9;
        let y0 = height;
        let y1 = height * (1.0 - normalized);
        // First triangle: TL, BL, BR.
        out.push(Vertex2D {
            x: x0,
            y: y1,
            attr: normalized,
        });
        out.push(Vertex2D {
            x: x0,
            y: y0,
            attr: 0.0,
        });
        out.push(Vertex2D {
            x: x1,
            y: y0,
            attr: 0.0,
        });
        // Second triangle: TL, BR, TR.
        out.push(Vertex2D {
            x: x0,
            y: y1,
            attr: normalized,
        });
        out.push(Vertex2D {
            x: x1,
            y: y0,
            attr: 0.0,
        });
        out.push(Vertex2D {
            x: x1,
            y: y1,
            attr: normalized,
        });
    }
    out
}

/// Build a single-meter quad — two triangles / six vertices. The
/// caller passes `value_db` (current peak or RMS) + the meter's
/// display range; this builds the height-filled rectangle.
pub fn meter_quad(
    value_db: f32,
    min_db: f32,
    max_db: f32,
    width: f32,
    height: f32,
) -> [Vertex2D; 6] {
    let db_range = (max_db - min_db).max(1e-3);
    let normalized = ((value_db - min_db) / db_range).clamp(0.0, 1.0);
    let y0 = height;
    let y1 = height * (1.0 - normalized);
    [
        Vertex2D {
            x: 0.0,
            y: y1,
            attr: normalized,
        },
        Vertex2D {
            x: 0.0,
            y: y0,
            attr: 0.0,
        },
        Vertex2D {
            x: width,
            y: y0,
            attr: 0.0,
        },
        Vertex2D {
            x: 0.0,
            y: y1,
            attr: normalized,
        },
        Vertex2D {
            x: width,
            y: y0,
            attr: 0.0,
        },
        Vertex2D {
            x: width,
            y: y1,
            attr: normalized,
        },
    ]
}

/// Frame-pacer — returns whether the next frame should render, given
/// the elapsed time since the last render and a target frame rate.
/// Drops duplicate requests that arrive within one frame's worth of
/// time so smooth 60 fps renders under CPU load by skipping
/// redundant draws.
pub struct FramePacer {
    target_fps: f32,
    last_render_time_ms: f32,
    first_call: bool,
    accumulated_skips: u32,
}

impl FramePacer {
    pub fn new(target_fps: f32) -> Self {
        Self {
            target_fps: target_fps.clamp(10.0, 240.0),
            last_render_time_ms: 0.0,
            first_call: true,
            accumulated_skips: 0,
        }
    }

    pub fn set_target_fps(&mut self, fps: f32) {
        self.target_fps = fps.clamp(10.0, 240.0);
    }

    pub fn target_fps(&self) -> f32 {
        self.target_fps
    }

    pub fn skipped_frames(&self) -> u32 {
        self.accumulated_skips
    }

    /// Call before each candidate render. Returns `true` if the
    /// render should proceed; `false` tells the caller to skip this
    /// frame and try again next vsync.
    pub fn should_render(&mut self, now_ms: f32) -> bool {
        if self.first_call {
            self.first_call = false;
            self.last_render_time_ms = now_ms;
            return true;
        }
        let interval_ms = 1000.0 / self.target_fps;
        if now_ms - self.last_render_time_ms >= interval_ms {
            self.last_render_time_ms = now_ms;
            true
        } else {
            self.accumulated_skips += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_pair_count_matches_target() {
        let samples = vec![0.5_f32; 4096];
        let strip = waveform_triangle_strip(&samples, 200, 800.0, 100.0, 0.0);
        assert_eq!(strip.len(), 400);
    }

    #[test]
    fn waveform_strip_mirrors_around_baseline() {
        let samples = vec![0.5_f32; 1024];
        let strip = waveform_triangle_strip(&samples, 10, 100.0, 50.0, 25.0);
        for pair in strip.chunks(2) {
            let top = pair[0];
            let bot = pair[1];
            assert!((top.y + bot.y - 50.0).abs() < 1e-3, "mirrored pair");
            assert!((top.x - bot.x).abs() < 1e-3, "same x");
        }
    }

    #[test]
    fn spectrum_bars_produce_six_verts_per_bar() {
        let spec = vec![-30.0_f32; 512];
        let verts = spectrum_bars_vertices(
            &spec, 48_000.0, 20.0, 20_000.0, 64, 800.0, 400.0, -60.0, 0.0,
        );
        assert_eq!(verts.len(), 64 * 6);
    }

    #[test]
    fn spectrum_bar_height_scales_with_db() {
        let loud = vec![-6.0_f32; 512];
        let quiet = vec![-48.0_f32; 512];
        let v_loud =
            spectrum_bars_vertices(&loud, 48_000.0, 20.0, 20_000.0, 4, 100.0, 100.0, -60.0, 0.0);
        let v_quiet = spectrum_bars_vertices(
            &quiet, 48_000.0, 20.0, 20_000.0, 4, 100.0, 100.0, -60.0, 0.0,
        );
        // Loud bar's top (y1) should be lower (smaller y = higher on screen)
        // than the quiet bar's top.
        assert!(v_loud[0].y < v_quiet[0].y);
    }

    #[test]
    fn meter_quad_clamps_above_zero_db() {
        let verts = meter_quad(12.0, -60.0, 0.0, 10.0, 100.0);
        // Clamped to max_db = 0.0 → fully filled → y1 = 0.0.
        assert!((verts[0].y).abs() < 1e-3);
        // Six vertices (two triangles).
        assert_eq!(verts.len(), 6);
    }

    #[test]
    fn meter_quad_empty_below_min_db() {
        let verts = meter_quad(-120.0, -60.0, 0.0, 10.0, 100.0);
        // Clamped to min_db → empty → y1 = height.
        assert!((verts[0].y - 100.0).abs() < 1e-3);
    }

    #[test]
    fn frame_pacer_honors_target_fps() {
        let mut p = FramePacer::new(60.0);
        assert!(p.should_render(0.0));
        // Immediately request another render at t = 5 ms — should skip.
        assert!(!p.should_render(5.0));
        assert_eq!(p.skipped_frames(), 1);
        // At t = 20 ms (> 16.67 ms), should render again.
        assert!(p.should_render(20.0));
    }

    #[test]
    fn frame_pacer_clamps_fps() {
        let mut p = FramePacer::new(9999.0);
        assert!((p.target_fps() - 240.0).abs() < 1e-3);
        p.set_target_fps(0.5);
        assert!((p.target_fps() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn empty_inputs_produce_empty_outputs() {
        let empty: Vec<f32> = Vec::new();
        assert!(waveform_triangle_strip(&empty, 10, 100.0, 50.0, 0.0).is_empty());
        assert!(spectrum_bars_vertices(
            &empty, 48_000.0, 20.0, 20_000.0, 10, 100.0, 100.0, -60.0, 0.0
        )
        .is_empty());
    }
}
