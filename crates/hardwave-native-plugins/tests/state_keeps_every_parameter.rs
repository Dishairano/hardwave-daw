//! Every setting of a built-in plug-in survives its saved state.
//!
//! The parameter sheet, a save and an undo all read a plug-in back through
//! its state, so a parameter the state leaves out reads back at its default
//! (and is lost on reload) even though the plug-in plays it.

use hardwave_native_plugins::{create_native, native_plugin_ids};

#[test]
fn every_parameter_reads_back_through_the_state() {
    let mut lost = Vec::new();
    for id in native_plugin_ids() {
        // Listed ids made elsewhere (not by `create_native`) are left out.
        let Some(mut plugin) = create_native(id) else {
            continue;
        };
        // The sampler's state is its sample: with none loaded it has no
        // state, and its settings travel in the song's copy instead.
        if id == "hardwave.native.sampler" {
            continue;
        }
        let infos: Vec<_> = (0..plugin.get_parameter_count())
            .filter_map(|i| plugin.get_parameter_info(i))
            .collect();
        // A value away from the default: a third of the way from the end
        // the default is furthest from.
        let mut set = Vec::new();
        for info in &infos {
            let span = info.max - info.min;
            let near_min = info.default_value - info.min < span / 2.0;
            let target = if near_min {
                info.max - span / 3.0
            } else {
                info.min + span / 3.0
            };
            plugin.set_parameter_value(info.id, target);
            set.push((info, plugin.get_parameter_value(info.id)));
        }
        let state = plugin.get_state();
        let mut copy = create_native(id).expect("a built-in");
        if let Err(e) = copy.set_state(&state) {
            lost.push(format!("{id}: state not taken back ({e})"));
            continue;
        }
        for (info, want) in set {
            let got = copy.get_parameter_value(info.id);
            let span = (info.max - info.min).abs().max(1e-9);
            if ((got - want) / span).abs() > 1e-3 {
                lost.push(format!(
                    "{id}: {} ({}) set {want}, read back {got}",
                    info.name, info.id
                ));
            }
        }
    }
    assert!(lost.is_empty(), "settings lost:\n{}", lost.join("\n"));
}
