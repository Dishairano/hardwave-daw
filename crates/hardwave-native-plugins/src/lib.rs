//! Hardwave Native Plugins — adapters for Hardwave's own plugins (Analyser, LoudLab, etc.)
//! These bypass the VST3/CLAP host and link directly as Rust crate dependencies.
//!
//! Phase 2: Will add direct integration with hardwave-loudlab, hardwave-analyser, etc.

pub fn native_plugin_ids() -> Vec<&'static str> {
    vec![
        "hardwave.analyser",
        "hardwave.loudlab",
        "hardwave.wettboi",
        "hardwave.kickforge",
    ]
}
