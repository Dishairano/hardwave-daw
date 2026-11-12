//! Smart disable: the host stops calling a plug-in whose input and output
//! have gone silent and tells it, with `skip_silence`, how long it slept.
//! A plug-in with its own clock (an LFO, a carrier, a slice grid) moves it
//! on there, so a song sounds the same whether the plug-in slept or not.

use hardwave_native_plugins::*;
use hardwave_plugin_host::types::HostedPlugin;

const RATE: f64 = 48_000.0;
const BLOCK: usize = 512;

/// Sets the parameter called `name` to `value` (0..1).
fn set(p: &mut dyn HostedPlugin, name: &str, value: f64) {
    let id = (0..p.get_parameter_count())
        .find(|&i| {
            p.get_parameter_info(i)
                .is_some_and(|info| info.name == name)
        })
        .unwrap_or_else(|| panic!("{} has no {name}", p.descriptor().name));
    p.set_parameter_value(id, value);
}

/// Runs one stereo block and returns both channels.
fn run(p: &mut dyn HostedPlugin, left: &[f32], right: &[f32]) -> [Vec<f32>; 2] {
    let mut outputs = vec![Vec::new(), Vec::new()];
    p.process(
        &[left, right],
        &mut outputs,
        &[],
        &mut Vec::new(),
        left.len(),
    );
    let right_out = outputs.pop().unwrap();
    [outputs.pop().unwrap(), right_out]
}

/// A chord of three sines a little apart on the two sides, so a moving
/// pan, gain, delay or filter shows in every sample.
fn music(start: usize, len: usize) -> (Vec<f32>, Vec<f32>) {
    let at =
        |i: usize, hz: f32| (std::f32::consts::TAU * hz * (start + i) as f32 / RATE as f32).sin();
    let left = (0..len)
        .map(|i| 0.3 * (at(i, 220.0) + at(i, 331.0) + at(i, 1_250.0)))
        .collect();
    let right = (0..len)
        .map(|i| 0.3 * (at(i, 223.0) + at(i, 329.0) + at(i, 1_270.0)))
        .collect();
    (left, right)
}

/// The gap the plug-in sleeps through, in the blocks a host would skip:
/// several whole ones and a short one, a length no LFO divides evenly.
const GAP: [usize; 4] = [BLOCK, BLOCK, BLOCK, 301];

/// One instance is called with silence through the gap, the other sleeps
/// through it; afterwards both get the same music and must give the same
/// samples, bit for bit. A third that neither played nor slept through the
/// gap has to sound different, or the clock was never heard and the test
/// proves nothing.
fn sleeping_keeps_the_clock(name: &str, make: impl Fn() -> Box<dyn HostedPlugin>) {
    let started = || {
        let mut p = make();
        p.activate(RATE, BLOCK as u32).unwrap();
        // One silent block first, as a plug-in that played before it fell
        // asleep has (some set their filters up on the first call).
        let silence = vec![0.0_f32; BLOCK];
        run(p.as_mut(), &silence, &silence);
        p
    };
    let mut awake = started();
    let mut asleep = started();
    let mut stayed = started();
    for &n in &GAP {
        let silence = vec![0.0_f32; n];
        run(awake.as_mut(), &silence, &silence);
        asleep.skip_silence(n);
    }
    let mut differs = false;
    for block in 0..4 {
        let (left, right) = music(block * BLOCK, BLOCK);
        let a = run(awake.as_mut(), &left, &right);
        let b = run(asleep.as_mut(), &left, &right);
        let c = run(stayed.as_mut(), &left, &right);
        for ch in 0..2 {
            for (i, (x, y)) in a[ch].iter().zip(b[ch].iter()).enumerate() {
                assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "{name}: block {block} channel {ch} sample {i}: {x} awake, {y} after sleeping"
                );
            }
            differs |= a[ch] != c[ch];
        }
    }
    assert!(
        differs,
        "{name}: the gap made no difference, so the clock was not tested"
    );
}

#[test]
fn tremolo_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Tremolo", || Box::new(NativeTremolo::new()));
}

#[test]
fn auto_pan_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Auto-Pan", || Box::new(NativeAutoPan::new()));
}

#[test]
fn phaser_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Phaser", || {
        let mut p = NativePhaser::new();
        set(&mut p, "Rate", 0.5);
        Box::new(p)
    });
}

#[test]
fn ring_mod_keeps_its_carrier_through_sleep() {
    sleeping_keeps_the_clock("Ring Mod", || Box::new(NativeRingMod::new()));
}

#[test]
fn chorus_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Chorus", || Box::new(NativeChorus::new()));
}

#[test]
fn flanger_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Flanger", || {
        let mut p = NativeFlanger::new();
        set(&mut p, "Rate", 0.6);
        Box::new(p)
    });
}

#[test]
fn vibrato_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Vibrato", || Box::new(NativeVibrato::new()));
}

#[test]
fn doubler_keeps_its_lfo_through_sleep() {
    sleeping_keeps_the_clock("Doubler", || Box::new(NativeStereoDouble::new()));
}

#[test]
fn tape_keeps_its_wow_through_sleep() {
    sleeping_keeps_the_clock("Tape", || {
        let mut p = NativeTape::new();
        set(&mut p, "Wow", 1.0);
        Box::new(p)
    });
}

#[test]
fn vocoder_keeps_its_carrier_through_sleep() {
    for carrier in [0.0, 0.5, 1.0] {
        sleeping_keeps_the_clock("Vocoder", || {
            let mut p = NativeVocoder::new();
            set(&mut p, "Carrier", carrier);
            Box::new(p)
        });
    }
}

#[test]
fn stutter_keeps_its_slices_through_sleep() {
    sleeping_keeps_the_clock("Stutter", || {
        let mut p = NativeStutter::new();
        set(&mut p, "Mix", 1.0);
        // The shortest slice, so the gap moves it into another slice.
        set(&mut p, "Slice", 0.0);
        Box::new(p)
    });
}

#[test]
fn bitcrush_keeps_its_sample_and_hold_through_sleep() {
    sleeping_keeps_the_clock("Bitcrush", || {
        let mut p = NativeBitcrush::new();
        set(&mut p, "Rate", 0.3);
        Box::new(p)
    });
}

#[test]
fn auto_filter_keeps_its_retune_steps_through_sleep() {
    sleeping_keeps_the_clock("Auto Filter", || Box::new(NativeAutoFilter::new()));
}
