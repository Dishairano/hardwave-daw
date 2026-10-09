//! What the .flp parser reads from a file: tempo, channels, patterns and
//! the playlist, in a few lines. For checking a real project against what
//! the importer makes of it.
//!
//!     cargo run -p hardwave-project --example flp_dump -- song.flp [more.flp]
use hardwave_project::fl_import::FlClipContent;

fn main() {
    for path in std::env::args().skip(1) {
        println!("== {path}");
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("   cannot read: {e}");
                continue;
            }
        };
        let fl = match hardwave_project::flp_parser::parse(&bytes) {
            Ok(fl) => fl,
            Err(e) => {
                println!("   parse error: {e}");
                continue;
            }
        };
        println!(
            "   bpm {}  signature {}/{}",
            fl.bpm, fl.time_sig_numerator, fl.time_sig_denominator
        );
        let with_sample = fl
            .channels
            .iter()
            .filter(|c| c.sample_path.is_some())
            .count();
        let with_plugin = fl
            .channels
            .iter()
            .filter(|c| c.plugin_name.is_some())
            .count();
        println!(
            "   channels {} (sample {}, plug-in {})",
            fl.channels.len(),
            with_sample,
            with_plugin
        );
        for (i, c) in fl.channels.iter().enumerate().take(8) {
            println!(
                "     #{i} {:?} {:?} plugin={:?} sample={}",
                c.name,
                c.kind,
                c.plugin_name,
                c.sample_path
                    .as_deref()
                    .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p))
                    .unwrap_or("-")
            );
        }
        let notes: usize = fl.pattern_notes.iter().map(|(_, _, n)| n.len()).sum();
        println!(
            "   patterns with notes {}  notes {}",
            fl.pattern_notes.len(),
            notes
        );
        let (mut pat, mut audio, mut auto) = (0, 0, 0);
        for c in &fl.playlist_clips {
            match c.content {
                FlClipContent::Pattern { .. } => pat += 1,
                FlClipContent::AudioSample { .. } => audio += 1,
                FlClipContent::Automation { .. } => auto += 1,
            }
        }
        println!(
            "   playlist clips {} (pattern {pat}, audio {audio}, automation {auto})",
            fl.playlist_clips.len()
        );
        println!("   mixer tracks {}", fl.mixer.len());
        println!(
            "   named playlist tracks {:?}",
            fl.playlist_track_names.iter().take(6).collect::<Vec<_>>()
        );
    }
}
