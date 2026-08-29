//! Builders for the music directories and collection files this crate reads,
//! for use in tests.
//!
//! Always available to this crate's own tests, and to other crates through the
//! `testing` feature. Nothing here is compiled into a normal build.
//!
//! The feature exports [`directory_holding`] and [`add_track`]. Everything else
//! here is `#[cfg(test)]`, for this crate's own tests, including the event
//! histories those tests store against a track.

use std::path::Path;
use tempfile::TempDir;

#[cfg(test)]
use crate::Collection;
#[cfg(test)]
use crate::CollectionFile;
#[cfg(test)]
use jive_core::Time;
#[cfg(test)]
use jive_core::track_events::PlaybackOutcome;
#[cfg(test)]
use jive_core::track_events::TimeTaggedTrackEvents;

/// The contents of every track laid down here.
///
/// Discovery goes by extension, and nothing that reads these plays one.
const NOT_REALLY_AUDIO: &[u8] = b"not really audio";

/// A temporary directory holding the named tracks.
///
/// A name may contain separators, and the directories it implies are created.
/// The directory is removed when the returned value is dropped, so a test must
/// keep it alive for as long as it needs the files.
///
/// # Panics
///
/// If the directory or any of the tracks cannot be created.
#[must_use]
pub fn directory_holding(files: &[&str]) -> TempDir {
    let directory = TempDir::new().expect("a temporary directory");
    for file in files {
        add_track(directory.path(), file);
    }
    directory
}

/// Lays one more track down in a directory that already exists.
///
/// For tests about music appearing between one scan and the next, where the
/// directory must outlive the addition.
///
/// # Panics
///
/// If the track cannot be created.
pub fn add_track(directory: &Path, name: &str) {
    let path = directory.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("a parent directory");
    }
    std::fs::write(&path, NOT_REALLY_AUDIO).expect("a file");
}

/// One finish, recorded at the epoch.
///
/// The smallest non-empty history, for tests that need a track with events but
/// do not care which.
#[cfg(test)]
pub(crate) fn finished_once() -> TimeTaggedTrackEvents {
    let mut events = TimeTaggedTrackEvents::new();
    events.record(Time::EPOCH, PlaybackOutcome::Finished);
    events
}

/// A collection scanned from a real directory, every track having finished
/// once.
#[cfg(test)]
pub(crate) fn collection_over(directory: &Path) -> Collection {
    let mut collection = Collection::new(directory);
    let tracks = collection.scan(None).expect("the directory can be read");
    for track in tracks {
        collection
            .history_mut()
            .store(track.identifier, finished_once());
    }
    collection
}

/// The relative path and identifier number of every track in the catalog.
#[cfg(test)]
pub(crate) fn named_tracks(collection: &Collection) -> Vec<(String, u32)> {
    collection
        .catalog()
        .tracks()
        .map(|(identifier, path)| (path.to_string_lossy().into_owned(), identifier.number()))
        .collect()
}

/// The collection a file holding `contents` loads as.
///
/// The file is removed when the call returns, so this is for what reading a
/// given text produces, not for anything written back.
#[cfg(test)]
pub(crate) fn loaded_from(contents: &str) -> Collection {
    let fixture = Fixture::new();
    let loaded = fixture.write_raw(contents).load();
    loaded.expect("the file loads").expect("a collection")
}

/// A collection file that is removed with the test, one directory deeper than
/// the root so that creating that directory is exercised too.
#[cfg(test)]
pub(crate) struct Fixture {
    directory: TempDir,
}

#[cfg(test)]
impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl Fixture {
    /// A fixture over a directory of its own.
    pub(crate) fn new() -> Self {
        Self {
            directory: TempDir::new().expect("a temporary directory"),
        }
    }

    /// The collection file, below a directory that does not exist yet.
    pub(crate) fn file(&self) -> CollectionFile {
        CollectionFile::at(self.directory.path().join("nested").join("state.json"))
    }

    /// Writes `contents` where the collection file lives, valid or not.
    pub(crate) fn write_raw(&self, contents: &str) -> CollectionFile {
        let file = self.file();
        std::fs::create_dir_all(file.path().parent().expect("a parent")).expect("a directory");
        std::fs::write(file.path(), contents).expect("a written file");
        file
    }

    /// The raw contents of the collection file.
    pub(crate) fn read_raw(&self) -> String {
        std::fs::read_to_string(self.file().path()).expect("a written file")
    }

    /// Writes a collection to this fixture's file.
    pub(crate) fn save(&self, collection: &Collection) {
        self.file()
            .save(collection)
            .expect("the collection is written");
    }

    /// The collection stored in this fixture's file.
    pub(crate) fn stored(&self) -> Collection {
        self.file()
            .load()
            .expect("the collection loads")
            .expect("a collection")
    }

    /// A whole session over a file: write it, load it, then save it back.
    pub(crate) fn after_a_session_over(contents: &str) -> Self {
        let fixture = Self::new();
        let file = fixture.write_raw(contents);
        let collection = file.load().expect("the file loads").expect("a collection");
        file.save(&collection).expect("the collection is written");
        fixture
    }

    /// The file names beside the collection file, for spotting scratch files.
    pub(crate) fn leftovers(&self) -> Vec<String> {
        let parent = self.file().path().parent().expect("a parent").to_path_buf();
        let Ok(entries) = std::fs::read_dir(parent) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::add_track;
    use super::directory_holding;

    #[test]
    fn a_name_without_a_separator_becomes_a_file_in_the_directory() {
        let directory = directory_holding(&["song.mp3"]);
        assert!(directory.path().join("song.mp3").is_file());
    }

    #[test]
    fn a_name_with_separators_creates_the_directories_it_implies() {
        let directory = directory_holding(&["rock/live/song.mp3"]);
        assert!(directory.path().join("rock").join("live").is_dir());
        assert!(
            directory
                .path()
                .join("rock")
                .join("live")
                .join("song.mp3")
                .is_file()
        );
    }

    /// Tests that scan, add a track, and scan again depend on a track added
    /// later being indistinguishable from one laid down at the start.
    #[test]
    fn a_track_added_later_holds_what_one_laid_down_at_the_start_holds() {
        let directory = directory_holding(&["first.mp3"]);
        add_track(directory.path(), "nested/second.mp3");

        let first = std::fs::read(directory.path().join("first.mp3")).expect("the first track");
        let second = std::fs::read(directory.path().join("nested").join("second.mp3"))
            .expect("the second track");
        assert_eq!(first, second);
        assert!(!first.is_empty(), "a track should be a file with contents");
    }

    #[test]
    fn a_directory_holding_no_tracks_is_still_an_empty_directory() {
        let directory = directory_holding(&[]);
        assert!(directory.path().is_dir());
        let entries = std::fs::read_dir(directory.path()).expect("a readable directory");
        assert_eq!(entries.count(), 0);
    }
}
