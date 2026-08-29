//! Listener preference and playback reliability, derived from track events.
//!
//! Only the events the listener decided determine preference: a finish counts
//! for a track, and a quick skip counts against it. A late skip, a stop, and a
//! playback failure say nothing about taste, and only failures affect
//! reliability.

use jive_core::Duration;
use jive_core::track_events::AnyTrackEvent;
use jive_core::track_events::TimeTaggedTrackEvents;

/// A skip before this much of a track has played counts against it.
pub const QUICK_SKIP_THRESHOLD: Duration = Duration::from_seconds(15);
/// Neutral evidence added to both sides of the preference ratio, so that the
/// first few events move it only so far.
pub const PREFERENCE_PRIOR: f64 = 10.0;
/// Lowest listener-preference factor.
pub const MINIMUM_PREFERENCE: f64 = 1.0 / 3.0;
/// Highest listener-preference factor.
pub const MAXIMUM_PREFERENCE: f64 = 3.0;
/// Lowest playback-reliability factor.
pub const MINIMUM_RELIABILITY: f64 = 0.05;
/// Preference of a track with no evidence either way.
pub const STARTING_PREFERENCE: f64 = 1.0;

/// The event counts the rating factors are computed from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Evidence {
    /// Plays that reached the end of the track.
    pub finishes: u64,
    /// Skips before [`QUICK_SKIP_THRESHOLD`].
    pub quick_skips: u64,
    /// Plays that failed.
    pub failures: u64,
}

impl Evidence {
    /// Adds one event to these counts. Events that count for nothing are
    /// ignored.
    pub fn observe(&mut self, event: &AnyTrackEvent) {
        if let Some(skipped) = event.as_skipped() {
            if skipped.listened_for < QUICK_SKIP_THRESHOLD {
                self.quick_skips += 1;
            }
        } else if let Some(outcome) = event.as_playback_outcome() {
            if outcome.is_finished() {
                self.finishes += 1;
            } else {
                self.failures += 1;
            }
        }
    }
}

/// The evidence a track's events amount to.
#[must_use]
pub fn evidence_of(events: &TimeTaggedTrackEvents) -> Evidence {
    let mut evidence = Evidence::default();
    for event in events {
        evidence.observe(&event.event);
    }
    evidence
}

/// Listener preference, from [`MINIMUM_PREFERENCE`] to [`MAXIMUM_PREFERENCE`].
///
/// A ratio of finishes to quick skips, each side offset by
/// [`PREFERENCE_PRIOR`], so that repeated evidence has a diminishing effect and
/// no track is ever driven out of the running or made certain to win.
#[must_use]
pub fn preference(evidence: Evidence) -> f64 {
    ((PREFERENCE_PRIOR + as_float(evidence.finishes))
        / (PREFERENCE_PRIOR + as_float(evidence.quick_skips)))
    .clamp(MINIMUM_PREFERENCE, MAXIMUM_PREFERENCE)
}

/// The share of recorded plays that succeeded, floored at
/// [`MINIMUM_RELIABILITY`].
///
/// Later finishes raise it again, so a track that has been repaired stops being
/// held back by its failures.
#[must_use]
pub fn reliability(evidence: Evidence) -> f64 {
    ((1.0 + as_float(evidence.finishes)) / (1.0 + as_float(evidence.finishes + evidence.failures)))
        .clamp(MINIMUM_RELIABILITY, 1.0)
}

#[allow(clippy::cast_precision_loss)]
fn as_float(value: u64) -> f64 {
    value as f64
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::Evidence;
    use super::MAXIMUM_PREFERENCE;
    use super::MINIMUM_PREFERENCE;
    use super::MINIMUM_RELIABILITY;
    use super::evidence_of;
    use super::preference;
    use super::reliability;
    use crate::testing::failed;
    use crate::testing::finished;
    use crate::testing::quick_skip;
    use crate::testing::recorded_at;
    use crate::testing::repeated;
    use crate::testing::skip_after;
    use jive_core::Time;
    use jive_core::track_events::AnyTrackEvent;

    fn evidence(events: Vec<AnyTrackEvent>) -> Evidence {
        evidence_of(&recorded_at(Time::EPOCH, events))
    }

    /// The preference produced by `times` events of one kind.
    fn preference_after(event: &AnyTrackEvent, times: usize) -> f64 {
        preference(evidence(repeated(event, times)))
    }

    /// The reliability produced by `times` events of one kind.
    fn reliability_after(event: &AnyTrackEvent, times: usize) -> f64 {
        reliability(evidence(repeated(event, times)))
    }

    /// Only what the listener chose says anything about taste. A finish counts
    /// for a track and a quick skip against it. A late skip and a playback
    /// failure count for neither.
    #[test]
    fn preference_moves_only_on_what_the_listener_decided() {
        assert!(preference_after(&finished(), 1) > 1.0, "a finish");
        assert!(preference_after(&quick_skip(), 1) < 1.0, "a quick skip");
        assert_eq!(preference_after(&skip_after(15), 1), 1.0, "a late skip");
        assert_eq!(preference_after(&failed(), 1), 1.0, "a failure");
    }

    /// However lopsided the evidence, each factor stops at its own bound, so no
    /// track can be driven out of the running or made certain to win.
    #[test]
    fn a_rating_never_runs_past_its_bounds() {
        assert_eq!(preference_after(&finished(), 10_000), MAXIMUM_PREFERENCE);
        assert_eq!(preference_after(&quick_skip(), 10_000), MINIMUM_PREFERENCE);
        assert_eq!(reliability_after(&failed(), 10_000), MINIMUM_RELIABILITY);
    }

    #[test]
    fn finishes_rehabilitate_a_track_that_kept_failing() {
        let broken = reliability_after(&failed(), 20);
        let recovered = reliability(evidence(
            [repeated(&failed(), 20), repeated(&finished(), 20)].concat(),
        ));
        assert!(
            broken < recovered,
            "{broken} should be worse than {recovered}"
        );
    }

    #[test]
    fn equal_extra_evidence_has_a_diminishing_effect() {
        let after_ten = preference_after(&quick_skip(), 10);
        let first = 1.0 - after_ten;
        let second = after_ten - preference_after(&quick_skip(), 20);
        assert!(second < first, "{second} should be smaller than {first}");
    }
}
