//! Print every built-in plug-in's parameters as JSON, for the UI layouts
//! (packages/daw-ui/src/components/plugins/) and the manual.
//!
//!   cargo run -p hardwave-native-plugins --example dump_params > params.json

use hardwave_native_plugins::*;
use hardwave_plugin_host::types::HostedPlugin;

fn main() {
    let plugins: Vec<Box<dyn HostedPlugin>> = vec![
        Box::new(NativeEq::new()),
        Box::new(NativeCompressor::new()),
        Box::new(NativeLimiter::new()),
        Box::new(NativeDistortion::new()),
        Box::new(NativeFilter::new()),
        Box::new(NativeDelay::new()),
        Box::new(NativeReverb::new()),
        Box::new(NativeStereo::new()),
        Box::new(NativeMultiband::new()),
        Box::new(NativeTripleOsc::new()),
        Box::new(NativeFmSynth::new()),
        Box::new(NativeWavetable::new()),
        Box::new(NativeChorus::new()),
        Box::new(NativePhaser::new()),
        Box::new(NativeConvReverb::new()),
        Box::new(NativeTremolo::new()),
        Box::new(NativeFlanger::new()),
        Box::new(NativeAutoPan::new()),
        Box::new(NativeBitcrush::new()),
        Box::new(NativeGain::new()),
        Box::new(NativeSaturator::new()),
        Box::new(NativeNoise::new()),
        Box::new(NativeSubBass::new()),
        Box::new(NativeVibrato::new()),
        Box::new(NativeMidSide::new()),
        Box::new(NativeGate::new()),
        Box::new(NativeTransient::new()),
        Box::new(NativeClipper::new()),
        Box::new(NativeExciter::new()),
        Box::new(NativeTape::new()),
        Box::new(NativeSoundgoodizer::new()),
        Box::new(NativeMonoFold::new()),
        Box::new(NativeRingMod::new()),
        Box::new(NativeAutoFilter::new()),
        Box::new(NativeStereoDouble::new()),
        Box::new(NativeVocoder::new()),
        Box::new(NativeStutter::new()),
        Box::new(NativeSampler::new()),
    ];
    let out: Vec<serde_json::Value> = plugins
        .iter()
        .map(|p| {
            let d = p.descriptor();
            let params: Vec<serde_json::Value> = (0..p.get_parameter_count())
                .filter_map(|i| p.get_parameter_info(i))
                .map(|info| {
                    // The value's text at 101 points across the range, so a
                    // mockup can show what the plug-in itself would say.
                    let texts: Vec<Option<String>> = (0..=100)
                        .map(|k| {
                            let v = info.min + (info.max - info.min) * k as f64 / 100.0;
                            p.parameter_text(info.id, v)
                        })
                        .collect();
                    let mut value = serde_json::to_value(&info).unwrap_or_default();
                    value["options"] = serde_json::json!(p.parameter_options(info.id));
                    value["texts"] = serde_json::json!(texts);
                    value
                })
                .collect();
            serde_json::json!({ "id": d.id, "name": d.name, "category": format!("{:?}", d.category), "params": params })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}
