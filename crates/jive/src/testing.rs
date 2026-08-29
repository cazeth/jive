//! Builders for jive's own tests, alongside the music directories
//! jive-filesystem lays out.
//!
//! Tracks built here are identified as a catalog would identify them: in the
//! order given, from zero. No directory is walked, so the paths are fabricated
//! and no file is opened.

use crate::cli::Arguments;
use crate::library::Library;
use crate::player::Player;
use crate::player::ViewModel;
use crate::selection::Shuffle;
use jive_core::AudioBackend;
use jive_core::BackendError;
use jive_core::BackendResult;
use jive_core::Duration;
use jive_core::History;
use jive_core::Time;
use jive_core::TrackId;
use jive_core::TrackName;
use jive_core::track_events::AnyTrackEvent;
use jive_core::track_events::PlaybackOutcome;
use jive_core::track_events::Skipped;
use jive_core::track_events::TimeTaggedTrackEvent;
use jive_core::track_events::TimeTaggedTrackEvents;
use jive_core::track_events::TrackFailure;
use jive_filesystem::CollectionFile;
use jive_filesystem::DiscoveredTrack;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use tempfile::TempDir;

pub use jive_filesystem::testing::directory_holding;

/// An event for a track played to its end.
pub fn finished() -> AnyTrackEvent {
    PlaybackOutcome::Finished.into()
}

/// An event for a track that would not play.
///
/// Which failure it was is immaterial here: jive weighs them all the same, and
/// the reasons themselves are jive-core's to test.
pub fn failed() -> AnyTrackEvent {
    PlaybackOutcome::from(TrackFailure::DecoderError).into()
}

/// An event for a track cut short after `seconds`.
pub fn skip_after(seconds: u64) -> AnyTrackEvent {
    Skipped::new(Duration::from_seconds(seconds)).into()
}

/// An event for a track cut short soon enough to count as evidence against it.
pub fn quick_skip() -> AnyTrackEvent {
    skip_after(2)
}

/// The same event, `times` over.
pub fn repeated(event: &AnyTrackEvent, times: usize) -> Vec<AnyTrackEvent> {
    std::iter::repeat_n(event.clone(), times).collect()
}

/// Events for one track, all recorded at `at`.
pub fn recorded_at(
    at: Time,
    events: impl IntoIterator<Item = AnyTrackEvent>,
) -> TimeTaggedTrackEvents {
    events
        .into_iter()
        .map(|event| TimeTaggedTrackEvent::new(at, event))
        .collect()
}

/// The fabricated path a track built here appears to live at.
pub fn path_of(name: &str) -> PathBuf {
    PathBuf::from(format!("/music/{name}.mp3"))
}

/// One track, as a catalog that had already assigned it `number` would report
/// it.
pub fn discovered_as(number: u32, name: &str) -> DiscoveredTrack {
    DiscoveredTrack {
        identifier: TrackId::new(number),
        name: TrackName::new(name),
        path: path_of(name),
    }
}

/// Tracks identified in the order they are given, from zero.
pub fn discovered(names: &[&str]) -> Vec<DiscoveredTrack> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| discovered_as(u32::try_from(index).expect("a small library"), name))
        .collect()
}

/// A library of tracks with no events recorded against any of them.
pub fn library_of(names: &[&str]) -> Library {
    Library::build(discovered(names), &History::new(), Time::EPOCH)
}

/// A library whose tracks carry the events listed beside them, all recorded at
/// the epoch.
pub fn library_with_history(tracks: &[(&str, Vec<AnyTrackEvent>)]) -> Library {
    let names: Vec<&str> = tracks.iter().map(|(name, _)| *name).collect();
    let discovered = discovered(&names);
    let mut history = History::new();
    for (track, (_, events)) in discovered.iter().zip(tracks) {
        history.store(track.identifier, recorded_at(Time::EPOCH, events.clone()));
    }
    Library::build(discovered, &history, Time::EPOCH)
}

/// A library where every track carries the same events.
pub fn library_where_every_track(names: &[&str], events: &[AnyTrackEvent]) -> Library {
    let tracks: Vec<(&str, Vec<AnyTrackEvent>)> =
        names.iter().map(|name| (*name, events.to_vec())).collect();
    library_with_history(&tracks)
}

pub fn identifiers_of(library: &Library) -> Vec<TrackId> {
    library.identifiers().iter().collect()
}

/// Records that a track finished at `at`.
pub fn finishes_at(library: &mut Library, identifier: TrackId, at: Time) {
    library.record(identifier, at, PlaybackOutcome::Finished);
}

/// Records one finish per track, a second apart and in discovery order, so that
/// which track played most recently is settled.
pub fn every_track_finishes_in_turn(library: &mut Library) -> Vec<TrackId> {
    let identifiers = identifiers_of(library);
    for (offset, identifier) in identifiers.iter().enumerate() {
        let seconds = u64::try_from(offset).expect("a small library");
        finishes_at(
            library,
            *identifier,
            Time::EPOCH + Duration::from_seconds(seconds),
        );
    }
    identifiers
}

pub fn names_of(library: &Library) -> Vec<String> {
    library
        .tracks()
        .map(|track| track.name.to_string())
        .collect()
}

pub fn identifier_named(library: &Library, name: &str) -> TrackId {
    let found = library
        .tracks()
        .find(|track| track.name.as_str() == name)
        .map(|track| track.identifier);
    match found {
        Some(identifier) => identifier,
        None => panic!("the library should hold a track called {name}"),
    }
}

/// An identifier no library built here assigns.
pub fn stranger() -> TrackId {
    TrackId::new(9_999)
}

/// Every event recorded against one named track.
pub fn events_of(library: &Library, name: &str) -> Vec<TimeTaggedTrackEvent> {
    library
        .events(identifier_named(library, name))
        .map(|events| events.iter().cloned().collect())
        .unwrap_or_default()
}

/// Asserts two values are equal to within floating point error.
pub fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} is not close enough to {expected}"
    );
}

/// A backend that produces no sound and records every request made of it.
#[derive(Debug, Default)]
pub struct FakeBackend {
    pub played: Vec<PathBuf>,
    pub stops: usize,
    outcomes: VecDeque<PlaybackOutcome>,
    failing: Option<Failing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failing {
    Play,
    Stop,
    Poll,
}

impl FakeBackend {
    pub fn failing_to_play() -> Self {
        Self::failing(Failing::Play)
    }

    pub fn failing_to_stop() -> Self {
        Self::failing(Failing::Stop)
    }

    pub fn failing_to_poll() -> Self {
        Self::failing(Failing::Poll)
    }

    fn failing(when: Failing) -> Self {
        Self {
            failing: Some(when),
            ..Self::default()
        }
    }

    fn refuse(&self, when: Failing) -> BackendResult<()> {
        if self.failing == Some(when) {
            return Err(BackendError::unavailable(
                "the fake backend was asked to fail",
            ));
        }
        Ok(())
    }
}

impl AudioBackend for FakeBackend {
    fn play(&mut self, path: &Path) -> BackendResult<()> {
        self.refuse(Failing::Play)?;
        self.played.push(path.to_path_buf());
        Ok(())
    }

    fn stop(&mut self) -> BackendResult<()> {
        self.refuse(Failing::Stop)?;
        self.stops += 1;
        Ok(())
    }

    fn poll_event(&mut self) -> BackendResult<Option<PlaybackOutcome>> {
        self.refuse(Failing::Poll)?;
        Ok(self.outcomes.pop_front())
    }
}

/// A player over a [`FakeBackend`], with a clock the test advances by hand.
pub struct PlayerFixture {
    pub player: Player<FakeBackend>,
    now: Time,
}

impl PlayerFixture {
    /// A player that has not started, over a library with no events recorded.
    pub fn idle(names: &[&str]) -> Self {
        Self::over(library_of(names), FakeBackend::default())
    }

    /// A started player over a library with no events recorded.
    pub fn playing(names: &[&str]) -> Self {
        let mut fixture = Self::idle(names);
        fixture.start();
        fixture
    }

    /// A started player driving the given backend, over two tracks.
    pub fn driving(backend: FakeBackend) -> Self {
        let mut fixture = Self::over(library_of(&["one", "two"]), backend);
        fixture.start();
        fixture
    }

    /// An unstarted player driving the given backend, over two tracks.
    pub fn idle_driving(backend: FakeBackend) -> Self {
        Self::over(library_of(&["one", "two"]), backend)
    }

    pub fn over(library: Library, backend: FakeBackend) -> Self {
        Self {
            player: Player::new(backend, library, Shuffle::seeded(1)),
            now: Time::EPOCH + Duration::from_seconds(1_000),
        }
    }

    pub fn start(&mut self) -> &mut Self {
        self.try_start().expect("playback starts");
        self
    }

    pub fn try_start(&mut self) -> crate::error::Result<()> {
        self.player.start(self.now)
    }

    pub fn try_skip(&mut self) -> crate::error::Result<()> {
        self.player.skip(self.now)
    }

    pub fn try_poll(&mut self) -> crate::error::Result<()> {
        self.player.poll(self.now)
    }

    pub fn wait(&mut self, seconds: u64) -> &mut Self {
        self.now += Duration::from_seconds(seconds);
        self
    }

    /// Moves the clock backwards, as a clock synchronization would.
    pub fn rewind(&mut self, seconds: u64) -> &mut Self {
        self.now -= Duration::from_seconds(seconds);
        self
    }

    pub fn press_next(&mut self) -> &mut Self {
        self.player.skip(self.now).expect("skipping works");
        self
    }

    pub fn backend_reports(&mut self, outcome: PlaybackOutcome) -> &mut Self {
        self.player.backend_mut().outcomes.push_back(outcome);
        self.player.poll(self.now).expect("polling works");
        self
    }

    pub fn backend_reports_nothing(&mut self) -> &mut Self {
        self.player.poll(self.now).expect("polling works");
        self
    }

    pub fn quit(&mut self) -> &mut Self {
        self.player.stop(self.now).expect("stopping works");
        self
    }

    pub fn view(&self) -> ViewModel {
        self.player.view(self.now)
    }

    pub fn now(&self) -> Time {
        self.now
    }

    pub fn tracks_played(&self) -> usize {
        self.player.backend().played.len()
    }

    pub fn play_order(&self) -> Vec<PathBuf> {
        self.player.backend().played.clone()
    }

    /// How many times the backend was told to stop.
    pub fn backend_stops(&self) -> usize {
        self.player.backend().stops
    }

    pub fn playing_name(&self) -> Option<String> {
        match self.view() {
            ViewModel::Playing { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn elapsed(&self) -> Option<Duration> {
        match self.view() {
            ViewModel::Playing { elapsed, .. } => Some(elapsed),
            _ => None,
        }
    }

    pub fn recorded(&self) -> Vec<TimeTaggedTrackEvent> {
        self.player
            .library()
            .tracks()
            .flat_map(|track| track.events.iter().cloned())
            .collect()
    }

    /// How many skips were recorded against one named track.
    pub fn skips_recorded_for(&self, name: &str) -> usize {
        events_of(self.player.library(), name)
            .iter()
            .filter_map(|event| event.event.as_skipped())
            .count()
    }

    /// How long each track had played when the listener ended playback.
    pub fn stops(&self) -> Vec<Duration> {
        self.recorded()
            .iter()
            .filter_map(|event| event.event.as_stopped())
            .map(|stopped| stopped.listened_for)
            .collect()
    }

    /// When each event was recorded, earliest first, excluding the events
    /// noting that a track entered the library.
    pub fn recorded_at(&self) -> Vec<Time> {
        let mut moments: Vec<Time> = self
            .recorded()
            .iter()
            .filter(|event| event.event.as_added().is_none())
            .map(|event| event.at)
            .collect();
        moments.sort_unstable();
        moments
    }

    pub fn skips(&self) -> Vec<Duration> {
        self.recorded()
            .iter()
            .filter_map(|event| event.event.as_skipped())
            .map(|skipped| skipped.listened_for)
            .collect()
    }

    pub fn outcomes(&self) -> Vec<PlaybackOutcome> {
        self.recorded()
            .iter()
            .filter_map(|event| event.event.as_playback_outcome())
            .copied()
            .collect()
    }
}

/// A collection file and a directory of music, both removed with the test.
pub struct StoreFixture {
    home: TempDir,
    music: TempDir,
}

impl StoreFixture {
    /// A fixture whose music directory holds the named files.
    pub fn holding(files: &[&str]) -> Self {
        Self {
            home: TempDir::new().expect("a temporary directory"),
            music: directory_holding(files),
        }
    }

    /// The collection file, one directory deeper than the root so that creating
    /// that directory is exercised too.
    pub fn file(&self) -> CollectionFile {
        CollectionFile::at(self.home.path().join("nested").join("state.json"))
    }

    /// This fixture's music directory.
    pub fn music(&self) -> &Path {
        self.music.path()
    }

    /// A command line pointing at this fixture's music.
    pub fn arguments(&self) -> Arguments {
        Arguments {
            directory: None,
            root: Some(self.music.path().to_path_buf()),
        }
    }
}
