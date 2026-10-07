#![no_main]
//! The sampler's saved state, as a project carries it.
use hardwave_plugin_host::types::HostedPlugin;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut sampler = hardwave_native_plugins::NativeSampler::new();
    let _ = sampler.set_state(data);
    let _ = sampler.get_state();
});
