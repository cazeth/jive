//! Domain types for the jive music player.
//!
//! * [`Time`] and [`Duration`] — timekeeping.
//! * [`track_events`] — the events recorded against a track.
//! * [`History`] — the events recorded against every track.
//! * [`TrackId`], [`TrackIds`], [`TrackName`], [`TrackNames`] — track identity.
//! * [`AudioBackend`] — the interface to the program that produces sound.
//!
//! # Feature flags
//!
//! * `serde` — derives [`serde::Serialize`] and [`serde::Deserialize`]. The
//!   encoding is stable across versions. See [`track_events::AnyTrackEvent`]
//!   for the rules it follows.

pub mod track_events;

mod backend;
mod history;
mod time;
mod track;

pub use backend::AudioBackend;
pub use backend::BackendError;
pub use backend::BackendResult;
pub use history::History;
pub use time::Duration;
pub use time::Time;
pub use track::TrackId;
pub use track::TrackIds;
pub use track::TrackName;
pub use track::TrackNames;
