//! Walking a directory for audio files.
//!
//! Only files whose extension appears in [`crate::formats`] are returned.
//! Results are sorted by path, so the same directory always produces the same
//! list in the same order.

use crate::Error;
use crate::Result;
use crate::formats::is_supported;
use jive_core::TrackId;
use jive_core::TrackName;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use walkdir::WalkDir;

/// An audio file found below the root, and the identifier assigned to it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiscoveredTrack {
    /// The identifier assigned by the catalog.
    pub identifier: TrackId,
    /// The name to display: the file name without its extension.
    pub name: TrackName,
    /// The absolute path of the file.
    pub path: PathBuf,
}

/// Every audio file below `root`, sorted by path.
///
/// Symbolic links are not followed. An unreadable directory below `root` is
/// skipped along with its contents rather than failing the scan.
///
/// # Errors
///
/// [`Error::NotADirectory`] if `root` is not one, [`Error::Unreadable`] if
/// `root` itself cannot be read.
pub(crate) fn audio_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        return Err(Error::NotADirectory {
            path: root.to_path_buf(),
        });
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(root) {
        if let Some(file) = audio_file(entry, root)? {
            files.push(file);
        }
    }
    files.sort();
    Ok(files)
}

/// The name a track is shown under: its file name, without the extension.
pub(crate) fn display_name(path: &Path) -> TrackName {
    let stem = path
        .file_stem()
        .or_else(|| path.file_name())
        .map(OsStr::to_string_lossy)
        .unwrap_or_default();
    TrackName::new(stem)
}

/// The audio file an entry names, or [`None`] for a directory, a file of
/// another kind, or an unreadable entry below the root.
fn audio_file(entry: walkdir::Result<walkdir::DirEntry>, root: &Path) -> Result<Option<PathBuf>> {
    let entry = match entry {
        Ok(entry) => entry,
        Err(error) if is_below_root(&error) => return Ok(None),
        Err(error) => return Err(unreadable(root, &error)),
    };
    if entry.file_type().is_file() && is_supported(entry.path()) {
        return Ok(Some(entry.path().to_path_buf()));
    }
    Ok(None)
}

fn is_below_root(error: &walkdir::Error) -> bool {
    error.depth() > 0
}

fn unreadable(root: &Path, error: &walkdir::Error) -> Error {
    Error::Unreadable {
        path: root.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::audio_files;
    use super::display_name;
    use crate::formats::SUPPORTED_EXTENSIONS;
    use crate::testing::directory_holding;
    use std::path::Path;
    use std::path::PathBuf;

    fn found_in(directory: &Path) -> Vec<PathBuf> {
        audio_files(directory).expect("the directory can be read")
    }

    fn names_found_in(directory: &Path) -> Vec<String> {
        found_in(directory)
            .iter()
            .map(|path| display_name(path).to_string())
            .collect()
    }

    /// The names found in a directory laid out with `files`.
    fn names_among(files: &[&str]) -> Vec<String> {
        let directory = directory_holding(files);
        names_found_in(directory.path())
    }

    fn one_of_each_extension() -> Vec<String> {
        SUPPORTED_EXTENSIONS
            .iter()
            .map(|extension| format!("track.{extension}"))
            .collect()
    }

    /// One test per `files in a directory => the tracks found there, named and
    /// in order` row.
    ///
    /// Rows name the expected tracks rather than counting them, so that a scan
    /// returning the right number of the wrong files still fails.
    macro_rules! finds {
        ($($name:ident: $files:expr => $tracks:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(names_among(&$files), $tracks);
                }
            )+
        };
    }

    finds! {
        an_empty_directory_yields_nothing: [] => Vec::<String>::new();
        a_directory_of_one_track_yields_it: ["song.mp3"] => ["song"];
        tracks_are_found_below_the_directory:
            ["b.mp3", "nested/a.flac", "deep/deeper/c.opus"] => ["b", "c", "a"];
        tracks_are_ordered_by_where_they_sit:
            ["b.mp3", "nested/a.flac", "deep/c.opus"] => ["b", "c", "a"];
        files_that_are_not_audio_are_left_alone:
            ["cover.jpg", "notes.txt", "song.mp3", "playlist.m3u"] => ["song"];
        a_file_without_an_extension_is_left_alone: ["README", "song.mp3"] => ["song"];
        a_file_that_is_only_an_extension_is_left_alone: [".mp3", "song.mp3"] => ["song"];
        extensions_are_matched_whatever_their_case:
            ["Loud.MP3", "Quiet.FlAc", "Mixed.OpUs"] => ["Loud", "Mixed", "Quiet"];
        a_name_keeps_its_spacing: ["A Long Song Name.mp3"] => ["A Long Song Name"];
        a_name_keeps_characters_beyond_ascii: ["東京の夜.mp3"] => ["東京の夜"];
        a_name_keeps_everything_before_the_last_dot:
            ["artist - track.take 2.mp3"] => ["artist - track.take 2"];
        a_hidden_file_is_still_a_track: [".hidden.mp3"] => [".hidden"];
        a_directory_named_like_a_track_is_not_one: ["album.mp3/song.flac"] => ["song"];
        an_empty_nested_directory_yields_nothing: ["nested/deeper/song.mp3"] => ["song"];
    }

    #[test]
    fn every_known_extension_is_found() {
        let files = one_of_each_extension();
        let borrowed: Vec<&str> = files.iter().map(String::as_str).collect();
        assert_eq!(names_among(&borrowed).len(), SUPPORTED_EXTENSIONS.len());
    }

    #[test]
    fn a_path_that_does_not_exist_is_reported() {
        assert!(audio_files(Path::new("no/such/directory")).is_err());
    }

    #[test]
    fn a_file_is_not_a_directory_to_play() {
        let directory = directory_holding(&["song.mp3"]);
        assert!(audio_files(&directory.path().join("song.mp3")).is_err());
    }

    #[test]
    fn a_track_keeps_the_whole_path_it_was_found_at() {
        let directory = directory_holding(&["nested/A Song.mp3"]);
        let found = found_in(directory.path());
        let path = found.first().expect("one track");
        assert_eq!(display_name(path).as_str(), "A Song");
        assert!(path.starts_with(directory.path()));
        assert!(path.ends_with(Path::new("nested").join("A Song.mp3")));
    }

    #[test]
    fn the_order_does_not_depend_on_what_the_file_system_says() {
        let directory = directory_holding(&["z.mp3", "a.mp3", "m/b.mp3", "m/a.mp3"]);
        let found = found_in(directory.path());
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted);
    }

    #[test]
    fn scanning_the_same_directory_twice_gives_the_same_answer() {
        let directory = directory_holding(&["b.mp3", "a/one.flac", "a/two.opus"]);
        assert_eq!(found_in(directory.path()), found_in(directory.path()));
    }

    #[test]
    fn a_large_collection_is_found_whole() {
        let files: Vec<String> = (0..300)
            .map(|number| format!("album {}/track {number}.mp3", number % 12))
            .collect();
        let borrowed: Vec<&str> = files.iter().map(String::as_str).collect();
        let directory = directory_holding(&borrowed);
        assert_eq!(found_in(directory.path()).len(), 300);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_track_is_not_followed() {
        let directory = directory_holding(&["real/song.mp3"]);
        std::os::unix::fs::symlink(
            directory.path().join("real").join("song.mp3"),
            directory.path().join("link.mp3"),
        )
        .expect("a symbolic link");
        assert_eq!(names_found_in(directory.path()), ["song"]);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_does_not_stop_the_rest_from_playing() {
        use std::os::unix::fs::PermissionsExt;

        let directory = directory_holding(&["reachable.mp3", "locked/hidden.mp3"]);
        let locked = directory.path().join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .expect("permissions can be set");

        if std::fs::read_dir(&locked).is_ok() {
            // Running as a user that bypasses permissions, so there is nothing
            // for this test to check.
            return;
        }

        let found = names_found_in(directory.path());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .expect("permissions can be restored");
        assert_eq!(found, ["reachable"]);
    }
}
