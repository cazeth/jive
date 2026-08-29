//! The events recorded against a track.
//!
//! A track carries no score, only its events, so the rules for rating a track
//! can change without rewriting its history. Each entry is a
//! [`TimeTaggedTrackEvent`]: an [`AnyTrackEvent`] and the [`Time`] it was
//! recorded at. The timestamp sits on the pair rather than in the payload, so
//! no event can omit it.
//!
//! ```
//! use jive_core::track_events::AnyTrackEvent;
//! use jive_core::track_events::PlaybackOutcome;
//! use jive_core::track_events::Skipped;
//! use jive_core::track_events::TimeTaggedTrackEvents;
//! use jive_core::Duration;
//! use jive_core::Time;
//!
//! let mut events = TimeTaggedTrackEvents::new();
//! events.record(Time::now(), Skipped::new(Duration::from_seconds(3)));
//! events.record(Time::now(), PlaybackOutcome::Finished);
//!
//! let last_on = events.iter().last().map(|event| event.at);
//! assert!(last_on.is_some());
//!
//! let finishes = events
//!     .iter()
//!     .filter_map(|event| event.event.as_playback_outcome())
//!     .filter(|outcome| outcome.is_finished())
//!     .count();
//! assert_eq!(finishes, 1);
//! ```
//!
//! [`AnyTrackEvent`] is `#[non_exhaustive]`, so matching on one requires a
//! catch-all arm: a later release may record an event this one does not know.

use crate::time::Duration;
use crate::time::Time;

/// Defines [`AnyTrackEvent`] from `Variant(Payload) => accessor` rows, with an
/// accessor and a [`From`] implementation per payload.
macro_rules! define_any_track_event {
    (
        $(
            $(#[$documentation:meta])*
            $variant:ident($payload:ty) => $accessor:ident
        ),+ $(,)?
    ) => {
        /// Any event that can be recorded against a track.
        ///
        /// `#[non_exhaustive]`: later releases may add variants. Events are
        /// compared but never hashed or ordered, so a future payload may carry
        /// values that are only [`PartialEq`].
        ///
        /// # Encoding
        ///
        /// With `serde`, the encoding is stable and covered by tests:
        ///
        /// * An event is a variant name and a payload beside it, tagged `event`
        ///   and `data`, so a newer reader understands what an older one wrote.
        /// * Payloads ignore unknown fields. Fields added later need
        ///   `#[serde(default)]`.
        /// * Variants and payload fields are never renamed or removed.
        ///
        /// Adding a variant is therefore not a breaking change. An unrecognized
        /// event fails to decode on its own, leaving the caller free to set it
        /// aside and continue with the rest — possible only when events are
        /// decoded one at a time from a self-describing format.
        ///
        /// Decoding a [`TimeTaggedTrackEvents`] as a whole is all-or-nothing by
        /// design: a history silently missing events would misrate the track.
        #[derive(Debug, Clone, PartialEq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(
            feature = "serde",
            serde(tag = "event", content = "data", rename_all = "snake_case")
        )]
        #[non_exhaustive]
        pub enum AnyTrackEvent {
            $(
                $(#[$documentation])*
                $variant($payload),
            )+
        }

        impl AnyTrackEvent {
            $(
                #[doc = concat!("Returns the [`", stringify!($payload), "`] payload, or [`None`] when the event is something else.")]
                #[must_use]
                pub fn $accessor(&self) -> Option<&$payload> {
                    match self {
                        Self::$variant(payload) => Some(payload),
                        // Unreachable, and so warned about, if the list above
                        // ever shrinks to a single variant.
                        #[allow(unreachable_patterns)]
                        _ => None,
                    }
                }
            )+
        }

        $(
            impl From<$payload> for AnyTrackEvent {
                fn from(payload: $payload) -> Self {
                    Self::$variant(payload)
                }
            }
        )+
    };
}

define_any_track_event! {
    /// The track entered the library.
    Added(Added) => as_added,
    /// The listener cut the track short.
    Skipped(Skipped) => as_skipped,
    /// The listener ended playback while the track was playing.
    Stopped(Stopped) => as_stopped,
    /// The backend stopped playing the track on its own.
    PlaybackOutcome(PlaybackOutcome) => as_playback_outcome,
}

/// One track event and the time it was recorded at.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeTaggedTrackEvent {
    /// When the event was recorded.
    pub at: Time,
    /// The event.
    pub event: AnyTrackEvent,
}

impl TimeTaggedTrackEvent {
    /// An event recorded at `at`.
    pub fn new(at: Time, event: impl Into<AnyTrackEvent>) -> Self {
        Self {
            at,
            event: event.into(),
        }
    }
}

/// The track entered the library.
///
/// Carries no payload. The timestamp is on the [`TimeTaggedTrackEvent`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Added {}

impl Added {
    /// The event.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

/// The listener cut the track short.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Skipped {
    /// How long the track had been playing.
    pub listened_for: Duration,
}

impl Skipped {
    /// A skip after `listened_for` of playback.
    #[must_use]
    pub const fn new(listened_for: Duration) -> Self {
        Self { listened_for }
    }
}

/// The listener ended playback while the track was playing.
///
/// Distinct from [`Skipped`]: the listener left, rather than moved on from this
/// particular track.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stopped {
    /// How long the track had been playing.
    pub listened_for: Duration,
}

impl Stopped {
    /// A stop after `listened_for` of playback.
    #[must_use]
    pub const fn new(listened_for: Duration) -> Self {
        Self { listened_for }
    }
}

/// How a backend stopped playing a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "outcome", content = "reason", rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum PlaybackOutcome {
    /// The track played to its end.
    Finished,
    /// The track could not be played to its end.
    Failed(TrackFailure),
}

impl PlaybackOutcome {
    /// Whether the track played to its end.
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Finished)
    }

    /// Why playback stopped early, or [`None`] if the track finished.
    #[must_use]
    pub const fn failure(self) -> Option<TrackFailure> {
        match self {
            Self::Failed(failure) => Some(failure),
            Self::Finished => None,
        }
    }
}

/// Why a backend could not play a track to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum TrackFailure {
    /// The file does not exist.
    FileNotFound,
    /// The backend does not understand the container or codec.
    UnsupportedFormat,
    /// The backend understood the file but failed while decoding it.
    DecoderError,
    /// The backend itself stopped running.
    BackendExited,
}

impl TrackFailure {
    /// Every variant this version defines, in no particular order.
    ///
    /// [`TrackFailure`] is `#[non_exhaustive]`, so code outside this crate
    /// cannot write the list out for itself. A later release may add to it, so
    /// iterate it rather than relying on its length.
    pub const ALL: &'static [Self] = &[
        Self::FileNotFound,
        Self::UnsupportedFormat,
        Self::DecoderError,
        Self::BackendExited,
    ];
}

impl From<TrackFailure> for PlaybackOutcome {
    fn from(failure: TrackFailure) -> Self {
        Self::Failed(failure)
    }
}

/// The events recorded against one track, in the order they were recorded.
///
/// Recording order, not timestamp order, so a clock that jumps backwards
/// remains visible rather than being silently reordered.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TimeTaggedTrackEvents {
    events: Vec<TimeTaggedTrackEvent>,
}

impl TimeTaggedTrackEvents {
    /// An empty collection.
    #[must_use]
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Appends an event recorded at `at`.
    pub fn record(&mut self, at: Time, event: impl Into<AnyTrackEvent>) {
        self.events.push(TimeTaggedTrackEvent::new(at, event));
    }

    /// How many events are recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether no event is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The events, in the order they were recorded.
    pub fn iter(&self) -> core::slice::Iter<'_, TimeTaggedTrackEvent> {
        self.events.iter()
    }

    /// The events as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[TimeTaggedTrackEvent] {
        &self.events
    }
}

impl FromIterator<TimeTaggedTrackEvent> for TimeTaggedTrackEvents {
    fn from_iter<I: IntoIterator<Item = TimeTaggedTrackEvent>>(events: I) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }
}

impl Extend<TimeTaggedTrackEvent> for TimeTaggedTrackEvents {
    fn extend<I: IntoIterator<Item = TimeTaggedTrackEvent>>(&mut self, events: I) {
        self.events.extend(events);
    }
}

impl IntoIterator for TimeTaggedTrackEvents {
    type Item = TimeTaggedTrackEvent;
    type IntoIter = std::vec::IntoIter<TimeTaggedTrackEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

impl<'collection> IntoIterator for &'collection TimeTaggedTrackEvents {
    type Item = &'collection TimeTaggedTrackEvent;
    type IntoIter = core::slice::Iter<'collection, TimeTaggedTrackEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::Added;
    use super::AnyTrackEvent;
    use super::PlaybackOutcome;
    use super::Skipped;
    use super::Stopped;
    use super::TimeTaggedTrackEvent;
    use super::TimeTaggedTrackEvents;
    use super::TrackFailure;
    use crate::time::Time;
    use crate::time::at;
    use crate::time::span;
    use std::collections::HashSet;

    fn added(seconds: u64) -> TimeTaggedTrackEvent {
        TimeTaggedTrackEvent::new(at(seconds), Added::new())
    }

    fn skipped(seconds: u64, listened_for: u64) -> TimeTaggedTrackEvent {
        TimeTaggedTrackEvent::new(at(seconds), Skipped::new(span(listened_for)))
    }

    fn stopped(seconds: u64, listened_for: u64) -> TimeTaggedTrackEvent {
        TimeTaggedTrackEvent::new(at(seconds), Stopped::new(span(listened_for)))
    }

    fn finished(seconds: u64) -> TimeTaggedTrackEvent {
        TimeTaggedTrackEvent::new(at(seconds), PlaybackOutcome::Finished)
    }

    fn failed(seconds: u64, reason: TrackFailure) -> TimeTaggedTrackEvent {
        TimeTaggedTrackEvent::new(at(seconds), PlaybackOutcome::from(reason))
    }

    /// Every failure this crate defines.
    fn every_failure() -> Vec<TrackFailure> {
        TrackFailure::ALL.to_vec()
    }

    /// A variant added to [`TrackFailure`] but left out of
    /// [`TrackFailure::ALL`] would silently stop being covered by everything
    /// that iterates the list.
    ///
    /// The match below is exhaustive, and `#[non_exhaustive]` does not apply
    /// inside the declaring crate, so a new variant fails to compile here until
    /// it is added to the list as well.
    #[test]
    fn every_reason_there_is_appears_in_the_list() {
        for reason in TrackFailure::ALL {
            match reason {
                TrackFailure::FileNotFound
                | TrackFailure::UnsupportedFormat
                | TrackFailure::DecoderError
                | TrackFailure::BackendExited => {}
            }
        }
        assert_eq!(
            TrackFailure::ALL.iter().collect::<HashSet<_>>().len(),
            TrackFailure::ALL.len(),
            "a reason is listed twice"
        );
    }

    /// One event of every variant, each at a distinct time.
    fn every_event() -> Vec<TimeTaggedTrackEvent> {
        let mut events = vec![added(1), skipped(2, 3), stopped(4, 5), finished(6)];
        for (number, reason) in every_failure().into_iter().enumerate() {
            events.push(failed(7 + number as u64, reason));
        }
        events
    }

    fn history(events: impl IntoIterator<Item = TimeTaggedTrackEvent>) -> TimeTaggedTrackEvents {
        events.into_iter().collect()
    }

    /// One test per accessor, asserting it matches its own event and no other.
    ///
    /// Each generated test walks the whole table, so a new row is checked
    /// against every existing accessor and they against it.
    macro_rules! accessors {
        ($($name:ident: $event:expr => $accessor:ident;)+) => {
            fn tabled_events() -> Vec<AnyTrackEvent> {
                vec![$($event.event),+]
            }

            $(
                #[test]
                fn $name() {
                    let answers = |event: &AnyTrackEvent| event.$accessor().is_some();
                    let own = $event.event;
                    assert!(answers(&own), "an accessor should answer for its own event");
                    for other in tabled_events() {
                        let is_own =
                            std::mem::discriminant(&own) == std::mem::discriminant(&other);
                        assert_eq!(answers(&other), is_own, "wrong answer for {other:?}");
                    }
                }
            )+
        };
    }

    accessors! {
        the_added_accessor_answers_for_added: added(1) => as_added;
        the_skipped_accessor_answers_for_skipped: skipped(2, 3) => as_skipped;
        the_stopped_accessor_answers_for_stopped: stopped(4, 5) => as_stopped;
        the_outcome_accessor_answers_for_an_outcome: finished(6) => as_playback_outcome;
    }

    #[test]
    fn an_event_keeps_the_moment_it_happened_and_what_it_carried() {
        assert_eq!(added(7).at, at(7));
        assert_eq!(added(1).event.as_added(), Some(&Added::new()));
        assert_eq!(
            skipped(9, 4)
                .event
                .as_skipped()
                .map(|skip| skip.listened_for),
            Some(span(4))
        );
        assert_eq!(
            stopped(9, 4)
                .event
                .as_stopped()
                .map(|stop| stop.listened_for),
            Some(span(4))
        );
    }

    #[test]
    fn a_payload_converts_into_the_wrapper_that_carries_it() {
        assert_eq!(AnyTrackEvent::from(Added::new()), added(1).event);
        assert_eq!(
            PlaybackOutcome::from(TrackFailure::FileNotFound),
            PlaybackOutcome::Failed(TrackFailure::FileNotFound)
        );
    }

    #[test]
    fn an_outcome_reports_whether_it_finished_and_why_not() {
        assert!(PlaybackOutcome::Finished.is_finished());
        assert_eq!(PlaybackOutcome::Finished.failure(), None);

        let failed = PlaybackOutcome::from(TrackFailure::DecoderError);
        assert!(!failed.is_finished());
        assert_eq!(failed.failure(), Some(TrackFailure::DecoderError));
    }

    #[test]
    fn a_history_counts_the_events_it_was_given() {
        assert_eq!(history([]).len(), 0);
        assert!(history([]).is_empty());
        assert!(!history([finished(1)]).is_empty());
        assert_eq!(history(every_event()).len(), every_event().len());
    }

    #[test]
    fn no_failure_counts_as_finishing() {
        for reason in every_failure() {
            let outcome = PlaybackOutcome::from(reason);
            assert!(
                !outcome.is_finished(),
                "{reason:?} should not count as finished"
            );
            assert_eq!(outcome.failure(), Some(reason));
        }
    }

    #[test]
    fn a_history_returns_what_was_recorded_in_the_order_it_arrived() {
        let recorded = every_event();
        let mut events = TimeTaggedTrackEvents::new();
        for event in recorded.clone() {
            events.record(event.at, event.event);
        }
        assert_eq!(
            events
                .iter()
                .cloned()
                .collect::<Vec<TimeTaggedTrackEvent>>(),
            recorded
        );
        assert_eq!(events.as_slice(), recorded.as_slice());
        assert_eq!(
            events
                .clone()
                .into_iter()
                .collect::<Vec<TimeTaggedTrackEvent>>(),
            recorded
        );
        assert_eq!((&events).into_iter().count(), recorded.len());
    }

    /// Events keep their recording order regardless of their timestamps.
    #[test]
    fn a_history_keeps_events_that_arrive_out_of_order_as_they_arrived() {
        let events = history([finished(9), skipped(2, 1)]);
        let moments: Vec<Time> = events.iter().map(|event| event.at).collect();
        assert_eq!(moments, [at(9), at(2)]);
    }

    #[test]
    fn a_history_takes_payloads_as_well_as_events() {
        let mut events = TimeTaggedTrackEvents::new();
        events.record(at(1), Added::new());
        events.record(at(2), Skipped::new(span(3)));
        events.record(at(3), PlaybackOutcome::Finished);
        assert_eq!(events, history([added(1), skipped(2, 3), finished(3)]));
    }

    #[test]
    fn a_history_can_be_extended_and_collected_the_same_way() {
        let mut extended = history([added(1)]);
        extended.extend(vec![finished(2)]);
        assert_eq!(extended, history([added(1), finished(2)]));
    }

    #[test]
    fn events_compare_by_what_they_hold_and_when() {
        assert_eq!(added(1), added(1));
        assert_ne!(added(1), added(2));
        assert_ne!(added(1), finished(1));
        assert_ne!(skipped(1, 2), skipped(1, 3));
        assert_ne!(
            failed(1, TrackFailure::DecoderError),
            failed(1, TrackFailure::FileNotFound)
        );
    }

    /// The encoding, shown in JSON for readability. The rules hold in any
    /// self-describing format.
    ///
    /// These tests state what this crate guarantees. Breaking one breaks data
    /// already written, so they are only added to.
    #[cfg(feature = "serde")]
    mod encoding {
        use super::PlaybackOutcome;
        use super::TimeTaggedTrackEvent;
        use super::TimeTaggedTrackEvents;
        use super::TrackFailure;
        use super::added;
        use super::at;
        use super::every_event;
        use super::every_failure;
        use super::failed;
        use super::finished;
        use super::history;
        use super::skipped;
        use super::stopped;
        use crate::time::Duration;
        use crate::time::Time;
        use serde::Serialize;
        use serde::de::DeserializeOwned;

        fn encode<T: Serialize>(value: &T) -> String {
            serde_json::to_string(value).expect("the value encodes")
        }

        fn decode<T: DeserializeOwned>(encoded: &str) -> T {
            serde_json::from_str(encoded).expect("the text decodes")
        }

        fn try_decode<T: DeserializeOwned>(encoded: &str) -> Result<T, serde_json::Error> {
            serde_json::from_str(encoded)
        }

        /// Asserts a value encodes to exactly `encoded`, and reads back as
        /// itself.
        fn assert_encodes<T>(value: &T, encoded: &str)
        where
            T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
        {
            assert_eq!(encode(value), encoded, "the encoding changed");
            assert_eq!(&decode::<T>(encoded), value, "the text no longer decodes");
        }

        /// One test per `value => text` row.
        macro_rules! encodings {
            ($($name:ident: $value:expr => $encoded:expr;)+) => {
                $(
                    #[test]
                    fn $name() {
                        assert_encodes(&$value, $encoded);
                    }
                )+
            };
        }

        // Times and durations encode as a bare count of milliseconds, and
        // failure reasons as their own names.
        encodings! {
            a_point_in_time_encodes_as_milliseconds:
                Time::from_unix_milliseconds(1_500) => "1500";
            the_epoch_encodes_as_zero: Time::EPOCH => "0";
            a_span_encodes_as_milliseconds: Duration::from_seconds(3) => "3000";
            an_unsupported_format_keeps_its_name:
                TrackFailure::UnsupportedFormat => r#""unsupported_format""#;
            a_decoder_error_keeps_its_name:
                TrackFailure::DecoderError => r#""decoder_error""#;
            a_backend_exit_keeps_its_name:
                TrackFailure::BackendExited => r#""backend_exited""#;
        }

        // An event pairs a timestamp with a tagged payload; a history is a bare
        // array of them, with nothing wrapped around it.
        encodings! {
            an_event_pairs_a_moment_with_what_happened: added(1) =>
                r#"{"at":1000,"event":{"event":"added","data":{}}}"#;
            a_skip_carries_only_what_was_heard: skipped(2, 3) =>
                r#"{"at":2000,"event":{"event":"skipped","data":{"listened_for":3000}}}"#;
            a_stop_carries_only_what_was_heard: stopped(2, 3) =>
                r#"{"at":2000,"event":{"event":"stopped","data":{"listened_for":3000}}}"#;
            a_finished_outcome_encodes_tagged: finished(4) =>
                r#"{"at":4000,"event":{"event":"playback_outcome","data":{"outcome":"finished"}}}"#;
            a_failure_encodes_with_its_reason: failed(5, TrackFailure::FileNotFound) =>
                r#"{"at":5000,"event":{"event":"playback_outcome","data":{"outcome":"failed","reason":"file_not_found"}}}"#;
            an_empty_history_encodes_as_an_empty_array: history([]) => "[]";
            a_history_encodes_as_a_bare_array: history([finished(4)]) =>
                r#"[{"at":4000,"event":{"event":"playback_outcome","data":{"outcome":"finished"}}}]"#;
        }

        #[test]
        fn everything_the_crate_can_produce_survives_a_round_trip() {
            let events = history(every_event());
            assert_eq!(decode::<TimeTaggedTrackEvents>(&encode(&events)), events);
        }

        #[test]
        fn every_failure_survives_a_round_trip_on_its_own() {
            for reason in every_failure() {
                let outcome = PlaybackOutcome::from(reason);
                assert_eq!(decode::<PlaybackOutcome>(&encode(&outcome)), outcome);
            }
        }

        #[test]
        fn a_field_added_to_a_payload_later_is_ignored_by_this_version() {
            let encoded =
                r#"{"at":7000,"event":{"event":"added","data":{"source":"a later scan"}}}"#;
            assert_eq!(decode::<TimeTaggedTrackEvent>(encoded), added(7));
        }

        #[test]
        fn an_event_from_a_later_version_is_reported_rather_than_guessed_at() {
            let encoded = r#"[{"at":1,"event":{"event":"invented_later","data":{}}}]"#;
            assert!(try_decode::<TimeTaggedTrackEvents>(encoded).is_err());
        }

        #[test]
        fn a_failure_reason_from_a_later_version_is_reported_rather_than_guessed_at() {
            let encoded = r#"{"outcome": "failed", "reason": "network_timeout"}"#;
            assert!(try_decode::<PlaybackOutcome>(encoded).is_err());
        }

        #[test]
        fn an_outcome_from_a_later_version_is_reported_rather_than_guessed_at() {
            assert!(try_decode::<PlaybackOutcome>(r#"{"outcome": "interrupted"}"#).is_err());
        }

        #[test]
        fn an_event_without_a_moment_is_reported_rather_than_filled_in() {
            let encoded =
                r#"[{"event":{"event":"playback_outcome","data":{"outcome":"finished"}}}]"#;
            assert!(try_decode::<TimeTaggedTrackEvents>(encoded).is_err());
        }

        #[test]
        fn a_payload_missing_a_field_is_reported_rather_than_filled_in() {
            let encoded = r#"[{"at":1,"event":{"event":"skipped","data":{}}}]"#;
            assert!(try_decode::<TimeTaggedTrackEvents>(encoded).is_err());
        }

        #[test]
        fn text_that_is_not_the_shape_of_an_event_is_reported() {
            for encoded in [
                "[3]",
                r#"["added"]"#,
                r#"[{"at":1}]"#,
                r#"{"at":1,"event":{"event":"added","data":{}}}"#,
            ] {
                assert!(
                    try_decode::<TimeTaggedTrackEvents>(encoded).is_err(),
                    "{encoded} should not decode as a history"
                );
            }
        }

        #[test]
        fn a_moment_at_the_end_of_the_range_survives_a_round_trip() {
            let latest = Time::from_unix_milliseconds(u64::MAX);
            assert_eq!(decode::<Time>(&encode(&latest)), latest);
            assert_eq!(decode::<Duration>(&encode(&Duration::MAX)), Duration::MAX);
        }

        #[test]
        fn a_moment_before_the_epoch_is_reported() {
            assert!(try_decode::<Time>("-1").is_err());
        }

        #[test]
        fn the_moment_is_read_back_as_it_was_written() {
            let event = decode::<TimeTaggedTrackEvent>(&encode(&skipped(1_234, 5)));
            assert_eq!(event.at, at(1_234));
        }
    }
}
