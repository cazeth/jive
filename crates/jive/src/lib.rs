//! A music player for a directory of files, with a shuffle that adapts to what
//! the listener skips and plays through.
//!
//! [`run`] is the whole of the `jive` command: it reads the collection, plays
//! what the arguments point at, and records what became of every track.
//!
//! # How the shuffle chooses
//!
//! A track is drawn with probability proportional to three factors multiplied
//! together, each derived from the events recorded against it:
//!
//! * preference — how far finishes outweigh quick skips,
//! * staleness — how long since the track last played,
//! * reliability — how much of its playback has succeeded.
//!
//! Each factor is bounded away from zero, so a track the listener keeps
//! skipping still comes up sometimes.
//!
//! The most recently played tracks are held back from each draw. See
//! [`Shuffle::next_track_excluding`] for the window and how it is relaxed.
//! [`rows`] and [`table`] report the same numbers the shuffle acts on, which is
//! what the `jive-debug` command prints.

mod app;
mod cli;
mod error;
mod explain;
mod library;
mod offer;
mod player;
mod rating;
mod selection;
#[cfg(test)]
mod testing;
mod ui;

pub use app::run;
pub use cli::Arguments;
pub use cli::Request;
pub use error::Error;
pub use error::Result;
pub use explain::Row;
pub use explain::legend;
pub use explain::rows;
pub use explain::table;
pub use library::Library;
pub use player::Player;
pub use selection::Exclusion;
pub use selection::Shuffle;
