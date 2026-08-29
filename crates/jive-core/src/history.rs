//! The events recorded against every track.
//!
//! A [`History`] holds the events of every track ever played, keyed by the
//! [`TrackId`] each was assigned. Resolving an identifier to a file, and
//! persisting any of this, is the caller's work.
//!
//! Three methods write to a history, each matching what a caller typically
//! holds:
//!
//! * [`History::record`] appends a single event, and is the usual choice.
//! * [`History::events_for_mut`] borrows a track's events for a caller doing
//!   more than appending.
//! * [`History::store`] replaces everything held against a track, which is what
//!   a reader restoring a whole history needs.

use crate::Time;
use crate::TrackId;
use crate::track_events::AnyTrackEvent;
use crate::track_events::TimeTaggedTrackEvents;
use std::collections::BTreeMap;

/// The events recorded against every track.
///
/// Tracks are held in identifier order, so writing one out twice gives the same
/// text both times.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    tracks: BTreeMap<TrackId, TimeTaggedTrackEvents>,
}

impl History {
    /// An empty history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracks: BTreeMap::new(),
        }
    }

    /// The events stored against a track.
    #[must_use]
    pub fn events_for(&self, identifier: TrackId) -> Option<&TimeTaggedTrackEvents> {
        self.tracks.get(&identifier)
    }

    /// The events stored against a track, for modification.
    ///
    /// [`None`] when nothing is stored against it yet, as with
    /// [`History::events_for`]. Use [`History::record`] to add to a track that
    /// may not be present.
    pub fn events_for_mut(&mut self, identifier: TrackId) -> Option<&mut TimeTaggedTrackEvents> {
        self.tracks.get_mut(&identifier)
    }

    /// Appends one event to a track, keeping everything already stored against
    /// it.
    ///
    /// The track need not be present: an empty entry is created for it, so a
    /// caller need not distinguish the first event from later ones.
    pub fn record(&mut self, identifier: TrackId, at: Time, event: impl Into<AnyTrackEvent>) {
        self.tracks.entry(identifier).or_default().record(at, event);
    }

    /// Stores the events of a track, replacing whatever was held against its
    /// identifier.
    ///
    /// For a caller holding a track's complete event list: one restoring a
    /// history it serialized earlier, or one returning events it obtained from
    /// here. To append instead, use [`History::record`].
    pub fn store(&mut self, identifier: TrackId, events: TimeTaggedTrackEvents) {
        self.tracks.insert(identifier, events);
    }

    /// Every track stored, in identifier order.
    pub fn tracks(&self) -> impl Iterator<Item = (TrackId, &TimeTaggedTrackEvents)> + '_ {
        self.tracks
            .iter()
            .map(|(identifier, events)| (*identifier, events))
    }

    /// How many tracks are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether no track is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::History;
    use crate::Time;
    use crate::TrackId;
    use crate::track_events::PlaybackOutcome;
    use crate::track_events::TimeTaggedTrackEvents;

    /// One finish, recorded at the epoch.
    fn finished() -> TimeTaggedTrackEvents {
        let mut events = TimeTaggedTrackEvents::new();
        events.record(Time::EPOCH, PlaybackOutcome::Finished);
        events
    }

    /// A history storing each of `numbers`, each having finished once.
    fn storing(numbers: &[u32]) -> History {
        let mut history = History::new();
        for number in numbers {
            history.store(TrackId::new(*number), finished());
        }
        history
    }

    /// The same history, with one more finish recorded against `number`.
    fn recording_against(number: u32, mut history: History) -> History {
        history.record(TrackId::new(number), Time::EPOCH, PlaybackOutcome::Finished);
        history
    }

    /// The same history, with one more finish appended through
    /// [`History::events_for_mut`].
    fn working_on(number: u32, mut history: History) -> History {
        history
            .events_for_mut(TrackId::new(number))
            .expect("a track that is stored")
            .record(Time::EPOCH, PlaybackOutcome::Finished);
        history
    }

    /// The same history, with the events against `number` replaced by none.
    fn emptying(number: u32, mut history: History) -> History {
        history.store(TrackId::new(number), TimeTaggedTrackEvents::new());
        history
    }

    fn events_of(history: &History, number: u32) -> Option<&TimeTaggedTrackEvents> {
        history.events_for(TrackId::new(number))
    }

    fn count_of(history: &History, number: u32) -> Option<usize> {
        events_of(history, number).map(TimeTaggedTrackEvents::len)
    }

    #[test]
    fn a_fresh_history_holds_nothing_at_all() {
        let mut history = History::new();
        assert_eq!(history.len(), 0);
        assert!(history.is_empty());
        assert_eq!(events_of(&history, 0), None);
        assert!(history.events_for_mut(TrackId::new(0)).is_none());
    }

    #[test]
    fn what_is_stored_comes_back_against_its_own_track() {
        let history = storing(&[1, 2, 3]);
        assert_eq!(history.len(), 3);
        assert!(!history.is_empty());
        assert_eq!(events_of(&history, 3), Some(&finished()));
        assert_eq!(events_of(&history, 4), None, "a track never stored");
    }

    #[test]
    fn recording_adds_to_what_a_track_already_held() {
        let history = recording_against(1, storing(&[1, 2]));
        assert_eq!(count_of(&history, 1), Some(2));
        assert_eq!(
            events_of(&history, 2),
            Some(&finished()),
            "every other track should be untouched"
        );
    }

    #[test]
    fn recording_starts_a_track_that_has_nothing_stored() {
        let history = recording_against(7, History::new());
        assert_eq!(events_of(&history, 7), Some(&finished()));
        assert_eq!(history.len(), 1, "the track should have been added");
    }

    #[test]
    fn a_track_already_stored_can_be_worked_on_through_its_own_handle() {
        assert_eq!(count_of(&working_on(1, storing(&[1])), 1), Some(2));
    }

    #[test]
    fn storing_a_track_again_replaces_what_it_held() {
        let history = emptying(1, storing(&[1, 2]));
        assert_eq!(
            events_of(&history, 1),
            Some(&TimeTaggedTrackEvents::new()),
            "what the track held should have been thrown away"
        );
        assert_eq!(history.len(), 2, "no track should have been added");
        assert_eq!(
            events_of(&history, 2),
            Some(&finished()),
            "every other track should be untouched"
        );
    }

    /// Identifiers need not be contiguous or start at zero, so a history must
    /// hold whatever it is given and return it in identifier order.
    #[test]
    fn tracks_come_back_in_identifier_order_however_sparse() {
        let identifiers: Vec<u32> = storing(&[900, 2, 41])
            .tracks()
            .map(|(identifier, _)| identifier.number())
            .collect();
        assert_eq!(identifiers, [2, 41, 900]);
    }
}
