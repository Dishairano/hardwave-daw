//! How built-in plug-ins write their values for people: one style for all,
//! so "1.20 kHz" in the EQ reads the same as in the filter.

/// 20 Hz .. 999 Hz as whole hertz, then kilohertz with two decimals.
pub fn hz(v: f64) -> String {
    if v.abs() >= 1000.0 {
        format!("{:.2} kHz", v / 1000.0)
    } else if v.abs() >= 100.0 {
        format!("{v:.0} Hz")
    } else {
        format!("{v:.1} Hz")
    }
}

/// Decibels with one decimal and a sign when positive; "-inf dB" at and
/// below -96.
pub fn db(v: f64) -> String {
    if v <= -96.0 {
        "-inf dB".to_string()
    } else if v > 0.0 {
        format!("+{v:.1} dB")
    } else {
        format!("{v:.1} dB")
    }
}

/// Milliseconds below a second, seconds from there.
pub fn ms(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2} s", v / 1000.0)
    } else if v >= 100.0 {
        format!("{v:.0} ms")
    } else if v >= 10.0 {
        format!("{v:.1} ms")
    } else {
        format!("{v:.2} ms")
    }
}

/// Seconds, as ms() for values under a second.
pub fn secs(v: f64) -> String {
    ms(v * 1000.0)
}

/// A 0..1 amount as a whole percentage.
pub fn pct(v01: f64) -> String {
    format!("{:.0} %", v01 * 100.0)
}

/// A compression ratio: "4.0:1", "inf:1" from 100:1 up.
pub fn ratio(v: f64) -> String {
    if v >= 100.0 {
        "inf:1".to_string()
    } else {
        format!("{v:.1}:1")
    }
}

/// On / Off for a switch stored 0 or 1.
pub fn on_off(v: f64) -> String {
    if v >= 0.5 { "On" } else { "Off" }.to_string()
}

/// A plain number with `decimals` places and an optional unit.
pub fn num(v: f64, decimals: usize, unit: &str) -> String {
    if unit.is_empty() {
        format!("{v:.decimals$}")
    } else {
        format!("{v:.decimals$} {unit}")
    }
}

/// Which of `options` a 0..1 value picks: choice i of n sits at i/(n-1).
pub fn option_index(v01: f64, count: usize) -> usize {
    if count <= 1 {
        return 0;
    }
    ((v01.clamp(0.0, 1.0) * (count - 1) as f64).round() as usize).min(count - 1)
}

/// The label a 0..1 value picks from `options`.
pub fn option_text(v01: f64, options: &[&str]) -> Option<String> {
    options
        .get(option_index(v01, options.len()))
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_the_same_everywhere() {
        assert_eq!(hz(1200.0), "1.20 kHz");
        assert_eq!(hz(250.0), "250 Hz");
        assert_eq!(db(-6.0), "-6.0 dB");
        assert_eq!(db(3.0), "+3.0 dB");
        assert_eq!(db(-120.0), "-inf dB");
        assert_eq!(ms(12.5), "12.5 ms");
        assert_eq!(ms(1500.0), "1.50 s");
        assert_eq!(pct(0.35), "35 %");
        assert_eq!(ratio(4.0), "4.0:1");
        assert_eq!(option_text(0.5, &["a", "b", "c"]).as_deref(), Some("b"));
        assert_eq!(option_index(1.0, 4), 3);
    }
}
