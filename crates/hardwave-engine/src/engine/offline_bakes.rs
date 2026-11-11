//! The bakes a render needs before it plays like playback: takes with
//! notes tuned by hand, stretched clips and warped clips. They are listed
//! from the engine in a moment and made without it.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// The bakes a render needs that the pool does not hold yet, listed by
/// [`DawEngine::offline_bakes`] and made by [`Self::run`], which needs
/// nothing from the engine. An export used to make them with the engine
/// locked: the first export after an FL import baked every stretched clip
/// of the song first, and the window could not even save for a minute and
/// a half.
pub struct OfflineBakes {
    pool: AudioPool,
    tunes: Vec<TuneBake>,
    stretches: Vec<(String, f64, f64)>,
    warps: Vec<WarpBake>,
    sample_rate: f64,
}

impl DawEngine {
    /// The bakes a render at `sample_rate` needs: of the clips that sound
    /// between `span`'s two sample positions when there is one, else of
    /// the whole song. Quick: it only reads the song and the pool.
    pub fn offline_bakes(&self, sample_rate: u32, span: Option<(u64, u64)>) -> OfflineBakes {
        let rate = sample_rate as f64;
        let span = span.map(|(from, to)| BakeSpan {
            from,
            to,
            sample_rate: rate,
        });
        let pool = &self.audio_pool;
        OfflineBakes {
            tunes: self
                .tune_bake_plan(span)
                .into_iter()
                .filter(|b| !pool.contains(&b.key))
                .collect(),
            stretches: self
                .stretch_bake_plan(span)
                .into_iter()
                .filter(|(s, st, se)| !pool.contains(&stretch_cache_key(s, *st, *se)))
                .collect(),
            warps: self
                .warp_bake_plan(rate, span)
                .into_iter()
                .filter(|b| !pool.contains(&b.key))
                .collect(),
            pool: pool.clone(),
            sample_rate: rate,
        }
    }
}

impl OfflineBakes {
    /// How many bakes there are to make.
    pub fn len(&self) -> usize {
        self.tunes.len() + self.stretches.len() + self.warps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Make the bakes, one after another, telling `progress(done, total)`
    /// after each. A bake that playback's own bake has made in the meantime
    /// is skipped.
    /// Stops between bakes once `cancel` is set and says false; what was
    /// made by then stays in the pool for the next try.
    pub fn run(&self, cancel: &AtomicBool, mut progress: impl FnMut(usize, usize)) -> bool {
        let total = self.len();
        let mut done = 0;
        progress(done, total);
        // Tuned takes first: a stretch or a warp is made from its clip's
        // tuned take.
        for bake in &self.tunes {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            DawEngine::bake_tune_variant(&self.pool, bake);
            done += 1;
            progress(done, total);
        }
        for (source_id, stretch, semitones) in &self.stretches {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            DawEngine::bake_stretch_variant(&self.pool, source_id, *stretch, *semitones);
            done += 1;
            progress(done, total);
        }
        for bake in &self.warps {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            DawEngine::bake_warp_variant(&self.pool, bake, self.sample_rate);
            done += 1;
            progress(done, total);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};

    const SR: u32 = 48_000;

    /// An engine with a second of a stereo chord in its pool and two
    /// clips stretched from it: by 1.5 at 0 s and by 2 at 4 s.
    fn two_stretched_clips() -> DawEngine {
        let engine = DawEngine::new();
        let tone = |f: f32| -> Vec<f32> {
            (0..SR)
                .map(|n| (std::f32::consts::TAU * f * n as f32 / SR as f32).sin() * 0.4)
                .collect()
        };
        engine.audio_pool.insert(
            "chord".into(),
            AudioBuffer {
                channels: vec![tone(220.0), tone(330.0)],
                sample_rate: SR,
                num_frames: SR as usize,
            },
        );
        let mut p = engine.project.lock();
        let track = p.add_audio_track("Chord".into());
        for (at_secs, stretch) in [(0.0, 1.5), (4.0, 2.0)] {
            let at = p
                .tempo_map
                .samples_to_tick((at_secs * SR as f64) as u64, SR as f64);
            let end = p
                .tempo_map
                .samples_to_tick(((at_secs + stretch) * SR as f64) as u64, SR as f64);
            let clip = AudioClip {
                id: format!("c{at_secs}"),
                name: "chord".into(),
                source_path: "chord".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: SR as u64,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: stretch,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: String::new(),
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            };
            p.track_mut(&track).unwrap().clips.push(ClipPlacement {
                content: ClipContent::Audio(clip),
                track_id: track.clone(),
                position_ticks: at,
                length_ticks: end - at,
                lane: 0,
                color: None,
                row: None,
            });
        }
        drop(p);
        engine
    }

    fn baked(engine: &DawEngine, stretch: f64) -> bool {
        engine
            .audio_pool
            .contains(&stretch_cache_key("chord", stretch, 0.0))
    }

    /// From 4 to 6 seconds, at 48 kHz.
    const SPAN: (u64, u64) = (4 * SR as u64, 6 * SR as u64);

    #[test]
    fn a_render_of_part_of_the_song_bakes_only_the_clips_in_it() {
        let engine = two_stretched_clips();
        assert_eq!(engine.offline_bakes(SR, None).len(), 2, "the whole song");
        let bakes = engine.offline_bakes(SR, Some(SPAN));
        assert_eq!(bakes.len(), 1, "only the clip at 4 s plays from 4 s to 6 s");
        assert!(bakes.run(&AtomicBool::new(false), |_, _| {}));
        assert!(baked(&engine, 2.0));
        assert!(!baked(&engine, 1.5), "the clip at 0 s was not baked");
        assert!(
            engine.offline_bakes(SR, Some(SPAN)).is_empty(),
            "made once, it is not made again"
        );
    }

    /// Baked only for the part it plays, a render of that part is the
    /// very same as with every clip of the song baked.
    #[test]
    fn a_render_of_part_of_the_song_is_the_same_as_with_every_clip_baked() {
        let render = |engine: &DawEngine, span: Option<(u64, u64)>| -> Vec<f32> {
            let mut out = Vec::new();
            engine
                .prepare_offline_span(SR, SPAN.0, span, None, |_| {})
                .run(SPAN.1 - SPAN.0, |block| {
                    out.extend_from_slice(block);
                    true
                });
            out
        };
        let whole = two_stretched_clips();
        let every_clip = render(&whole, None);
        assert!(baked(&whole, 1.5) && baked(&whole, 2.0));

        let part = two_stretched_clips();
        assert!(part
            .offline_bakes(SR, Some(SPAN))
            .run(&AtomicBool::new(false), |_, _| {}));
        let its_clips = render(&part, Some(SPAN));
        assert!(!baked(&part, 1.5));

        assert!(every_clip.iter().any(|s| s.abs() > 0.05), "it plays");
        assert_eq!(every_clip.len(), its_clips.len());
        assert!(
            every_clip == its_clips,
            "the same samples, baked for the part or for the song"
        );
    }

    #[test]
    fn a_stopped_bake_stops_between_bakes_and_says_so() {
        let engine = two_stretched_clips();
        let bakes = engine.offline_bakes(SR, None);
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let finished = bakes.run(&stop, |done, total| {
            seen.push((done, total));
            if done == 1 {
                stop.store(true, Ordering::Relaxed);
            }
        });
        assert!(!finished);
        assert_eq!(seen, vec![(0, 2), (1, 2)]);
        assert!(baked(&engine, 1.5), "what was made stays made");
        assert!(!baked(&engine, 2.0), "the second was never started");
    }
}
