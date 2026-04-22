//! Sidechain routing — declarative graph of "any mixer track ↔ any
//! plugin sidechain input" and a helper that builds a typical
//! sidechain-compressor ducking setup in one call.

use serde::{Deserialize, Serialize};

/// One routing edge — which mixer track audio feeds which plugin
/// sidechain input. Stored by id so swapping plugins / renaming
/// tracks doesn't break the reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SidechainRoute {
    pub source_track_id: String,
    pub target_plugin_id: String,
    pub target_input_index: u8,
    pub send_level_db: i16,
}

/// The full routing table. Every `connect()` call adds an edge; the
/// audio engine reads `routes_to(plugin_id)` at activate time to
/// build the actual graph wiring.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SidechainRoutingTable {
    routes: Vec<SidechainRoute>,
}

impl SidechainRoutingTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Connect a mixer track to a plugin sidechain input. Duplicate
    /// edges (same source + target + index) are overwritten to update
    /// the send level; returns `true` if a new edge was created, or
    /// `false` if an existing one was updated.
    pub fn connect(
        &mut self,
        source_track_id: impl Into<String>,
        target_plugin_id: impl Into<String>,
        target_input_index: u8,
        send_level_db: i16,
    ) -> bool {
        let source = source_track_id.into();
        let target = target_plugin_id.into();
        if let Some(existing) = self.routes.iter_mut().find(|r| {
            r.source_track_id == source
                && r.target_plugin_id == target
                && r.target_input_index == target_input_index
        }) {
            existing.send_level_db = send_level_db;
            return false;
        }
        self.routes.push(SidechainRoute {
            source_track_id: source,
            target_plugin_id: target,
            target_input_index,
            send_level_db,
        });
        true
    }

    /// Remove an edge. Returns `true` if something was removed.
    pub fn disconnect(
        &mut self,
        source_track_id: &str,
        target_plugin_id: &str,
        target_input_index: u8,
    ) -> bool {
        let before = self.routes.len();
        self.routes.retain(|r| {
            !(r.source_track_id == source_track_id
                && r.target_plugin_id == target_plugin_id
                && r.target_input_index == target_input_index)
        });
        self.routes.len() != before
    }

    /// Remove all edges feeding `plugin_id` — called when the plugin
    /// is deleted.
    pub fn clear_target(&mut self, plugin_id: &str) {
        self.routes.retain(|r| r.target_plugin_id != plugin_id);
    }

    /// Remove all edges fed by `track_id` — called when the track is
    /// deleted.
    pub fn clear_source(&mut self, track_id: &str) {
        self.routes.retain(|r| r.source_track_id != track_id);
    }

    pub fn routes_to<'a>(&'a self, plugin_id: &'a str) -> impl Iterator<Item = &'a SidechainRoute> {
        self.routes
            .iter()
            .filter(move |r| r.target_plugin_id == plugin_id)
    }

    pub fn routes_from<'a>(
        &'a self,
        track_id: &'a str,
    ) -> impl Iterator<Item = &'a SidechainRoute> {
        self.routes
            .iter()
            .filter(move |r| r.source_track_id == track_id)
    }

    pub fn all(&self) -> &[SidechainRoute] {
        &self.routes
    }
}

/// Describes a common use case — kick-ducks-bass sidechain. Applied
/// by `apply_compressor_ducking_preset` which adds the routing edge
/// and returns the recommended compressor parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckingPreset {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub sidechain_hpf_hz: f32,
}

impl DuckingPreset {
    /// Canonical "kick ducks bass" sidechain compressor preset —
    /// -18 dB threshold, 4:1 ratio, 1 ms attack, 120 ms release,
    /// 4 dB knee, sidechain HPF at 50 Hz.
    pub fn kick_ducks_bass() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 1.0,
            release_ms: 120.0,
            knee_db: 4.0,
            sidechain_hpf_hz: 50.0,
        }
    }

    /// "Pump the pad with the kick" variant — softer ratio + longer
    /// release for a pumping EDM chord feel.
    pub fn pad_pump() -> Self {
        Self {
            threshold_db: -24.0,
            ratio: 3.0,
            attack_ms: 8.0,
            release_ms: 280.0,
            knee_db: 6.0,
            sidechain_hpf_hz: 120.0,
        }
    }
}

/// Wire up a sidechain-compressor ducking setup in one shot. Adds
/// the `kick → bass_compressor` route at send_level_db 0, returns
/// the compressor preset parameters the caller applies to the
/// targeted compressor plugin.
pub fn apply_compressor_ducking_preset(
    table: &mut SidechainRoutingTable,
    kick_track_id: impl Into<String>,
    bass_compressor_plugin_id: impl Into<String>,
    preset: DuckingPreset,
) -> DuckingPreset {
    table.connect(kick_track_id, bass_compressor_plugin_id, 0, 0);
    preset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_new_edge_returns_true() {
        let mut t = SidechainRoutingTable::new();
        assert!(t.connect("kick", "comp-1", 0, -6));
        assert_eq!(t.all().len(), 1);
    }

    #[test]
    fn connect_duplicate_edge_updates_level() {
        let mut t = SidechainRoutingTable::new();
        t.connect("kick", "comp-1", 0, -6);
        assert!(!t.connect("kick", "comp-1", 0, 0));
        assert_eq!(t.all()[0].send_level_db, 0);
    }

    #[test]
    fn disconnect_removes_specific_edge() {
        let mut t = SidechainRoutingTable::new();
        t.connect("kick", "comp-1", 0, -6);
        t.connect("kick", "comp-2", 0, -6);
        assert!(t.disconnect("kick", "comp-1", 0));
        assert_eq!(t.all().len(), 1);
        assert_eq!(t.all()[0].target_plugin_id, "comp-2");
    }

    #[test]
    fn clear_target_removes_all_edges_feeding_plugin() {
        let mut t = SidechainRoutingTable::new();
        t.connect("kick", "comp", 0, 0);
        t.connect("bass", "comp", 1, 0);
        t.connect("other", "otherplugin", 0, 0);
        t.clear_target("comp");
        assert_eq!(t.all().len(), 1);
    }

    #[test]
    fn routes_to_iterates_only_matching() {
        let mut t = SidechainRoutingTable::new();
        t.connect("kick", "comp", 0, -3);
        t.connect("bass", "comp", 1, -3);
        t.connect("kick", "otherplugin", 0, 0);
        let hits: Vec<&SidechainRoute> = t.routes_to("comp").collect();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn ducking_preset_values_are_sensible() {
        let kd = DuckingPreset::kick_ducks_bass();
        assert!(kd.threshold_db < 0.0);
        assert!(kd.ratio > 1.0);
        assert!(kd.attack_ms < 10.0);
        let pp = DuckingPreset::pad_pump();
        assert!(pp.release_ms > kd.release_ms);
    }

    #[test]
    fn apply_preset_adds_routing_and_returns_params() {
        let mut t = SidechainRoutingTable::new();
        let p = apply_compressor_ducking_preset(
            &mut t,
            "kick",
            "bass-comp",
            DuckingPreset::kick_ducks_bass(),
        );
        assert!((p.ratio - 4.0).abs() < 1e-3);
        assert_eq!(t.all().len(), 1);
        assert_eq!(t.all()[0].source_track_id, "kick");
        assert_eq!(t.all()[0].target_plugin_id, "bass-comp");
    }
}
