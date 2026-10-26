//! Export to DAWproject, the open format Bitwig Studio, Studio One and
//! Cubase read (see `hardwave_project::dawproject`).
//!
//! This is the part that knows the computer: where each sample is, what
//! each plug-in is called by other hosts, and what it holds now. The
//! documents are built in the project crate; the zip is written here.

use crate::AppState;
use hardwave_project::clip::{AudioClip, ClipContent};
use hardwave_project::dawproject::{self, FileSource, MediaInfo, PluginFormat, PluginInfo};
use hardwave_project::track::PluginSlot;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{Manager, State};

/// What the export did, for the app to say.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DawProjectReport {
    pub path: String,
    pub tracks: usize,
    pub files: usize,
    /// What the format could not carry, one line each.
    pub left_out: Vec<String>,
}

/// Write the open song as a .dawproject file at `path`.
#[tauri::command]
pub async fn export_dawproject(
    app: tauri::AppHandle,
    path: String,
) -> Result<DawProjectReport, String> {
    // Off the main thread: reading every sample into the zip takes a while.
    tauri::async_runtime::spawn_blocking(move || export_blocking(app.state::<AppState>(), &path))
        .await
        .map_err(|e| format!("the export stopped: {e}"))?
}

fn export_blocking(state: State<AppState>, path: &str) -> Result<DawProjectReport, String> {
    let mut path = PathBuf::from(path);
    if path.extension().and_then(|e| e.to_str()) != Some("dawproject") {
        path.set_extension("dawproject");
    }
    let (mut project, pool_rates, live) = {
        let engine = state.engine.lock();
        // What the plug-ins hold now, not at the last save.
        let live = engine
            .snapshot_plugin_states(std::time::Duration::from_millis(800))
            .unwrap_or_default();
        let project = engine.project.lock().clone();
        let mut rates: HashMap<String, f64> = HashMap::new();
        for track in &project.tracks {
            for clip in &track.clips {
                if let ClipContent::Audio(a) = &clip.content {
                    if let Some(buf) = engine.audio_pool.get(&a.source_path) {
                        rates.insert(a.source_path.clone(), buf.sample_rate as f64);
                    }
                }
            }
        }
        (project, rates, live)
    };
    for track in &mut project.tracks {
        let track_id = track.id.clone();
        for slot in &mut track.inserts {
            if let Some(bytes) = live.get(&(track_id.clone(), slot.id.clone())) {
                slot.state = Some(bytes.clone());
            }
        }
    }
    for track in &mut project.tracks {
        for slot in &mut track.inserts {
            if slot.state.is_none() {
                slot.state = project_plugin_state(&project.plugin_states, &slot.id);
            }
        }
    }

    // Each plug-in kind once: its name, maker, role and id elsewhere.
    let descriptors: HashMap<String, hardwave_plugin_host::types::PluginDescriptor> = {
        let engine = state.engine.lock();
        let scanner = engine.plugin_scanner.lock();
        project
            .tracks
            .iter()
            .flat_map(|t| t.inserts.iter())
            .filter_map(|s| {
                scanner
                    .find(&s.plugin_id)
                    .map(|d| (s.plugin_id.clone(), d.clone()))
            })
            .collect()
    };
    let mut vst3_ids: HashMap<String, Option<String>> = HashMap::new();
    for (plugin_id, d) in &descriptors {
        if d.format == hardwave_plugin_host::types::PluginFormat::Vst3 {
            vst3_ids.insert(plugin_id.clone(), vst3_class_id(d));
        }
    }

    let media = |a: &AudioClip| -> Option<MediaInfo> {
        let file = if a.source_file.is_empty() {
            PathBuf::from(&a.source_path)
        } else {
            PathBuf::from(&a.source_file)
        };
        if !file.is_file() {
            return None;
        }
        let info = hardwave_dsp::audio_file::AudioFileReader::probe(&file)?;
        Some(MediaInfo {
            frames_per_second: pool_rates
                .get(&a.source_path)
                .copied()
                .unwrap_or(info.sample_rate as f64),
            file,
            sample_rate: info.sample_rate,
            channels: info.channels,
            seconds: info.duration_secs,
        })
    };
    let plugin = |slot: &PluginSlot| -> Option<PluginInfo> {
        use hardwave_plugin_host::types::{PluginCategory, PluginFormat as Host};
        let d = descriptors.get(&slot.plugin_id)?;
        // A built-in is listed as a CLAP at "<native>": no other DAW has it.
        if d.path == Path::new("<native>") || slot.plugin_id.starts_with("hardwave.native.") {
            return None;
        }
        let instrument = d.category == PluginCategory::Instrument;
        match d.format {
            Host::Vst3 => {
                let cid = vst3_ids.get(&slot.plugin_id).cloned().flatten()?;
                Some(PluginInfo {
                    format: PluginFormat::Vst3,
                    state: slot
                        .state
                        .as_ref()
                        .map(|s| hardwave_plugin_host::vstpreset::write(&cid, s, None)),
                    device_id: cid,
                    name: d.name.clone(),
                    vendor: d.vendor.clone(),
                    instrument,
                })
            }
            Host::Clap => Some(PluginInfo {
                format: PluginFormat::Clap,
                device_id: slot.plugin_id.trim_start_matches("clap:").to_string(),
                name: d.name.clone(),
                vendor: d.vendor.clone(),
                instrument,
                state: slot.state.clone(),
            }),
        }
    };

    let version = env!("CARGO_PKG_VERSION");
    let out = dawproject::export(&project, version, &media, &plugin);
    let tracks = out.project_xml.matches("<Track ").count();
    write_zip(&path, &out)?;
    Ok(DawProjectReport {
        path: path.display().to_string(),
        tracks,
        files: out.files.len(),
        left_out: out.left_out,
    })
}

fn project_plugin_state(
    states: &[hardwave_project::project::PluginStateEntry],
    slot_id: &str,
) -> Option<Vec<u8>> {
    states
        .iter()
        .find(|e| e.plugin_instance_id == slot_id)
        .map(|e| e.chunk.clone())
}

/// A VST3's class id as other hosts know it: from its bundle's
/// moduleinfo.json, or else asked of the bundle in a process of its own.
fn vst3_class_id(d: &hardwave_plugin_host::types::PluginDescriptor) -> Option<String> {
    hardwave_plugin_host::scanner::moduleinfo_class_id(&d.path, &d.name).or_else(|| {
        let classes = crate::plugin_describe::describe_vst3_out_of_process(&d.path)?;
        let named = classes.iter().find(|c| c.name == d.name);
        named
            .or_else(|| (classes.len() == 1).then(|| &classes[0]))
            .map(|c| c.cid.clone())
            .filter(|cid| cid.len() == 32)
    })
}

/// The zip: the two documents compressed, the samples stored as they are
/// (they do not shrink), the plug-in states compressed.
fn write_zip(path: &Path, out: &dawproject::Export) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let tmp = path.with_extension("dawproject.part");
    let file = std::fs::File::create(&tmp)
        .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let packed = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true);
    let fail = |e: &dyn std::fmt::Display| format!("could not write the project: {e}");
    zip.start_file("project.xml", packed)
        .map_err(|e| fail(&e))?;
    zip.write_all(out.project_xml.as_bytes())
        .map_err(|e| fail(&e))?;
    zip.start_file("metadata.xml", packed)
        .map_err(|e| fail(&e))?;
    zip.write_all(out.metadata_xml.as_bytes())
        .map_err(|e| fail(&e))?;
    for f in &out.files {
        match &f.source {
            FileSource::Bytes(bytes) => {
                zip.start_file(f.path.as_str(), packed)
                    .map_err(|e| fail(&e))?;
                zip.write_all(bytes).map_err(|e| fail(&e))?;
            }
            FileSource::Path(source) => {
                let mut input = std::fs::File::open(source)
                    .map_err(|e| format!("could not read {}: {e}", source.display()))?;
                zip.start_file(f.path.as_str(), stored)
                    .map_err(|e| fail(&e))?;
                std::io::copy(&mut input, &mut zip).map_err(|e| fail(&e))?;
            }
        }
    }
    zip.finish().map_err(|e| fail(&e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(())
}

/// The largest project.xml read: a song's document is a few megabytes.
const MAX_DOCUMENT: u64 = 64 * 1024 * 1024;
/// The largest sample and plug-in state unpacked, and all of them together.
const MAX_ENTRY: u64 = 4 * 1024 * 1024 * 1024;
const MAX_UNPACKED: u64 = 32 * 1024 * 1024 * 1024;
/// The largest plug-in state read: a sampler's can carry its samples.
const MAX_STATE: u64 = 256 * 1024 * 1024;

/// What opening a DAWproject did, for the app to say.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DawProjectImportReport {
    pub application: String,
    pub tracks: usize,
    pub clips: usize,
    pub notes: usize,
    pub bpm: f64,
    /// Where its samples were unpacked.
    pub samples_folder: Option<String>,
    pub left_out: Vec<String>,
}

/// Open a .dawproject file from Bitwig Studio, Studio One or Cubase as a
/// song of its own.
#[tauri::command]
pub async fn import_dawproject(
    app: tauri::AppHandle,
    path: String,
) -> Result<DawProjectImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_blocking(&app, &path))
        .await
        .map_err(|e| format!("opening stopped: {e}"))?
}

fn import_blocking(app: &tauri::AppHandle, path: &str) -> Result<DawProjectImportReport, String> {
    use hardwave_project::dawproject_import::{self as dpi, DeviceFormat};
    use std::io::Read;
    let file = Path::new(path);
    let mut zip = zip::ZipArchive::new(
        std::fs::File::open(file).map_err(|e| format!("could not read that file: {e}"))?,
    )
    .map_err(|_| "that is not a DAWproject file (not a zip)".to_string())?;
    let read_entry =
        |zip: &mut zip::ZipArchive<std::fs::File>, name: &str, max: u64| -> Option<Vec<u8>> {
            let entry = zip.by_name(name).ok()?;
            if entry.size() > max {
                return None;
            }
            let claimed = entry.size();
            let mut bytes = Vec::with_capacity(claimed as usize);
            entry.take(claimed + 1).read_to_end(&mut bytes).ok()?;
            (bytes.len() as u64 <= claimed).then_some(bytes)
        };
    let xml = read_entry(&mut zip, "project.xml", MAX_DOCUMENT)
        .ok_or("that DAWproject has no project.xml, or one too large to be a song")?;
    let xml = String::from_utf8(xml).map_err(|_| "the project.xml is not text".to_string())?;
    let name = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "DAWproject".into());
    let mut im = dpi::import(&xml, &name)?;

    // Past what any song holds (a looped clip can ask for a lot), nothing
    // is opened.
    let (mut clips, mut notes) = (0usize, 0usize);
    for t in &im.project.tracks {
        clips += t.clips.len();
        for c in &t.clips {
            if let ClipContent::Midi(m) = &c.content {
                notes += m.clip.notes.len();
            }
        }
    }
    if notes > hardwave_project::sanitize::MAX_NOTES
        || clips > hardwave_project::sanitize::MAX_CLIPS
        || im.project.tracks.len() > hardwave_project::sanitize::MAX_TRACKS
    {
        return Err("that project is larger than any song holds; it was not opened".into());
    }

    // The samples it carries, beside the file: "<name> Samples".
    let folder = file
        .parent()
        .map(|p| p.join(format!("{name} Samples")))
        .unwrap_or_else(|| PathBuf::from(format!("{name} Samples")));
    let mut unpacked: HashMap<String, PathBuf> = HashMap::new();
    let mut total = 0u64;
    let mut missing: Vec<String> = Vec::new();
    for audio in &im.audio {
        if audio.external || unpacked.contains_key(&audio.file) {
            continue;
        }
        let local = match unpack(&mut zip, &audio.file, &folder, &mut total) {
            Ok(local) => local,
            Err(_) => {
                missing.push(audio.file.clone());
                continue;
            }
        };
        unpacked.insert(audio.file.clone(), local);
    }
    let mut finish: Vec<dpi::ImportedAudio> = Vec::new();
    for audio in &im.audio {
        let local = if audio.external {
            Some(PathBuf::from(&audio.file)).filter(|p| p.is_file())
        } else {
            unpacked.get(&audio.file).cloned()
        };
        let Some(local) = local else {
            if audio.external {
                missing.push(audio.file.clone());
            }
            continue;
        };
        let local = local.to_string_lossy().into_owned();
        if let Some(t) = im.project.track_mut(&audio.track_id) {
            for placement in &mut t.clips {
                if let ClipContent::Audio(a) = &mut placement.content {
                    if a.id == audio.clip_id {
                        a.source_path = local.clone();
                        a.source_file = local.clone();
                    }
                }
            }
        }
        finish.push(audio.clone());
    }
    // Clips whose sample is not there are taken out; the report says so.
    let found: std::collections::HashSet<&str> =
        finish.iter().map(|a| a.clip_id.as_str()).collect();
    for t in &mut im.project.tracks {
        t.clips.retain(|c| match &c.content {
            ClipContent::Audio(a) => found.contains(a.id.as_str()),
            ClipContent::Midi(_) => true,
        });
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        im.left_out.push(format!(
            "{} sample{} that could not be found or unpacked",
            missing.len(),
            if missing.len() == 1 { "" } else { "s" }
        ));
    }

    // Plug-ins: each device found among this computer's, with its settings.
    let installed: Vec<hardwave_plugin_host::types::PluginDescriptor> = {
        let state = app.state::<AppState>();
        let engine = state.engine.lock();
        let scanner = engine.plugin_scanner.lock();
        scanner.plugins().to_vec()
    };
    let mut not_here: Vec<String> = Vec::new();
    let mut no_settings: Vec<String> = Vec::new();
    for device in &im.plugins {
        let found = find_plugin(device, &installed);
        let Some((plugin_id, same_format)) = found else {
            not_here.push(device.name.clone());
            if let Some(t) = im.project.track_mut(&device.track_id) {
                t.inserts.retain(|s| s.id != device.slot_id);
            }
            continue;
        };
        let state = if !same_format || device.external_state {
            None
        } else {
            device
                .state_file
                .as_deref()
                .and_then(|f| read_entry(&mut zip, f, MAX_STATE))
                .and_then(|bytes| match device.format {
                    DeviceFormat::Vst3 => hardwave_plugin_host::vstpreset::parse(&bytes)
                        .ok()
                        .map(|p| p.component),
                    _ => Some(bytes),
                })
        };
        if state.is_none() && device.state_file.is_some() {
            no_settings.push(device.name.clone());
        }
        if let Some(t) = im.project.track_mut(&device.track_id) {
            if let Some(slot) = t.inserts.iter_mut().find(|s| s.id == device.slot_id) {
                slot.plugin_id = plugin_id;
                slot.state = state;
            }
        }
    }
    if !not_here.is_empty() {
        not_here.sort();
        not_here.dedup();
        im.left_out.push(format!(
            "plug-ins that are not on this computer: {}",
            not_here.join(", ")
        ));
    }
    if !no_settings.is_empty() {
        no_settings.sort();
        no_settings.dedup();
        im.left_out.push(format!(
            "the settings of {} (they open at their defaults)",
            no_settings.join(", ")
        ));
    }

    let made = im
        .project
        .tracks
        .iter()
        .filter(|t| !t.clips.is_empty() || !t.inserts.is_empty() || !t.sends.is_empty())
        .count();
    let bpm = im
        .project
        .tempo_map
        .entries
        .first()
        .map(|e| e.bpm)
        .unwrap_or(120.0);
    let tempo_map = im.project.tempo_map.clone();
    crate::commands::project::open_project_blocking(app, im.project, None, move |song, pool| {
        finish_audio(song, pool, &finish, &tempo_map)
    })?;
    Ok(DawProjectImportReport {
        application: im.application,
        tracks: made,
        clips,
        notes,
        bpm,
        samples_folder: (!unpacked.is_empty()).then(|| folder.to_string_lossy().into_owned()),
        left_out: im.left_out,
    })
}

/// Unpack one entry of the zip into `folder` under its own file name. Only
/// what the project names is taken, its name is cleaned (no folders, no
/// climbing out), and it may not unpack to more than it claims.
fn unpack(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    folder: &Path,
    total: &mut u64,
) -> Result<PathBuf, String> {
    use std::io::Read;
    let mut entry = zip.by_name(name).map_err(|e| e.to_string())?;
    let claimed = entry.size();
    if claimed > MAX_ENTRY || total.saturating_add(claimed) > MAX_UNPACKED {
        return Err("too large".into());
    }
    let base = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            n.chars()
                .map(|c| {
                    if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                        '_'
                    } else {
                        c
                    }
                })
                .collect::<String>()
        })
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .ok_or("no name")?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut target = folder.join(&base);
    let mut n = 2;
    while target.exists() {
        let (stem, ext) = match base.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (base.clone(), String::new()),
        };
        target = folder.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
    let written =
        std::io::copy(&mut (&mut entry).take(claimed + 1), &mut out).map_err(|e| e.to_string())?;
    if written > claimed {
        drop(out);
        let _ = std::fs::remove_file(&target);
        return Err("unpacks to more than it says".into());
    }
    *total += written;
    Ok(target)
}

/// One of this computer's plug-ins for a device from the other DAW, and
/// whether it is the same format (so its settings fit).
fn find_plugin(
    device: &hardwave_project::dawproject_import::ImportedPlugin,
    installed: &[hardwave_plugin_host::types::PluginDescriptor],
) -> Option<(String, bool)> {
    use hardwave_plugin_host::types::PluginFormat as Host;
    use hardwave_project::dawproject_import::DeviceFormat;
    let same_name = |d: &&hardwave_plugin_host::types::PluginDescriptor| {
        d.name.eq_ignore_ascii_case(&device.name)
    };
    match device.format {
        DeviceFormat::Clap => {
            let id = format!("clap:{}", device.device_id);
            installed
                .iter()
                .find(|d| d.id == id)
                .map(|d| (d.id.clone(), true))
                .or_else(|| {
                    installed
                        .iter()
                        .filter(|d| d.format == Host::Clap)
                        .find(same_name)
                        .map(|d| (d.id.clone(), false))
                })
        }
        DeviceFormat::Vst3 => {
            let cid = device.device_id.to_ascii_uppercase();
            let by_id = |d: &&hardwave_plugin_host::types::PluginDescriptor| {
                d.format == Host::Vst3
                    && hardwave_plugin_host::scanner::moduleinfo_class_id(&d.path, &d.name)
                        .is_some_and(|c| c == cid)
            };
            // The same name first (cheap), checked by its id when its
            // bundle says; then any bundle with that id.
            let named: Vec<_> = installed
                .iter()
                .filter(|d| d.format == Host::Vst3)
                .filter(same_name)
                .collect();
            named
                .iter()
                .find(|d| by_id(d))
                .or_else(|| (named.len() == 1).then(|| &named[0]))
                .map(|d| (d.id.clone(), true))
                .or_else(|| installed.iter().find(by_id).map(|d| (d.id.clone(), true)))
        }
        // A VST2 or an Audio Unit: the VST3 or CLAP of the same name, whose
        // settings are kept differently.
        DeviceFormat::Vst2 | DeviceFormat::Au => installed
            .iter()
            .filter(|d| d.path != Path::new("<native>"))
            .find(same_name)
            .map(|d| (d.id.clone(), false)),
        DeviceFormat::Builtin => None,
    }
}

/// Finish the audio clips once their files are loaded: where each starts
/// in its file, and its warp markers when it does not play at the file's
/// own speed through the song's tempo.
fn finish_audio(
    project: &mut hardwave_project::Project,
    pool: &hardwave_engine::audio_pool::AudioPool,
    audio: &[hardwave_project::dawproject_import::ImportedAudio],
    tempo: &hardwave_project::tempo::TempoMap,
) {
    for a in audio {
        let Some(track) = project.track_mut(&a.track_id) else {
            continue;
        };
        for placement in &mut track.clips {
            let position = placement.position_ticks;
            let ClipContent::Audio(clip) = &mut placement.content else {
                continue;
            };
            if clip.id != a.clip_id {
                continue;
            }
            let Some(buffer) = pool.get(&clip.source_path) else {
                continue;
            };
            let rate = buffer.sample_rate.max(1) as f64;
            let frames = buffer.num_frames as u64;
            clip.source_start = ((a.start_secs * rate).round().max(0.0) as u64).min(frames);
            clip.source_end = frames;
            clip.warp_markers = warp_markers(&a.warps, rate, position, tempo);
        }
    }
}

/// Warp markers for warp points (beats into the clip, seconds into the
/// file), or none when every stretch plays at the file's own speed.
fn warp_markers(
    warps: &[(f64, f64)],
    rate: f64,
    clip_tick: u64,
    tempo: &hardwave_project::tempo::TempoMap,
) -> Vec<hardwave_project::clip::WarpMarker> {
    if warps.len() < 2 {
        return Vec::new();
    }
    let natural = warps.windows(2).all(|w| {
        let beats = w[1].0 - w[0].0;
        if beats <= 0.0 {
            return true;
        }
        let tick = clip_tick + (w[0].0 * 960.0) as u64;
        let seconds_per_beat = 60.0 / tempo.bpm_at(tick).max(1.0);
        let pace = (w[1].1 - w[0].1) / beats;
        (pace / seconds_per_beat - 1.0).abs() < 0.005
    });
    if natural {
        return Vec::new();
    }
    let mut markers: Vec<hardwave_project::clip::WarpMarker> = warps
        .iter()
        .map(|(beats, secs)| hardwave_project::clip::WarpMarker {
            clip_tick: (beats.max(0.0) * 960.0).round() as u64,
            source_sample: (secs.max(0.0) * rate).round() as u64,
        })
        .collect();
    markers.sort_by_key(|m| m.clip_tick);
    markers.dedup_by_key(|m| m.clip_tick);
    markers
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::dawproject::PackedFile;
    use std::io::Read;

    #[test]
    fn the_zip_holds_the_documents_the_samples_and_the_states() {
        let dir = std::env::temp_dir().join(format!("hw-dawproject-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = dir.join("kick.wav");
        std::fs::write(&sample, b"RIFF....WAVEfake").unwrap();
        let out = dawproject::Export {
            project_xml: "<Project version=\"1.0\"/>".into(),
            metadata_xml: "<MetaData/>".into(),
            files: vec![
                PackedFile {
                    path: "audio/kick.wav".into(),
                    source: FileSource::Path(sample.clone()),
                },
                PackedFile {
                    path: "plugins/lead-s1.vstpreset".into(),
                    source: FileSource::Bytes(b"VST3state".to_vec()),
                },
            ],
            left_out: Vec::new(),
        };
        let path = dir.join("song.dawproject");
        write_zip(&path, &out).expect("written");
        assert!(
            !path.with_extension("dawproject.part").exists(),
            "no half-written file left"
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut read = |name: &str| {
            let mut s = Vec::new();
            zip.by_name(name).unwrap().read_to_end(&mut s).unwrap();
            s
        };
        assert_eq!(read("project.xml"), b"<Project version=\"1.0\"/>");
        assert_eq!(read("metadata.xml"), b"<MetaData/>");
        assert_eq!(read("audio/kick.wav"), b"RIFF....WAVEfake");
        assert_eq!(read("plugins/lead-s1.vstpreset"), b"VST3state");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn zip_with(entries: &[(&str, &[u8])], at: &Path) -> zip::ZipArchive<std::fs::File> {
        let mut w = zip::ZipWriter::new(std::fs::File::create(at).unwrap());
        for (name, bytes) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap();
        zip::ZipArchive::new(std::fs::File::open(at).unwrap()).unwrap()
    }

    /// A sample unpacks into the song's folder under its own name, even
    /// when the project names it with folders or a way out of the folder.
    #[test]
    fn a_sample_unpacks_into_the_folder_and_nowhere_else() {
        let dir = std::env::temp_dir().join(format!("hw-dpi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut zip = zip_with(
            &[("audio/kick.wav", b"kick"), ("../../evil.wav", b"evil")],
            &dir.join("song.dawproject"),
        );
        let folder = dir.join("song Samples");
        let mut total = 0;
        let kick = unpack(&mut zip, "audio/kick.wav", &folder, &mut total).unwrap();
        assert_eq!(kick, folder.join("kick.wav"));
        assert_eq!(std::fs::read(&kick).unwrap(), b"kick");
        let evil = unpack(&mut zip, "../../evil.wav", &folder, &mut total).unwrap();
        assert_eq!(evil, folder.join("evil.wav"), "kept inside the folder");
        // A second file of the same name gets its own.
        let again = unpack(&mut zip, "audio/kick.wav", &folder, &mut total).unwrap();
        assert_eq!(again, folder.join("kick (2).wav"));
        assert_eq!(total, 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warp_markers_only_where_the_clip_does_not_play_at_its_own_speed() {
        let tempo = hardwave_project::tempo::TempoMap::default(); // 140 bpm
        let beat = 60.0 / 140.0;
        // Eight beats over eight beats' worth of file: as it is.
        assert!(
            warp_markers(&[(0.0, 1.0), (8.0, 1.0 + 8.0 * beat)], 48_000.0, 0, &tempo).is_empty()
        );
        // Eight beats over four beats' worth: half speed, marked.
        let m = warp_markers(&[(0.0, 1.0), (8.0, 1.0 + 4.0 * beat)], 48_000.0, 0, &tempo);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].clip_tick, m[0].source_sample), (0, 48_000));
        assert_eq!(m[1].clip_tick, 960 * 8);
    }

    #[test]
    fn a_device_is_found_by_its_id_or_its_name() {
        use hardwave_plugin_host::types::{PluginCategory, PluginDescriptor, PluginFormat as Host};
        use hardwave_project::dawproject_import::{DeviceFormat, ImportedPlugin};
        let desc = |id: &str, name: &str, format: Host| PluginDescriptor {
            id: id.into(),
            name: name.into(),
            vendor: String::new(),
            version: String::new(),
            format,
            path: PathBuf::from("/nowhere"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        };
        let installed = vec![
            desc("clap:org.surge-synth-team.surge-xt", "Surge XT", Host::Clap),
            desc("vst3:serum", "Serum", Host::Vst3),
        ];
        let device = |format, id: &str, name: &str| ImportedPlugin {
            track_id: String::new(),
            slot_id: String::new(),
            format,
            device_id: id.into(),
            name: name.into(),
            vendor: String::new(),
            instrument: true,
            state_file: None,
            external_state: false,
        };
        assert_eq!(
            find_plugin(
                &device(DeviceFormat::Clap, "org.surge-synth-team.surge-xt", "Surge"),
                &installed
            ),
            Some(("clap:org.surge-synth-team.surge-xt".into(), true))
        );
        assert_eq!(
            find_plugin(&device(DeviceFormat::Vst3, "ABCD", "serum"), &installed),
            Some(("vst3:serum".into(), true)),
            "the only VST3 of that name"
        );
        assert_eq!(
            find_plugin(&device(DeviceFormat::Vst2, "1234", "Serum"), &installed),
            Some(("vst3:serum".into(), false)),
            "a VST2 opens as its VST3, without its settings"
        );
        assert_eq!(
            find_plugin(&device(DeviceFormat::Vst3, "ABCD", "Massive"), &installed),
            None
        );
    }
}
