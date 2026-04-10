use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AutomationTarget {
    TrackVolume,
    TrackPan,
    TrackMute,
    PluginParam { slot_id: String, param_id: u32 },
    SendLevel { send_index: usize },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CurveMode {
    Linear,
    Bezier,
    Step,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub tick: u64,
    /// Normalized value 0.0..1.0
    pub value: f64,
    pub curve: CurveMode,
    /// Bezier tension (-1.0..1.0), only used when curve is Bezier.
    pub tension: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    pub id: String,
    pub target: AutomationTarget,
    pub points: Vec<AutomationPoint>,
    pub visible: bool,
}

impl AutomationLane {
    /// Get the interpolated value at a given tick.
    pub fn value_at(&self, tick: u64) -> f64 {
        if self.points.is_empty() {
            return 0.5;
        }
        if tick <= self.points[0].tick {
            return self.points[0].value;
        }
        let last = self.points.last().unwrap();
        if tick >= last.tick {
            return last.value;
        }

        // Find surrounding points
        let idx = self.points.partition_point(|p| p.tick <= tick);
        if idx == 0 {
            return self.points[0].value;
        }

        let a = &self.points[idx - 1];
        let b = &self.points[idx];

        match a.curve {
            CurveMode::Step => a.value,
            CurveMode::Linear => {
                let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
                a.value + (b.value - a.value) * t
            }
            CurveMode::Bezier => {
                let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
                // Simple power curve based on tension
                let curved_t = if a.tension >= 0.0 {
                    t.powf(1.0 + a.tension * 3.0)
                } else {
                    1.0 - (1.0 - t).powf(1.0 + (-a.tension) * 3.0)
                };
                a.value + (b.value - a.value) * curved_t
            }
        }
    }
}
