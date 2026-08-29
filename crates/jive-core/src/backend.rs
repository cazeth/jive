//! The interface between jive and the program that produces sound.
//!
//! [`AudioBackend`] is the trait an implementation provides. [`BackendError`]
//! reports that the backend as a whole is unusable, as distinct from a single
//! track that will not play.

use crate::track_events::PlaybackOutcome;
use std::error::Error;
use std::path::Path;
use std::path::PathBuf;

/// The result of a request to a backend.
pub type BackendResult<T> = Result<T, BackendError>;

/// The underlying cause of a backend failure, boxed so that a backend can be
/// built on any error type.
type Cause = Box<dyn Error + Send + Sync + 'static>;

/// The backend is unusable, so nothing will play: the program is missing,
/// failed to start, or stopped responding.
///
/// A single track that will not play is *not* one of these. That is
/// [`PlaybackOutcome::Failed`], carrying a [`TrackFailure`]. The distinction
/// determines what the caller does next: move on to another track, or give up.
///
/// Each variant names the request that failed and retains its cause, reachable
/// through [`Error::source`].
///
/// [`TrackFailure`]: crate::track_events::TrackFailure
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BackendError {
    /// The backend could not be started, so nothing has played.
    #[error("the audio backend is unavailable: {source}")]
    Unavailable {
        /// What was attempted and how it failed.
        #[source]
        source: Cause,
    },
    /// The backend could not be asked to play a track.
    #[error("the audio backend could not be asked to play `{}`: {source}", path.display())]
    Play {
        /// The track that was requested.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: Cause,
    },
    /// The current track could not be stopped.
    #[error("the audio backend could not be stopped: {source}")]
    Stop {
        /// What went wrong.
        #[source]
        source: Cause,
    },
    /// The backend could not be polled for the state of the current track.
    #[error("the audio backend could not be asked for its state: {source}")]
    Poll {
        /// What went wrong.
        #[source]
        source: Cause,
    },
}

impl BackendError {
    /// A [`BackendError::Unavailable`] with `cause`.
    pub fn unavailable(cause: impl Into<Cause>) -> Self {
        Self::Unavailable {
            source: cause.into(),
        }
    }

    /// A [`BackendError::Play`] for `path`, with `cause`.
    pub fn play(path: impl Into<PathBuf>, cause: impl Into<Cause>) -> Self {
        Self::Play {
            path: path.into(),
            source: cause.into(),
        }
    }

    /// A [`BackendError::Stop`] with `cause`.
    pub fn stop(cause: impl Into<Cause>) -> Self {
        Self::Stop {
            source: cause.into(),
        }
    }

    /// A [`BackendError::Poll`] with `cause`.
    pub fn poll(cause: impl Into<Cause>) -> Self {
        Self::Poll {
            source: cause.into(),
        }
    }
}

/// A program that plays one track at a time.
///
/// Driven from a single thread: [`play`], then [`poll_event`] until an outcome
/// arrives, or [`stop`] if the listener moves on first.
///
/// # Track failure against backend failure
///
/// A track that will not play is not an error: the backend works, this track
/// does not. Report it from [`poll_event`] as [`PlaybackOutcome::Failed`] with
/// the matching [`TrackFailure`], and the caller records it and moves on.
/// Return [`BackendError`] only when the backend itself is unusable.
///
/// The boundary is [`TrackFailure::BackendExited`]. Report a track failure
/// while a retry might succeed, and a [`BackendError`] once none can.
///
/// [`play`]: AudioBackend::play
/// [`poll_event`]: AudioBackend::poll_event
/// [`stop`]: AudioBackend::stop
/// [`TrackFailure`]: crate::track_events::TrackFailure
/// [`TrackFailure::BackendExited`]: crate::track_events::TrackFailure::BackendExited
pub trait AudioBackend {
    /// Starts playing `path`, replacing whatever was playing.
    ///
    /// # Errors
    ///
    /// Only when the backend is unusable. A track that will not play is
    /// reported later by [`AudioBackend::poll_event`] instead.
    fn play(&mut self, path: &Path) -> BackendResult<()>;

    /// Stops the current track without producing an outcome.
    ///
    /// # Errors
    ///
    /// Only when the backend is unusable.
    fn stop(&mut self) -> BackendResult<()>;

    /// The outcome of the current track, once there is one.
    ///
    /// Does not block. [`None`] while the track plays, and after its outcome
    /// has been reported once.
    ///
    /// # Errors
    ///
    /// Only when the backend is unusable. A track that will not play arrives as
    /// [`PlaybackOutcome::Failed`], which is not an error.
    fn poll_event(&mut self) -> BackendResult<Option<PlaybackOutcome>>;
}
