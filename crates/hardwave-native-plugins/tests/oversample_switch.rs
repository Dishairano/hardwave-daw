//! The plug-ins that bend the waveform share one oversampling switch: Off,
//! 2x and 4x, kept in the song, and old songs keep the sound they had.

use hardwave_native_plugins::exciter::NativeExciter;
use hardwave_native_plugins::saturator::NativeSaturator;
use hardwave_native_plugins::soundgoodizer::NativeSoundgoodizer;
use hardwave_native_plugins::tape::NativeTape;
use hardwave_plugin_host::types::HostedPlugin;

fn switch_of(p: &dyn HostedPlugin) -> u32 {
    (0..p.get_parameter_count())
        .find(|&i| {
            p.get_parameter_info(i)
                .is_some_and(|info| info.name == "Oversample")
        })
        .expect("an Oversample parameter")
}

fn check(mut fresh: Box<dyn HostedPlugin>, mut reloaded: Box<dyn HostedPlugin>) {
    let id = switch_of(fresh.as_ref());
    assert_eq!(
        fresh.parameter_options(id),
        Some(vec!["Off".to_string(), "2x".to_string(), "4x".to_string()])
    );
    // A new one starts at 2x.
    assert_eq!(fresh.get_parameter_value(id), 0.5);
    fresh.set_parameter_value(id, 1.0);
    assert_eq!(fresh.parameter_text(id, 1.0).as_deref(), Some("4x"));
    // Kept in the song.
    reloaded.set_state(&fresh.get_state()).unwrap();
    assert_eq!(reloaded.get_parameter_value(id), 1.0);
    // And it still makes sound.
    reloaded.activate(48_000.0, 256).unwrap();
    let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.07).sin() * 0.5).collect();
    let mut out = vec![Vec::new(), Vec::new()];
    reloaded.process(&[&input, &input], &mut out, &[], &mut Vec::new(), 256);
    assert!(out[0].iter().any(|v| v.abs() > 1e-4));
    assert!(out[0].iter().all(|v| v.is_finite()));
}

#[test]
fn tape_exciter_and_master_have_the_switch_and_keep_it() {
    check(Box::new(NativeTape::new()), Box::new(NativeTape::new()));
    check(
        Box::new(NativeExciter::new()),
        Box::new(NativeExciter::new()),
    );
    check(
        Box::new(NativeSoundgoodizer::new()),
        Box::new(NativeSoundgoodizer::new()),
    );
    check(
        Box::new(NativeSaturator::new()),
        Box::new(NativeSaturator::new()),
    );
}

#[test]
fn a_song_saved_before_the_switch_keeps_its_sound() {
    // Tape, exciter and master ran without oversampling before.
    let mut tape = NativeTape::new();
    let id = switch_of(&tape);
    tape.set_state(br#"{"wow":0.3,"drv":0.4,"hf":12000,"out":0}"#)
        .unwrap();
    assert_eq!(tape.get_parameter_value(id), 0.0, "Off, as it played");
    let mut master = NativeSoundgoodizer::new();
    let id = switch_of(&master);
    master.set_state(br#"{"amt":0.4,"out":0}"#).unwrap();
    assert_eq!(master.get_parameter_value(id), 0.0);
}
