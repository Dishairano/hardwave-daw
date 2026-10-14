//! An FL project through the import and the engine, offline: parse the
//! .flp, build the song, give every sampler channel the built-in sampler
//! with the settings the import derives from FL, render, and check what
//! comes out. The project's own samples live on the machine that made it,
//! so each sampler plays a short synthetic hit instead; what is tested is
//! the import and the sampler, not the sound of the samples.
//!
//!   FLP_PATH=/path/to/project.flp cargo test -p hardwave-native-plugins \
//!     --test fl_import_render -- --ignored --nocapture

use hardwave_engine::DawEngine;
use hardwave_native_plugins::sampler::{self, NativeSampler};
use hardwave_plugin_host::types::HostedPlugin;
use hardwave_project::track::PluginSlot;

const SR: u32 = 48_000;

/// A decaying 110 Hz hit, a quarter second long: something every note can
/// be heard by.
fn hit() -> Vec<Vec<f32>> {
    let n = SR as usize / 4;
    let ch: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            (std::f32::consts::TAU * 110.0 * t).sin() * (-t * 12.0).exp() * 0.8
        })
        .collect();
    vec![ch.clone(), ch]
}

#[test]
#[ignore = "needs a real .flp: FLP_PATH=..."]
fn a_real_fl_project_imports_and_renders_with_its_sample_settings() {
    let Ok(path) = std::env::var("FLP_PATH") else {
        eprintln!("FLP_PATH not set; nothing to test");
        return;
    };
    let bytes = std::fs::read(&path).expect("the .flp reads");
    let fl = hardwave_project::flp_parser::parse(&bytes).expect("the .flp parses");
    // Every sample is "found": the import keeps the path, the test makes
    // the sound.
    let built =
        hardwave_project::fl_build::build_project(&fl, "test", &|p: &str| Some(p.to_string()));
    let mut project = built.project;
    let mut kinds = std::collections::BTreeMap::new();
    for c in &fl.channels {
        *kinds.entry(format!("{:?}", c.kind)).or_insert(0) += 1;
    }
    println!("channel kinds: {kinds:?}");

    // What the app's import does for each sampler channel (flp.rs).
    let mut expected = Vec::new();
    for s in &built.sampler_samples {
        let settings = sampler::settings_from_fl(&s.setup, s.root_note);
        let state =
            sampler::encode_sample_state_with(SR, s.root_note, hit(), &s.file, settings.clone());
        let slot_id = format!("sampler-{}", s.track_id);
        let track = project.track_mut(&s.track_id).expect("the channel's track");
        track.inserts.insert(
            0,
            PluginSlot {
                id: slot_id.clone(),
                plugin_id: NativeSampler::ID.to_string(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            },
        );
        project.set_plugin_state(slot_id.clone(), "native-sampler", state);
        expected.push((s.track_id.clone(), slot_id, settings));
    }

    // The saved state comes back as the settings it was given.
    for (track_id, slot_id, settings) in &expected {
        let bytes = &project.plugin_state(slot_id).expect("state kept").chunk;
        let mut p = NativeSampler::new();
        p.set_state(bytes).expect("the state loads");
        for (id, v) in settings {
            let got = p.get_parameter_value(*id) as f32;
            assert!(
                (got - v).abs() < 1e-6,
                "track {track_id}: parameter {id} is {got}, the import said {v}"
            );
        }
    }

    let tracks = project.tracks.len();
    let notes: usize = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match &c.content {
            hardwave_project::clip::ClipContent::Midi(m) => Some(m.clip.notes.len()),
            _ => None,
        })
        .sum();

    // Audio clips: every file they play is the same synthetic hit, two
    // seconds of it, so the clips' treatment (normalize, trim, declicking,
    // fade stereo) runs on real samples.
    let engine = DawEngine::new();
    let mut sources = std::collections::BTreeSet::new();
    let mut treatments = std::collections::BTreeMap::new();
    for t in &project.tracks {
        for c in &t.clips {
            if let hardwave_project::clip::ClipContent::Audio(a) = &c.content {
                sources.insert(a.source_path.clone());
                let tr = a.treatment;
                *treatments
                    .entry(format!(
                        "norm {} dc {} fadeSt {} trim {:.2} declick {:?} start {:.2} len {:.2}",
                        tr.normalize,
                        tr.remove_dc,
                        tr.fade_stereo,
                        tr.trim,
                        tr.declick,
                        tr.start,
                        tr.length
                    ))
                    .or_insert(0) += 1;
            }
        }
    }
    let two_seconds: Vec<f32> = (0..SR as usize * 2)
        .map(|i| {
            let t = i as f32 / SR as f32;
            (std::f32::consts::TAU * 110.0 * t).sin() * (-t * 1.5).exp() * 0.5
        })
        .collect();
    for src in &sources {
        engine.audio_pool.insert(
            src.clone(),
            hardwave_engine::AudioBuffer {
                channels: vec![two_seconds.clone(), two_seconds.clone()],
                sample_rate: SR,
                num_frames: two_seconds.len(),
            },
        );
    }
    println!(
        "audio clip files: {}; clip treatments: {treatments:#?}",
        sources.len()
    );
    *engine.project.lock() = project;
    engine.rebuild_graph();

    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        (id == NativeSampler::ID).then(|| Box::new(NativeSampler::new()) as Box<dyn HostedPlugin>)
    };
    let seconds = 60u64;
    let mut out: Vec<f32> = Vec::with_capacity((SR as u64 * seconds * 2) as usize);
    engine
        .render_offline_with(
            SR,
            SR as u64 * seconds,
            0,
            Some(&factory),
            |_| {},
            |b| {
                out.extend_from_slice(b);
                true
            },
        )
        .expect("it renders");

    let nan = out.iter().filter(|v| !v.is_finite()).count();
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let loud_seconds = out
        .chunks(SR as usize * 2)
        .filter(|s| s.iter().any(|v| v.abs() > 0.01))
        .count();
    println!(
        "{path}: {} sampler channels, {tracks} tracks, {notes} notes; {seconds} s rendered, peak {peak:.3}, {loud_seconds} s with sound, {nan} bad samples",
        built.sampler_samples.len()
    );
    for (track_id, _, settings) in &expected {
        let name = built
            .sampler_samples
            .iter()
            .find(|s| &s.track_id == track_id)
            .map(|s| s.file.rsplit(['\\', '/']).next().unwrap_or("").to_string())
            .unwrap_or_default();
        let get = |id: u32| {
            settings
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, v)| *v)
                .unwrap_or(f32::NAN)
        };
        println!(
            "  {name:<40} envelope {:.0} loop {:.0} mono {:.0} cut {:.0} declick {:.1} root {:.0}",
            get(sampler::PARAM_ENVELOPE),
            get(sampler::PARAM_LOOP),
            get(sampler::PARAM_MONO),
            get(sampler::PARAM_CUT_ITSELF),
            get(sampler::PARAM_DECLICK) * 5.0,
            get(sampler::PARAM_ROOT) * 127.0,
        );
    }
    assert_eq!(nan, 0, "no NaN or infinity in the render");
    assert!(peak > 0.01, "the samplers sound");
}
