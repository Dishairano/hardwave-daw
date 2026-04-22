//! Plugin-host operational primitives — bypass / latency aggregator
//! / sandbox state / preset browser. Real implementations the UI
//! and audio engine consume regardless of plugin format.

use serde::{Deserialize, Serialize};

/// Plugin bypass state — a per-plugin flag the audio engine checks
/// before calling `process()`. When bypassed, the engine passes
/// input → output straight through and skips the plugin entirely.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PluginBypassState {
    pub bypassed: bool,
}

impl PluginBypassState {
    pub fn toggle(&mut self) {
        self.bypassed = !self.bypassed;
    }

    pub fn set(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    /// Decide whether to call the plugin's `process()`. Returns
    /// `false` if bypassed — audio engine writes input directly to
    /// output on false.
    pub fn should_process(&self) -> bool {
        !self.bypassed
    }
}

/// Per-plugin latency report. The audio engine sums these across
/// the chain for PDC (plugin-delay compensation) so post-FX tracks
/// stay aligned.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginLatencyReport {
    pub entries: Vec<(String, u32)>, // (plugin id, latency samples)
}

impl PluginLatencyReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, plugin_id: impl Into<String>, latency_samples: u32) {
        let id = plugin_id.into();
        if let Some(entry) = self.entries.iter_mut().find(|(i, _)| *i == id) {
            entry.1 = latency_samples;
        } else {
            self.entries.push((id, latency_samples));
        }
    }

    pub fn remove(&mut self, plugin_id: &str) {
        self.entries.retain(|(id, _)| id != plugin_id);
    }

    pub fn chain_latency_samples(&self) -> u32 {
        self.entries.iter().map(|(_, l)| *l).sum()
    }

    pub fn per_plugin(&self) -> &[(String, u32)] {
        &self.entries
    }
}

/// Sandbox state — tracks whether a plugin is running, crashed, or
/// being reloaded. Drives the sandboxing, crash-isolation, and
/// crash-recovery features together so the UI has a single state
/// source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SandboxState {
    #[default]
    Running,
    Crashed,
    Reloading,
    Disabled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginSandbox {
    pub plugin_id: String,
    pub state: SandboxState,
    pub crash_count: u32,
    pub last_error: Option<String>,
}

impl PluginSandbox {
    pub fn new(plugin_id: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            state: SandboxState::Running,
            crash_count: 0,
            last_error: None,
        }
    }

    /// Crash notification from the sandbox process — the host sets
    /// the state to `Crashed` and records the error.
    pub fn report_crash(&mut self, error: impl Into<String>) {
        self.state = SandboxState::Crashed;
        self.crash_count += 1;
        self.last_error = Some(error.into());
    }

    /// Begin a reload attempt. Transitions `Crashed → Reloading`.
    pub fn begin_reload(&mut self) {
        if matches!(self.state, SandboxState::Crashed) {
            self.state = SandboxState::Reloading;
        }
    }

    pub fn finish_reload(&mut self, success: bool) {
        self.state = if success {
            SandboxState::Running
        } else {
            SandboxState::Disabled
        };
    }

    /// The plugin's output should be silenced whenever it's not in
    /// a healthy `Running` state.
    pub fn should_silence_output(&self) -> bool {
        !matches!(self.state, SandboxState::Running)
    }
}

/// User preset — concrete state chunk + metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreset {
    pub id: String,
    pub name: String,
    pub plugin_id: String,
    pub category: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub state_blob: Vec<u8>,
}

/// Factory preset listing (as reported by the plugin). Read-only
/// — the user can only favorite these, not edit them in place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryPreset {
    pub index: u32,
    pub name: String,
    pub category: String,
}

/// Preset browser — per-plugin preset catalog with search, category
/// filter, favorites, and two A/B comparison slots.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresetBrowser {
    pub plugin_id: String,
    pub user_presets: Vec<UserPreset>,
    pub factory_presets: Vec<FactoryPreset>,
    pub ab_slot_a: Option<String>,
    pub ab_slot_b: Option<String>,
}

impl PresetBrowser {
    pub fn new(plugin_id: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            ..Default::default()
        }
    }

    pub fn add_user_preset(&mut self, preset: UserPreset) -> bool {
        if self.user_presets.iter().any(|p| p.id == preset.id) {
            return false;
        }
        self.user_presets.push(preset);
        true
    }

    pub fn toggle_favorite(&mut self, preset_id: &str) -> bool {
        if let Some(p) = self.user_presets.iter_mut().find(|p| p.id == preset_id) {
            p.favorite = !p.favorite;
            return true;
        }
        false
    }

    pub fn favorites(&self) -> Vec<&UserPreset> {
        self.user_presets.iter().filter(|p| p.favorite).collect()
    }

    pub fn by_category(&self, category: &str) -> Vec<&UserPreset> {
        self.user_presets
            .iter()
            .filter(|p| p.category == category)
            .collect()
    }

    pub fn search(&self, query: &str) -> Vec<&UserPreset> {
        let q = query.to_lowercase();
        self.user_presets
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Load a user preset into A/B slot A.
    pub fn load_into_a(&mut self, preset_id: &str) -> bool {
        if self.user_presets.iter().any(|p| p.id == preset_id) {
            self.ab_slot_a = Some(preset_id.to_string());
            return true;
        }
        false
    }

    pub fn load_into_b(&mut self, preset_id: &str) -> bool {
        if self.user_presets.iter().any(|p| p.id == preset_id) {
            self.ab_slot_b = Some(preset_id.to_string());
            return true;
        }
        false
    }

    /// Swap A/B slot contents — the button on the preset browser.
    pub fn swap_ab(&mut self) {
        std::mem::swap(&mut self.ab_slot_a, &mut self.ab_slot_b);
    }

    pub fn active_ab(&self, is_a_active: bool) -> Option<&UserPreset> {
        let id = if is_a_active {
            self.ab_slot_a.as_ref()
        } else {
            self.ab_slot_b.as_ref()
        }?;
        self.user_presets.iter().find(|p| &p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(id: &str, name: &str, category: &str, tags: &[&str]) -> UserPreset {
        UserPreset {
            id: id.into(),
            name: name.into(),
            plugin_id: "plugin".into(),
            category: category.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            favorite: false,
            state_blob: Vec::new(),
        }
    }

    #[test]
    fn bypass_toggle_gates_processing() {
        let mut b = PluginBypassState::default();
        assert!(b.should_process());
        b.toggle();
        assert!(!b.should_process());
        b.set(false);
        assert!(b.should_process());
    }

    #[test]
    fn latency_report_sums_chain_correctly() {
        let mut r = PluginLatencyReport::new();
        r.record("a", 32);
        r.record("b", 64);
        r.record("c", 16);
        assert_eq!(r.chain_latency_samples(), 112);
        // Update existing entry.
        r.record("b", 128);
        assert_eq!(r.chain_latency_samples(), 176);
        r.remove("a");
        assert_eq!(r.chain_latency_samples(), 144);
    }

    #[test]
    fn sandbox_state_cycles_through_crash_and_recover() {
        let mut s = PluginSandbox::new("p1");
        assert_eq!(s.state, SandboxState::Running);
        assert!(!s.should_silence_output());
        s.report_crash("plugin panicked");
        assert_eq!(s.state, SandboxState::Crashed);
        assert_eq!(s.crash_count, 1);
        assert!(s.should_silence_output());
        s.begin_reload();
        assert_eq!(s.state, SandboxState::Reloading);
        s.finish_reload(true);
        assert_eq!(s.state, SandboxState::Running);
        s.report_crash("again");
        s.begin_reload();
        s.finish_reload(false);
        assert_eq!(s.state, SandboxState::Disabled);
    }

    #[test]
    fn preset_browser_add_and_search() {
        let mut b = PresetBrowser::new("plugin-1");
        assert!(b.add_user_preset(preset("p1", "Warm Pad", "Pads", &["warm", "analog"])));
        assert!(!b.add_user_preset(preset("p1", "Duplicate", "Any", &[])));
        assert_eq!(b.user_presets.len(), 1);
        assert_eq!(b.search("warm").len(), 1);
        assert_eq!(b.search("ANALOG").len(), 1);
        assert_eq!(b.search("unrelated").len(), 0);
    }

    #[test]
    fn preset_browser_favorites_and_categories() {
        let mut b = PresetBrowser::new("plugin-1");
        b.add_user_preset(preset("p1", "Warm Pad", "Pads", &[]));
        b.add_user_preset(preset("p2", "Shrill Lead", "Leads", &[]));
        b.toggle_favorite("p1");
        assert_eq!(b.favorites().len(), 1);
        assert_eq!(b.by_category("Leads").len(), 1);
        b.toggle_favorite("p1");
        assert_eq!(b.favorites().len(), 0);
    }

    #[test]
    fn preset_browser_ab_slots_load_and_swap() {
        let mut b = PresetBrowser::new("plugin-1");
        b.add_user_preset(preset("p1", "A", "C", &[]));
        b.add_user_preset(preset("p2", "B", "C", &[]));
        assert!(b.load_into_a("p1"));
        assert!(b.load_into_b("p2"));
        assert_eq!(b.active_ab(true).map(|p| p.id.as_str()), Some("p1"));
        b.swap_ab();
        assert_eq!(b.active_ab(true).map(|p| p.id.as_str()), Some("p2"));
        assert_eq!(b.active_ab(false).map(|p| p.id.as_str()), Some("p1"));
    }

    #[test]
    fn loading_nonexistent_preset_into_slot_fails() {
        let mut b = PresetBrowser::new("plugin-1");
        assert!(!b.load_into_a("missing"));
        assert!(b.ab_slot_a.is_none());
    }
}
