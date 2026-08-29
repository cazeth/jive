//! The file extensions jive treats as audio.
//!
//! [`SUPPORTED_EXTENSIONS`] determines which files become tracks. It is not
//! checked against what an [`AudioBackend`] can decode, and the two may
//! disagree in either direction: a listed extension no backend can decode
//! becomes a track that fails when played, and an extension a backend supports
//! but the list omits is never discovered.
//!
//! [`AudioBackend`]: jive_core::AudioBackend

use std::ffi::OsStr;
use std::path::Path;

/// The file extensions jive treats as tracks, lowercase.
pub const SUPPORTED_EXTENSIONS: [&str; 16] = [
    "aac", "aif", "aiff", "alac", "ape", "flac", "m4a", "m4b", "mka", "mp3", "oga", "ogg", "opus",
    "wav", "wma", "wv",
];

/// Whether a path names a file type jive treats as a track.
///
/// Determined by extension alone. A file whose contents do not match its
/// extension is detected only when a backend tries to play it.
#[must_use]
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .map(OsStr::to_string_lossy)
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| SUPPORTED_EXTENSIONS.contains(&extension.as_str()))
}

#[cfg(test)]
mod tests {
    use super::SUPPORTED_EXTENSIONS;
    use super::is_supported;
    use std::collections::HashSet;
    use std::path::Path;

    fn supports(name: &str) -> bool {
        is_supported(Path::new(name))
    }

    /// One test per `file name => whether it counts as a track` row.
    macro_rules! recognizes {
        ($($name:ident: $file:expr => $supported:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(supports($file), $supported);
                }
            )+
        };
    }

    recognizes! {
        a_listed_type_is_a_track: "song.mp3" => true;
        a_listed_type_in_capitals_is_a_track: "song.MP3" => true;
        a_listed_type_in_mixed_case_is_a_track: "song.FlAc" => true;
        an_unlisted_type_is_not_a_track: "song.dsf" => false;
        artwork_is_not_a_track: "cover.jpg" => false;
        a_cue_sheet_is_not_a_track: "album.cue" => false;
        a_file_without_an_extension_is_not_a_track: "README" => false;
        a_file_that_is_only_an_extension_is_not_a_track: ".mp3" => false;
        a_name_with_dots_goes_by_its_last_one: "artist - track.take 2.mp3" => true;
        a_directory_named_like_a_track_is_judged_the_same_way: "album.mp3" => true;
        a_path_is_judged_by_its_last_part: "/music/covers.jpg/song.flac" => true;
    }

    /// Every rule [`SUPPORTED_EXTENSIONS`] must follow for matching to work,
    /// checked per entry so that a failure names the extension that broke it.
    #[test]
    fn every_listed_extension_is_written_the_way_matching_expects() {
        let mut seen = HashSet::new();
        for extension in SUPPORTED_EXTENSIONS {
            assert!(!extension.is_empty(), "the list holds an empty extension");
            assert!(
                !extension.contains('.'),
                "{extension} should be written without its dot"
            );
            assert_eq!(
                extension.to_ascii_lowercase(),
                extension,
                "{extension} should be lowercase, since matching lowercases first"
            );
            assert!(seen.insert(extension), "{extension} is listed twice");
            assert!(
                supports(&format!("song.{extension}")),
                "{extension} is listed but not recognized"
            );
        }
    }
}
