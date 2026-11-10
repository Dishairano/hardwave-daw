//! Audio clips that follow the song's tempo, as the engine plays them.
//!
//! A clip that follows the tempo (`SampleTreatment::follow_bpm`) lays its file
//! over the beats at a fixed number of frames per tick, so it stays on the
//! beat whatever the tempo map does. It used to keep the speed it was
//! imported at, so after a tempo change a loop drifted off the grid it was
//! drawn on.
//!
//! The engine already has two ways to play a file at another speed with its
//! pitch kept: one even stretch, and warp markers. Such a clip becomes one of
//! them here, before anything else looks at it, so playback, the bakes and an
//! export all see the same clip and cannot disagree.

use std::borrow::Cow;

use hardwave_project::clip::{AudioClip, ClipPlacement, WarpMarker};
use hardwave_project::tempo::TempoRamp;
use hardwave_project::TempoMap;

/// How far the tempo may move inside one piece of a clip cut up to follow a
/// ramp, as a share of the tempo. Within a piece the file is played at one
/// speed, so its middle runs a little ahead of or behind the beat; at one
/// percent over a bar at 150 BPM that is under two milliseconds.
const MAX_TEMPO_MOVE_PER_PIECE: f64 = 0.01;

/// The shortest piece a ramp is cut into: a sixteenth.
const MIN_PIECE_TICKS: u64 = hardwave_midi::PPQ / 4;

/// The clip as the engine plays it under `tempo_map`. A clip that follows
/// the tempo comes back as a plain stretch where the tempo holds still over
/// it, and as warp markers pinning its file to the beats where it moves.
/// Everything else, and a warped clip, comes back as it is: warp markers
/// already pin it to ticks.
///
/// `file_rate` is the rate of the file as it sits in the pool; without it
/// (not loaded yet) the clip is stretched evenly at the average tempo it
/// covers, which is right at both of its edges.
pub fn tempo_followed<'a>(
    clip: &ClipPlacement,
    audio: &'a AudioClip,
    tempo_map: &TempoMap,
    file_rate: Option<f64>,
) -> Cow<'a, AudioClip> {
    let Some(bpm) = audio.follow_bpm() else {
        return Cow::Borrowed(audio);
    };
    if !audio.warp_markers.is_empty() {
        return Cow::Borrowed(audio);
    }
    let start = clip.position_ticks;
    let end = start + clip.length_ticks.max(1);
    let stretch = if audio.stretch_ratio <= 0.01 {
        1.0
    } else {
        audio.stretch_ratio
    };
    let mut out = audio.clone();
    let per_tick = file_rate.and_then(|rate| audio.file_frames_per_tick(rate));
    match (tempo_map.constant_bpm_between(start, end), per_tick) {
        // A reversed clip plays its window from the end back, which warp
        // markers do not describe: it is stretched evenly instead, on the
        // beat at both ends.
        (None, Some(per_tick)) if !audio.reversed => {
            out.warp_markers = anchor_ticks(tempo_map, start, end)
                .into_iter()
                .map(|t| WarpMarker {
                    clip_tick: t,
                    source_sample: (audio.source_start as f64 + t as f64 * per_tick).round() as u64,
                })
                .collect();
        }
        (steady, _) => {
            let tempo = steady.unwrap_or_else(|| tempo_map.average_bpm_between(start, end));
            out.stretch_ratio = stretch * bpm / tempo.max(1.0);
        }
    }
    Cow::Owned(out)
}

/// Where a clip from `start` to `end` is pinned to the beat, in ticks from
/// its start: its two edges, every tempo change inside it, and inside a ramp
/// often enough that the tempo moves little within each piece.
fn anchor_ticks(map: &TempoMap, start: u64, end: u64) -> Vec<u64> {
    let mut ticks = vec![0, end - start];
    for (i, entry) in map.entries.iter().enumerate() {
        let seg_end = map.entries.get(i + 1).map(|e| e.tick);
        if entry.tick > start && entry.tick < end {
            ticks.push(entry.tick - start);
        }
        let Some(next) = map.entries.get(i + 1) else {
            continue;
        };
        if entry.ramp != TempoRamp::Linear || next.tick <= entry.tick {
            continue;
        }
        // Only the ramps the clip overlaps.
        if entry.tick >= end || seg_end.is_some_and(|e| e <= start) {
            continue;
        }
        let low = entry.bpm.min(next.bpm).max(1.0);
        let moves = (entry.bpm - next.bpm).abs() / (low * MAX_TEMPO_MOVE_PER_PIECE);
        let span = next.tick - entry.tick;
        let pieces = (moves.ceil() as u64).clamp(1, (span / MIN_PIECE_TICKS).max(1));
        for k in 1..pieces {
            let at = entry.tick + span * k / pieces;
            if at > start && at < end {
                ticks.push(at - start);
            }
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_midi::PPQ;
    use hardwave_project::clip::{ClipContent, SampleTreatment};
    use hardwave_project::tempo::TempoEntry;

    const BAR: u64 = PPQ * 4;
    const RATE: f64 = 48_000.0;

    fn entry(tick: u64, bpm: f64, ramp: TempoRamp) -> TempoEntry {
        TempoEntry {
            tick,
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            ramp,
        }
    }

    fn build_and_drop() -> TempoMap {
        TempoMap {
            entries: vec![
                entry(0, 150.0, TempoRamp::Instant),
                entry(BAR * 8, 150.0, TempoRamp::Linear),
                entry(BAR * 16 - 1, 160.0, TempoRamp::Instant),
                entry(BAR * 16, 150.0, TempoRamp::Instant),
            ],
        }
    }

    fn placed(at: u64, length: u64, follow_bpm: Option<f64>) -> (ClipPlacement, AudioClip) {
        let audio = AudioClip {
            id: "a".into(),
            name: "Loop 150 BPM".into(),
            source_path: "loop".into(),
            source_hash: String::new(),
            source_start: 0,
            source_end: 0,
            gain_db: 0.0,
            fade_in_ticks: 0,
            fade_out_ticks: 0,
            muted: false,
            reversed: false,
            pitch_semitones: 0.0,
            stretch_ratio: 1.0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            warp_markers: Vec::new(),
            source_file: String::new(),
            treatment: SampleTreatment {
                follow_bpm,
                ..Default::default()
            },
            tuned_notes: Vec::new(),
        };
        (
            ClipPlacement {
                content: ClipContent::Audio(audio.clone()),
                track_id: "t".into(),
                position_ticks: at,
                length_ticks: length,
                lane: 0,
                color: None,
                row: None,
            },
            audio,
        )
    }

    #[test]
    fn a_clip_that_keeps_its_speed_is_played_as_it_is() {
        let (clip, audio) = placed(0, BAR * 4, None);
        assert!(matches!(
            tempo_followed(&clip, &audio, &build_and_drop(), Some(RATE)),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn a_loop_at_a_steady_tempo_is_one_even_stretch() {
        // A 150 loop in the drop, which is at 150: its own speed.
        let (clip, audio) = placed(BAR * 16, BAR * 4, Some(150.0));
        let played = tempo_followed(&clip, &audio, &build_and_drop(), Some(RATE));
        assert!((played.stretch_ratio - 1.0).abs() < 1e-12);
        assert!(played.warp_markers.is_empty());

        // A 128 loop in a 150 song plays faster: 128/150 as long.
        let (clip, audio) = placed(0, BAR * 4, Some(128.0));
        let steady = TempoMap {
            entries: vec![entry(0, 150.0, TempoRamp::Instant)],
        };
        let played = tempo_followed(&clip, &audio, &steady, Some(RATE));
        assert!((played.stretch_ratio - 128.0 / 150.0).abs() < 1e-12);
    }

    /// Across the build, the loop's file is pinned to the beats: the frame
    /// the file has at beat n (its own tempo, 150) is played at the tick of
    /// beat n, wherever the ramp has put that tick in time.
    #[test]
    fn a_loop_across_a_ramp_is_pinned_to_the_beats() {
        let (clip, audio) = placed(BAR * 4, BAR * 12, Some(150.0));
        let played = tempo_followed(&clip, &audio, &build_and_drop(), Some(RATE));
        let markers = &played.warp_markers;
        assert!(markers.len() > 4, "cut into pieces: {}", markers.len());
        assert_eq!(markers[0].clip_tick, 0);
        assert_eq!(markers.last().unwrap().clip_tick, BAR * 12);
        // 20 frames a tick at 150 and 48 kHz, wherever the marker is.
        for m in markers {
            assert_eq!(m.source_sample, m.clip_tick * 20, "{m:?}");
        }
        // The ramp starts at bar 9 and every piece moves the tempo by at
        // most a percent.
        assert!(markers.iter().any(|m| m.clip_tick == BAR * 4));
        let map = build_and_drop();
        for w in markers.windows(2) {
            let a = map.bpm_at(BAR * 4 + w[0].clip_tick);
            let b = map.bpm_at(BAR * 4 + w[1].clip_tick - 1);
            assert!((b - a).abs() / a <= 0.0101, "{a} to {b}");
        }
    }

    #[test]
    fn a_jump_inside_a_loop_is_an_anchor() {
        let map = TempoMap {
            entries: vec![
                entry(0, 150.0, TempoRamp::Instant),
                entry(BAR * 2, 175.0, TempoRamp::Instant),
            ],
        };
        let (clip, audio) = placed(BAR, BAR * 2, Some(150.0));
        let played = tempo_followed(&clip, &audio, &map, Some(RATE));
        let ticks: Vec<u64> = played.warp_markers.iter().map(|m| m.clip_tick).collect();
        assert_eq!(ticks, vec![0, BAR, BAR * 2]);
    }

    #[test]
    fn a_warped_clip_already_follows_its_markers() {
        let (clip, mut audio) = placed(0, BAR * 16, Some(150.0));
        audio.warp_markers = vec![WarpMarker {
            clip_tick: PPQ,
            source_sample: 12_345,
        }];
        let played = tempo_followed(&clip, &audio, &build_and_drop(), Some(RATE));
        assert!(matches!(played, Cow::Borrowed(_)));
    }

    #[test]
    fn without_its_file_or_reversed_it_is_stretched_on_the_average() {
        let map = build_and_drop();
        let (clip, audio) = placed(BAR * 8, BAR * 8, Some(150.0));
        let avg = map.average_bpm_between(BAR * 8, BAR * 16);
        let played = tempo_followed(&clip, &audio, &map, None);
        assert!((played.stretch_ratio - 150.0 / avg).abs() < 1e-12);
        let mut reversed = audio.clone();
        reversed.reversed = true;
        let played = tempo_followed(&clip, &reversed, &map, Some(RATE));
        assert!(played.warp_markers.is_empty());
        assert!((played.stretch_ratio - 150.0 / avg).abs() < 1e-12);
    }
}
