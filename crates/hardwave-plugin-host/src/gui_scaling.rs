//! Plugin GUI DPI scaling — computes a scale factor and a sized
//! frame for the plugin editor window based on the host monitor's
//! DPI. Embedded plugin GUIs don't participate in the host's own
//! scaling stack, so the host has to propagate DPI explicitly.

/// Per-monitor DPI context — what the caller passes in at
/// `open_editor` time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DpiContext {
    /// Logical DPI of the monitor the editor will open on. 96 is
    /// the Windows-standard baseline; macOS reports 144 at 2x
    /// scale; Linux varies.
    pub monitor_dpi: f32,
    /// User override (UI setting) in range `[0.5, 3.0]`. 1.0 means
    /// use the monitor's native DPI; anything else multiplies on
    /// top of the monitor detection.
    pub user_scale: f32,
}

impl DpiContext {
    pub const BASELINE_DPI: f32 = 96.0;

    pub fn standard() -> Self {
        Self {
            monitor_dpi: Self::BASELINE_DPI,
            user_scale: 1.0,
        }
    }

    /// Final scale factor the plugin GUI should apply. Clamped to a
    /// sensible range so a mis-detected monitor doesn't blow up the
    /// plugin window.
    pub fn effective_scale(&self) -> f32 {
        let monitor = (self.monitor_dpi / Self::BASELINE_DPI).clamp(0.5, 4.0);
        let user = self.user_scale.clamp(0.5, 3.0);
        (monitor * user).clamp(0.5, 4.0)
    }
}

/// Rect in logical pixels. Plugins report their native (1×) size;
/// the host scales via `scale_rect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PluginRect {
    pub width: u32,
    pub height: u32,
}

impl PluginRect {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn scaled_by(&self, scale: f32) -> Self {
        let scale = scale.clamp(0.1, 8.0);
        Self {
            width: ((self.width as f32) * scale).round() as u32,
            height: ((self.height as f32) * scale).round() as u32,
        }
    }
}

/// Size the plugin editor window should open at for the given DPI
/// context and the plugin's native 1× rect.
pub fn plugin_window_size(native: PluginRect, dpi: DpiContext) -> PluginRect {
    native.scaled_by(dpi.effective_scale())
}

/// Convert a child-window mouse coordinate (already-scaled by the
/// host) into the plugin's native coordinate space so the plugin
/// can do its own hit testing unaware of DPI.
pub fn mouse_logical_to_native(x: f32, y: f32, dpi: DpiContext) -> (f32, f32) {
    let scale = dpi.effective_scale().max(0.1);
    (x / scale, y / scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_dpi_is_unit_scale() {
        let ctx = DpiContext::standard();
        assert!((ctx.effective_scale() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn retina_macos_double_scale_resolves_to_1_5_times() {
        let ctx = DpiContext {
            monitor_dpi: 144.0,
            user_scale: 1.0,
        };
        assert!((ctx.effective_scale() - 1.5).abs() < 1e-3);
    }

    #[test]
    fn user_scale_multiplies_monitor_scale() {
        let ctx = DpiContext {
            monitor_dpi: 192.0,
            user_scale: 0.75,
        };
        // 192/96 = 2.0; 2.0 * 0.75 = 1.5.
        assert!((ctx.effective_scale() - 1.5).abs() < 1e-3);
    }

    #[test]
    fn scale_is_clamped_for_garbage_input() {
        let ctx = DpiContext {
            monitor_dpi: 10_000.0,
            user_scale: 100.0,
        };
        let s = ctx.effective_scale();
        assert!(s <= 4.0);
        assert!(s > 0.0);
    }

    #[test]
    fn plugin_rect_scales_dimensions() {
        let rect = PluginRect::new(400, 300);
        let big = rect.scaled_by(2.0);
        assert_eq!(big.width, 800);
        assert_eq!(big.height, 600);
    }

    #[test]
    fn plugin_window_size_uses_effective_scale() {
        let native = PluginRect::new(400, 300);
        let ctx = DpiContext {
            monitor_dpi: 144.0,
            user_scale: 1.0,
        };
        let sized = plugin_window_size(native, ctx);
        assert_eq!(sized.width, 600);
        assert_eq!(sized.height, 450);
    }

    #[test]
    fn mouse_scaling_roundtrips_at_1x() {
        let (x, y) = mouse_logical_to_native(123.0, 456.0, DpiContext::standard());
        assert!((x - 123.0).abs() < 1e-3);
        assert!((y - 456.0).abs() < 1e-3);
    }

    #[test]
    fn mouse_scaling_divides_by_scale() {
        let ctx = DpiContext {
            monitor_dpi: 192.0,
            user_scale: 1.0,
        };
        let (x, y) = mouse_logical_to_native(400.0, 200.0, ctx);
        // 192 / 96 = 2.0 scale → coordinates halved.
        assert!((x - 200.0).abs() < 1e-3);
        assert!((y - 100.0).abs() < 1e-3);
    }
}
