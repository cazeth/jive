//! Whole sessions, driven through the same pieces the binary uses.
//!
//! Real directories and real collection files, covering the seams the unit
//! tests stub out: discovery into the library, the library into the collection
//! file, and that file into the next session.

use jive::Library;
use jive::Player;
use jive::Shuffle;
use jive_core::AudioBackend;
use jive_core::BackendResult;
use jive_core::Duration;
use jive_core::Time;
use jive_core::track_events::AnyTrackEvent;
use jive_core::track_events::PlaybackOutcome;
use jive_core::track_events::TimeTaggedTrackEvent;
use jive_core::track_events::TrackFailure;
use jive_filesystem::Collection;
use jive_filesystem::CollectionFile;
use jive_filesystem::testing::add_track;
use jive_filesystem::testing::directory_holding;
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use tempfile::TempDir;

/// What a backend was asked to play, readable once the player owns it.
type PlayedTracks = Rc<RefCell<Vec<PathBuf>>>;

/// A backend reporting the same outcome for every track.
#[derive(Debug)]
struct Predictable {
    outcome: Option<PlaybackOutcome>,
    played: PlayedTracks,
}

impl Predictable {
    fn always(outcome: PlaybackOutcome) -> Self {
        Self {
            outcome: Some(outcome),
            played: PlayedTracks::default(),
        }
    }

    fn silent() -> Self {
        Self {
            outcome: None,
            played: PlayedTracks::default(),
        }
    }

    fn tracks_played(&self) -> PlayedTracks {
        Rc::clone(&self.played)
    }
}

impl AudioBackend for Predictable {
    fn play(&mut self, path: &Path) -> BackendResult<()> {
        self.played.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn stop(&mut self) -> BackendResult<()> {
        Ok(())
    }

    fn poll_event(&mut self) -> BackendResult<Option<PlaybackOutcome>> {
        Ok(self.outcome)
    }
}

/// A music directory and its collection file.
struct Session {
    music: TempDir,
    home: TempDir,
    clock: Time,
}

impl Session {
    fn holding(files: &[&str]) -> Self {
        Self {
            music: directory_holding(files),
            home: TempDir::new().expect("a collection directory"),
            clock: Time::EPOCH + Duration::from_seconds(1_000),
        }
    }

    fn collection_file(&self) -> CollectionFile {
        CollectionFile::at(self.home.path().join("jive").join("state.json"))
    }

    /// The stored collection, or an empty one for a first session over this
    /// music.
    fn stored(&self) -> Collection {
        self.collection_file()
            .load()
            .expect("the collection loads")
            .unwrap_or_else(|| Collection::new(self.music.path()))
    }

    /// Plays `rounds` tracks of everything below the root, then saves.
    fn play(&mut self, backend: Predictable, rounds: usize) -> Vec<PathBuf> {
        self.play_under(backend, None, rounds)
    }

    /// Plays `rounds` tracks from one directory below the root.
    fn play_below(&mut self, backend: Predictable, directory: &str, rounds: usize) {
        let directory = self.music.path().join(directory);
        self.play_under(backend, Some(&directory), rounds);
    }

    /// Plays `rounds` tracks, then saves, as a real session would.
    fn play_under(
        &mut self,
        backend: Predictable,
        directory: Option<&Path>,
        rounds: usize,
    ) -> Vec<PathBuf> {
        let played = backend.tracks_played();
        self.run(backend, directory, |clock, player| {
            for _ in 0..rounds {
                *clock += Duration::from_seconds(200);
                player.poll(*clock).expect("polling works");
            }
        });
        played.borrow().clone()
    }

    /// Plays one track, cut short after `seconds`.
    fn play_and_skip_after(&mut self, seconds: u64) {
        self.run(Predictable::silent(), None, |clock, player| {
            *clock += Duration::from_seconds(seconds);
            player.skip(*clock).expect("skipping works");
        });
    }

    /// Reads the collection, plays, and writes it back: the shape of every
    /// session the binary runs.
    fn run(
        &mut self,
        backend: Predictable,
        directory: Option<&Path>,
        drive: impl FnOnce(&mut Time, &mut Player<Predictable>),
    ) {
        let mut collection = self.stored();
        let tracks = collection
            .scan(directory)
            .expect("the directory can be read");
        let library = Library::build(tracks, collection.history(), self.clock);

        let mut player = Player::new(backend, library, Shuffle::seeded(4));
        player.start(self.clock).expect("playback starts");
        drive(&mut self.clock, &mut player);
        player.stop(self.clock).expect("stopping works");

        player.library().store_into(collection.history_mut());
        self.collection_file()
            .save(&collection)
            .expect("the collection is written");
    }

    /// Every event stored against one file below the music directory.
    fn remembered(&self, file: &str) -> Vec<TimeTaggedTrackEvent> {
        let collection = self.stored();
        let Some(identifier) = collection.catalog().identifier_for(Path::new(file)) else {
            return Vec::new();
        };
        collection
            .history()
            .events_for(identifier)
            .map(|events| events.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn remembered_tracks(&self) -> usize {
        self.stored().history().len()
    }
}

fn count_of(events: &[TimeTaggedTrackEvent], is: fn(&AnyTrackEvent) -> bool) -> usize {
    events.iter().filter(|event| is(&event.event)).count()
}

fn is_skip(event: &AnyTrackEvent) -> bool {
    event.as_skipped().is_some()
}

fn is_outcome(event: &AnyTrackEvent) -> bool {
    event.as_playback_outcome().is_some()
}

fn is_added(event: &AnyTrackEvent) -> bool {
    event.as_added().is_some()
}

/// How many times a session recorded that one file entered the library.
fn times_seen(session: &Session, file: &str) -> usize {
    count_of(&session.remembered(file), is_added)
}

#[test]
fn a_first_session_records_that_it_saw_every_track() {
    let mut session = Session::holding(&["one.mp3", "nested/two.flac"]);
    session.play(Predictable::silent(), 0);
    assert_eq!(session.remembered_tracks(), 2);
    assert_eq!(times_seen(&session, "one.mp3"), 1);
}

#[test]
fn what_happened_in_one_session_is_there_in_the_next() {
    const ROUNDS: usize = 4;

    let mut session = Session::holding(&["one.mp3", "two.mp3"]);
    session.play(Predictable::always(PlaybackOutcome::Finished), ROUNDS);
    let after_first = outcomes_remembered(&session);
    assert_eq!(after_first, ROUNDS, "each round should finish one track");

    session.play(Predictable::always(PlaybackOutcome::Finished), ROUNDS);
    assert_eq!(
        outcomes_remembered(&session),
        after_first + ROUNDS,
        "the second session should add to the first rather than replace it"
    );
    assert_eq!(session.remembered_tracks(), 2);
}

fn outcomes_remembered(session: &Session) -> usize {
    ["one.mp3", "two.mp3"]
        .iter()
        .map(|file| count_of(&session.remembered(file), is_outcome))
        .sum()
}

#[test]
fn a_track_is_seen_once_however_many_sessions_run() {
    let mut session = Session::holding(&["one.mp3"]);
    for _ in 0..4 {
        session.play(Predictable::always(PlaybackOutcome::Finished), 1);
    }
    assert_eq!(times_seen(&session, "one.mp3"), 1);
}

#[test]
fn cutting_a_track_short_is_remembered_across_sessions() {
    let mut session = Session::holding(&["one.mp3"]);
    session.play_and_skip_after(3);
    let events = session.remembered("one.mp3");
    assert_eq!(count_of(&events, is_skip), 1);
    assert_eq!(
        events
            .iter()
            .filter_map(|event| event.event.as_skipped())
            .map(|skip| skip.listened_for)
            .collect::<Vec<Duration>>(),
        [Duration::from_seconds(3)]
    );
}

#[test]
fn a_track_added_to_the_directory_later_joins_the_library() {
    let mut session = Session::holding(&["one.mp3"]);
    session.play(Predictable::always(PlaybackOutcome::Finished), 2);

    add_track(session.music.path(), "two.mp3");
    session.play(Predictable::always(PlaybackOutcome::Finished), 2);

    assert_eq!(session.remembered_tracks(), 2);
    assert!(!session.remembered("two.mp3").is_empty());
}

/// A track sorting before the others must not take their identifiers, or every
/// history shifts onto the wrong track.
#[test]
fn a_track_added_before_the_others_leaves_their_ratings_where_they_were() {
    let mut session = Session::holding(&["b.mp3", "c.mp3"]);
    session.play(Predictable::always(PlaybackOutcome::Finished), 6);
    let before = session.remembered("b.mp3");
    assert!(!before.is_empty());

    add_track(session.music.path(), "a.mp3");
    session.play(Predictable::silent(), 0);

    assert!(
        session.remembered("b.mp3").starts_with(before.as_slice()),
        "the earlier track should have kept every event recorded against it"
    );
}

#[test]
fn a_track_taken_out_of_the_directory_keeps_its_history() {
    let mut session = Session::holding(&["one.mp3", "two.mp3"]);
    session.play(Predictable::always(PlaybackOutcome::Finished), 4);
    let before = session.remembered("two.mp3");
    assert!(!before.is_empty());

    std::fs::remove_file(session.music.path().join("two.mp3")).expect("the file is removed");
    session.play(Predictable::always(PlaybackOutcome::Finished), 2);

    assert_eq!(session.remembered("two.mp3"), before);
}

/// Moving the whole collection changes where each track is but not which track
/// it is, so no history starts over.
#[test]
fn music_moved_somewhere_else_keeps_every_rating() {
    let mut session = Session::holding(&["rock/one.mp3", "two.mp3"]);
    session.play(Predictable::always(PlaybackOutcome::Finished), 4);
    let before = session.remembered("two.mp3");
    assert!(count_of(&before, is_outcome) > 0);

    let moved = Session::holding(&["rock/one.mp3", "two.mp3"]);
    let mut collection = session.stored();
    collection.set_root(moved.music.path());
    let tracks = collection
        .scan(None)
        .expect("the music is where it now lives");
    let library = Library::build(tracks, collection.history(), session.clock);

    assert_eq!(library.len(), 2);
    let two = moved.music.path().join("two.mp3");
    let identifier = library
        .identifiers()
        .iter()
        .find(|identifier| library.path(*identifier) == Some(two.as_path()))
        .expect("the track is where it now lives");
    assert_eq!(
        library
            .events(identifier)
            .expect("the track is in the library")
            .iter()
            .cloned()
            .collect::<Vec<TimeTaggedTrackEvent>>(),
        before,
        "the track should have carried its whole history across"
    );
}

/// Playing one directory below the root narrows the library rather than
/// replacing it: tracks outside it keep every event stored against them.
#[test]
fn playing_one_directory_leaves_the_rest_of_the_music_alone() {
    let mut session = Session::holding(&["rock/one.mp3", "jazz/two.mp3"]);
    session.play(Predictable::always(PlaybackOutcome::Finished), 6);
    let jazz = session.remembered("jazz/two.mp3");
    assert!(!jazz.is_empty());

    session.play_below(Predictable::always(PlaybackOutcome::Finished), "rock", 4);

    assert_eq!(session.remembered_tracks(), 2);
    assert_eq!(
        session.remembered("jazz/two.mp3"),
        jazz,
        "the directory that did not play should be untouched"
    );
}

#[test]
fn a_directory_outside_the_music_is_refused() {
    let session = Session::holding(&["one.mp3"]);
    let elsewhere = Session::holding(&["other.mp3"]);
    let mut collection = session.stored();

    assert!(collection.scan(Some(elsewhere.music.path())).is_err());
}

#[test]
fn a_session_plays_only_tracks_that_are_in_the_directory() {
    let mut session = Session::holding(&["one.mp3", "two.mp3", "notes.txt"]);
    let played = session.play(Predictable::always(PlaybackOutcome::Finished), 10);
    let expected = [
        session.music.path().join("one.mp3"),
        session.music.path().join("two.mp3"),
    ];
    assert!(played.iter().all(|path| expected.contains(path)));
    assert!(played.len() > 1);
}

/// A directory holding both listed and unlisted extensions plays the listed
/// ones and ignores the rest.
#[test]
fn a_directory_of_mixed_types_plays_the_ones_jive_knows() {
    let mut session = Session::holding(&[
        "known.mp3",
        "known.flac",
        "unknown.dsf",
        "unknown.mid",
        "cover.jpg",
    ]);
    let played = session.play(Predictable::always(PlaybackOutcome::Finished), 12);

    assert_eq!(session.remembered_tracks(), 2);
    for file in ["known.mp3", "known.flac"] {
        assert!(
            !session.remembered(file).is_empty(),
            "{file} should have played"
        );
    }
    for file in ["unknown.dsf", "unknown.mid", "cover.jpg"] {
        assert!(
            session.remembered(file).is_empty(),
            "{file} should never have been offered"
        );
        assert!(!played.contains(&session.music.path().join(file)));
    }
}

/// A backend that cannot decode a listed type fails that track, and jive plays
/// the others rather than stopping.
#[test]
fn a_track_the_backend_will_not_play_does_not_stop_the_rest() {
    let mut session = Session::holding(&["one.mp3", "two.mp3", "three.mp3"]);
    let played = session.play(
        Predictable::always(PlaybackOutcome::from(TrackFailure::UnsupportedFormat)),
        6,
    );

    assert!(
        played.len() > 1,
        "jive should have moved on to other tracks"
    );
    let failures: usize = ["one.mp3", "two.mp3", "three.mp3"]
        .iter()
        .map(|file| count_of(&session.remembered(file), is_outcome))
        .sum();
    assert!(failures > 0, "the failures should have been recorded");
}

#[test]
fn a_directory_of_files_that_are_not_audio_is_reported_as_empty() {
    let session = Session::holding(&["cover.jpg", "notes.txt"]);
    let tracks = session
        .stored()
        .scan(None)
        .expect("the directory can be read");
    assert!(tracks.is_empty());
}

/// A collection file holding an event this jive does not understand is carried
/// rather than rejected.
///
/// An event a later release added must not stop this one from playing, must not
/// count towards any rating, and must still be present afterwards.
#[test]
fn a_state_file_holding_an_unknown_event_is_carried_through_a_whole_session() {
    let mut session = Session::holding(&["one.mp3"]);
    let file = session.collection_file();
    std::fs::create_dir_all(file.path().parent().expect("a parent")).expect("a directory");
    let written = format!(
        r#"{{"version": 2, "root": {music:?}, "next_id": 1, "tracks": [
            {{"id": 0, "path": "one.mp3",
              "events": [{{"at": 0, "event": {{"event": "rated", "data": {{"stars": 5}}}}}}]}}
        ]}}"#,
        music = session.music.path(),
    );
    std::fs::write(file.path(), written).expect("a written file");

    session.play(Predictable::always(PlaybackOutcome::Finished), 1);

    assert!(
        count_of(&session.remembered("one.mp3"), is_outcome) >= 1,
        "the session should have run and recorded what it understood"
    );
    let after = std::fs::read_to_string(file.path()).expect("a written file");
    assert!(
        after.contains("rated") && after.contains("stars"),
        "the unknown event should have been written back: {after}"
    );
}

/// A collection file from the version that keyed tracks by absolute path is
/// migrated rather than discarded.
#[test]
fn a_state_file_from_the_previous_version_keeps_its_ratings() {
    let session = Session::holding(&["one.mp3"]);
    let file = session.collection_file();
    std::fs::create_dir_all(file.path().parent().expect("a parent")).expect("a directory");
    let written = format!(
        r#"{{"version": 1, "default_directory": {music:?}, "tracks": [
            {{"path": {track:?}, "name": "one",
             "events": [{{"at": 0, "event": {{"event": "playback_outcome", "data": {{"outcome": "finished"}}}}}}]}}
        ]}}"#,
        music = session.music.path(),
        track = session.music.path().join("one.mp3"),
    );
    std::fs::write(file.path(), written).expect("a written file");

    assert_eq!(count_of(&session.remembered("one.mp3"), is_outcome), 1);
    assert_eq!(session.remembered_tracks(), 1);
}
