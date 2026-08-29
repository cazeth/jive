//! What can stop the player from running.

use jive_core::BackendError;
use std::io;
use std::path::PathBuf;

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything that stops the player.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An argument that is not recognized.
    #[error("unknown argument `{argument}`: pass --help for what is understood")]
    UnknownArgument {
        /// The argument as it was given.
        argument: String,
    },

    /// A flag that takes a value was given without one.
    #[error("`{flag}` needs a directory after it")]
    MissingValue {
        /// The flag given without a value.
        flag: String,
    },

    /// A value that should have been a number was not.
    #[error("`{value}` is not a number of {unit}")]
    NotANumber {
        /// The value as it was given.
        value: String,
        /// The unit it was meant to count.
        unit: &'static str,
    },

    /// Neither the command line nor the collection file says where the music
    /// is.
    #[error("nothing to play: pass a directory, or say where your music lives with --root")]
    NoDirectory,

    /// The directory holds no audio files.
    #[error("`{}` holds no audio files", path.display())]
    NoTracks {
        /// The directory that was searched.
        path: PathBuf,
    },

    /// The music or its collection file could not be read or written.
    #[error(transparent)]
    Filesystem(#[from] jive_filesystem::Error),

    /// The terminal could not be driven.
    #[error("the terminal could not be used: {source}")]
    Terminal {
        /// What went wrong.
        #[from]
        source: io::Error,
    },

    /// The audio backend is unusable.
    #[error(transparent)]
    Backend {
        /// What went wrong.
        #[from]
        source: BackendError,
    },
}
