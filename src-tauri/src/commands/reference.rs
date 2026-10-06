//! A record to mix against.
//!
//! Mixing against a reference means switching between your own mix and
//! a track you trust, at the same loudness, so what you hear is the
//! difference in the mix rather than the difference in level. Doing it
//! by eye with a track on another channel gets the level wrong, which
//! is the one thing that makes the comparison useless.

use crate::AppState;
use serde::Serialize;
use tauri::State;

/// What the reference is doing.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceStatus {
    pub loaded: bool,
    pub playing: bool,
    /// Gain on the reference, in decibels.
    pub gain_db: f64,
    /// Its own integrated loudness, measured when it was loaded.
    pub lufs: f32,
    pub name: String,
}

#[tauri::command]
pub fn get_reference(state: State<AppState>) -> ReferenceStatus {
    let (loaded, playing, gain_db) = state.engine.lock().reference_status();
    let (lufs, name) = {
        let meta = state.reference_meta.lock();
        (meta.0, meta.1.clone())
    };
    ReferenceStatus {
        loaded,
        playing,
        gain_db,
        lufs,
        name,
    }
}

/// Load a track to compare against, and measure how loud it is.
///
/// The loudness is measured once, here, over the whole file: that is
/// the number the match uses, and measuring it per block would move
/// with the song rather than describing the record.
#[tauri::command]
pub fn load_reference(state: State<AppState>, path: String) -> Result<ReferenceStatus, String> {
    let file = std::path::PathBuf::from(&path);
    let name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Reference")
        .to_string();

    let lufs = {
        let engine = state.engine.lock();
        let (source_id, _info) = engine.load_audio_file(&file)?;
        let buffer = engine
            .audio_pool
            .get(&source_id)
            .ok_or_else(|| "the file loaded but is not in the pool".to_string())?;
        let left = buffer.channels.first().map(|c| c.as_slice()).unwrap_or(&[]);
        let right = buffer.channels.get(1).map(|c| c.as_slice()).unwrap_or(left);
        let stats = hardwave_dsp::loudness::measure(left, right, buffer.sample_rate as f32);
        engine.set_reference_source(Some(&source_id));
        stats.integrated_lufs
    };

    *state.reference_meta.lock() = (lufs, name.clone());
    let (loaded, playing, gain_db) = state.engine.lock().reference_status();
    Ok(ReferenceStatus {
        loaded,
        playing,
        gain_db,
        lufs,
        name,
    })
}

/// Hear the reference instead of the mix, or the mix again.
///
/// The reference does not go through the master chain or the master
/// fader: either would change what is being compared.
#[tauri::command]
pub fn set_reference_playing(state: State<AppState>, playing: bool) {
    state.engine.lock().set_reference_on(playing);
}

/// Put the reference at the mix's loudness.
///
/// `mix_lufs` is what the loudness meter says the mix is doing, which
/// the app already measures for the meter panel. The difference is the
/// gain: comparing at two different levels compares the levels.
#[tauri::command]
pub fn match_reference_loudness(state: State<AppState>, mix_lufs: f32) -> Result<f64, String> {
    let lufs = state.reference_meta.lock().0;
    if !lufs.is_finite() {
        return Err("the reference has no measured loudness".into());
    }
    if !mix_lufs.is_finite() {
        return Err("the mix is too quiet to measure; play the loudest part first".into());
    }
    let gain = (mix_lufs - lufs) as f64;
    state.engine.lock().set_reference_gain_db(gain);
    Ok(gain)
}

/// Set the reference's gain by hand.
#[tauri::command]
pub fn set_reference_gain(state: State<AppState>, gain_db: f64) {
    state.engine.lock().set_reference_gain_db(gain_db);
}

/// Put the reference away.
#[tauri::command]
pub fn clear_reference(state: State<AppState>) {
    let engine = state.engine.lock();
    engine.set_reference_on(false);
    engine.set_reference_source(None);
    drop(engine);
    *state.reference_meta.lock() = (f32::NEG_INFINITY, String::new());
}
