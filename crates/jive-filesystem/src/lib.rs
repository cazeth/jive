//! A catalog of the audio tracks below a directory, and the events recorded
//! against them, kept in one file.
//!
//! # Overview
//!
//! A [`Collection`] is a root directory, a [`Catalog`] assigning a
//! [`TrackId`](jive_core::TrackId) to every track below it, and the
//! [`History`](jive_core::History) recorded against those identifiers.
//! [`CollectionFile`] loads and saves one as a single JSON file.
//!
//! [`Collection::scan`] walks the root, or any directory below it, and returns
//! a [`DiscoveredTrack`] for each file whose extension appears in [`formats`].
//! File contents are never examined and symbolic links are not followed.
//!
//! ```no_run
//! use jive_filesystem::Collection;
//! use jive_filesystem::CollectionFile;
//!
//! let file = CollectionFile::in_data_directory()?;
//! let mut collection = file
//!     .load()?
//!     .unwrap_or_else(|| Collection::new("/home/you/music"));
//!
//! for track in collection.scan(None)? {
//!     println!("{} is {}", track.name, track.identifier);
//! }
//!
//! file.save(&collection)?;
//! # Ok::<(), jive_filesystem::Error>(())
//! ```
//!
//! # Track identity
//!
//! A track is keyed by its path relative to the root — `rock/one.mp3` rather
//! than `/home/you/music/rock/one.mp3` — and the root is the only absolute path
//! stored. Three consequences follow:
//!
//! * The root may be moved or renamed. Every identifier, and so every event
//!   recorded against it, still applies.
//! * Renaming a track, or any directory above it, is not tracked. The new path
//!   has not been seen before, so it is assigned a new identifier and starts
//!   with an empty history. The file system records no link between the two
//!   paths, so none is inferred.
//! * A directory outside the root cannot be scanned, because its files have no
//!   path relative to the root. [`Collection::scan`] returns
//!   [`Error::OutsideRoot`].
//!
//! Identifiers are never reused. A track that disappears from the root keeps
//! its entry, so restoring the file to the same path restores its history. The
//! next identifier to assign is stored rather than derived from the entries
//! present, so an entry removed by other means never has its identifier
//! assigned to another track.
//!
//! # Compatibility
//!
//! A file written by a later version of jive still loads. Unknown fields are
//! ignored, and unknown events are preserved verbatim and written back on the
//! next save, though they never reach a [`History`](jive_core::History). Only a
//! [`COLLECTION_VERSION`] above this one is rejected, since that indicates the
//! layout has changed rather than merely grown.
//!
//! # Feature flags
//!
//! * `testing` — exposes the `testing` module, which lays out music directories
//!   and collection files for test code. It carries no stability guarantee.

pub mod formats;

mod catalog;
mod collection;
mod discovery;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use catalog::Catalog;
pub use collection::COLLECTION_VERSION;
pub use collection::Collection;
pub use collection::CollectionFile;
pub use discovery::DiscoveredTrack;

use std::path::PathBuf;

/// What can go wrong reading the music or its collection file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The platform reports no application data directory.
    #[error("this platform has no application data directory: pass a state file explicitly")]
    NoDataDirectory,

    /// The collection file could not be read, parsed, or written.
    #[error("the state file `{}` could not be used: {message}", path.display())]
    File {
        /// The collection file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },

    /// The collection file was written by a later version of jive.
    #[error("the state file `{}` has unsupported version {version}", path.display())]
    UnsupportedVersion {
        /// The collection file.
        path: PathBuf,
        /// The version found in the file.
        version: u32,
    },

    /// The directory to play is not below the root.
    #[error("`{}` is not below `{}`: pass --root to play music kept somewhere else", path.display(), root.display())]
    OutsideRoot {
        /// The directory that was requested.
        path: PathBuf,
        /// The root the music lives below.
        root: PathBuf,
    },

    /// The path to play is not a directory.
    #[error("`{}` is not a directory", path.display())]
    NotADirectory {
        /// The path that was requested.
        path: PathBuf,
    },

    /// The directory could not be walked.
    #[error("`{}` could not be read: {message}", path.display())]
    Unreadable {
        /// The directory that was searched.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
