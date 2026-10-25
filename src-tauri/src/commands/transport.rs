use crate::AppState;
use hardwave_engine::TransportCommand;
use hardwave_project::RecordSource;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Serialize)]
pub struct TransportInfo {
    playing: bool,
    recording: bool,
    looping: bool,
    position_samples: u64,
    bpm: f64,
    loop_start: u64,
    loop_end: u64,
    master_volume_db: f64,
    time_sig_numerator: u32,
    time_sig_denominator: u32,
    pattern_mode: bool,
    /// Armed-and-parked: Play/Record pressed with wait-for-input on,
    /// playback starts on the first MIDI event.
    waiting_for_input: bool,
}

#[tauri::command]
pub fn play(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let mut engine = state.engine.lock();
    // The device may have been let go while another program was in front.
    crate::audio_focus::reopen_if_released(&mut engine);
    // Wait-for-input (FL Ctrl+I): park instead of starting — the audio
    // thread flips `playing` on the first MIDI event. The engine-side
    // TransportCommand handler applies the same rule, so both paths
    // agree regardless of which runs first.
    if engine.transport.wait_for_input.load(Ordering::Relaxed)
        && !engine.transport.playing.load(Ordering::Relaxed)
    {
        engine.transport.wait_pending.store(true, Ordering::Relaxed);
    } else {
        engine.transport.playing.store(true, Ordering::Relaxed);
    }
    crate::commands::automation_write::note_play_start(&state, &engine);
    engine.send_command(TransportCommand::Play);
    state
        .collab
        .send(hardwave_project::multiplayer::SyncKind::Transport(
            hardwave_project::multiplayer::TransportSync::Play,
        ));
}

/// Toggle FL-style "wait for input" — with it on, Play/Record park the
/// transport until the first MIDI event arrives.
#[tauri::command]
pub fn set_wait_for_input(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .wait_for_input
        .store(enabled, Ordering::Relaxed);
    // Disabling while parked honours the earlier Play press.
    if !enabled && engine.transport.wait_pending.swap(false, Ordering::Relaxed) {
        engine.transport.playing.store(true, Ordering::Relaxed);
    }
    engine.send_command(TransportCommand::SetWaitForInput(enabled));
}

#[tauri::command]
pub fn stop(state: State<AppState>) -> Result<Option<RecordedTake>, String> {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    // Latch and Write passes end where the song stopped, before the
    // playhead goes back.
    crate::commands::automation_write::finish_passes(&state, &engine);
    engine
        .transport
        .wait_pending
        .store(false, Ordering::Relaxed);
    // Stop during a count-in cancels it. Without this the count would finish
    // in the background and start playing after the user had already stopped.
    engine
        .transport
        .count_in_then_play
        .store(false, Ordering::Relaxed);
    engine
        .transport
        .count_in_remaining
        .store(0, Ordering::Relaxed);
    engine.transport.count_in_total.store(0, Ordering::Relaxed);
    let was_playing = engine.transport.playing.swap(false, Ordering::Relaxed);
    if !was_playing {
        let loop_start = if engine.transport.looping.load(Ordering::Relaxed) {
            engine.transport.loop_start.load(Ordering::Relaxed)
        } else {
            0
        };
        engine.transport.set_position(loop_start);
    }
    // Stop also has to finalise an in-flight recording: previously this
    // flipped the recording flag without draining the capture buffer,
    // so pressing Space (which calls stop) mid-record silently dropped
    // the take. Now we drain, write the WAV, and return its path so the
    // frontend can place the clip on the armed track — mirroring how
    // toggle_recording does it on the trailing edge.
    let was_recording = engine.transport.recording.swap(false, Ordering::Relaxed);
    engine.send_command(TransportCommand::Stop);
    // The other side stops too; play and seek were sent, stop was not.
    state
        .collab
        .send(hardwave_project::multiplayer::SyncKind::Transport(
            hardwave_project::multiplayer::TransportSync::Stop,
        ));
    if was_recording {
        finalize_recording_session(&engine).map(Some)
    } else {
        Ok(None)
    }
}

/// Drain the engine's capture buffer and write a fresh WAV to the
/// recordings scratch dir. Returns the file path on success or `None`
/// when no samples were captured. Shared by `stop` and the trailing
/// edge of `toggle_recording` so the two paths can't drift.
/// Where a take is written.
///
/// Next to the project when there is one, in a `Recordings` folder beside the
/// .hwp, so a saved project keeps its takes and "collect samples" can find
/// them. Otherwise the app's own data folder.
///
/// This used to be the operating system's temp folder. Windows empties that
/// on its own schedule, and Disk Cleanup and Storage Sense both target it, so
/// a saved project could lose the vocal that was recorded into it with no
/// warning and nothing to recover.
/// How takes are written: as 24-bit or 32-bit float WAVs, and in which
/// folder. Set in Audio settings and kept with them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeFormat {
    /// 24-bit integer: smaller, and what most other programs expect. Off
    /// keeps a take exactly as captured, in 32-bit float.
    pub bits_24: bool,
    /// One folder for every take, whatever the song; none keeps takes in
    /// the song's own Recordings folder.
    pub folder: Option<String>,
}

static TAKE_FORMAT: std::sync::Mutex<TakeFormat> = std::sync::Mutex::new(TakeFormat {
    bits_24: false,
    folder: None,
});

pub fn take_format() -> TakeFormat {
    TAKE_FORMAT.lock().map(|f| f.clone()).unwrap_or_default()
}

pub fn set_take_format_value(format: TakeFormat) {
    if let Ok(mut f) = TAKE_FORMAT.lock() {
        *f = format;
    }
}

#[tauri::command]
pub fn get_take_format() -> TakeFormat {
    take_format()
}

#[tauri::command]
pub fn set_take_format(state: State<AppState>, format: TakeFormat) {
    let format = TakeFormat {
        folder: format.folder.filter(|f| !f.trim().is_empty()),
        ..format
    };
    set_take_format_value(format);
    crate::commands::engine::persist_audio_prefs_public(&state);
}

fn recordings_dir(engine: &hardwave_engine::DawEngine) -> Result<PathBuf, String> {
    if let Some(folder) = take_format().folder {
        return Ok(PathBuf::from(folder));
    }
    if let Some(project_dir) = engine.project_dir() {
        return Ok(project_dir.join("Recordings"));
    }
    let base = dirs::data_dir()
        .ok_or_else(|| "No writable data folder for recordings on this system".to_string())?;
    Ok(base.join("Hardwave").join("Recordings"))
}

/// A readable, unique file name for a take.
///
/// `Take 2026-09-17 21-04-11.wav` rather than `take-1789503851.wav`: a folder
/// of takes should be readable without converting unix seconds, and a second
/// take inside the same second must not overwrite the first.
fn next_take_name(dir: &Path) -> String {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let base = format!("Take {stamp}");
    let mut candidate = format!("{base}.wav");
    let mut n = 2;
    while dir.join(&candidate).exists() {
        candidate = format!("{base} ({n}).wav");
        n += 1;
    }
    candidate
}

/// What a finished take turned out to be.
///
/// The command used to answer with a path or null, so the three ways a take
/// can disappoint were indistinguishable in the UI and all three ended in
/// silence with no message: nothing captured at all, a captured take that is
/// pure silence because the wrong input was selected, and a take that lost
/// audio on its way to disk.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedTake {
    pub path: Option<String>,
    pub seconds: f64,
    /// Highest sample in the take, so the UI can warn about a silent one.
    pub peak: f32,
    /// True when audio was lost on the way to disk: the disk fell behind, or
    /// the recordings folder could not be written.
    pub truncated: bool,
    /// One entry per loop pass, in the order played, each with the position
    /// it starts at. A single entry when the loop never wrapped. With armed
    /// tracks on different inputs, the first input's.
    pub passes: Vec<TakePass>,
    /// Each armed audio track's own passes, by track id: the take of the
    /// input that track records, so two tracks on two inputs each get their
    /// own audio instead of one shared take.
    pub by_track: std::collections::HashMap<String, Vec<TakePass>>,
}

/// One loop pass of a take, written as its own file.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TakePass {
    pub path: String,
    /// Timeline position, in samples, where this pass begins.
    pub start_samples: u64,
    /// The same position in ticks, through the project's tempo map, so the
    /// UI places the clip without redoing the conversion at one tempo.
    pub start_ticks: u64,
    pub seconds: f64,
    pub peak: f32,
}

/// How a take's file says which input it came from, when a recording had
/// more than one: `In 1`, `In 3-4`, or the name of the channel recorded.
fn source_label(project: &hardwave_project::Project, source: &RecordSource) -> String {
    let label = match source {
        RecordSource::Interface(pick) if pick.mono => format!("In {}", pick.first + 1),
        RecordSource::Interface(pick) => format!("In {}-{}", pick.first + 1, pick.first + 2),
        RecordSource::Track(id) => project
            .track(id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| id.clone()),
    };
    label
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}

/// `name` in `dir`, or `name (2)`, `name (3)`... when it is taken, so a
/// take never overwrites another.
fn free_name(dir: &Path, stem: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.wav"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}).wav"));
        n += 1;
    }
    candidate
}

fn finalize_recording_session(engine: &hardwave_engine::DawEngine) -> Result<RecordedTake, String> {
    let truncated = engine.capture_overflowed();
    let takes = engine.stop_capture_sources();
    let total: usize = takes
        .iter()
        .flat_map(|t| t.passes.iter())
        .map(|(_, r)| r.len())
        .sum();
    if total == 0 {
        return Ok(RecordedTake {
            path: None,
            seconds: 0.0,
            peak: 0.0,
            truncated,
            passes: Vec::new(),
            by_track: Default::default(),
        });
    }

    let sample_rate = engine.current_sample_rate();
    let dir = recordings_dir(engine)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = next_take_name(&dir);
    let stem = base.trim_end_matches(".wav").to_string();
    // What each armed track records, and each input's name, read once:
    // the files below are written without holding the song.
    let default = engine.default_input();
    let (labels, track_sources) = {
        let project = engine.project.lock();
        let labels: Vec<String> = takes
            .iter()
            .map(|t| source_label(&project, &t.source))
            .collect();
        let track_sources: Vec<(String, RecordSource)> = project
            .record_armed()
            .map(|t| (t.id.clone(), project.record_source_of(t, default)))
            .collect();
        (labels, track_sources)
    };
    let named = takes
        .iter()
        .filter(|t| t.passes.iter().any(|(_, r)| !r.is_empty()))
        .count()
        > 1;

    let mut peak = 0.0_f32;
    let mut per_source: Vec<Vec<TakePass>> = Vec::with_capacity(takes.len());
    for (take, label) in takes.iter().zip(&labels) {
        let source_stem = if named {
            format!("{stem} {label}")
        } else {
            stem.clone()
        };
        let several = take.passes.iter().filter(|(_, r)| !r.is_empty()).count() > 1;
        let mut files = Vec::new();
        for (i, (start, range)) in take.passes.iter().enumerate() {
            if range.is_empty() {
                continue;
            }
            let path = if several {
                free_name(&dir, &format!("{source_stem} pass {}", i + 1))
            } else {
                free_name(&dir, &source_stem)
            };
            let pass_peak = write_take_wav(&take.file, range.clone(), &path, sample_rate)?;
            peak = peak.max(pass_peak);
            let start_ticks = {
                let project = engine.project.lock();
                project
                    .tempo_map
                    .samples_to_tick(*start, sample_rate as f64)
            };
            files.push(TakePass {
                path: path.to_string_lossy().to_string(),
                start_samples: *start,
                start_ticks,
                seconds: range.len() as f64 / 2.0 / sample_rate.max(1) as f64,
                peak: pass_peak,
            });
        }
        per_source.push(files);
    }

    let by_track = track_sources
        .into_iter()
        .filter_map(|(id, source)| {
            let k = takes.iter().position(|t| t.source == source)?;
            Some((id, per_source[k].clone()))
        })
        .collect();
    let first = per_source.into_iter().next().unwrap_or_default();
    let last = first.last().cloned();
    Ok(RecordedTake {
        path: last.as_ref().map(|p| p.path.clone()),
        seconds: first.iter().map(|p| p.seconds).sum(),
        peak,
        truncated,
        passes: first,
        by_track,
    })
}

/// One pass of a take, `range` of its samples, as a stereo WAV: 32-bit float
/// by default, the engine's own format, so a take is stored exactly as it
/// was captured, or 24-bit when chosen in Audio settings. Read from the
/// take's file a block at a time, so a long take never sits in memory.
/// Returns the pass's peak.
fn write_take_wav(
    take: &hardwave_engine::input_node::TakeFile,
    range: std::ops::Range<usize>,
    path: &Path,
    sample_rate: u32,
) -> Result<f32, String> {
    let bits_24 = take_format().bits_24;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: if bits_24 { 24 } else { 32 },
        sample_format: if bits_24 {
            hound::SampleFormat::Int
        } else {
            hound::SampleFormat::Float
        },
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let mut peak = 0.0_f32;
    let mut at = range.start;
    const BLOCK: usize = 1 << 16;
    while at < range.end {
        let end = (at + BLOCK).min(range.end);
        for s in take.read(at..end).map_err(|e| e.to_string())? {
            peak = peak.max(s.abs());
            if bits_24 {
                let v = (s.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                writer.write_sample(v).map_err(|e| e.to_string())?;
            } else {
                writer.write_sample(s).map_err(|e| e.to_string())?;
            }
        }
        at = end;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(peak)
}

#[cfg(test)]
mod take_name_tests {
    use super::*;

    /// A take's raw file, as the engine leaves it when recording stops.
    fn raw_take(dir: &Path, samples: &[f32]) -> hardwave_engine::input_node::TakeFile {
        let path = dir.join("raw.f32");
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        hardwave_engine::input_node::TakeFile {
            path,
            samples: samples.len(),
        }
    }

    #[test]
    fn a_pass_is_written_from_its_range_of_the_take_in_the_chosen_format() {
        let dir = tempfile::tempdir().unwrap();
        let take = raw_take(dir.path(), &[0.1, 0.1, 0.5, -0.5, -1.0, 1.0, 0.2, 0.2]);

        set_take_format_value(TakeFormat::default());
        let float = dir.path().join("float.wav");
        let peak = write_take_wav(&take, 2..6, &float, 48_000).unwrap();
        assert_eq!(peak, 1.0, "the peak is the pass's, not the take's");
        let mut r = hound::WavReader::open(&float).unwrap();
        assert_eq!(r.spec().bits_per_sample, 32);
        let got: Vec<f32> = r.samples::<f32>().map(|s| s.unwrap()).collect();
        assert_eq!(got, vec![0.5, -0.5, -1.0, 1.0]);

        set_take_format_value(TakeFormat {
            bits_24: true,
            folder: None,
        });
        let int = dir.path().join("int.wav");
        write_take_wav(&take, 2..6, &int, 48_000).unwrap();
        set_take_format_value(TakeFormat::default());
        let mut r = hound::WavReader::open(&int).unwrap();
        assert_eq!(r.spec().bits_per_sample, 24);
        assert_eq!(r.spec().sample_format, hound::SampleFormat::Int);
        let got: Vec<i32> = r.samples::<i32>().map(|s| s.unwrap()).collect();
        assert_eq!(got, vec![4_194_304, -4_194_304, -8_388_607, 8_388_607]);
    }

    /// With two inputs recorded at once, each file says which input it is,
    /// and a name already taken gets a number instead of being overwritten.
    #[test]
    fn each_input_s_take_is_named_after_it() {
        let mut p = hardwave_project::Project::default();
        let mut t = hardwave_project::Track::new_audio("bus".into(), "Drum: bus".into());
        t.armed = false;
        p.tracks.push(t);
        let ch =
            |first, mono| RecordSource::Interface(hardwave_project::InputChannel { first, mono });
        assert_eq!(source_label(&p, &ch(0, true)), "In 1");
        assert_eq!(source_label(&p, &ch(2, false)), "In 3-4");
        assert_eq!(
            source_label(&p, &RecordSource::Track("bus".into())),
            "Drum_ bus",
            "no characters a file name cannot hold"
        );

        let dir = tempfile::tempdir().unwrap();
        let first = free_name(dir.path(), "Take In 1");
        assert!(first.ends_with("Take In 1.wav"));
        std::fs::write(&first, b"x").unwrap();
        assert!(free_name(dir.path(), "Take In 1").ends_with("Take In 1 (2).wav"));
    }

    #[test]
    fn a_take_is_named_readably_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let first = next_take_name(dir.path());
        assert!(first.starts_with("Take 20"), "got {first}");
        assert!(first.ends_with(".wav"));

        std::fs::write(dir.path().join(&first), b"x").unwrap();
        let second = next_take_name(dir.path());
        assert_ne!(
            second, first,
            "a second take in the same second must differ"
        );
        assert!(second.contains("(2)"), "got {second}");
    }
}

#[tauri::command]
pub fn set_position(state: State<AppState>, position: u64) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.set_position(position);
    // The other person's playhead goes where yours does, so "listen
    // to this bit" means the same bar on both screens.
    {
        let bpm = engine.transport.bpm.load(Ordering::Relaxed);
        let sample_rate = engine.current_sample_rate() as f64;
        let beats = position as f64 / sample_rate * bpm / 60.0;
        state
            .collab
            .send(hardwave_project::multiplayer::SyncKind::Transport(
                hardwave_project::multiplayer::TransportSync::Seek {
                    tick: (beats * hardwave_midi::PPQ as f64) as u64,
                },
            ));
    }
    // Also queue for the audio thread so double-stop logic stays consistent.
    engine.send_command(TransportCommand::SetPosition(position));

    // Tempo and signature at the new position. While playing the audio thread
    // follows the map itself, but a seek while stopped never reaches it, so
    // the transport read-out and the click kept the values from wherever the
    // playhead used to be.
    let (bpm, num, den) = {
        let project = engine.project.lock();
        if project.tempo_map.entries.len() < 2 {
            return;
        }
        let tick = project
            .tempo_map
            .samples_to_tick(position, engine.current_sample_rate() as f64);
        let (num, den) = project.tempo_map.time_sig_at(tick);
        (project.tempo_map.bpm_at(tick), num, den)
    };
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(num, den),
        Ordering::Relaxed,
    );
    engine.send_command(TransportCommand::SetTimeSignature(num, den));
}

#[tauri::command]
pub fn set_bpm(state: State<AppState>, bpm: f64) {
    use std::sync::atomic::Ordering;
    if !bpm.is_finite() {
        return;
    }
    let bpm = bpm.clamp(20.0, 999.0);
    // The tempo is part of the song, so undo restores it. A drag over the
    // tempo field is one gesture on the UI side, which wraps it in a
    // history group, so this does not fill the list with one step per pixel.
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    engine.transport.bpm.store(bpm, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetBpm(bpm));
    // Persist to project so save_project sees the current BPM.
    let mut project = engine.project.lock();
    if let Some(entry) = project.tempo_map.entries.get_mut(0) {
        entry.bpm = bpm;
    }
}

/// Flip the transport's recording flag and start / stop the matching
/// capture session. When recording flips on, we clear the capture
/// buffer and arm the InputNode tap so the next audio block begins
/// streaming input samples into memory. When it flips off — either by
/// the user pressing Record again or by a Stop — we drain the captured
/// samples, write them to a `.wav` under the project's autosave dir,
/// and place a fresh audio clip on the first armed track at the sample
/// position the recording started.
///
/// Returns the path of the freshly-written WAV when a session ends, or
/// `None` when this call started one. The frontend uses this to show a
/// "took #N saved" notification.
#[tauri::command]
pub fn toggle_recording(state: State<AppState>) -> Result<Option<RecordedTake>, String> {
    use std::sync::atomic::Ordering;
    let mut engine = state.engine.lock();
    crate::audio_focus::reopen_if_released(&mut engine);
    let was_recording = engine.transport.recording.load(Ordering::Relaxed);

    if !was_recording {
        // Every input the armed tracks ask for records at once, each to a
        // take of its own, up to a limit; past it, say so rather than leave
        // some of them out.
        let wanted = engine
            .project
            .lock()
            .record_sources(engine.default_input())
            .len();
        let most = hardwave_engine::input_node::MAX_RECORD_SOURCES;
        if wanted > most {
            return Err(format!(
                "The armed tracks ask for {wanted} different inputs; up to {most} record at once. \
                 Disarm some, or give them the same input."
            ));
        }
        // Begin: arm the InputNode tap and remember where we started so
        // the resulting clip gets placed at the right timeline position.
        engine.transport.recording.store(true, Ordering::Relaxed);
        // The take streams to disk as it is played, next to where it will
        // end up, so a long take fills the disk the recordings live on.
        if let Ok(dir) = recordings_dir(&engine) {
            engine.set_capture_dir(dir);
        }
        engine.start_capture();
        return Ok(None);
    }

    // End: stop capturing, drain samples, write a WAV, return its path.
    engine.transport.recording.store(false, Ordering::Relaxed);
    finalize_recording_session(&engine).map(Some)
}

/// Cancel an in-flight recording without finalising the take. FL
/// Studio's Tools → Macros → Panic → Cancel recording — drops the
/// captured samples on the floor instead of writing the WAV, so the
/// user can bail from a recording mistake without leaving an
/// orphaned take on disk or in the project.
///
/// No-op when not currently recording.
#[tauri::command]
pub fn cancel_recording(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let was_recording = engine.transport.recording.swap(false, Ordering::Relaxed);
    if !was_recording {
        return;
    }
    // Drain and drop — same path as the regular stop, just no WAV write.
    let _ = engine.stop_capture();
}

#[tauri::command]
pub fn toggle_loop(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    // The audio thread reads `transport.looping` directly each block, so
    // toggling the atomic here is enough — no command needs to be queued.
    // Sending TransportCommand::ToggleLoop in addition would re-toggle on
    // the next dispatch tick and cancel the change, leaving the test
    // observing `false → false → true` instead of `false → true → false`.
    let current = engine.transport.looping.load(Ordering::Relaxed);
    engine.transport.looping.store(!current, Ordering::Relaxed);
}

/// Set the punch range used while recording.
///
/// The playlist has let people set a punch range, drawn it in the ruler and
/// saved it with the project since it was written, and nothing read it:
/// recording captured the whole pass regardless. Positions are ticks, so the
/// engine converts them through the project's tempo map.
#[tauri::command]
pub fn set_punch_range(state: State<AppState>, enabled: bool, in_ticks: u64, out_ticks: u64) {
    state
        .engine
        .lock()
        .set_punch_ticks(enabled, in_ticks, out_ticks);
}

/// The punch window the audio thread is using, in samples. Lets the UI place
/// a punched take at the punch point rather than where record was pressed.
/// The playhead in ticks, through the project's tempo map. The UI only sees
/// the playhead in samples, and a one-tempo conversion there puts anything
/// placed "at the playhead" in the wrong bar after a tempo change.
#[tauri::command]
pub fn get_playhead_tick(state: State<AppState>) -> u64 {
    let engine = state.engine.lock();
    let sample_rate = engine.current_sample_rate() as f64;
    let position = engine.transport.position();
    let project = engine.project.lock();
    project.tempo_map.samples_to_tick(position, sample_rate)
}

#[tauri::command]
pub fn get_punch_range(state: State<AppState>) -> (bool, u64, u64) {
    state.engine.lock().punch_samples()
}

#[tauri::command]
pub fn set_loop(state: State<AppState>, start: u64, end: u64) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine.transport.loop_start.store(start, Ordering::Relaxed);
    engine.transport.loop_end.store(end, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetLoop(start, end));
}

#[tauri::command]
pub fn set_master_volume(state: State<AppState>, db: f64) {
    use std::sync::atomic::Ordering;
    if !db.is_finite() {
        return;
    }
    let db = db.clamp(-100.0, 12.0);
    let engine = state.engine.lock();
    engine
        .transport
        .master_volume_db
        .store(db, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetMasterVolume(db));
}

/// Set the project's time signature.
///
/// This used to write the transport atomics only, so the signature was lost
/// on save: reopening the project put it back to whatever the tempo map said,
/// which was 4/4 for every project ever made in this DAW. It now writes the
/// tempo map as well.
///
/// Entries later in the song that carry the old signature follow the change,
/// because they inherited it rather than being set deliberately. An entry
/// with a different signature is a deliberate mid-song change and is left
/// alone.
#[tauri::command]
pub fn set_time_signature(
    state: State<AppState>,
    numerator: u32,
    denominator: u32,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let (num, den) = crate::commands::project::validate_time_signature(numerator, denominator)?;
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    {
        let mut project = engine.project.lock();
        let previous = project.tempo_map.time_sig_at(0);
        for entry in project.tempo_map.entries.iter_mut() {
            if (entry.time_sig_num, entry.time_sig_den) == previous {
                entry.time_sig_num = num;
                entry.time_sig_den = den;
            }
        }
    }
    engine.transport.time_sig.store(
        hardwave_engine::transport::pack_time_sig(num, den),
        Ordering::Relaxed,
    );
    engine.send_command(TransportCommand::SetTimeSignature(num, den));
    Ok(())
}

#[tauri::command]
pub fn set_pattern_mode(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    engine
        .transport
        .pattern_mode
        .store(enabled, Ordering::Relaxed);
    engine.send_command(TransportCommand::SetPatternMode(enabled));
}

#[tauri::command]
pub fn get_transport_state(state: State<AppState>) -> TransportInfo {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let t = &engine.transport;
    let (num, den) =
        hardwave_engine::transport::unpack_time_sig(t.time_sig.load(Ordering::Relaxed));
    TransportInfo {
        playing: t.is_playing(),
        recording: t.recording.load(Ordering::Relaxed),
        looping: t.looping.load(Ordering::Relaxed),
        position_samples: t.position(),
        bpm: t.bpm.load(Ordering::Relaxed),
        loop_start: t.loop_start.load(Ordering::Relaxed),
        loop_end: t.loop_end.load(Ordering::Relaxed),
        master_volume_db: t.master_volume_db.load(Ordering::Relaxed),
        time_sig_numerator: num,
        time_sig_denominator: den,
        pattern_mode: t.pattern_mode.load(Ordering::Relaxed),
        waiting_for_input: t.wait_pending.load(Ordering::Relaxed),
    }
}

/// Count off `bars` before playback starts.
///
/// The count-in used to be a row of WebAudio oscillators in the WebView plus a
/// `setTimeout` that called play when it thought they had finished, so the
/// clicks a take was counted in against came from a different clock than the
/// audio, and playback began wherever the timeout landed. The audio thread now
/// owns both: it counts in samples and starts playback on the sample the count
/// ends.
#[tauri::command]
pub fn start_count_in(state: State<AppState>, bars: u32) {
    use std::sync::atomic::Ordering;
    let mut engine = state.engine.lock();
    crate::audio_focus::reopen_if_released(&mut engine);
    if bars == 0 {
        engine.transport.playing.store(true, Ordering::Relaxed);
        return;
    }
    let bpm = engine.transport.bpm.load(Ordering::Relaxed).max(1.0);
    let sample_rate = engine.transport.sample_rate.load(Ordering::Relaxed).max(1) as f64;
    let (beats_per_bar, den) = hardwave_engine::transport::unpack_time_sig(
        engine.transport.time_sig.load(Ordering::Relaxed),
    );
    let beats = (bars * beats_per_bar.max(1)) as f64;
    // A beat is a note value, so the denominator sets its length. Counting a
    // bar of 7/8 in quarter notes made the count-in twice as long as the bar
    // it was counting in.
    let samples = (beats * 60.0 / bpm * sample_rate * 4.0 / den.max(1) as f64).round() as u64;

    engine.transport.playing.store(false, Ordering::Relaxed);
    engine
        .transport
        .count_in_total
        .store(samples, Ordering::Relaxed);
    engine
        .transport
        .count_in_remaining
        .store(samples, Ordering::Relaxed);
    engine
        .transport
        .count_in_then_play
        .store(true, Ordering::Relaxed);
}

/// How far a count-in has got, for the on-screen counter.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountInState {
    pub active: bool,
    /// Beats already counted, and the total, so the UI can show "2 of 4".
    pub beat: u32,
    pub total_beats: u32,
}

#[tauri::command]
pub fn get_count_in_state(state: State<AppState>) -> CountInState {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let remaining = engine.transport.count_in_remaining.load(Ordering::Relaxed);
    let total = engine.transport.count_in_total.load(Ordering::Relaxed);
    if total == 0 {
        return CountInState {
            active: false,
            beat: 0,
            total_beats: 0,
        };
    }
    let bpm = engine.transport.bpm.load(Ordering::Relaxed).max(1.0);
    let sample_rate = engine.transport.sample_rate.load(Ordering::Relaxed).max(1) as f64;
    let samples_per_beat = (60.0 / bpm * sample_rate).max(1.0);
    let elapsed = total.saturating_sub(remaining) as f64;
    CountInState {
        active: remaining > 0,
        beat: (elapsed / samples_per_beat).floor() as u32,
        total_beats: (total as f64 / samples_per_beat).round() as u32,
    }
}

/// The video being scored to, if there is one.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStatus {
    pub path: String,
    pub offset_ticks: u64,
    pub muted: bool,
}

#[tauri::command]
pub fn get_video(state: State<AppState>) -> Option<VideoStatus> {
    let engine = state.engine.lock();
    let video = engine.project.lock().video.clone()?;
    // The window opens whatever path this returns, so a project's video
    // goes through the same rules as its samples.
    let path = match engine.resolve_video_file(&video.path) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(why) => {
            log::warn!("the project's video was not opened: {why}");
            return None;
        }
    };
    Some(VideoStatus {
        path,
        offset_ticks: video.offset_ticks,
        muted: video.muted,
    })
}

/// Score to a film: the video follows the playhead, so a hit can be
/// written where the cut is rather than where it sounded about right.
///
/// The picture is played by the window rather than decoded here: a
/// frame on screen is the window's job, and the song's clock is ours.
#[tauri::command]
pub fn set_video(
    state: State<AppState>,
    path: String,
    offset_ticks: Option<u64>,
) -> Result<VideoStatus, String> {
    // Chosen by the person, so its network server is one they use.
    state.engine.lock().trust_network_location(&path);
    crate::commands::engine::persist_audio_prefs_public(&state);
    if !std::path::Path::new(&path).exists() {
        return Err("there is no file there".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let video = hardwave_project::project::VideoTrack {
        path: path.clone(),
        offset_ticks: offset_ticks.unwrap_or(0),
        muted: true,
    };
    project.video = Some(video.clone());
    Ok(VideoStatus {
        path: video.path,
        offset_ticks: video.offset_ticks,
        muted: video.muted,
    })
}

/// Move the film against the song, in ticks.
#[tauri::command]
pub fn set_video_offset(state: State<AppState>, offset_ticks: u64) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(video) = project.video.as_mut() {
        video.offset_ticks = offset_ticks;
    }
}

/// Whether the film's own sound is heard. Off by default: the song is
/// the point, and a film's dialogue under it is rarely what anyone
/// wants while writing.
#[tauri::command]
pub fn set_video_muted(state: State<AppState>, muted: bool) {
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    if let Some(video) = project.video.as_mut() {
        video.muted = muted;
    }
}

#[tauri::command]
pub fn clear_video(state: State<AppState>) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    engine.project.lock().video = None;
}
