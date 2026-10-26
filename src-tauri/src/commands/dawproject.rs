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
}
