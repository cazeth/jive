//! Driving the backend and recording the events that result.

use crate::error::Result;
use crate::library::Library;
use crate::selection::Shuffle;
use jive_core::AudioBackend;
use jive_core::Duration;
use jive_core::Time;
use jive_core::TrackId;
use jive_core::track_events::AnyTrackEvent;
use jive_core::track_events::PlaybackOutcome;
use jive_core::track_events::Skipped;
use jive_core::track_events::Stopped;
use std::path::Path;

/// The player's state, as the user interface needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewModel {
    /// A track is playing.
    Playing {
        /// The name to display.
        name: String,
        /// How long the track has been playing.
        elapsed: Duration,
    },
    /// The library holds no tracks.
    Empty,
    /// Every track was tried and none would play.
    Stalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Playing {
    identifier: TrackId,
    started_at: Time,
}

/// A library, a shuffle, and the backend playing them.
///
/// Stepped by the caller: [`Player::start`] to begin, [`Player::poll`] each
/// turn, [`Player::skip`] on request, and [`Player::stop`] to finish.
///
/// Every event it records lands in its [`Library`]. Persisting them is the
/// caller's work. [`Player::has_unsaved_events`] reports when there is
/// something to save.
///
/// A track that will not play is recorded and stepped past. Once every track
/// has failed without one playing, the player stalls rather than spinning, and
/// tries again only on the next [`Player::skip`].
#[derive(Debug)]
pub struct Player<Backend> {
    backend: Backend,
    library: Library,
    shuffle: Shuffle,
    playing: Option<Playing>,
    failed_tracks: Vec<TrackId>,
    unsaved_events: bool,
}

impl<Backend: AudioBackend> Player<Backend> {
    /// A player that has not started. Nothing plays until [`Player::start`].
    pub fn new(backend: Backend, library: Library, shuffle: Shuffle) -> Self {
        Self {
            backend,
            library,
            shuffle,
            playing: None,
            failed_tracks: Vec::new(),
            unsaved_events: false,
        }
    }

    /// Draws a track and starts playing it.
    ///
    /// # Errors
    ///
    /// If the backend is unusable.
    pub fn start(&mut self, now: Time) -> Result<()> {
        self.failed_tracks.clear();
        self.advance(now, None)
    }

    /// Records a [`Skipped`] against the current track and moves to another.
    ///
    /// # Errors
    ///
    /// If the backend is unusable.
    pub fn skip(&mut self, now: Time) -> Result<()> {
        let Some(playing) = self.playing.take() else {
            self.failed_tracks.clear();
            return self.advance(now, None);
        };
        self.backend.stop()?;
        let listened_for = now.duration_since(playing.started_at);
        self.record(playing.identifier, now, Skipped::new(listened_for));
        self.failed_tracks.clear();
        self.advance(now, Some(playing.identifier))
    }

    /// Polls the backend, recording any outcome and moving to another track.
    ///
    /// Does nothing while the current track is still playing.
    ///
    /// # Errors
    ///
    /// If the backend is unusable.
    pub fn poll(&mut self, now: Time) -> Result<()> {
        let Some(outcome) = self.backend.poll_event()? else {
            return Ok(());
        };
        let ended = self.playing.take();
        if let Some(playing) = ended {
            self.record(playing.identifier, now, outcome);
            self.note_outcome(playing.identifier, outcome);
        }
        if self.has_stalled() {
            return Ok(());
        }
        self.advance(now, ended.map(|playing| playing.identifier))
    }

    /// Stops playing, recording a [`Stopped`] against the current track.
    ///
    /// # Errors
    ///
    /// If the backend cannot be stopped.
    pub fn stop(&mut self, now: Time) -> Result<()> {
        if let Some(playing) = self.playing.take() {
            let listened_for = now.duration_since(playing.started_at);
            self.record(playing.identifier, now, Stopped::new(listened_for));
        }
        self.backend.stop().map_err(Into::into)
    }

    /// The player's state as of `now`.
    #[must_use]
    pub(crate) fn view(&self, now: Time) -> ViewModel {
        if self.library.is_empty() {
            return ViewModel::Empty;
        }
        match self.playing.and_then(|playing| self.name_of(playing, now)) {
            Some((name, elapsed)) => ViewModel::Playing { name, elapsed },
            None => ViewModel::Stalled,
        }
    }

    /// The backend being driven, for a test to inspect what it was asked.
    #[cfg(test)]
    pub(crate) fn backend(&self) -> &Backend {
        &self.backend
    }

    /// The backend being driven, for a test to arrange what it reports.
    #[cfg(test)]
    pub(crate) fn backend_mut(&mut self) -> &mut Backend {
        &mut self.backend
    }

    /// The library, including everything recorded this session.
    #[must_use]
    pub fn library(&self) -> &Library {
        &self.library
    }

    /// Whether anything recorded has yet to reach the store.
    #[must_use]
    pub fn has_unsaved_events(&self) -> bool {
        self.unsaved_events
    }

    /// Records that everything recorded so far has reached the store.
    pub fn mark_saved(&mut self) {
        self.unsaved_events = false;
    }

    fn name_of(&self, playing: Playing, now: Time) -> Option<(String, Duration)> {
        let name = self.library.name(playing.identifier)?.to_string();
        Some((name, now.duration_since(playing.started_at)))
    }

    fn record(&mut self, identifier: TrackId, at: Time, event: impl Into<AnyTrackEvent>) {
        self.library.record(identifier, at, event);
        self.unsaved_events = true;
    }

    fn note_outcome(&mut self, identifier: TrackId, outcome: PlaybackOutcome) {
        if outcome.is_finished() {
            self.failed_tracks.clear();
        } else if !self.failed_tracks.contains(&identifier) {
            self.failed_tracks.push(identifier);
        }
    }

    fn has_stalled(&self) -> bool {
        self.failed_tracks.len() >= self.library.len()
    }

    /// Draws and starts the next track, avoiding `just_played` where possible.
    ///
    /// Leaves nothing playing if the draw comes up empty or the drawn track has
    /// no path, either of which shows as `ViewModel::Stalled`.
    fn advance(&mut self, now: Time, just_played: Option<TrackId>) -> Result<()> {
        let Some(identifier) =
            self.shuffle
                .next_track_excluding(&self.library, just_played, now, &self.failed_tracks)
        else {
            self.playing = None;
            return Ok(());
        };
        let Some(path) = self.library.path(identifier).map(Path::to_path_buf) else {
            self.playing = None;
            return Ok(());
        };
        self.backend.play(&path)?;
        self.playing = Some(Playing {
            identifier,
            started_at: now,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ViewModel;
    use crate::testing::FakeBackend;
    use crate::testing::PlayerFixture;
    use crate::testing::finished;
    use crate::testing::library_where_every_track;
    use crate::testing::path_of;
    use crate::testing::repeated;
    use jive_core::Duration;
    use jive_core::track_events::PlaybackOutcome;
    use jive_core::track_events::TrackFailure;

    fn failing(reason: TrackFailure) -> PlaybackOutcome {
        PlaybackOutcome::from(reason)
    }

    fn after_playing(names: &[&str], seconds: u64) -> PlayerFixture {
        let mut fixture = PlayerFixture::playing(names);
        fixture.wait(seconds);
        fixture
    }

    fn repeats_in(fixture: &PlayerFixture) -> usize {
        fixture
            .play_order()
            .windows(2)
            .filter(|pair| pair[0] == pair[1])
            .count()
    }

    #[test]
    fn starting_plays_one_track_and_records_nothing_about_it_yet() {
        let fixture = PlayerFixture::playing(&["one", "two"]);
        assert_eq!(fixture.tracks_played(), 1);
        assert_eq!(
            fixture.recorded().len(),
            2,
            "both tracks should only have been noted as seen"
        );
    }

    /// Which track a library of several starts on is up to the shuffle, so the
    /// displayed name is settled only when there is one track to choose.
    #[test]
    fn the_track_that_started_is_the_one_shown() {
        let fixture = PlayerFixture::playing(&["only"]);
        assert_eq!(fixture.playing_name(), Some(String::from("only")));
        assert_eq!(fixture.elapsed(), Some(Duration::ZERO));
        assert!(matches!(fixture.view(), ViewModel::Playing { .. }));
    }

    #[test]
    fn an_idle_player_has_not_started_yet() {
        let mut fixture = PlayerFixture::idle(&["one"]);
        assert_eq!(fixture.tracks_played(), 0);
        assert_eq!(fixture.view(), ViewModel::Stalled);
        assert!(matches!(fixture.start().view(), ViewModel::Playing { .. }));
    }

    #[test]
    fn an_empty_library_has_nothing_to_play() {
        let mut fixture = PlayerFixture::playing(&[]);
        assert_eq!(fixture.tracks_played(), 0);
        assert_eq!(fixture.view(), ViewModel::Empty);
        assert_eq!(fixture.quit().stops().len(), 0);
    }

    #[test]
    fn the_elapsed_time_follows_the_clock() {
        let fixture = after_playing(&["one"], 90);
        assert_eq!(fixture.elapsed(), Some(Duration::from_seconds(90)));
    }

    #[test]
    fn skipping_records_what_was_heard_and_moves_on() {
        let mut fixture = after_playing(&["one", "two"], 3);
        fixture.press_next();
        assert_eq!(fixture.skips(), [Duration::from_seconds(3)]);
        assert_eq!(fixture.backend_stops(), 1);
        assert_eq!(fixture.tracks_played(), 2);
    }

    #[test]
    fn skipping_at_once_records_no_listening() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        assert_eq!(fixture.press_next().skips(), [Duration::ZERO]);
    }

    /// A track that ran out is not one the listener rejected, so it records an
    /// outcome and no skip, and the backend is not told to stop.
    #[test]
    fn finishing_records_an_outcome_and_moves_on() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        fixture.backend_reports(PlaybackOutcome::Finished);
        assert_eq!(fixture.outcomes(), [PlaybackOutcome::Finished]);
        assert_eq!(fixture.skips().len(), 0);
        assert_eq!(fixture.tracks_played(), 2);
        assert_eq!(fixture.backend_stops(), 0);
    }

    #[test]
    fn a_lone_track_keeps_playing_after_it_finishes() {
        let mut fixture = PlayerFixture::playing(&["alone"]);
        let view = fixture.backend_reports(PlaybackOutcome::Finished).view();
        assert!(matches!(view, ViewModel::Playing { .. }));
    }

    #[test]
    fn a_backend_with_nothing_to_say_changes_nothing() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        assert_eq!(fixture.backend_reports_nothing().tracks_played(), 1);
    }

    #[test]
    fn quitting_stops_the_backend_and_records_that_the_listener_left() {
        let mut fixture = after_playing(&["one"], 5);
        fixture.quit();
        assert_eq!(fixture.backend_stops(), 1);
        assert_eq!(fixture.stops(), [Duration::from_seconds(5)]);
        assert_eq!(fixture.skips().len(), 0, "leaving is not a rejection");
        assert_eq!(fixture.playing_name(), None);
    }

    #[test]
    fn quitting_before_anything_plays_records_nothing() {
        let mut fixture = PlayerFixture::idle(&["one"]);
        assert_eq!(fixture.quit().stops().len(), 0);
    }

    /// A backend failure must surface, since carrying on would leave the player
    /// displaying a track that is not playing.
    #[test]
    fn a_backend_that_fails_is_reported_rather_than_ignored() {
        assert!(
            PlayerFixture::idle_driving(FakeBackend::failing_to_play())
                .try_start()
                .is_err(),
            "a backend that will not play"
        );
        assert!(
            PlayerFixture::driving(FakeBackend::failing_to_stop())
                .try_skip()
                .is_err(),
            "a backend that will not stop"
        );
        assert!(
            PlayerFixture::driving(FakeBackend::failing_to_poll())
                .try_poll()
                .is_err(),
            "a backend that cannot be polled"
        );
    }

    /// The recent window depends on these timestamps, so each event must carry
    /// the time it was recorded rather than the time its track started.
    #[test]
    fn an_event_is_recorded_at_the_moment_it_happens() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        let started = fixture.now();
        fixture.wait(30).backend_reports(PlaybackOutcome::Finished);
        let after_finishing = fixture.now();
        fixture.wait(30).press_next();
        let after_skipping = fixture.now();
        fixture.wait(30).quit();

        assert!(after_finishing > started);
        assert_eq!(
            fixture.recorded_at(),
            [after_finishing, after_skipping, fixture.now()],
            "each event should carry its own moment"
        );
    }

    #[test]
    fn a_track_never_follows_itself_however_the_one_before_ended() {
        let mut fixture = PlayerFixture::playing(&["one", "two", "three"]);
        for _ in 0..40 {
            fixture.backend_reports(PlaybackOutcome::Finished);
            fixture.press_next();
            fixture.backend_reports(failing(TrackFailure::DecoderError));
        }
        assert_eq!(repeats_in(&fixture), 0, "played {:?}", fixture.play_order());
    }

    #[test]
    fn a_lone_track_is_allowed_to_follow_itself() {
        let mut fixture = PlayerFixture::playing(&["alone"]);
        for _ in 0..5 {
            fixture.backend_reports(PlaybackOutcome::Finished);
        }
        assert_eq!(fixture.tracks_played(), 6);
        assert_eq!(repeats_in(&fixture), 5);
    }

    #[test]
    fn every_way_a_track_can_fail_is_recorded_and_moved_past() {
        for &reason in TrackFailure::ALL {
            let mut fixture = PlayerFixture::playing(&["one", "two"]);
            fixture.backend_reports(failing(reason));
            assert_eq!(fixture.outcomes(), [failing(reason)]);
            assert_eq!(
                fixture.tracks_played(),
                2,
                "{reason:?} should be moved past"
            );
        }
    }

    #[test]
    fn a_library_where_nothing_plays_stalls_instead_of_spinning() {
        let mut fixture = PlayerFixture::playing(&["one", "two", "three"]);
        for _ in 0..3 {
            fixture.backend_reports(failing(TrackFailure::FileNotFound));
        }
        assert_eq!(fixture.view(), ViewModel::Stalled);
        assert_eq!(fixture.tracks_played(), 3);
    }

    #[test]
    fn a_stalled_player_tries_again_when_the_listener_asks() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        for _ in 0..2 {
            fixture.backend_reports(failing(TrackFailure::FileNotFound));
        }
        assert_eq!(fixture.view(), ViewModel::Stalled);
        fixture.press_next();
        assert_eq!(fixture.tracks_played(), 3);
    }

    #[test]
    fn a_run_of_failures_is_forgotten_once_something_plays() {
        let mut fixture = PlayerFixture::playing(&["one", "two", "three"]);
        fixture
            .backend_reports(failing(TrackFailure::FileNotFound))
            .backend_reports(PlaybackOutcome::Finished)
            .backend_reports(failing(TrackFailure::FileNotFound))
            .backend_reports(failing(TrackFailure::FileNotFound));
        assert!(matches!(fixture.view(), ViewModel::Playing { .. }));
    }

    #[test]
    fn a_run_of_failures_is_forgotten_once_the_listener_skips() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        fixture
            .backend_reports(failing(TrackFailure::FileNotFound))
            .press_next()
            .backend_reports(failing(TrackFailure::FileNotFound));
        assert!(matches!(fixture.view(), ViewModel::Playing { .. }));
    }

    #[test]
    fn a_clock_that_jumps_backwards_records_no_listening_rather_than_nonsense() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        fixture.rewind(60).press_next();
        assert_eq!(fixture.skips(), [Duration::ZERO]);
    }

    #[test]
    fn what_is_recorded_lands_on_the_track_it_happened_to() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        let played = fixture.playing_name().expect("a track is playing");
        fixture.wait(4).press_next();

        let untouched = if played == "one" { "two" } else { "one" };
        assert_eq!(fixture.skips_recorded_for(&played), 1);
        assert_eq!(fixture.skips_recorded_for(untouched), 0);
    }

    #[test]
    fn a_played_track_is_one_the_library_knows_about() {
        let fixture = PlayerFixture::playing(&["one", "two"]);
        let expected = [path_of("one"), path_of("two")];
        let played = fixture.play_order();
        assert!(!played.is_empty(), "nothing played, so nothing was checked");
        assert!(played.iter().all(|played| expected.contains(played)));
    }

    #[test]
    fn recording_an_event_marks_the_player_unsaved_until_it_is_stored() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        assert!(
            !fixture.player.has_unsaved_events(),
            "starting records nothing of its own"
        );
        fixture.press_next();
        assert!(fixture.player.has_unsaved_events());
        fixture.player.mark_saved();
        assert!(!fixture.player.has_unsaved_events());
    }

    #[test]
    fn nothing_new_leaves_the_player_saved() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        fixture.player.mark_saved();
        fixture.wait(30).backend_reports_nothing();
        assert!(!fixture.player.has_unsaved_events());
    }

    #[test]
    fn a_library_the_listener_has_opinions_about_still_plays_everything() {
        let library =
            library_where_every_track(&["one", "two", "three"], &repeated(&finished(), 30));
        let mut fixture = PlayerFixture::over(library, FakeBackend::default());
        fixture.start();
        for _ in 0..60 {
            fixture.backend_reports(PlaybackOutcome::Finished);
        }
        let played: std::collections::HashSet<_> = fixture.play_order().into_iter().collect();
        assert_eq!(played.len(), 3);
    }

    #[test]
    fn a_failure_never_counts_as_listening() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        fixture
            .wait(120)
            .backend_reports(failing(TrackFailure::DecoderError));
        assert!(fixture.skips().is_empty());
    }
}
