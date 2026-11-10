//! The mixer by number changes nothing that is heard until it is used.
//!
//! An empty mixer number made into an insert is an empty bus into the
//! master: it builds no node and the song renders the same, bit for bit.
//! A channel routed into a new, untouched insert plays through it into the
//! master, changed only by the insert's pan law at centre, as routing into
//! any bus always was. Numbering, naming and the rack's groups are display
//! only.

mod common;

use common::*;
use hardwave_engine::{AudioBuffer, DawEngine};
use hardwave_project::clip::{ClipContent, ClipPlacement};
use hardwave_project::Project;

const RATE: u32 = 48_000;

fn render(project: Project, tone: &AudioBuffer) -> Vec<f32> {
    let engine = DawEngine::new();
    engine.audio_pool.insert("tone".into(), tone.clone());
    *engine.project.lock() = project;
    let mut out = Vec::new();
    engine
        .render_offline(RATE, RATE as u64, |block| {
            out.extend_from_slice(block);
            true
        })
        .unwrap();
    out
}

fn assert_same(name: &str, before: &[f32], after: &[f32]) {
    assert_eq!(before.len(), after.len(), "{name}: another length");
    if let Some(i) = before
        .iter()
        .zip(after)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!(
            "{name}: sample {i} was {} and is now {}",
            before[i], after[i]
        );
    }
}

/// A song made before FL's three lists: two sample tracks, each a channel
/// with an insert of its own, one turned down and panned.
fn song(tone: &AudioBuffer) -> (Project, String, String) {
    let frames = tone.num_frames as u64;
    let mut p = Project::default();
    let mut add = |name: &str, at: u64| {
        let id = p.add_audio_track(name.into());
        let t = p.track_mut(&id).unwrap();
        t.clips.push(ClipPlacement {
            content: ClipContent::Audio(make_audio_clip("tone", frames)),
            track_id: id.clone(),
            position_ticks: at,
            length_ticks: 1440,
            lane: 0,
            color: None,
            row: None,
        });
        id
    };
    let kick = add("Kick", 0);
    let hat = add("Hat", 480);
    let t = p.track_mut(&hat).unwrap();
    t.volume_db = -4.5;
    t.pan = 0.3;
    (p, kick, hat)
}

#[test]
fn making_and_routing_to_empty_inserts_sounds_the_same() {
    let tone = make_sine_buffer(RATE, 1.0, 110.0, 0.4);
    let (mut p, kick, hat) = song(&tone);
    // Saved before inserts were at 0 dB: the old law stays.
    p.inserts_unity_at_centre = false;
    let before = render(p.clone(), &tone);
    assert!(
        before.iter().any(|s| s.abs() > 1e-4),
        "the song makes a sound"
    );

    // An empty number made into an insert, named, nothing in it.
    let mut made = p.clone();
    made.make_insert(600, Some("KICK VERB")).unwrap();
    assert_same("an empty insert made", &before, &render(made, &tone));

    // Both channels routed into new inserts (one each, then both into one).
    // In a song made before, an insert is a track like any other, so its
    // pan law takes its centre (-3 dB by default) once more, as routing
    // into any bus always did: the level drops by exactly that and nothing
    // else changes, so such a song keeps sounding as it was mixed.
    let centre = hardwave_engine::pan::gains(0.0, hardwave_engine::pan::PanLaw::Minus3dB).0;
    let mut routed = p.clone();
    assert!(routed.route_to_insert(&kick, 610).unwrap().is_some());
    assert!(routed.route_to_insert(&hat, 611).unwrap().is_some());
    let each = render(routed.clone(), &tone);
    assert_scaled("channels into new inserts", &before, &each, centre);
    routed.route_to_insert(&hat, 610).unwrap();
    let one = render(routed.clone(), &tone);
    assert_scaled("two channels into one insert", &before, &one, centre);

    // And back to the master: as it was, bit for bit.
    routed.route_to_insert(&kick, 0).unwrap();
    routed.route_to_insert(&hat, 0).unwrap();
    assert_same("routed back", &before, &render(routed, &tone));
}

/// `after` is `before` times `gain`, sample for sample, to rounding.
fn assert_scaled(name: &str, before: &[f32], after: &[f32], gain: f32) {
    assert_eq!(before.len(), after.len(), "{name}: another length");
    for (i, (a, b)) in before.iter().zip(after).enumerate() {
        assert!(
            (a * gain - b).abs() <= 1e-6,
            "{name}: sample {i} was {a} and is now {b}, not {}",
            a * gain
        );
    }
}

/// In a new song the inserts are at 0 dB at centre, as in FL: a channel
/// routed into an insert (or two into one, or through two inserts in a
/// row) is exactly as loud as one playing straight into the master.
#[test]
fn in_a_new_song_routing_into_an_insert_keeps_the_level() {
    let tone = make_sine_buffer(RATE, 1.0, 110.0, 0.4);
    let (p, kick, hat) = song(&tone);
    assert!(p.inserts_unity_at_centre, "a new song");
    let before = render(p.clone(), &tone);

    let mut routed = p.clone();
    routed.route_to_insert(&kick, 610).unwrap();
    routed.route_to_insert(&hat, 611).unwrap();
    assert_scaled(
        "channels into new inserts",
        &before,
        &render(routed.clone(), &tone),
        1.0,
    );
    routed.route_to_insert(&hat, 610).unwrap();
    assert_scaled(
        "two channels into one insert",
        &before,
        &render(routed.clone(), &tone),
        1.0,
    );
    let into = routed.insert_numbered(610).unwrap().id.clone();
    routed.route_to_insert(&into, 620).unwrap();
    assert_scaled("through two inserts", &before, &render(routed, &tone), 1.0);
}

/// An insert in a new song still pans: hard left keeps the left side at
/// 0 dB and silences the right, as FL's law does (nothing is boosted).
#[test]
fn an_insert_at_0_db_still_pans_without_boosting() {
    let (l, r) = hardwave_engine::pan::gains(0.0, hardwave_engine::pan::PanLaw::Zero);
    assert_eq!((l, r), (1.0, 1.0));
    let (l, r) = hardwave_engine::pan::gains(-1.0, hardwave_engine::pan::PanLaw::Zero);
    assert_eq!((l, r), (1.0, 0.0));
    let (l, r) = hardwave_engine::pan::gains(0.5, hardwave_engine::pan::PanLaw::Zero);
    assert!((l - 0.5).abs() < 1e-6 && r == 1.0);
}
