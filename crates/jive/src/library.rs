//! The tracks currently in play, and the events recorded against them.
//!
//! A [`Library`] is built once per session from the tracks discovered on disk
//! and the [`History`] stored for them. It caches each track's rating evidence
//! and last-played time, so scoring a draw does not walk every event again.

use crate::offer;
use crate::rating;
use jive_core::History;
use jive_core::Time;
use jive_core::TrackId;
use jive_core::TrackIds;
use jive_core::TrackName;
use jive_core::TrackNames;
use jive_core::track_events::Added;
use jive_core::track_events::AnyTrackEvent;
use jive_core::track_events::TimeTaggedTrackEvents;
use jive_filesystem::DiscoveredTrack;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

/// A borrowed view of one track of a library.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackView<'library> {
    /// The identifier of the track.
    pub identifier: TrackId,
    /// The name to display.
    pub name: &'library TrackName,
    /// The absolute path of the file.
    pub path: &'library Path,
    /// The events recorded against the track, in recording order.
    pub events: &'library TimeTaggedTrackEvents,
}

#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    events: TimeTaggedTrackEvents,
    evidence: rating::Evidence,
    last_played: Option<Time>,
}

/// The tracks currently in play, and the events recorded against them.
///
/// Built once per session from the tracks found on disk and the [`History`]
/// stored for them. Each track's rating evidence and last-played time are
/// cached, so scoring a draw does not walk every event again.
#[derive(Debug, Clone, Default)]
pub struct Library {
    identifiers: TrackIds,
    names: TrackNames,
    entries: HashMap<TrackId, Entry>,
}

impl Library {
    /// A library of the tracks in `discovered`, carrying the events `history`
    /// holds for each.
    ///
    /// A track absent from `history` is recorded as [`Added`] at `now`.
    #[must_use]
    pub fn build(discovered: Vec<DiscoveredTrack>, history: &History, now: Time) -> Self {
        let mut library = Self::default();
        for track in discovered {
            library.insert(track, history, now);
        }
        library
    }

    fn insert(&mut self, track: DiscoveredTrack, history: &History, now: Time) {
        let identifier = track.identifier;
        self.identifiers.push(identifier);
        self.names.insert(identifier, track.name);
        let events = history.events_for(identifier).cloned().unwrap_or_else(|| {
            let mut fresh = TimeTaggedTrackEvents::new();
            fresh.record(now, Added::new());
            fresh
        });
        let evidence = rating::evidence_of(&events);
        let last_played = offer::last_offered_at(&events);
        self.entries.insert(
            identifier,
            Entry {
                path: track.path,
                events,
                evidence,
                last_played,
            },
        );
    }

    /// The identifiers of every track, in discovery order.
    #[must_use]
    pub fn identifiers(&self) -> &TrackIds {
        &self.identifiers
    }

    /// How many tracks the library holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.identifiers.len()
    }

    /// Whether the library is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.identifiers.is_empty()
    }

    /// The name a track is displayed under.
    #[must_use]
    pub fn name(&self, identifier: TrackId) -> Option<&TrackName> {
        self.names.get(identifier)
    }

    /// The path of a track's file.
    #[must_use]
    pub fn path(&self, identifier: TrackId) -> Option<&Path> {
        self.entries
            .get(&identifier)
            .map(|entry| entry.path.as_path())
    }

    /// The events recorded against a track.
    #[must_use]
    pub fn events(&self, identifier: TrackId) -> Option<&TimeTaggedTrackEvents> {
        self.entries.get(&identifier).map(|entry| &entry.events)
    }

    /// The measured listener preference for a track, or the starting preference
    /// for one the library does not hold.
    #[must_use]
    pub fn preference(&self, identifier: TrackId) -> f64 {
        self.entries
            .get(&identifier)
            .map_or(rating::STARTING_PREFERENCE, |entry| {
                rating::preference(entry.evidence)
            })
    }

    /// The independent factors a track is scored from.
    #[must_use]
    pub(crate) fn factors(&self, identifier: TrackId, now: Time) -> offer::Factors {
        self.entries.get(&identifier).map_or_else(
            || offer::factors(rating::Evidence::default(), None, now),
            |entry| offer::factors(entry.evidence, entry.last_played, now),
        )
    }

    /// When a track was last played.
    #[must_use]
    pub fn last_played(&self, identifier: TrackId) -> Option<Time> {
        self.entries
            .get(&identifier)
            .and_then(|entry| entry.last_played)
    }

    /// Records an event against a track at `at`.
    ///
    /// An identifier the library does not hold is ignored.
    pub fn record(&mut self, identifier: TrackId, at: Time, event: impl Into<AnyTrackEvent>) {
        if let Some(entry) = self.entries.get_mut(&identifier) {
            let event = event.into();
            entry.evidence.observe(&event);
            if event.as_added().is_none() {
                entry.last_played = Some(entry.last_played.map_or(at, |last| last.max(at)));
            }
            entry.events.record(at, event);
        }
    }

    /// The tracks, in discovery order.
    pub(crate) fn tracks(&self) -> impl Iterator<Item = TrackView<'_>> + '_ {
        self.identifiers
            .iter()
            .filter_map(|identifier| self.track(identifier))
    }

    /// Writes every track's events into `history`, replacing what it held for
    /// those tracks and leaving every other track untouched.
    pub fn store_into(&self, history: &mut History) {
        for track in self.tracks() {
            history.store(track.identifier, track.events.clone());
        }
    }

    /// A view of one track.
    #[must_use]
    pub(crate) fn track(&self, identifier: TrackId) -> Option<TrackView<'_>> {
        let entry = self.entries.get(&identifier)?;
        Some(TrackView {
            identifier,
            name: self.names.get(identifier)?,
            path: entry.path.as_path(),
            events: &entry.events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Library;
    use crate::rating::Evidence;
    use crate::rating::MAXIMUM_PREFERENCE;
    use crate::rating::STARTING_PREFERENCE;
    use crate::rating::preference;
    use crate::testing::assert_close;
    use crate::testing::discovered;
    use crate::testing::discovered_as;
    use crate::testing::events_of;
    use crate::testing::finished;
    use crate::testing::finishes_at;
    use crate::testing::identifier_named;
    use crate::testing::identifiers_of;
    use crate::testing::library_of;
    use crate::testing::library_where_every_track;
    use crate::testing::library_with_history;
    use crate::testing::names_of;
    use crate::testing::path_of;
    use crate::testing::quick_skip;
    use crate::testing::repeated;
    use crate::testing::stranger;
    use jive_core::History;
    use jive_core::Time;
    use jive_core::TrackId;
    use jive_core::track_events::TimeTaggedTrackEvent;
    use jive_core::track_events::TimeTaggedTrackEvents;

    fn first_of(library: &Library) -> TrackId {
        identifiers_of(library).first().copied().expect("a track")
    }

    /// The measured preference for the first track of a library.
    fn preference_of(library: &Library) -> f64 {
        library.preference(first_of(library))
    }

    fn added_events_in(library: &Library, name: &str) -> usize {
        events_of(library, name)
            .iter()
            .filter_map(|event| event.event.as_added())
            .count()
    }

    /// How many events a library holds against one of its tracks.
    fn events_against(library: &Library, identifier: TrackId) -> Option<usize> {
        library.events(identifier).map(TimeTaggedTrackEvents::len)
    }

    /// `history` with the library written into it.
    fn stored(library: &Library, mut history: History) -> History {
        library.store_into(&mut history);
        history
    }

    /// How many events a history holds against one identifier.
    fn events_stored_for(history: &History, number: u32) -> Option<usize> {
        history
            .events_for(TrackId::new(number))
            .map(TimeTaggedTrackEvents::len)
    }

    #[test]
    fn an_empty_library_holds_nothing() {
        let library = library_of(&[]);
        assert_eq!(library.len(), 0);
        assert!(library.is_empty());
        assert_eq!(library.tracks().count(), 0);
    }

    #[test]
    fn a_library_keeps_the_tracks_it_discovered_in_the_order_it_found_them() {
        let library = library_of(&["first", "second"]);
        assert_eq!(library.len(), 2);
        assert!(!library.is_empty());
        assert_eq!(names_of(&library), ["first", "second"]);
        assert_eq!(
            library_of(&["same", "same"]).len(),
            2,
            "tracks sharing a name are still two tracks"
        );
    }

    #[test]
    fn a_track_keeps_the_path_and_name_it_was_found_with() {
        let library = library_of(&["one"]);
        let identifier = first_of(&library);
        assert_eq!(library.path(identifier), Some(path_of("one").as_path()));
        assert_eq!(
            library.name(identifier).map(ToString::to_string),
            Some(String::from("one"))
        );
    }

    #[test]
    fn a_new_track_is_recorded_as_seen_once() {
        assert_eq!(added_events_in(&library_of(&["fresh"]), "fresh"), 1);
    }

    /// A track already carrying events was seen on an earlier run, so recording
    /// `Added` again would count one discovery per startup.
    #[test]
    fn a_track_already_stored_keeps_what_it_had_and_is_not_seen_again() {
        let library = library_with_history(&[("known", vec![finished()])]);
        assert_eq!(added_events_in(&library, "known"), 0);
        assert_eq!(
            events_of(&library, "known"),
            [TimeTaggedTrackEvent::new(Time::EPOCH, finished())]
        );
    }

    #[test]
    fn recorded_events_move_a_preference_off_the_starting_one() {
        let liked = library_where_every_track(&["liked"], &[finished()]);
        let disliked = library_where_every_track(&["disliked"], &[quick_skip()]);
        assert!(preference_of(&liked) > STARTING_PREFERENCE);
        assert!(preference_of(&disliked) < STARTING_PREFERENCE);
    }

    #[test]
    fn a_track_with_no_events_has_the_starting_preference() {
        assert_close(preference_of(&library_of(&["one"])), STARTING_PREFERENCE);
    }

    #[test]
    fn recording_an_event_moves_the_preference() {
        let mut library = library_of(&["one"]);
        let identifier = first_of(&library);
        assert_close(library.preference(identifier), STARTING_PREFERENCE);

        finishes_at(&mut library, identifier, Time::EPOCH);

        assert_close(
            library.preference(identifier),
            preference(Evidence {
                finishes: 1,
                quick_skips: 0,
                failures: 0,
            }),
        );
    }

    #[test]
    fn every_track_of_a_library_resolves_to_itself() {
        let library = library_of(&["one", "two", "three"]);
        for track in library.tracks() {
            assert_eq!(library.name(track.identifier), Some(track.name));
            assert_eq!(library.path(track.identifier), Some(track.path));
            assert_eq!(
                events_against(&library, track.identifier),
                Some(track.events.len())
            );
        }
    }

    #[test]
    fn a_library_of_the_same_tracks_twice_over_keeps_them_apart() {
        let mut library = library_of(&["same", "same"]);
        let identifiers = identifiers_of(&library);

        finishes_at(&mut library, identifiers[0], Time::EPOCH);

        assert_eq!(events_against(&library, identifiers[0]), Some(2));
        assert_eq!(events_against(&library, identifiers[1]), Some(1));
    }

    #[test]
    fn a_history_about_other_tracks_is_left_alone() {
        let mut history = History::new();
        history.record(stranger(), Time::EPOCH, finished());
        let library = Library::build(discovered(&["here"]), &history, Time::EPOCH);
        assert_eq!(library.len(), 1);
        assert_eq!(added_events_in(&library, "here"), 1);
    }

    #[test]
    fn a_long_history_is_carried_over_whole() {
        let events = repeated(&finished(), 300);
        let library = library_where_every_track(&["well loved"], &events);
        assert_eq!(events_of(&library, "well loved").len(), 300);
        assert_close(
            library.preference(identifier_named(&library, "well loved")),
            MAXIMUM_PREFERENCE,
        );
    }

    #[test]
    fn a_large_library_holds_every_track_it_was_given() {
        let names: Vec<String> = (0..1_000).map(|number| format!("track {number}")).collect();
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        let library = library_of(&borrowed);
        assert_eq!(library.len(), 1_000);
        assert_eq!(library.tracks().count(), 1_000);
        assert_eq!(names_of(&library), names);
    }

    /// Two libraries drawn from one catalog never share an identifier, so
    /// writing one into a history leaves the other's tracks unchanged.
    #[test]
    fn storing_a_library_leaves_the_tracks_of_another_alone() {
        let first = stored(
            &library_where_every_track(&["one"], &[finished()]),
            History::new(),
        );
        let elsewhere = Library::build(vec![discovered_as(7, "two")], &first, Time::EPOCH);

        let history = stored(&elsewhere, first);

        assert_eq!(history.len(), 2);
        assert_eq!(events_stored_for(&history, 0), Some(1));
    }

    #[test]
    fn storing_the_same_library_again_replaces_what_it_held() {
        let first = stored(&library_of(&["one"]), History::new());

        let history = stored(
            &library_where_every_track(&["one"], &repeated(&finished(), 3)),
            first,
        );

        assert_eq!(history.len(), 1);
        assert_eq!(
            events_stored_for(&history, 0),
            Some(3),
            "the second library should replace the first rather than add to it"
        );
    }

    /// A track no longer in the directory belongs to no library, so nothing
    /// overwrites its identifier and its events are retained.
    #[test]
    fn a_track_that_has_gone_from_the_directory_is_still_stored() {
        let whole = library_where_every_track(&["kept", "removed"], &[finished()]);
        let removed = identifier_named(&whole, "removed");
        let before = stored(&whole, History::new());

        let history = stored(&library_where_every_track(&["kept"], &[finished()]), before);

        assert_eq!(history.len(), 2);
        assert!(history.events_for(removed).is_some());
    }
}
