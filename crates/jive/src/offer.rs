//! Three independent factors, and the draw priority that is their product.
//!
//! Every factor is positive, so a priority is too: no single factor can drop a
//! track out of the running.

use crate::rating;
use crate::rating::Evidence;
use jive_core::Duration;
use jive_core::Time;
use jive_core::track_events::TimeTaggedTrackEvents;

/// Staleness of a track that has just played.
pub const FRESH_MULTIPLIER: f64 = 1.0;
/// Staleness of a track left for [`GONE_STALE_AFTER`] or longer.
pub const STALEST_MULTIPLIER: f64 = 3.0;
/// How long a track takes to become fully stale.
pub const GONE_STALE_AFTER: Duration = Duration::from_seconds(2 * 60 * 60);

/// The independent factors of a draw priority.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Factors {
    /// Measured listener preference.
    pub preference: f64,
    /// What the time since the track last played counts for.
    pub staleness: f64,
    /// Measured playback reliability.
    pub reliability: f64,
}

impl Factors {
    /// The factors multiplied together. Always positive.
    #[must_use]
    pub fn priority(self) -> f64 {
        self.preference * self.staleness * self.reliability
    }
}

/// The factors of a track with this evidence, last played at `last_played`.
#[must_use]
pub fn factors(evidence: Evidence, last_played: Option<Time>, now: Time) -> Factors {
    Factors {
        preference: rating::preference(evidence),
        staleness: staleness(last_played, now),
        reliability: rating::reliability(evidence),
    }
}

/// Staleness, rising linearly from [`FRESH_MULTIPLIER`] to
/// [`STALEST_MULTIPLIER`] over [`GONE_STALE_AFTER`].
///
/// A track that has never played is fully stale.
#[must_use]
pub fn staleness(last_played: Option<Time>, now: Time) -> f64 {
    let Some(last) = last_played else {
        return STALEST_MULTIPLIER;
    };
    let elapsed = now.duration_since(last);
    let share = if elapsed >= GONE_STALE_AFTER {
        1.0
    } else {
        ratio(
            elapsed.as_milliseconds(),
            GONE_STALE_AFTER.as_milliseconds(),
        )
    };
    FRESH_MULTIPLIER + share * (STALEST_MULTIPLIER - FRESH_MULTIPLIER)
}

/// When the track was last played, or [`None`] if it never has been.
///
/// The latest timestamp of any event except [`Added`], which records the track
/// entering the library rather than a play.
///
/// [`Added`]: jive_core::track_events::Added
#[must_use]
pub fn last_offered_at(events: &TimeTaggedTrackEvents) -> Option<Time> {
    events
        .iter()
        .filter(|event| event.event.as_added().is_none())
        .map(|event| event.at)
        .max()
}

#[allow(clippy::cast_precision_loss)]
fn ratio(part: u64, whole: u64) -> f64 {
    part as f64 / whole as f64
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::FRESH_MULTIPLIER;
    use super::GONE_STALE_AFTER;
    use super::STALEST_MULTIPLIER;
    use super::factors;
    use super::staleness;
    use crate::rating::Evidence;
    use jive_core::Time;

    /// Late enough that a track played at the epoch is fully stale.
    fn now() -> Time {
        Time::EPOCH + GONE_STALE_AFTER
    }

    #[test]
    fn staleness_runs_from_just_played_to_left_long_enough() {
        assert_eq!(staleness(Some(now()), now()), FRESH_MULTIPLIER);
        assert_eq!(staleness(Some(Time::EPOCH), now()), STALEST_MULTIPLIER);
        assert_eq!(
            staleness(None, Time::EPOCH),
            STALEST_MULTIPLIER,
            "a track never played is as stale as one left long enough"
        );
    }

    /// Each factor must differ from one, or dropping it from the product would
    /// go unnoticed.
    #[test]
    fn priority_multiplies_the_independent_factors() {
        let opinionated = Evidence {
            finishes: 5,
            quick_skips: 0,
            failures: 3,
        };
        let value = factors(opinionated, Some(Time::EPOCH), now());

        assert_ne!(value.preference, 1.0, "a preference of one is invisible");
        assert_ne!(value.staleness, 1.0, "a staleness of one is invisible");
        assert_ne!(value.reliability, 1.0, "a reliability of one is invisible");
        assert_eq!(
            value.priority(),
            value.preference * value.staleness * value.reliability
        );
    }
}
