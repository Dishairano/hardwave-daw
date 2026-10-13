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
        println!("   mixer inserts {}", fl.mixer.len());
        for m in &fl.mixer {
            let default_routes =
                m.routes.len() == 1 && m.routes[0].to == 0 && m.routes[0].level == 12_800;
            if m.name.is_empty() && m.slots.is_empty() && default_routes && m.volume == 12_800 {
                continue;
            }
            let routes: Vec<String> = m
                .routes
                .iter()
                .map(|r| format!("{}@{}", r.to, r.level))
                .collect();
            let slots: Vec<String> = m
                .slots
                .iter()
                .map(|s| {
                    format!(
                        "{}:{}{}{}",
                        s.index,
                        s.label(),
                        if s.enabled { "" } else { " (off)" },
                        if s.mix == 12_800 {
                            String::new()
                        } else {
                            format!(" mix {}", s.mix)
                        }
                    )
                })
                .collect();
            println!(
                "     insert {} {:?} vol {} pan {} sep {}{}{} -> [{}] | {}",
                m.index,
                m.name,
                m.volume,
                m.pan,
                m.stereo_separation,
                if m.enabled { "" } else { " OFF" },
                if m.effects_enabled { "" } else { " FX-OFF" },
                routes.join(", "),
                slots.join(", ")
            );
        }
        let routed: std::collections::BTreeMap<i32, usize> =
            fl.channels.iter().fold(Default::default(), |mut acc, c| {
                *acc.entry(c.settings.mixer_insert).or_default() += 1;
                acc
            });
        println!("   channels per insert {routed:?}");
        println!(
            "   named playlist tracks {:?}",
            fl.playlist_track_names.iter().take(6).collect::<Vec<_>>()
        );
        // What the import makes of it, with no samples found here.
        let built = hardwave_project::fl_build::build_project(&fl, "dump", &|_| None);
        let r = &built.report;
        println!(
            "   import: {} instrument + {} audio tracks, {} audio + {} pattern clips, {} notes, {} samples to find, {} automation clips",
            r.instrument_tracks,
            r.audio_tracks,
            r.audio_clips,
            r.pattern_clips,
            r.notes,
            r.missing_samples.len(),
            r.automation_clips
        );
        let rows: Vec<String> = built
            .project
            .tracks
            .iter()
            .skip(1)
            .take(14)
            .map(|t| format!("{} ({})", t.name, t.clips.len()))
            .collect();
        println!("   rows: {}", rows.join(", "));
        println!("   plug-ins left: {}", r.plugins_left_behind.join(", "));
        println!("   playlist tracks off: {:?}", fl.playlist_tracks_off);
        println!(
            "   quick fades left: {}",
            r.sample_fades_left_behind.join(", ")
        );
        println!("   mixed lanes: {}", r.mixed_lanes.join(", "));
        println!("   mixer inserts as tracks: {}", r.mixer_inserts);
        println!("   effects left: {}", r.effects_left_behind.len());
        println!("   silent inserts: {:?}", r.silent_inserts);
        println!("   route levels left: {:?}", r.route_levels_left_behind);
        println!("   without settings: {:?}", r.plugins_without_settings);
        let carried: Vec<String> = built
            .external_plugins
            .iter()
            .map(|e| {
                format!(
                    "{} [{:?}{}, {} bytes] -> {}",
                    e.place,
                    e.plugin.format,
                    if e.plays_track { " instrument" } else { "" },
                    e.plugin.state.len(),
                    e.candidates.join(" | ")
                )
            })
            .collect();
        println!("   other people's plug-ins ({}):", carried.len());
        for c in carried {
            println!("     {c}");
        }
        for track in &built.project.tracks {
            let audio: Vec<_> = track
                .clips
                .iter()
                .filter_map(|c| match &c.content {
                    hardwave_project::clip::ClipContent::Audio(a) => Some(a),
                    _ => None,
                })
                .collect();
            if audio.is_empty()
                && track.volume_db == 0.0
                && !track.muted
                && track.output_bus.is_none()
                && track.sends.is_empty()
                && track.clips.is_empty()
            {
                continue;
            }
            let name_of = |id: &str| {
                built
                    .project
                    .tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| id.to_string())
            };
            let sends: Vec<String> = track
                .sends
                .iter()
                .map(|s| format!("{} {:.1} dB", name_of(&s.target), s.gain_db))
                .collect();
            println!(
                "   {:?} {:<30} -> {}{}{}",
                track.kind,
                track.name,
                track
                    .output_bus
                    .as_deref()
                    .map(name_of)
                    .unwrap_or_else(|| "master".into()),
                if sends.is_empty() {
                    String::new()
                } else {
                    format!(" + sends [{}]", sends.join(", "))
                },
                if track.stereo_separation != 1.0 {
                    format!(" sep {:.2}", track.stereo_separation)
                } else {
                    String::new()
                }
            );
            let gains: std::collections::BTreeSet<String> =
                audio.iter().map(|a| format!("{:.1}", a.gain_db)).collect();
            println!(
                "   {:<34} vol {:>5.1} pan {:>4.2}{} | {} clips, {} muted, {} reversed, gain dB {:?}",
                track.name,
                track.volume_db,
                track.pan,
                if track.muted { " MUTED" } else { "" },
                audio.len(),
                audio.iter().filter(|a| a.muted).count(),
                audio.iter().filter(|a| a.reversed).count(),
                gains
            );
        }
        let stretched: Vec<String> = built
            .audio_fixups
            .iter()
            .filter(|f| f.fit_seconds.is_some() || f.pitch_semitones != 0.0 || f.multiplier != 1.0)
            .map(|f| {
                format!(
                    "fit {:?} s, pitch {}, mul {}, resample {}",
                    f.fit_seconds.map(|s| (s * 100.0).round() / 100.0),
                    f.pitch_semitones,
                    f.multiplier,
                    f.resample
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        println!("   stretch/pitch settings in use: {stretched:#?}");
    }
}
