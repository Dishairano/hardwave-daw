//! The tracks' meters as the window's meter tick reads them.
//!
//! The tick ran 30 times a second and took the engine's meter map twice a
//! tick (once for the levels, once for what automation does to each
//! fader), copying every track's id while it held it. The audio thread
//! wants that same map (it only `try_lock`s it, and skips a graph swap, the
//! meters' reset on stop or an envelope route when it is busy), and the
//! tick then wrote every track out as JSON just to see whether it changed.
//!
//! This keeps its own list of the tracks' meters, taken from the map only
//! when the map's tracks changed and never waiting for it, reads the levels
//! from the meters' atomics, and hands over only the tracks whose reading
//! changed since it was last handed over.

use crate::engine::TrackMeterMap;
use crate::track_node::TrackMeterState;
use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

/// One track's meter as the mixer shows it.
#[derive(Debug, Clone, Copy)]
pub struct TrackMeterReading {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms: f32,
    pub pre_fader_peak: f32,
    /// Where automation has the fader, in dB, when it is automated.
    pub auto_volume_db: Option<f32>,
    /// Where automation has the pan, when it is automated.
    pub auto_pan: Option<f32>,
}

impl TrackMeterReading {
    fn of(meter: &TrackMeterState) -> Self {
        let (volume, pan) = meter.automation();
        Self {
            peak_l: meter.peak_db_l.load(Relaxed),
            peak_r: meter.peak_db_r.load(Relaxed),
            rms: meter.rms_db.load(Relaxed),
            pre_fader_peak: meter.pre_fader_peak_db.load(Relaxed),
            auto_volume_db: volume.map(|lin| {
                if lin > 1e-6 {
                    20.0 * lin.log10()
                } else {
                    -120.0
                }
            }),
            auto_pan: pan,
        }
    }

    /// The same reading, bit for bit (so a NaN equals itself).
    fn same_as(&self, other: &Self) -> bool {
        let bits = |v: Option<f32>| v.map(f32::to_bits);
        self.peak_l.to_bits() == other.peak_l.to_bits()
            && self.peak_r.to_bits() == other.peak_r.to_bits()
            && self.rms.to_bits() == other.rms.to_bits()
            && self.pre_fader_peak.to_bits() == other.pre_fader_peak.to_bits()
            && bits(self.auto_volume_db) == bits(other.auto_volume_db)
            && bits(self.auto_pan) == bits(other.auto_pan)
    }
}

/// See the module documentation.
pub struct TrackMeterReader {
    map: TrackMeterMap,
    /// The map's tracks, in the map's order, as last taken.
    tracks: Vec<(String, Arc<TrackMeterState>)>,
    /// What was last handed over per track.
    sent: HashMap<String, TrackMeterReading>,
}

impl TrackMeterReader {
    pub fn new(map: TrackMeterMap) -> Self {
        Self {
            map,
            tracks: Vec::new(),
            sent: HashMap::new(),
        }
    }

    /// Follow the map when its tracks changed. Never waits: when the audio
    /// thread or a graph build holds the map, the list stays as it was
    /// until the next tick. Checking costs one walk over the map in its own
    /// order with no hashing and no copying.
    fn follow_map(&mut self) {
        let Some(map) = self.map.try_lock() else {
            return;
        };
        let same = map.len() == self.tracks.len()
            && map
                .iter()
                .zip(&self.tracks)
                .all(|((id, meter), (had_id, had))| Arc::ptr_eq(meter, had) && id == had_id);
        if same {
            return;
        }
        self.tracks.clear();
        self.tracks.extend(
            map.iter()
                .map(|(id, meter)| (id.clone(), Arc::clone(meter))),
        );
        drop(map);
        let tracks = &self.tracks;
        self.sent
            .retain(|id, _| tracks.iter().any(|(track, _)| track == id));
    }

    /// Hand `changed(id, reading)` every track whose reading differs from
    /// the one handed over last time, or every track with `everything`
    /// (so a window that started listening late catches up).
    pub fn changed(&mut self, everything: bool, mut changed: impl FnMut(&str, &TrackMeterReading)) {
        self.follow_map();
        for (id, meter) in &self.tracks {
            let now = TrackMeterReading::of(meter);
            let unchanged = self.sent.get(id).is_some_and(|was| was.same_as(&now));
            if everything || !unchanged {
                changed(id, &now);
                match self.sent.get_mut(id) {
                    Some(was) => *was = now,
                    None => {
                        self.sent.insert(id.clone(), now);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    fn map_with(ids: &[&str]) -> (TrackMeterMap, Vec<Arc<TrackMeterState>>) {
        let meters: Vec<_> = ids
            .iter()
            .map(|_| Arc::new(TrackMeterState::default()))
            .collect();
        let map: TrackMeterMap = Arc::new(Mutex::new(
            ids.iter()
                .zip(&meters)
                .map(|(id, m)| (id.to_string(), Arc::clone(m)))
                .collect(),
        ));
        (map, meters)
    }

    fn changed_ids(reader: &mut TrackMeterReader, everything: bool) -> Vec<String> {
        let mut ids = Vec::new();
        reader.changed(everything, |id, _| ids.push(id.to_string()));
        ids.sort();
        ids
    }

    #[test]
    fn only_tracks_whose_meter_moved_are_handed_over() {
        let (map, meters) = map_with(&["a", "b", "c"]);
        let mut reader = TrackMeterReader::new(map);
        assert_eq!(
            changed_ids(&mut reader, false),
            ["a", "b", "c"],
            "first look"
        );
        assert!(changed_ids(&mut reader, false).is_empty(), "nothing moved");

        meters[1].peak_db_l.store(-6.0, Relaxed);
        assert_eq!(changed_ids(&mut reader, false), ["b"]);
        assert!(changed_ids(&mut reader, false).is_empty());

        // Automation moving a fader counts as a change, in dB.
        meters[2].auto_volume.store(0.5, Relaxed);
        meters[2].auto_on.store(1, Relaxed);
        let mut got = Vec::new();
        reader.changed(false, |id, r| got.push((id.to_string(), r.auto_volume_db)));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "c");
        assert!((got[0].1.unwrap() + 6.0206).abs() < 1e-3);

        assert_eq!(
            changed_ids(&mut reader, true),
            ["a", "b", "c"],
            "everything again"
        );
    }

    #[test]
    fn a_track_added_or_rebuilt_is_followed() {
        let (map, _meters) = map_with(&["a"]);
        let mut reader = TrackMeterReader::new(Arc::clone(&map));
        changed_ids(&mut reader, false);

        let fresh = Arc::new(TrackMeterState::default());
        fresh.rms_db.store(-12.0, Relaxed);
        map.lock().insert("b".into(), Arc::clone(&fresh));
        assert_eq!(changed_ids(&mut reader, false), ["b"]);

        // A graph build gives "a" a new meter: the reader reads the new one.
        let rebuilt = Arc::new(TrackMeterState::default());
        rebuilt.peak_db_r.store(-3.0, Relaxed);
        map.lock().insert("a".into(), Arc::clone(&rebuilt));
        let mut got = Vec::new();
        reader.changed(false, |id, r| got.push((id.to_string(), r.peak_r)));
        assert_eq!(got, vec![("a".to_string(), -3.0)]);

        map.lock().remove("b");
        fresh.rms_db.store(-1.0, Relaxed);
        assert!(
            changed_ids(&mut reader, false).is_empty(),
            "a removed track is let go"
        );
    }

    #[test]
    fn a_busy_map_is_not_waited_for() {
        let (map, meters) = map_with(&["a"]);
        let mut reader = TrackMeterReader::new(Arc::clone(&map));
        changed_ids(&mut reader, false);
        let held = map.lock();
        meters[0].peak_db_l.store(-1.0, Relaxed);
        // Read from the list it already has, without the map.
        assert_eq!(changed_ids(&mut reader, false), ["a"]);
        drop(held);
    }
}
