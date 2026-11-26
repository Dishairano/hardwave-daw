//! The meter tick's payload: what `daw:trackMeters` carries for a track and
//! for the master strip. The tick itself is in `lib.rs`; the tracks' levels
//! come from `hardwave_engine::meter_reader`.

use hardwave_engine::meter_reader::TrackMeterReading;
use hardwave_metering::MeterSnapshot;
use hardwave_project::{Project, TrackKind};
use parking_lot::Mutex;
use std::sync::Arc;

/// One track's entry, as the mixer reads it.
pub fn track_entry(id: &str, r: &TrackMeterReading) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "peakL": r.peak_l,
        "peakR": r.peak_r,
        "rms": r.rms,
        "preFaderPeak": r.pre_fader_peak,
        "autoVolumeDb": r.auto_volume_db,
        "autoPan": r.auto_pan,
    })
}

/// How many ticks the master track's id is kept before it is looked up
/// again (half a second): it only changes when another song is opened.
const MASTER_ID_TICKS: u32 = 15;

/// The master strip's entry: the master track's id with the master meter's
/// levels, sent when they move.
#[derive(Default)]
pub struct MasterStrip {
    id: Option<String>,
    ticks: u32,
    sent: Option<(String, u32, u32)>,
}

impl MasterStrip {
    /// The master's entry when it changed since last sent (or always with
    /// `everything`). The project is only `try_lock`ed, every half second:
    /// the tick locked it every time and waited when a command held it.
    pub fn changed(
        &mut self,
        project: &Arc<Mutex<Project>>,
        meters: &MeterSnapshot,
        everything: bool,
    ) -> Option<serde_json::Value> {
        if self.id.is_none() || everything || self.ticks == 0 {
            if let Some(project) = project.try_lock() {
                self.id = project
                    .tracks
                    .iter()
                    .find(|t| matches!(t.kind, TrackKind::Master))
                    .map(|t| t.id.clone());
                self.ticks = MASTER_ID_TICKS;
            }
        }
        self.ticks = self.ticks.saturating_sub(1);
        let id = self.id.as_ref()?;
        let now = (meters.peak_db.to_bits(), meters.rms_db.to_bits());
        let unchanged = self
            .sent
            .as_ref()
            .is_some_and(|(was_id, peak, rms)| was_id == id && (*peak, *rms) == now);
        if unchanged && !everything {
            return None;
        }
        self.sent = Some((id.clone(), now.0, now.1));
        Some(serde_json::json!({
            "id": id,
            "peakL": meters.peak_db,
            "peakR": meters.peak_db,
            "rms": meters.rms_db,
            "preFaderPeak": meters.peak_db,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys the mixer reads (packages/daw-ui stores/meterStore.ts and
    /// its consumers): unchanged by reading the meters differently.
    #[test]
    fn a_track_entry_keeps_the_mixers_keys() {
        let entry = track_entry(
            "t1",
            &TrackMeterReading {
                peak_l: -6.0,
                peak_r: -7.0,
                rms: -12.0,
                pre_fader_peak: -3.0,
                auto_volume_db: Some(-1.5),
                auto_pan: None,
            },
        );
        assert_eq!(
            entry,
            serde_json::json!({
                "id": "t1",
                "peakL": -6.0,
                "peakR": -7.0,
                "rms": -12.0,
                "preFaderPeak": -3.0,
                "autoVolumeDb": -1.5,
                "autoPan": null,
            })
        );
    }

    fn snapshot(peak: f32, rms: f32) -> MeterSnapshot {
        MeterSnapshot {
            peak_db: peak,
            peak_hold_db: peak,
            true_peak_db: peak,
            rms_db: rms,
            lufs_m: None,
            lufs_s: None,
            lufs_i: None,
            clipped: false,
            correlation: 1.0,
        }
    }

    #[test]
    fn the_master_strip_is_sent_when_it_moves() {
        let project = Arc::new(Mutex::new(Project::default()));
        let master_id = project
            .lock()
            .tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .map(|t| t.id.clone())
            .expect("a new song has a master");
        let mut strip = MasterStrip::default();
        let first = strip
            .changed(&project, &snapshot(-6.0, -9.0), false)
            .unwrap();
        assert_eq!(
            first,
            serde_json::json!({
                "id": master_id,
                "peakL": -6.0,
                "peakR": -6.0,
                "rms": -9.0,
                "preFaderPeak": -6.0,
            })
        );
        assert!(strip
            .changed(&project, &snapshot(-6.0, -9.0), false)
            .is_none());
        assert!(strip
            .changed(&project, &snapshot(-5.0, -9.0), false)
            .is_some());
        assert!(strip
            .changed(&project, &snapshot(-5.0, -9.0), true)
            .is_some());
    }

    /// A command holding the song does not hold up the meters.
    #[test]
    fn a_busy_song_does_not_stop_the_master_strip() {
        let project = Arc::new(Mutex::new(Project::default()));
        let mut strip = MasterStrip::default();
        strip.changed(&project, &snapshot(-6.0, -9.0), false);
        let held = project.lock();
        for _ in 0..(MASTER_ID_TICKS * 2) {
            strip.changed(&project, &snapshot(-6.0, -9.0), false);
        }
        assert!(strip
            .changed(&project, &snapshot(-1.0, -2.0), false)
            .is_some());
        drop(held);
    }
}
