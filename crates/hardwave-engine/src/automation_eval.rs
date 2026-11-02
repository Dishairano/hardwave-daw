//! What a track's automation says at one tick: its fader, its pan, whether
//! it is muted, and the plug-in parameters it moves (applied to the chain
//! straight away). Shared by audio tracks and instrument tracks; instrument
//! tracks used to ignore their automation entirely, so a lane or clip drawn
//! on a synth track did nothing.

use crate::insert_chain::InsertChain;
use hardwave_project::automation::{AutomationLane, AutomationTarget};
use hardwave_project::automation_clip::AutomationClip;

/// The mix values automation decided for this block.
pub struct AutomatedMix {
    pub volume_linear: f32,
    pub pan: f32,
    /// `Some(true)` when a mute lane or clip silences the track here.
    pub mute: Option<bool>,
    /// Whether automation set the volume or the pan, for the mixer to
    /// follow.
    pub volume_automated: bool,
    pub pan_automated: bool,
}

/// The bottom of a volume lane is silence, as the bottom of a send lane is
/// and as the fader reads it (-∞). It played -60 dB, so a fade-out drawn
/// all the way down left the part playing quietly under what came next.
fn db_to_linear(db: f64) -> f32 {
    if db <= VOLUME_LANE_FLOOR_DB {
        0.0
    } else {
        10.0_f64.powf(db / 20.0) as f32
    }
}

/// The level at the bottom of a volume lane, in dB.
const VOLUME_LANE_FLOOR_DB: f64 = -60.0;

/// Evaluate `lanes` then `clips` at `tick`. Where both drive the same thing
/// the clip wins, as in FL. Plug-in parameters are stored 0..1 and reach
/// the plug-in in its own range (`set_parameter_normalized`).
pub fn evaluate(
    lanes: &[AutomationLane],
    clips: &[AutomationClip],
    tick: u64,
    static_volume: f32,
    static_pan: f32,
    chain: &mut InsertChain,
) -> AutomatedMix {
    let mut out = AutomatedMix {
        volume_linear: static_volume,
        pan: static_pan,
        mute: None,
        volume_automated: false,
        pan_automated: false,
    };
    for lane in lanes {
        // A lane without points drives nothing. It used to play 0.5, so
        // adding an empty lane jumped the knob it pointed at to half-way.
        if !lane.visible || lane.points.is_empty() {
            continue;
        }
        match &lane.target {
            // Stored 0..1 across the fader's range (-60 dB..+6 dB).
            AutomationTarget::TrackVolume => {
                out.volume_linear = db_to_linear(lane.denormalized_value_at(tick, -60.0, 6.0));
                out.volume_automated = true;
            }
            AutomationTarget::TrackPan => {
                out.pan = lane.denormalized_value_at(tick, -1.0, 1.0) as f32;
                out.pan_automated = true;
            }
            // Above 0.5 is muted, so a step lane works as a kill switch.
            AutomationTarget::TrackMute => out.mute = Some(lane.value_at(tick) > 0.5),
            AutomationTarget::PluginParam { slot_id, param_id } => {
                chain.set_parameter_normalized(slot_id, *param_id, lane.value_at(tick));
            }
            // A send is a graph edge; the engine moves its gain.
            AutomationTarget::SendLevel { .. } => {}
        }
    }
    for clip in clips {
        // An automation clip without points drives nothing, like a lane.
        // A new clip is empty, and it played half-way: the track dropped
        // to -27 dB across the clip as soon as it was added.
        if clip.lane.points.is_empty() {
            continue;
        }
        let Some(v) = clip.value_at_timeline(tick) else {
            continue;
        };
        match &clip.target {
            AutomationTarget::TrackVolume => {
                out.volume_linear = db_to_linear(AutomationLane::denormalize(v, -60.0, 6.0));
                out.volume_automated = true;
            }
            AutomationTarget::TrackPan => {
                out.pan = AutomationLane::denormalize(v, -1.0, 1.0) as f32;
                out.pan_automated = true;
            }
            AutomationTarget::TrackMute => out.mute = Some(v > 0.5),
            AutomationTarget::PluginParam { slot_id, param_id } => {
                chain.set_parameter_normalized(slot_id, *param_id, v);
            }
            AutomationTarget::SendLevel { .. } => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lane without points leaves the fader where it is; it used to play
    /// 0.5, which on a volume lane is -27 dB.
    #[test]
    fn an_empty_lane_drives_nothing() {
        let lane = AutomationLane {
            id: "l".into(),
            target: AutomationTarget::TrackVolume,
            points: Vec::new(),
            visible: true,
            row: None,
        };
        let mut chain = InsertChain::new();
        let mix = evaluate(&[lane], &[], 0, 0.8, 0.0, &mut chain);
        assert_eq!(mix.volume_linear, 0.8);
        assert_eq!(mix.mute, None);
    }

    /// A volume lane drawn to the bottom is silence, not -60 dB, whether it
    /// is a lane or an automation clip.
    #[test]
    fn the_bottom_of_a_volume_lane_is_silence() {
        use hardwave_project::automation::{AutomationPoint, CurveMode};
        let lane = AutomationLane {
            id: "l".into(),
            target: AutomationTarget::TrackVolume,
            points: vec![AutomationPoint {
                tick: 0,
                value: 0.0,
                curve: CurveMode::Linear,
                tension: 0.0,
            }],
            visible: true,
            row: None,
        };
        let mut chain = InsertChain::new();
        let mix = evaluate(&[lane], &[], 0, 0.8, 0.0, &mut chain);
        assert_eq!(mix.volume_linear, 0.0);

        let mut clip = AutomationClip::new("c", AutomationTarget::TrackVolume, 0, 960);
        clip.lane.points = vec![AutomationPoint {
            tick: 0,
            value: 0.0,
            curve: CurveMode::Linear,
            tension: 0.0,
        }];
        let mix = evaluate(&[], &[clip], 10, 0.8, 0.0, &mut chain);
        assert_eq!(mix.volume_linear, 0.0);
    }

    /// A new automation clip has no points yet: the track keeps its fader
    /// under it instead of dropping to half-way.
    #[test]
    fn an_empty_clip_drives_nothing() {
        let clip = AutomationClip::new("c", AutomationTarget::TrackVolume, 0, 15_360);
        let mut chain = InsertChain::new();
        let mix = evaluate(&[], &[clip], 100, 0.8, 0.0, &mut chain);
        assert_eq!(mix.volume_linear, 0.8);
        assert!(!mix.volume_automated);
    }

    /// The mixer follows what automation does, so the result says which
    /// of fader and pan it drove.
    #[test]
    fn it_says_which_controls_automation_drove() {
        use hardwave_project::automation::{AutomationPoint, CurveMode};
        let lane = AutomationLane {
            id: "l".into(),
            target: AutomationTarget::TrackPan,
            points: vec![AutomationPoint {
                tick: 0,
                value: 1.0,
                curve: CurveMode::Linear,
                tension: 0.0,
            }],
            visible: true,
            row: None,
        };
        let mut chain = InsertChain::new();
        let mix = evaluate(&[lane], &[], 0, 0.8, 0.0, &mut chain);
        assert!(mix.pan_automated && !mix.volume_automated);
        assert_eq!(mix.pan, 1.0);
    }
}
