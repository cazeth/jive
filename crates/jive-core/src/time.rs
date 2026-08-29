//! Timekeeping.
//!
//! [`Time`] is a point on a timeline, [`Duration`] a length of one. Both are
//! opaque wrappers around a millisecond counter, reachable only through the
//! millisecond constructors and accessors, so the representation may change.
//! With `serde`, both encode as a bare count of milliseconds — [`Time`]
//! counting from the Unix epoch — and that encoding is stable across versions.
//!
//! Arithmetic saturates rather than panicking or wrapping. The checked forms
//! report the overflow instead.

use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// A point in time, to millisecond resolution.
///
/// Ordered. Arithmetic saturates at both ends of the range. With `serde`,
/// encodes as a bare count of milliseconds since the Unix epoch, an encoding
/// that is stable across versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Time {
    milliseconds: u64,
}

impl Time {
    /// The Unix epoch, the earliest representable point in time.
    pub const EPOCH: Self = Self { milliseconds: 0 };

    /// The current time. A system clock set before the Unix epoch reads as
    /// [`Time::EPOCH`].
    #[must_use]
    pub fn now() -> Self {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            });
        Self {
            milliseconds: since_epoch,
        }
    }

    /// The point `milliseconds` after the Unix epoch.
    #[must_use]
    pub const fn from_unix_milliseconds(milliseconds: u64) -> Self {
        Self { milliseconds }
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub const fn as_unix_milliseconds(self) -> u64 {
        self.milliseconds
    }

    /// The time elapsed since `earlier`, or [`Duration::ZERO`] if `earlier` is
    /// the later of the two.
    #[must_use]
    pub const fn duration_since(self, earlier: Self) -> Duration {
        Duration::from_milliseconds(self.milliseconds.saturating_sub(earlier.milliseconds))
    }

    /// The time elapsed since `earlier`, or [`None`] if `earlier` is the later
    /// of the two.
    #[must_use]
    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.milliseconds
            .checked_sub(earlier.milliseconds)
            .map(Duration::from_milliseconds)
    }
}

impl Add<Duration> for Time {
    type Output = Self;

    fn add(self, span: Duration) -> Self {
        Self {
            milliseconds: self.milliseconds.saturating_add(span.as_milliseconds()),
        }
    }
}

impl AddAssign<Duration> for Time {
    fn add_assign(&mut self, span: Duration) {
        *self = *self + span;
    }
}

impl Sub<Duration> for Time {
    type Output = Self;

    fn sub(self, span: Duration) -> Self {
        Self {
            milliseconds: self.milliseconds.saturating_sub(span.as_milliseconds()),
        }
    }
}

impl SubAssign<Duration> for Time {
    fn sub_assign(&mut self, span: Duration) {
        *self = *self - span;
    }
}

impl Sub<Time> for Time {
    type Output = Duration;

    fn sub(self, earlier: Self) -> Duration {
        self.duration_since(earlier)
    }
}

impl Add<Time> for Duration {
    type Output = Time;

    fn add(self, point: Time) -> Time {
        point + self
    }
}

/// A length of time, to millisecond resolution.
///
/// Ordered. Arithmetic saturates at both ends of the range. With `serde`,
/// encodes as a bare count of milliseconds, an encoding that is stable across
/// versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Duration {
    milliseconds: u64,
}

impl Duration {
    /// A duration of zero.
    pub const ZERO: Self = Self { milliseconds: 0 };

    /// The longest representable duration.
    pub const MAX: Self = Self {
        milliseconds: u64::MAX,
    };

    /// The largest count of whole seconds [`Duration::from_seconds`] represents
    /// exactly.
    pub const MAX_WHOLE_SECONDS: u64 = u64::MAX / MILLISECONDS_PER_SECOND;

    /// A duration of `milliseconds`.
    #[must_use]
    pub const fn from_milliseconds(milliseconds: u64) -> Self {
        Self { milliseconds }
    }

    /// A duration of whole seconds.
    ///
    /// Saturates to [`Duration::MAX`] above [`Duration::MAX_WHOLE_SECONDS`].
    /// Use [`Duration::checked_from_seconds`] to detect that instead.
    #[must_use]
    pub const fn from_seconds(seconds: u64) -> Self {
        Self {
            milliseconds: seconds.saturating_mul(MILLISECONDS_PER_SECOND),
        }
    }

    /// A duration of whole seconds, or [`None`] if it exceeds
    /// [`Duration::MAX`].
    #[must_use]
    pub const fn checked_from_seconds(seconds: u64) -> Option<Self> {
        match seconds.checked_mul(MILLISECONDS_PER_SECOND) {
            Some(milliseconds) => Some(Self { milliseconds }),
            None => None,
        }
    }

    /// The duration in milliseconds.
    #[must_use]
    pub const fn as_milliseconds(self) -> u64 {
        self.milliseconds
    }

    /// The duration in whole seconds, truncated.
    #[must_use]
    pub const fn as_whole_seconds(self) -> u64 {
        self.milliseconds / MILLISECONDS_PER_SECOND
    }

    /// Whether the duration is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.milliseconds == 0
    }

    /// This duration minus `other`, or [`None`] if `other` is longer.
    #[must_use]
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.milliseconds
            .checked_sub(other.milliseconds)
            .map(Self::from_milliseconds)
    }
}

impl Add for Duration {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            milliseconds: self.milliseconds.saturating_add(other.milliseconds),
        }
    }
}

impl AddAssign for Duration {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl Sub for Duration {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            milliseconds: self.milliseconds.saturating_sub(other.milliseconds),
        }
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

/// The point `seconds` whole seconds after the epoch.
///
/// Built from the millisecond constructor rather than from addition, so the
/// arithmetic tests do not lean on the arithmetic they check.
#[cfg(test)]
pub(crate) fn at(seconds: u64) -> Time {
    Time::from_unix_milliseconds(seconds * MILLISECONDS_PER_SECOND)
}

/// A duration of whole seconds.
#[cfg(test)]
pub(crate) fn span(seconds: u64) -> Duration {
    Duration::from_seconds(seconds)
}

#[cfg(test)]
mod tests {
    use super::Duration;
    use super::Time;
    use super::at;
    use super::span;

    fn latest() -> Time {
        Time::from_unix_milliseconds(u64::MAX)
    }

    fn longest_exact_span() -> u64 {
        Duration::MAX_WHOLE_SECONDS
    }

    /// One test per `expression => result` row of time arithmetic.
    ///
    /// Rows differ in result type — a [`Time`], a [`Duration`], or an
    /// [`Option`] — so each becomes its own test rather than a row in a single
    /// typed table.
    macro_rules! arithmetic {
        ($($name:ident: $expression:expr => $value:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!($expression, $value);
                }
            )+
        };
    }

    arithmetic! {
        adding_a_span_moves_a_point_forward: at(10) + span(5) => at(15);
        a_span_and_a_point_add_in_either_order: span(5) + at(10) => at(15);
        subtracting_a_span_moves_a_point_backward: at(10) - span(5) => at(5);
        subtracting_two_points_gives_the_span_between: at(10) - at(4) => span(6);
        a_point_minus_itself_is_no_span: at(10) - at(10) => Duration::ZERO;
        spans_add: span(4) + span(5) => span(9);
        spans_subtract: span(9) - span(4) => span(5);
    }

    // Arithmetic that would run off the end of the range.
    arithmetic! {
        a_reversed_subtraction_gives_no_span: at(4) - at(10) => Duration::ZERO;
        a_reversed_subtraction_is_refused_when_checked:
            at(4).checked_duration_since(at(10)) => None;
        a_forward_subtraction_is_allowed_when_checked:
            at(10).checked_duration_since(at(4)) => Some(span(6));
        a_reversed_span_subtraction_saturates: span(4) - span(9) => Duration::ZERO;
        a_reversed_span_subtraction_is_refused_when_checked:
            span(4).checked_sub(span(9)) => None;
        a_forward_span_subtraction_is_allowed_when_checked:
            span(9).checked_sub(span(4)) => Some(span(5));
        adding_past_the_last_point_saturates: latest() + span(1) => latest();
        subtracting_past_the_epoch_saturates: Time::EPOCH - span(1) => Time::EPOCH;
        adding_past_the_longest_span_saturates: Duration::MAX + span(1) => Duration::MAX;
        the_span_since_the_epoch_is_the_whole_timeline: latest() - Time::EPOCH => Duration::MAX;
        a_span_beyond_the_longest_saturates:
            Duration::from_seconds(longest_exact_span() + 1) => Duration::MAX;
        a_span_beyond_the_longest_is_refused_when_checked:
            Duration::checked_from_seconds(longest_exact_span() + 1) => None;
    }

    #[test]
    fn each_type_defaults_to_its_own_zero() {
        assert_eq!(Time::default(), Time::EPOCH);
        assert_eq!(Duration::default(), Duration::ZERO);
        assert!(Duration::ZERO.is_zero());
        assert!(!Duration::from_milliseconds(1).is_zero());
    }

    #[test]
    fn a_count_of_milliseconds_reads_back_as_itself() {
        assert_eq!(
            Time::from_unix_milliseconds(1_234).as_unix_milliseconds(),
            1_234
        );
        assert_eq!(Duration::from_milliseconds(1_234).as_milliseconds(), 1_234);
    }

    #[test]
    fn a_span_reports_whole_seconds_and_drops_the_remainder() {
        assert_eq!(Duration::from_milliseconds(1_999).as_whole_seconds(), 1);
        assert_eq!(Duration::from_milliseconds(999).as_whole_seconds(), 0);
        assert_eq!(span(3_600).as_whole_seconds(), 3_600);
    }

    /// [`Duration::from_seconds`] is exact up to [`Duration::MAX_WHOLE_SECONDS`]
    /// and saturates above it.
    #[test]
    fn the_longest_exact_span_survives_but_anything_longer_does_not() {
        let longest = longest_exact_span();
        assert_eq!(span(longest).as_whole_seconds(), longest);
        assert_eq!(Duration::checked_from_seconds(longest), Some(span(longest)));
        assert_eq!(Duration::checked_from_seconds(longest + 1), None);
    }

    #[test]
    fn points_and_spans_each_order_by_size() {
        assert!(at(1) < at(2) && at(2) < at(3));
        assert!(span(1) < span(2) && span(2) < span(3));
        assert!(Time::EPOCH <= at(0), "the epoch is the earliest point");
    }

    #[test]
    fn the_system_clock_is_after_the_epoch() {
        assert!(Time::now() > Time::EPOCH);
    }

    #[test]
    fn assigning_forms_agree_with_their_operators() {
        let mut point = at(10);
        point += span(5);
        assert_eq!(point, at(10) + span(5));
        point -= span(3);
        assert_eq!(point, at(12));

        let mut length = span(10);
        length += span(5);
        assert_eq!(length, span(15));
        length -= span(3);
        assert_eq!(length, span(12));
    }

    #[test]
    fn a_point_plus_a_span_walks_back_to_where_it_started() {
        for seconds in [0, 1, 59, 60, 3_599, 3_600, 86_400] {
            let moved = at(1_000) + span(seconds);
            assert_eq!(moved - span(seconds), at(1_000));
            assert_eq!(moved - at(1_000), span(seconds));
        }
    }

    #[test]
    fn the_system_clock_does_not_run_backwards_between_two_readings() {
        let first = Time::now();
        let second = Time::now();
        assert!(second >= first, "{second:?} came before {first:?}");
    }
}
