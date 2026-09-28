//! Macros: one knob that moves several parameters at once.
//!
//! A hard dance patch is rarely one control. "Open the filter" means the
//! cutoff on two synths, the send to the reverb and a touch of drive, all
//! moving together. Without macros that is three or four automation lanes
//! drawn by hand and kept in step by memory.
//!
//! A macro is a value from 0 to 1 with a list of links. Each link names one
//! target and the two parameter values the macro travels between, so the
//! same knob can open one filter and close another by giving the second
//! link a range that runs backwards.

use serde::{Deserialize, Serialize};

/// What one link moves.
///
/// The plug-in case names the slot and the parameter inside it, the same
/// pair automation uses, so a macro and an automation lane can point at the
/// same knob and mean the same thing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MacroTarget {
    TrackVolume,
    TrackPan,
    PluginParam { slot_id: String, param_id: u32 },
}

/// One thing a macro moves, and how far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroLink {
    pub id: String,
    pub track_id: String,
    pub target: MacroTarget,
    /// The target's value when the macro sits at 0.
    pub min: f64,
    /// The target's value when the macro sits at 1.
    pub max: f64,
}

impl MacroLink {
    /// Where this link stands when the macro is at `value`.
    ///
    /// `min` above `max` is not a mistake: it is how a link is made to run
    /// backwards, which is what closing one filter while opening another
    /// needs.
    pub fn value_at(&self, macro_value: f64) -> f64 {
        let t = macro_value.clamp(0.0, 1.0);
        self.min + (self.max - self.min) * t
    }
}

/// A named knob and everything it moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    pub id: String,
    pub name: String,
    /// 0 to 1. Saved with the song, so a patch opens where it was left.
    pub value: f64,
    pub links: Vec<MacroLink>,
}

impl Macro {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            value: 0.0,
            links: Vec::new(),
        }
    }

    /// Every link and where it stands at the macro's current value.
    pub fn resolved(&self) -> Vec<(&MacroLink, f64)> {
        self.links
            .iter()
            .map(|link| (link, link.value_at(self.value)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(min: f64, max: f64) -> MacroLink {
        MacroLink {
            id: "l1".into(),
            track_id: "t1".into(),
            target: MacroTarget::PluginParam {
                slot_id: "s1".into(),
                param_id: 3,
            },
            min,
            max,
        }
    }

    #[test]
    fn a_link_travels_between_the_two_values_it_was_given() {
        let l = link(200.0, 8000.0);
        assert_eq!(l.value_at(0.0), 200.0);
        assert_eq!(l.value_at(1.0), 8000.0);
        assert_eq!(l.value_at(0.5), 4100.0);
    }

    #[test]
    fn a_link_with_its_range_the_other_way_round_runs_backwards() {
        let l = link(1.0, 0.0);
        assert_eq!(l.value_at(0.0), 1.0);
        assert_eq!(l.value_at(1.0), 0.0);
    }

    #[test]
    fn a_macro_value_outside_the_knob_cannot_push_a_link_past_its_range() {
        let l = link(0.0, 1.0);
        assert_eq!(l.value_at(-4.0), 0.0);
        assert_eq!(l.value_at(9.0), 1.0);
    }

    #[test]
    fn one_macro_moves_every_link_it_owns() {
        let mut m = Macro::new("m1".into(), "Open".into());
        m.links.push(link(0.0, 1.0));
        m.links.push(link(1.0, 0.0));
        m.value = 0.25;
        let resolved: Vec<f64> = m.resolved().into_iter().map(|(_, v)| v).collect();
        assert_eq!(resolved, vec![0.25, 0.75]);
    }
}
