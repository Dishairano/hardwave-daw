//! An FL project's structure, as FL has it and as the importer makes it,
//! as JSON: FL's channel rack (with each channel's mixer insert), patterns
//! (the channels each plays), playlist rows per arrangement, and mixer
//! inserts (routes and effects); then the tracks the import makes, with
//! where each plays into. For comparing a real project side by side.
//!
//!     cargo run -p hardwave-project --example flp_structure -- song.flp > song.json
use hardwave_project::clip::ClipContent;
use hardwave_project::fl_import::FlClipContent;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).expect("an .flp path");
    let bytes = std::fs::read(&path).expect("readable");
    let fl = hardwave_project::flp_parser::parse(&bytes).expect("an FL project");

    let pattern_name = |p: u32| {
        fl.pattern_names
            .iter()
            .find(|(i, _)| *i == p)
            .map(|(_, n)| n.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Pattern {p}"))
    };
    let channels: Vec<Value> = fl
        .channels
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "index": i, "name": c.name, "kind": format!("{:?}", c.kind),
                "plugin": c.plugin_name, "insert": c.settings.mixer_insert,
                "color": c.color, "layerChildren": c.layer_children,
                "sample": c.sample_path.as_deref().map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p)),
            })
        })
        .collect();
    let mut by_pattern: BTreeMap<u32, Vec<(u32, usize)>> = BTreeMap::new();
    for (p, ch, notes) in &fl.pattern_notes {
        by_pattern.entry(*p).or_default().push((*ch, notes.len()));
    }
    let patterns: Vec<Value> = by_pattern
        .iter()
        .map(|(p, chs)| {
            json!({ "index": p, "name": pattern_name(*p),
                "channels": chs.iter().map(|(c, n)| json!({ "channel": c, "notes": n })).collect::<Vec<_>>() })
        })
        .collect();
    let arrangement_count = fl.arrangements.len().max(1);
    let arrangements: Vec<Value> = (0..arrangement_count)
        .map(|a| {
            let mut rows: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
            for c in fl.playlist_clips.iter().filter(|c| c.arrangement == a) {
                let bar = c.start_tick as f64 / (hardwave_midi::PPQ as f64 * 4.0) + 1.0;
                let item = match &c.content {
                    FlClipContent::Pattern { pattern_index, offset_ticks } => json!({
                        "bar": bar, "kind": "pattern", "name": pattern_name(*pattern_index), "offset": offset_ticks, "muted": c.muted }),
                    FlClipContent::AudioSample { channel_index, sample_path, .. } => json!({
                        "bar": bar, "kind": "audio", "channel": channel_index,
                        "name": sample_path.rsplit(['\\', '/']).next().unwrap_or(sample_path), "muted": c.muted }),
                    FlClipContent::Automation { channel_index, target, .. } => json!({
                        "bar": bar, "kind": "automation", "channel": channel_index, "name": target, "muted": c.muted }),
                };
                rows.entry(c.track_index).or_default().push(item);
            }
            let names: &[(u32, String)] = match fl.arrangements.get(a) {
                Some(arr) => &arr.track_names,
                None => &fl.playlist_track_names,
            };
            json!({
                "name": fl.arrangements.get(a).map(|x| x.name.clone()).unwrap_or_else(|| "Arrangement".into()),
                "open": a == fl.current_arrangement,
                "rows": rows.iter().map(|(r, items)| json!({
                    "row": r,
                    "name": names.iter().find(|(i, _)| i == r).map(|(_, n)| n.clone()),
                    "items": items,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mixer: Vec<Value> = fl
        .mixer
        .iter()
        .map(|m| {
            json!({ "index": m.index, "name": m.name, "enabled": m.enabled, "color": m.color,
                "routes": m.routes.iter().map(|r| json!({ "to": r.to, "level": r.level })).collect::<Vec<_>>(),
                "slots": m.slots.iter().map(|s| json!({ "slot": s.index, "plugin": s.plugin, "name": s.name })).collect::<Vec<_>>() })
        })
        .collect();

    let name = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let built =
        hardwave_project::fl_build::build_project(&fl, &name, &|p: &str| Some(p.to_string()));
    let p = &built.project;
    let name_of = |id: &str| {
        p.track(id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let tracks: Vec<Value> = p
        .tracks
        .iter()
        .filter(|t| {
            !(t.id.starts_with("insert-")
                && t.clips.is_empty()
                && t.inserts.is_empty()
                && t.name.starts_with("Insert "))
        })
        .map(|t| {
            let (midi, audio) = t.clips.iter().fold((0, 0), |(m, a), c| match c.content {
                ClipContent::Midi(_) => (m + 1, a),
                ClipContent::Audio(_) => (m, a + 1),
            });
            json!({ "id": t.id, "name": t.name, "kind": format!("{:?}", t.kind),
                "playsInto": t.output_bus.as_deref().map(name_of),
                "sends": t.sends.iter().map(|s| name_of(&s.target)).collect::<Vec<_>>(),
                "plugins": t.inserts.len(), "midiClips": midi, "audioClips": audio,
                "automationClips": t.automation_clips.len(), "muted": t.muted,
                "midiRouteTo": t.midi_route_to.iter().map(|i| name_of(i)).collect::<Vec<_>>() })
        })
        .collect();
    let out = json!({
        "file": path, "bpm": fl.bpm,
        "fl": { "channels": channels, "patterns": patterns, "arrangements": arrangements, "mixer": mixer },
        "ours": { "tracks": tracks,
            "arrangements": p.arrangements.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
            "report": format!("{:?}", built.report) },
    });
    println!("{}", serde_json::to_string_pretty(&out).expect("json"));
}
