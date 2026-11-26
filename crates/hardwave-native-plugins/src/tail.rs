//! How long a built-in plug-in goes on sounding after its input falls
//! silent, worked out from its current settings. The host uses it to stop
//! calling a plug-in that has nothing left to say (smart disable).
//!
//! Every figure here errs long: a tail that is too long only costs a
//! little CPU, one that is too short cuts the end off a sound. They are
//! asked for on the audio thread, so none of them allocates.

use hardwave_dsp::oversample::OversampleFactor;
use hardwave_plugin_host::types::Tail;

/// The host counts a sample as silent below -150 dBFS.
const SILENT_DB: f64 = 150.0;
/// Room on top of the 150 dB for ringing that comes out louder than what
/// went in: a resonant filter, a boost, a sound that built up in a loop.
const HEADROOM_DB: f64 = 24.0;
/// A level falls by a factor of e every this many dB.
const DB_PER_NEPER: f64 = 8.685_889_638_065_037;
/// A tail longer than this is a loop that, in practice, never dies (a
/// delay with its feedback all the way up): the host keeps calling it.
const LONGEST_SECS: f64 = 600.0;

/// How long the oversampler's half-band filters ring after the last
/// sound: they sit just under the project rate's Nyquist, so it is well
/// under a millisecond; this is ten times that and then some.
const OVERSAMPLER_RING_SECS: f64 = 0.01;

/// How long the oversampler rings: nothing when it is off.
pub fn oversampler_secs(factor: OversampleFactor) -> f64 {
    if factor == OversampleFactor::Off {
        0.0
    } else {
        OVERSAMPLER_RING_SECS
    }
}

/// A tail of `secs` seconds at `sample_rate`, rounded up. Too long or not
/// a number at all (an endless loop) is `Tail::Infinite`.
pub fn from_secs(secs: f64, sample_rate: f32) -> Tail {
    if !secs.is_finite() || secs > LONGEST_SECS {
        return Tail::Infinite;
    }
    from_samples(secs.max(0.0) * sample_rate.max(1.0) as f64)
}

/// A tail of `samples` samples, rounded up.
pub fn from_samples(samples: f64) -> Tail {
    if !samples.is_finite() || samples >= u32::MAX as f64 {
        return Tail::Infinite;
    }
    Tail::Samples(samples.max(0.0).ceil() as u32)
}

/// How long a sound takes to die in a loop that takes `loop_secs` to go
/// round and keeps `feedback` of the level each time: the first trip, then
/// as many more as it takes to fall below silence from the level a steady
/// sound could have built up to (1 / (1 - feedback) times the input).
/// Infinite when the loop keeps all of it or more.
pub fn feedback_secs(loop_secs: f64, feedback: f64) -> f64 {
    let g = feedback.abs();
    if g >= 1.0 {
        return f64::INFINITY;
    }
    if g <= 0.0 {
        return loop_secs;
    }
    let build_up_db = -20.0 * (1.0 - g).log10();
    let loss_per_trip_db = -20.0 * g.log10();
    let trips = ((SILENT_DB + build_up_db) / loss_per_trip_db).ceil();
    loop_secs * (trips + 1.0)
}

/// How long an envelope follower whose release time constant is
/// `release_secs` takes to fall from full scale to silence, which is when
/// it stops shaping what comes next the way it did.
pub fn release_secs(release_secs: f64) -> f64 {
    release_secs.max(0.0) * (SILENT_DB + 6.0) / DB_PER_NEPER
}

/// How long `stages` biquads in a row, tuned to `freq_hz` with poles of
/// quality `q`, ring after the last sound. From the poles' decay rate:
/// `w / 2q` for a resonant pair, the slower of the two real poles when
/// `q` is under one half. A louder resonance (higher `q`) needs longer to
/// fall below silence, and each further stage a few time constants more.
pub fn filter_ring_secs(freq_hz: f64, q: f64, stages: usize) -> f64 {
    let w = std::f64::consts::TAU * freq_hz.max(1.0);
    let q = q.max(0.01);
    let decay_rate = if q >= 0.5 {
        w / (2.0 * q)
    } else {
        let a = 1.0 / (2.0 * q);
        w * (a - (a * a - 1.0).sqrt())
    };
    let tau = 1.0 / decay_rate;
    let peak_db = 20.0 * q.max(1.0).log10();
    let one = tau * (SILENT_DB + HEADROOM_DB + peak_db) / DB_PER_NEPER;
    one + tau * 5.0 * stages.saturating_sub(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_tail_grows_with_feedback_and_never_ends_at_unity() {
        let short = feedback_secs(0.25, 0.3);
        let long = feedback_secs(0.25, 0.8);
        assert!(short > 0.25 && long > short, "{short} {long}");
        assert_eq!(feedback_secs(0.25, 0.0), 0.25);
        assert!(feedback_secs(0.25, 1.0).is_infinite());
        assert_eq!(from_secs(f64::INFINITY, 48_000.0), Tail::Infinite);
        assert_eq!(from_secs(1.0, 48_000.0), Tail::Samples(48_000));
    }

    #[test]
    fn a_lower_or_sharper_filter_rings_longer() {
        let base = filter_ring_secs(1_000.0, 0.707, 1);
        assert!(filter_ring_secs(100.0, 0.707, 1) > base);
        assert!(filter_ring_secs(1_000.0, 8.0, 1) > base);
        assert!(filter_ring_secs(1_000.0, 0.1, 1) > base);
        assert!(filter_ring_secs(1_000.0, 0.707, 2) > base);
    }
}
