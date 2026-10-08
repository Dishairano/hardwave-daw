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
}

fn db_to_linear(db: f64) -> f32 {
    10.0_f64.powf(db / 20.0) as f32
}

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
    };
    for lane in lanes {
        if !lane.visible {
            continue;
        }
        match &lane.target {
            // Stored 0..1 across the fader's range (-60 dB..+6 dB).
            AutomationTarget::TrackVolume => {
                out.volume_linear = db_to_linear(lane.denormalized_value_at(tick, -60.0, 6.0))
            }
            AutomationTarget::TrackPan => {
                out.pan = lane.denormalized_value_at(tick, -1.0, 1.0) as f32
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
        let Some(v) = clip.value_at_timeline(tick) else {
            continue;
        };
        match &clip.target {
            AutomationTarget::TrackVolume => {
                out.volume_linear = db_to_linear(AutomationLane::denormalize(v, -60.0, 6.0))
            }
            AutomationTarget::TrackPan => {
                out.pan = AutomationLane::denormalize(v, -1.0, 1.0) as f32
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
