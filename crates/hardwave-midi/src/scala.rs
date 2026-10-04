//! Scala tuning files, and the note-to-frequency table they describe.
//!
//! Equal temperament is a choice, not a law. Hardstyle screeches and a
//! lot of the older dance records are built on scales that are not it,
//! and every synth that can be retuned reads the same format: a Scala
//! `.scl` file, a list of degrees in cents or as ratios.
//!
//! A tuning is kept in the song as the degrees themselves rather than a
//! path to a file, so a project opened on another machine still sounds
//! the way it did.

use serde::{Deserialize, Serialize};

/// A scale: the degrees of one octave, and where it is anchored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    pub name: String,
    /// Degrees in cents above the root, rising, ending with the repeat
    /// interval (1200 for an octave).
    pub degrees_cents: Vec<f64>,
    /// MIDI note the scale starts on.
    pub root_note: u8,
    /// What that note sounds at, in hertz.
    pub root_hz: f64,
}

impl Tuning {
    /// Twelve equal, which is what every note played without a tuning
    /// already sounds like.
    pub fn twelve_equal() -> Self {
        Self {
            name: "12 equal".into(),
            degrees_cents: (1..=12).map(|i| i as f64 * 100.0).collect(),
            root_note: 69,
            root_hz: 440.0,
        }
    }

    /// Frequency of every MIDI note under this tuning.
    ///
    /// The scale repeats at its last degree, so a scale that ends at
    /// 1200 cents repeats every octave and one that ends elsewhere
    /// repeats there, which is what a non-octave scale means.
    pub fn table(&self) -> [f32; 128] {
        let mut table = [440.0f32; 128];
        let degrees: Vec<f64> = if self.degrees_cents.is_empty() {
            Tuning::twelve_equal().degrees_cents
        } else {
            self.degrees_cents.clone()
        };
        let size = degrees.len() as i64;
        let repeat = *degrees.last().unwrap_or(&1200.0);
        for (note, slot) in table.iter_mut().enumerate() {
            let steps = note as i64 - self.root_note as i64;
            // Which repeat of the scale this note lands in, and where
            // inside it. Euclidean so notes below the root keep going
            // down instead of folding back up.
            let cycle = steps.div_euclid(size);
            let index = steps.rem_euclid(size);
            // Degree zero of a cycle is the root itself: the list holds
            // the steps above it, with the last one being the repeat.
            let within = if index == 0 {
                0.0
            } else {
                degrees[(index - 1) as usize]
            };
            let cents = cycle as f64 * repeat + within;
            *slot = (self.root_hz * 2f64.powf(cents / 1200.0)) as f32;
        }
        table
    }
}

/// Read a Scala `.scl` file.
///
/// The format: comment lines start with `!`, the first line that is not
/// a comment is a description, the next is how many degrees follow, and
/// then one degree per line, either cents (a number with a dot) or a
/// ratio (`3/2`). Anything after the first token on a line is a comment
/// the format allows, so it is dropped.
pub fn parse_scl(source: &str, fallback_name: &str) -> Result<Tuning, String> {
    let mut description: Option<String> = None;
    let mut expected: Option<usize> = None;
    let mut degrees: Vec<f64> = Vec::new();

    for raw in source.lines() {
        let line = raw.trim();
        if line.starts_with('!') {
            continue;
        }
        if description.is_none() {
            description = Some(line.to_string());
            continue;
        }
        if expected.is_none() {
            expected = line
                .split_whitespace()
                .next()
                .and_then(|t| t.parse::<usize>().ok());
            if expected.is_none() {
                return Err("the line after the description should say how many degrees".into());
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or("");
        let cents = if let Some((num, den)) = token.split_once('/') {
            let num: f64 = num
                .trim()
                .parse()
                .map_err(|_| format!("{token} is not a ratio"))?;
            let den: f64 = den
                .trim()
                .parse()
                .map_err(|_| format!("{token} is not a ratio"))?;
            if num <= 0.0 || den <= 0.0 {
                return Err(format!("{token} is not a ratio"));
            }
            1200.0 * (num / den).log2()
        } else {
            token
                .parse::<f64>()
                .map_err(|_| format!("{token} is not a number of cents"))?
        };
        degrees.push(cents);
    }

    if degrees.is_empty() {
        return Err("that file lists no degrees".into());
    }
    if let Some(expected) = expected {
        if expected != degrees.len() {
            return Err(format!(
                "the file says {expected} degrees and lists {}",
                degrees.len()
            ));
        }
    }

    let name = description
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    Ok(Tuning {
        name,
        degrees_cents: degrees,
        root_note: 60,
        root_hz: 261.625_565_3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWELVE_EQUAL_SCL: &str = "! 12edo.scl\n!\n12 tone equal temperament\n 12\n!\n 100.0\n 200.0\n 300.0\n 400.0\n 500.0\n 600.0\n 700.0\n 800.0\n 900.0\n 1000.0\n 1100.0\n 2/1\n";

    #[test]
    fn a_twelve_equal_file_is_read_as_twelve_equal() {
        let tuning = parse_scl(TWELVE_EQUAL_SCL, "file").expect("parse");
        assert_eq!(tuning.name, "12 tone equal temperament");
        assert_eq!(tuning.degrees_cents.len(), 12);
        assert!(
            (tuning.degrees_cents[11] - 1200.0).abs() < 1e-9,
            "2/1 is an octave"
        );
    }

    #[test]
    fn twelve_equal_gives_the_pitches_everyone_already_hears() {
        let mut tuning = parse_scl(TWELVE_EQUAL_SCL, "file").expect("parse");
        tuning.root_note = 69;
        tuning.root_hz = 440.0;
        let table = tuning.table();
        assert!((table[69] - 440.0).abs() < 0.01, "A4");
        assert!((table[81] - 880.0).abs() < 0.02, "an octave up");
        assert!((table[57] - 220.0).abs() < 0.01, "an octave down");
        assert!((table[60] - 261.6256).abs() < 0.05, "middle C");
    }

    #[test]
    fn a_ratio_is_read_as_the_interval_it_is() {
        let scl = "!\njust fifth\n 1\n 3/2\n";
        let tuning = parse_scl(scl, "file").expect("parse");
        assert!((tuning.degrees_cents[0] - 701.955).abs() < 0.01);
    }

    #[test]
    fn a_scale_with_fewer_degrees_repeats_at_its_own_interval() {
        // Five equal steps of an octave.
        let scl = "!\nfive equal\n 5\n 240.0\n 480.0\n 720.0\n 960.0\n 1200.0\n";
        let mut tuning = parse_scl(scl, "file").expect("parse");
        tuning.root_note = 60;
        tuning.root_hz = 100.0;
        let table = tuning.table();
        assert!((table[60] - 100.0).abs() < 0.01, "the root");
        assert!((table[65] - 200.0).abs() < 0.05, "five steps is the repeat");
        assert!((table[55] - 50.0).abs() < 0.05, "five steps down halves it");
    }

    #[test]
    fn a_file_that_does_not_add_up_is_refused() {
        let scl = "!\nwrong count\n 3\n 100.0\n 200.0\n";
        assert!(parse_scl(scl, "file").is_err());
    }

    #[test]
    fn a_file_with_no_degrees_is_refused() {
        assert!(parse_scl("!\nnothing\n 0\n", "file").is_err());
    }
}
