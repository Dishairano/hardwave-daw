//! What a new song opens with: Kick, Clap, Hat and Snare in the channel
//! rack, each with a sound, the way FL Studio opens.
//!
//! A new song opened with an empty channel rack, and the 4-Track Beat
//! template made four channels with nothing in them: a new producer
//! clicked steps, pressed play and heard nothing. Now the kick is the
//! built-in kick synth and the other three are one-shots in Hardwave
//! Sampler, from the drums folder installed with the DAW (`drums/` in
//! the app's resources, made by hardwave-dsp's `make_drum_kit` example).
//! The steps start empty, as in FL.

use std::path::{Path, PathBuf};

use hardwave_native_plugins::sampler::{self, NativeSampler};
use hardwave_project::track::{NativeInstrument, PluginSlot};
use hardwave_project::{Project, TrackKind};
use tauri::{AppHandle, Manager, State};

use crate::AppState;

/// The drums folder among the app's resources (`bundle.resources` in
/// tauri.conf.json): beside the program on Windows, in
/// Contents/Resources on macOS.
pub const DRUM_FOLDER: &str = "drums";

/// The note a step plays (the channel rack's steps play middle C), which
/// is where each one-shot plays at its own pitch.
const ROOT_NOTE: u8 = 60;

/// One channel a new song opens with.
pub(crate) struct NewSongChannel {
    pub name: &'static str,
    pub color: &'static str,
    /// The one-shot in the drums folder; none for the kick synth.
    pub sample: Option<&'static str>,
    /// The channel's fader, so the four sit together when they first
    /// play: the hat is quieter than the hits around it.
    pub volume_db: f64,
}

/// The channels, on the first four playlist rows, in this order.
pub(crate) const NEW_SONG_CHANNELS: [NewSongChannel; 4] = [
    NewSongChannel {
        name: "Kick",
        color: "#DC2626",
        sample: None,
        volume_db: 0.0,
    },
    NewSongChannel {
        name: "Clap",
        color: "#10B981",
        sample: Some("Clap 1.wav"),
        volume_db: -3.0,
    },
    NewSongChannel {
        name: "Hat",
        color: "#A855F7",
        sample: Some("Hat 1.wav"),
        volume_db: -8.0,
    },
    NewSongChannel {
        name: "Snare",
        color: "#F59E0B",
        sample: Some("Snare 1.wav"),
        volume_db: -3.0,
    },
];

/// The drums folder installed with the app.
pub(crate) fn drum_folder(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .resolve(DRUM_FOLDER, tauri::path::BaseDirectory::Resource)
        .map_err(|e| format!("The drums folder was not found: {e}"))
}

/// Where the drums installed with the DAW are, for the browser's
/// "Hardwave drums" folder.
#[tauri::command]
pub fn get_drum_folder(app: AppHandle) -> Result<String, String> {
    drum_folder(&app).map(|p| p.to_string_lossy().into_owned())
}

/// Whether the first rows are still the empty inserts every new song
/// has, so the drums never land on top of someone's work.
fn rows_are_free(project: &Project) -> bool {
    (1..=NEW_SONG_CHANNELS.len()).all(|row| {
        project.track(&format!("insert-{row:03}")).is_some_and(|t| {
            matches!(t.kind, TrackKind::Audio) && t.clips.is_empty() && t.inserts.is_empty()
        })
    })
}

/// A sampler's state with this one-shot loaded, set up as loading it by
/// hand would set it up. `None` when the file cannot be read.
fn one_shot_state(path: &Path) -> Option<Vec<u8>> {
    let (info, channels) = match hardwave_dsp::audio_file::AudioFileReader::read(path) {
        Ok(read) => read,
        Err(e) => {
            log::warn!("new song: {} could not be read: {e:?}", path.display());
            return None;
        }
    };
    let frames = channels.first().map_or(0, |c| c.len());
    let params =
        crate::commands::plugins::params_for_new_sample(Vec::new(), ROOT_NOTE, frames, &[]);
    Some(sampler::encode_sample_state_with(
        info.sample_rate,
        ROOT_NOTE,
        channels,
        &path.to_string_lossy(),
        params,
    ))
}

/// Put the four channels on the first playlist rows of a new song.
/// A one-shot that cannot be read leaves its channel with an empty
/// sampler, which a dropped sample fills; the song still opens.
/// Returns the channels' ids.
pub(crate) fn add_new_song_channels_to(project: &mut Project, drums: &Path) -> Vec<String> {
    let mut ids = Vec::with_capacity(NEW_SONG_CHANNELS.len());
    for (row, channel) in NEW_SONG_CHANNELS.iter().enumerate() {
        let id = project.claim_row(row + 1, channel.name, true);
        let sampler_state = channel.sample.map(|file| one_shot_state(&drums.join(file)));
        let mut slot_id = None;
        if let Some(track) = project.track_mut(&id) {
            track.color = channel.color.to_string();
            track.volume_db = channel.volume_db;
            if sampler_state.is_some() {
                let slot = uuid::Uuid::new_v4().to_string();
                track.inserts.push(PluginSlot {
                    id: slot.clone(),
                    plugin_id: NativeSampler::ID.to_string(),
                    enabled: true,
                    state: None,
                    sidechain_source: None,
                    wet: 1.0,
                });
                slot_id = Some(slot);
            } else {
                track.instrument = NativeInstrument::KickSynth;
            }
        }
        if let (Some(slot), Some(Some(bytes))) = (slot_id, sampler_state) {
            project.set_plugin_state(slot, "native-sampler", bytes);
        }
        ids.push(id);
    }
    ids
}

/// Give the new, empty song its four channels: Kick, Clap, Hat and Snare.
///
/// They are part of the new song, not an edit: undo does not take them
/// away and closing the untouched song does not ask to save it. A song
/// whose first rows are already in use is left alone.
#[tauri::command]
pub fn add_new_song_channels(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let drums = drum_folder(&app)?;
    let ids = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        if !rows_are_free(&project) {
            return Err("The song already has tracks on its first rows".into());
        }
        add_new_song_channels_to(&mut project, &drums)
    };
    {
        let engine = state.engine.lock();
        engine.sync_track_meters();
        engine.rebuild_graph();
    }
    for id in &ids {
        crate::commands::plugins::hydrate_track_chain(&state, id)?;
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drums as they ship: the folder tauri.conf.json bundles.
    fn shipped_drums() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(DRUM_FOLDER)
    }

    fn bundled_resources(conf: &str) -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(conf);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect(conf)).expect(conf);
        json["bundle"]["resources"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Both bundle configurations ship the drums folder: the unsigned
    /// build's list replaces the main one rather than adding to it.
    #[test]
    fn every_installer_ships_the_drums() {
        for conf in ["tauri.conf.json", "tauri.unsigned.conf.json"] {
            assert!(
                bundled_resources(conf)
                    .iter()
                    .any(|r| r == &format!("{DRUM_FOLDER}/*")),
                "{conf} bundles {DRUM_FOLDER}/*"
            );
        }
        for channel in &NEW_SONG_CHANNELS {
            if let Some(file) = channel.sample {
                assert!(
                    shipped_drums().join(file).is_file(),
                    "{file} is in the folder"
                );
            }
        }
        let total: u64 = std::fs::read_dir(shipped_drums())
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(
            total < 1_000_000,
            "the drums stay well under a megabyte: {total}"
        );
    }

    #[test]
    fn a_new_song_opens_with_kick_clap_hat_and_snare_loaded() {
        let mut project = Project::default();
        let ids = add_new_song_channels_to(&mut project, &shipped_drums());
        assert_eq!(
            ids,
            ["insert-001", "insert-002", "insert-003", "insert-004"]
        );
        let kick = project.track("insert-001").unwrap();
        assert_eq!(kick.name, "Kick");
        assert!(matches!(kick.kind, TrackKind::Midi));
        assert_eq!(kick.instrument, NativeInstrument::KickSynth);
        assert!(kick.inserts.is_empty(), "the kick synth needs no plug-in");
        for (id, name, file) in [
            ("insert-002", "Clap", "Clap 1.wav"),
            ("insert-003", "Hat", "Hat 1.wav"),
            ("insert-004", "Snare", "Snare 1.wav"),
        ] {
            let track = project.track(id).unwrap();
            assert_eq!(track.name, name);
            assert!(matches!(track.kind, TrackKind::Midi));
            assert_eq!(track.inserts.len(), 1);
            assert_eq!(track.inserts[0].plugin_id, NativeSampler::ID);
            let saved = project
                .plugin_state(&track.inserts[0].id)
                .expect("a sample");
            let state = sampler::decode_sample_state(&saved.chunk).expect("sampler bytes");
            assert!(state.sample.frames() > 1000, "{name} has its one-shot");
            assert_eq!(state.sample.base_note, ROOT_NOTE);
            assert!(
                state.path.ends_with(file),
                "{name} names {file}: {}",
                state.path
            );
            assert_eq!(
                sampler::sample_path_of(&saved.chunk),
                Some(state.path.clone())
            );
        }
        assert_eq!(
            project.tracks.len(),
            hardwave_project::project::DEFAULT_INSERT_COUNT + 1,
            "the channels take rows, they add none"
        );
        assert!(
            project.channel_rack_state.is_none(),
            "the steps start empty"
        );
    }

    #[test]
    fn a_missing_sample_leaves_an_empty_channel_and_the_song_still_opens() {
        let mut project = Project::default();
        let ids = add_new_song_channels_to(&mut project, Path::new("/nowhere/drums"));
        assert_eq!(ids.len(), 4);
        let clap = project.track("insert-002").unwrap();
        assert_eq!(clap.name, "Clap");
        assert_eq!(clap.inserts.len(), 1, "a sampler waiting for a sample");
        assert_eq!(clap.inserts[0].plugin_id, NativeSampler::ID);
        assert!(
            project.plugin_states.is_empty(),
            "with nothing loaded in it"
        );
    }

    #[test]
    fn the_drums_never_land_on_rows_in_use() {
        let mut project = Project::default();
        assert!(rows_are_free(&project));
        add_new_song_channels_to(&mut project, &shipped_drums());
        assert!(
            !rows_are_free(&project),
            "a second call finds the drums there"
        );
        let mut used = Project::default();
        used.claim_row(3, "Lead", true);
        assert!(!rows_are_free(&used));
    }

    /// A note on each channel makes a sound, as a clicked step does.
    #[test]
    fn every_channel_plays_when_its_step_is_on() {
        use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
        let engine = hardwave_engine::DawEngine::new();
        {
            let mut project = engine.project.lock();
            for id in add_new_song_channels_to(&mut project, &shipped_drums()) {
                let mut clip = hardwave_midi::MidiClip::new(
                    uuid::Uuid::new_v4().to_string(),
                    "Step".into(),
                    hardwave_midi::PPQ * 4,
                );
                clip.notes = vec![crate::commands::starter::note(
                    0,
                    hardwave_midi::PPQ / 4,
                    ROOT_NOTE,
                    0.78,
                )];
                project.track_mut(&id).unwrap().clips.push(ClipPlacement {
                    content: ClipContent::Midi(MidiClipRef {
                        id: uuid::Uuid::new_v4().to_string(),
                        clip,
                    }),
                    track_id: id.clone(),
                    position_ticks: 0,
                    length_ticks: hardwave_midi::PPQ * 4,
                    lane: 0,
                    color: None,
                });
            }
        }
        engine.rebuild_graph();
        let native = |id: &str| -> Option<Box<dyn hardwave_plugin_host::types::HostedPlugin>> {
            let descriptor = hardwave_native_plugins::native_plugin_descriptors()
                .into_iter()
                .find(|d| d.id == id)?;
            crate::commands::plugins::instantiate_plugin(&descriptor).ok()
        };
        for (solo, channel) in NEW_SONG_CHANNELS.iter().enumerate() {
            let solo_id = format!("insert-{:03}", solo + 1);
            let (mut peak, mut energy, mut n) = (0.0f32, 0.0f64, 0usize);
            engine
                .render_offline_with(
                    48_000,
                    24_000,
                    0,
                    Some(&native),
                    |p| {
                        for t in p.tracks.iter_mut().filter(|t| t.id.starts_with("insert-")) {
                            t.muted = t.id != solo_id;
                        }
                    },
                    |block| {
                        for s in block {
                            peak = peak.max(s.abs());
                            energy += (*s as f64) * (*s as f64);
                            n += 1;
                        }
                        true
                    },
                )
                .unwrap();
            let rms_db = 10.0 * (energy / n.max(1) as f64).log10();
            eprintln!(
                "{}: peak {:.1} dBFS, rms over 0.5 s {:.1} dB",
                channel.name,
                20.0 * peak.log10(),
                rms_db
            );
            assert!(peak > 0.05, "{} plays: peak {peak}", channel.name);
            assert!(peak <= 1.0, "{} does not clip: peak {peak}", channel.name);
        }
    }
}
