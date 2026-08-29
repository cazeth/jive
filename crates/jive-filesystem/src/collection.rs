//! A music collection and the file it is stored in.
//!
//! A [`Collection`] pairs a [`Catalog`] with the [`History`] recorded against
//! it. [`CollectionFile`] reads and writes one as a single JSON file.
//!
//! # Compatibility
//!
//! [`COLLECTION_VERSION`] identifies the layout of the file. It is incremented
//! only when that layout changes incompatibly — a field removed, renamed, or
//! given a new meaning — and a reader rejects any version above its own.
//!
//! Additions need no new version, so a file written by a later version of jive
//! still loads:
//!
//! * Unknown fields are ignored. Fields added later carry `#[serde(default)]`
//!   so that files predating them still parse. An older jive cannot preserve
//!   their values, having nowhere to put them.
//! * Unknown events are retained as the text they were read as and written back
//!   unchanged, so running an older jive does not discard what a newer one
//!   recorded. They never reach a [`History`].
//!
//! A file is therefore rejected only if its version demands it, or if it cannot
//! be parsed at all. A single unreadable event is not enough.
//!
//! Version 1 keyed tracks by absolute path. Such a file is migrated by rebasing
//! those paths onto a root. See [`CollectionFile::load`].

use crate::Error;
use crate::Result;
use crate::catalog::Catalog;
use crate::discovery::DiscoveredTrack;
use crate::discovery::audio_files;
use crate::discovery::display_name;
use jive_core::History;
use jive_core::TrackId;
use jive_core::track_events::TimeTaggedTrackEvent;
use jive_core::track_events::TimeTaggedTrackEvents;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

/// The version this build writes collection files as.
pub const COLLECTION_VERSION: u32 = 2;

/// The directory the collection file lives in, below the platform data
/// directory.
const DIRECTORY_NAME: &str = "jive";

/// The name of the collection file.
const FILE_NAME: &str = "state.json";

/// Stands in for a track with no events recorded against it.
static NOTHING: TimeTaggedTrackEvents = TimeTaggedTrackEvents::new();

/// A root directory, the tracks below it, and the events recorded against each.
#[derive(Debug, Clone, PartialEq)]
pub struct Collection {
    catalog: Catalog,
    history: History,
    /// Events that could not be parsed, keyed by the track they were recorded
    /// against and written back verbatim on the next save.
    kept: BTreeMap<TrackId, Vec<serde_json::Value>>,
}

impl Collection {
    /// An empty collection for the music below `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            catalog: Catalog::new(root),
            history: History::new(),
            kept: BTreeMap::new(),
        }
    }

    /// The root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.catalog.root()
    }

    /// Changes the root directory.
    ///
    /// Identifiers, and the history recorded against them, are unaffected.
    pub fn set_root(&mut self, root: impl Into<PathBuf>) {
        self.catalog.set_root(root);
    }

    /// The tracks below the root and their identifiers.
    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The events recorded against every track.
    #[must_use]
    pub fn history(&self) -> &History {
        &self.history
    }

    /// The events recorded against every track, for modification.
    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// Every track below `under`, or below the root if `under` is [`None`].
    ///
    /// Tracks seen for the first time are assigned identifiers, which reach the
    /// disk only when the collection is next saved.
    ///
    /// # Errors
    ///
    /// [`Error::OutsideRoot`] if `under` is not below the root,
    /// [`Error::NotADirectory`] if it is not a directory, and
    /// [`Error::Unreadable`] if it cannot be read.
    pub fn scan(&mut self, under: Option<&Path>) -> Result<Vec<DiscoveredTrack>> {
        let root = self.catalog.root().to_path_buf();
        let directory = under.unwrap_or(&root);
        if !directory.starts_with(&root) {
            return Err(Error::OutsideRoot {
                path: directory.to_path_buf(),
                root,
            });
        }

        let mut tracks = Vec::new();
        for file in audio_files(directory)? {
            let Ok(below_root) = file.strip_prefix(&root) else {
                continue;
            };
            tracks.push(DiscoveredTrack {
                identifier: self.catalog.identify(below_root),
                name: display_name(&file),
                path: file,
            });
        }
        Ok(tracks)
    }
}

/// A collection stored in a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionFile {
    path: PathBuf,
}

impl CollectionFile {
    /// The collection file inside the platform data directory.
    ///
    /// # Errors
    ///
    /// [`Error::NoDataDirectory`] if the platform reports none.
    pub fn in_data_directory() -> Result<Self> {
        let directory = dirs::data_dir().ok_or(Error::NoDataDirectory)?;
        Ok(Self::at(directory.join(DIRECTORY_NAME).join(FILE_NAME)))
    }

    /// The collection file at `path`.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The path of the file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the collection, or [`None`] if the file does not exist.
    ///
    /// Events this version cannot parse are retained, and written back
    /// unchanged by [`CollectionFile::save`].
    ///
    /// A version 1 file keyed tracks by absolute path. It is migrated by
    /// rebasing those paths onto the root directory it recorded, or, if it
    /// recorded none, onto the deepest directory containing all of them. Tracks
    /// outside that directory are dropped.
    ///
    /// # Errors
    ///
    /// [`Error::File`] if the file cannot be read or parsed, or if its contents
    /// are inconsistent, and [`Error::UnsupportedVersion`] if a later version
    /// wrote it.
    pub fn load(&self) -> Result<Option<Collection>> {
        let Some(contents) = self.read()? else {
            return Ok(None);
        };
        let written: serde_json::Value =
            serde_json::from_str(&contents).map_err(|error| self.failure(&error))?;
        let version = self.version_of(&written)?;
        match version {
            1 => Ok(self.parse::<WrittenV1>(written)?.carried_over()),
            COLLECTION_VERSION => self
                .parse::<Written>(written)?
                .restored()
                .map(Some)
                .map_err(|message| self.failure(&message)),
            version => Err(Error::UnsupportedVersion {
                path: self.path.clone(),
                version,
            }),
        }
    }

    /// Writes the collection, replacing the file only once it is complete.
    ///
    /// The contents are written to a scratch file, flushed, and renamed over
    /// the collection file. Closing a file only hands it to the operating
    /// system, so without the flush the rename may reach the disk before the
    /// data it points at, leaving an empty file behind a crash. Concurrent
    /// writers cannot corrupt each other, each using its own scratch file, but
    /// the last to finish wins.
    ///
    /// Every event read is written back. Events this version could not parse
    /// are written after those it could, so their position within a track is
    /// not preserved. Nothing depends on it, as they never reach a [`History`].
    ///
    /// # Errors
    ///
    /// [`Error::File`] if the file cannot be written.
    pub fn save(&self, collection: &Collection) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| self.failure(&error))?;
        }
        let encoded = serde_json::to_string_pretty(&Writing::of(collection))
            .map_err(|error| self.failure(&error))?;
        let scratch = self.scratch_path();
        self.replace_with(&scratch, &encoded).inspect_err(|_| {
            let _ = std::fs::remove_file(&scratch);
        })
    }

    /// The contents of the file, or [`None`] if it does not exist.
    fn read(&self) -> Result<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(self.failure(&error)),
        }
    }

    /// The layout version the file claims to be written in.
    fn version_of(&self, written: &serde_json::Value) -> Result<u32> {
        let versioned: Versioned =
            serde_json::from_value(written.clone()).map_err(|error| self.failure(&error))?;
        Ok(versioned.version)
    }

    fn parse<Shape: serde::de::DeserializeOwned>(
        &self,
        written: serde_json::Value,
    ) -> Result<Shape> {
        serde_json::from_value(written).map_err(|error| self.failure(&error))
    }

    /// Fills the scratch file, flushes it, and renames it over the collection
    /// file.
    fn replace_with(&self, scratch: &Path, encoded: &str) -> Result<()> {
        let mut file = File::create(scratch).map_err(|error| self.failure(&error))?;
        file.write_all(encoded.as_bytes())
            .map_err(|error| self.failure(&error))?;
        file.sync_all().map_err(|error| self.failure(&error))?;
        drop(file);
        std::fs::rename(scratch, &self.path).map_err(|error| self.failure(&error))?;
        // The rename is a change to the directory, and it is durable only once
        // the directory itself is flushed. A directory cannot be opened this
        // way on Windows, and a failed flush costs no more than the durability
        // it was meant to add, so nothing here is reported.
        #[cfg(unix)]
        if let Some(parent) = self.path.parent() {
            let _ = File::open(parent).and_then(|directory| directory.sync_all());
        }
        Ok(())
    }

    fn scratch_path(&self) -> PathBuf {
        self.path
            .with_extension(format!("json.{}.writing", std::process::id()))
    }

    fn failure(&self, error: &dyn std::fmt::Display) -> Error {
        Error::File {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

/// Enough of any collection file to tell which layout the rest of it uses.
#[derive(serde::Deserialize)]
struct Versioned {
    version: u32,
}

/// One event as stored in the file: either an event this version understands,
/// or the JSON it was read as.
///
/// An unknown event is set aside rather than rejecting the whole file, and is
/// written back unchanged on the next save, so running an older jive does not
/// discard what a newer one recorded. Only [`Known`] events reach a
/// [`History`].
///
/// Deserialization is by shape, so anything unrecognized becomes [`Unknown`]:
/// an event from a later version, one from before this format settled, or
/// damaged text. These cannot be distinguished, and treating them alike costs
/// only the space they occupy.
///
/// [`Known`]: StoredEvent::Known
/// [`Unknown`]: StoredEvent::Unknown
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
enum StoredEvent {
    /// An event this version understands.
    Known(TimeTaggedTrackEvent),
    /// Anything else, as it was read.
    Unknown(serde_json::Value),
}

/// A track's stored events, split into those this version understands and those
/// it only carries.
fn split(stored: Vec<StoredEvent>) -> (TimeTaggedTrackEvents, Vec<serde_json::Value>) {
    let mut known = Vec::new();
    let mut kept = Vec::new();
    for event in stored {
        match event {
            StoredEvent::Known(event) => known.push(event),
            StoredEvent::Unknown(value) => kept.push(value),
        }
    }
    (known.into_iter().collect(), kept)
}

/// A collection file as it is written.
#[derive(serde::Serialize)]
struct Writing<'collection> {
    version: u32,
    root: &'collection Path,
    next_id: u32,
    tracks: Vec<WritingTrack<'collection>>,
}

#[derive(serde::Serialize)]
struct WritingTrack<'collection> {
    id: TrackId,
    path: &'collection Path,
    events: Vec<WritingEvent<'collection>>,
}

/// One event on its way to the file: one this version recorded, or one it
/// retained unparsed.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum WritingEvent<'collection> {
    Known(&'collection TimeTaggedTrackEvent),
    Kept(&'collection serde_json::Value),
}

impl<'collection> Writing<'collection> {
    fn of(collection: &'collection Collection) -> Self {
        Self {
            version: COLLECTION_VERSION,
            root: collection.catalog.root(),
            next_id: collection.catalog.next(),
            tracks: collection
                .catalog
                .tracks()
                .map(|(identifier, path)| WritingTrack {
                    id: identifier,
                    path,
                    events: events_of(collection, identifier),
                })
                .collect(),
        }
    }
}

/// Everything to write against a track: the events this version recorded, then
/// the ones it retained but could not parse.
fn events_of(collection: &Collection, identifier: TrackId) -> Vec<WritingEvent<'_>> {
    let known = collection
        .history
        .events_for(identifier)
        .unwrap_or(&NOTHING)
        .iter()
        .map(WritingEvent::Known);
    let kept = collection
        .kept
        .get(&identifier)
        .into_iter()
        .flatten()
        .map(WritingEvent::Kept);
    known.chain(kept).collect()
}

/// A collection file as it is read.
#[derive(serde::Deserialize)]
struct Written {
    root: PathBuf,
    next_id: u32,
    #[serde(default)]
    tracks: Vec<WrittenTrack>,
}

#[derive(serde::Deserialize)]
struct WrittenTrack {
    id: TrackId,
    path: PathBuf,
    events: Vec<StoredEvent>,
}

impl Written {
    /// The collection this file describes, or a description of why it is
    /// unusable.
    fn restored(self) -> std::result::Result<Collection, String> {
        let tracks: Vec<(PathBuf, TrackId)> = self
            .tracks
            .iter()
            .map(|track| (track.path.clone(), track.id))
            .collect();
        let catalog = Catalog::restore(self.root, self.next_id, tracks)?;
        let mut history = History::new();
        let mut kept = BTreeMap::new();
        for track in self.tracks {
            let (known, unknown) = split(track.events);
            history.store(track.id, known);
            if !unknown.is_empty() {
                kept.insert(track.id, unknown);
            }
        }
        Ok(Collection {
            catalog,
            history,
            kept,
        })
    }
}

/// A collection file as version 1 wrote one: tracks keyed by absolute path,
/// plus the directory to play when none was given.
#[derive(serde::Deserialize)]
struct WrittenV1 {
    #[serde(default)]
    default_directory: Option<PathBuf>,
    #[serde(default)]
    tracks: Vec<WrittenV1Track>,
}

#[derive(serde::Deserialize)]
struct WrittenV1Track {
    path: PathBuf,
    events: Vec<StoredEvent>,
}

impl WrittenV1 {
    /// The collection this file describes, or [`None`] if no root can be
    /// derived, leaving no relative path to key tracks by.
    fn carried_over(mut self) -> Option<Collection> {
        self.tracks.sort_by(|one, other| one.path.cmp(&other.path));
        let root = self.default_directory.clone().or_else(|| {
            deepest_shared_directory(self.tracks.iter().map(|track| track.path.as_path()))
        })?;

        let mut collection = Collection::new(root.clone());
        for track in self.tracks {
            let Ok(path) = track.path.strip_prefix(&root) else {
                continue;
            };
            let identifier = collection.catalog.identify(path);
            let (known, unknown) = split(track.events);
            collection.history.store(identifier, known);
            if !unknown.is_empty() {
                collection.kept.insert(identifier, unknown);
            }
        }
        Some(collection)
    }
}

/// The deepest directory containing every one of `paths`, which are files.
fn deepest_shared_directory<'paths>(paths: impl Iterator<Item = &'paths Path>) -> Option<PathBuf> {
    let mut shared: Option<PathBuf> = None;
    for path in paths {
        let directory = path.parent()?;
        shared = Some(match shared {
            None => directory.to_path_buf(),
            Some(so_far) => shared_prefix(&so_far, directory),
        });
    }
    shared.filter(|directory| !directory.as_os_str().is_empty())
}

/// The longest leading run of components two paths share.
fn shared_prefix(left: &Path, right: &Path) -> PathBuf {
    let mut shared = PathBuf::new();
    for (one, other) in left.components().zip(right.components()) {
        if one != other {
            break;
        }
        shared.push(one.as_os_str());
    }
    shared
}

#[cfg(test)]
mod tests {
    use super::Collection;
    use super::CollectionFile;
    use crate::testing::Fixture;
    use crate::testing::add_track;
    use crate::testing::collection_over;
    use crate::testing::directory_holding;
    use crate::testing::finished_once;
    use crate::testing::loaded_from;
    use crate::testing::named_tracks as names;
    use jive_core::TrackId;
    use std::path::Path;
    use std::path::PathBuf;

    /// One test per file that must be rejected rather than accepted.
    ///
    /// Each row gives only the file contents. Writing them where the collection
    /// lives and reading them back is the same every time, so it is written out
    /// once here.
    macro_rules! refuses {
        ($($name:ident: $contents:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    let loaded = Fixture::new().write_raw($contents).load();
                    assert!(loaded.is_err(), "the file should have been reported");
                }
            )+
        };
    }

    #[test]
    fn a_missing_file_holds_no_state_yet() {
        assert!(Fixture::new().file().load().expect("a read").is_none());
    }

    #[test]
    fn a_version_1_file_remembering_nothing_at_all_holds_no_state() {
        let loaded = Fixture::new()
            .write_raw(r#"{"version": 1, "tracks": []}"#)
            .load();
        assert!(loaded.expect("the file loads").is_none());
    }

    #[test]
    fn a_state_file_can_be_pointed_anywhere() {
        let elsewhere = PathBuf::from("/tmp/elsewhere.json");
        assert_eq!(CollectionFile::at(elsewhere.clone()).path(), elsewhere);
    }

    #[test]
    fn a_state_survives_a_round_trip() {
        let fixture = Fixture::new();
        let music = directory_holding(&["one.mp3", "nested/two.flac"]);
        let collection = collection_over(music.path());

        fixture.save(&collection);

        assert_eq!(fixture.stored(), collection);
    }

    const NOT_JSON: &str = "this is not json";
    const NOTHING_AT_ALL: &str = "";
    const NOT_AN_OBJECT: &str = "[]";

    const FROM_A_LATER_FORMAT: &str =
        r#"{"version": 99, "root": "/music", "next_id": 0, "tracks": []}"#;
    const WITHOUT_A_VERSION: &str = r#"{"root": "/music", "next_id": 0, "tracks": []}"#;
    const WITHOUT_A_ROOT: &str = r#"{"version": 2, "next_id": 0, "tracks": []}"#;

    const EVENTS_THAT_ARE_NOT_A_LIST: &str = r#"{
        "version": 2, "root": "/music", "next_id": 1,
        "tracks": [{"id": 0, "path": "x.mp3", "events": "nonsense"}]
    }"#;

    const A_TRACK_WITHOUT_AN_IDENTIFIER: &str = r#"{
        "version": 2, "root": "/music", "next_id": 1,
        "tracks": [{"path": "x.mp3", "events": []}]
    }"#;

    const TWO_TRACKS_SHARING_AN_IDENTIFIER: &str = r#"{
        "version": 2, "root": "/music", "next_id": 1,
        "tracks": [{"id": 0, "path": "a.mp3", "events": []},
                   {"id": 0, "path": "b.mp3", "events": []}]
    }"#;

    /// `next_id` is an identifier a track already holds, so the next scan would
    /// assign one that is taken.
    const AN_IDENTIFIER_ASSIGNED_TWICE: &str = r#"{
        "version": 2, "root": "/music", "next_id": 0,
        "tracks": [{"id": 0, "path": "a.mp3", "events": []}]
    }"#;

    refuses! {
        a_damaged_file_is_reported: NOT_JSON;
        an_empty_file_is_reported: NOTHING_AT_ALL;
        a_file_that_is_not_an_object_is_reported: NOT_AN_OBJECT;
        a_file_from_a_later_format_is_reported: FROM_A_LATER_FORMAT;
        a_file_missing_its_version_is_reported: WITHOUT_A_VERSION;
        a_file_missing_its_root_is_reported: WITHOUT_A_ROOT;
        a_track_whose_events_are_not_a_list_is_reported: EVENTS_THAT_ARE_NOT_A_LIST;
        a_track_missing_its_identifier_is_reported: A_TRACK_WITHOUT_AN_IDENTIFIER;
        two_tracks_sharing_an_identifier_are_reported: TWO_TRACKS_SHARING_AN_IDENTIFIER;
        an_identifier_that_would_be_assigned_again_is_reported: AN_IDENTIFIER_ASSIGNED_TWICE;
    }

    #[test]
    fn a_damaged_file_is_reported_against_its_own_path() {
        let fixture = Fixture::new();
        let message = fixture
            .write_raw("this is not json")
            .load()
            .expect_err("a damaged file fails")
            .to_string();
        assert!(message.contains("state.json"), "{message}");
    }

    #[test]
    fn saving_creates_the_directory_it_needs() {
        let fixture = Fixture::new();
        fixture.save(&Collection::new("/music"));
        assert!(fixture.file().path().exists());
    }

    #[test]
    fn saving_leaves_no_scratch_file_behind() {
        let fixture = Fixture::new();
        let music = directory_holding(&["one.mp3"]);
        let collection = collection_over(music.path());
        for _ in 0..3 {
            fixture.save(&collection);
        }
        assert_eq!(fixture.leftovers(), ["state.json"]);
    }

    /// A save that cannot finish must not leave its scratch file behind, since
    /// nothing else ever reads one or removes it.
    #[test]
    fn a_failed_save_clears_up_after_itself() {
        let fixture = Fixture::new();
        let file = fixture.file();
        std::fs::create_dir_all(file.path()).expect("a directory standing in for the file");

        assert!(
            file.save(&Collection::new("/music")).is_err(),
            "a directory cannot be replaced by a file"
        );
        assert_eq!(fixture.leftovers(), ["state.json"]);
    }

    #[test]
    fn tracks_are_written_in_a_settled_order() {
        let fixture = Fixture::new();
        let music = directory_holding(&["zebra.mp3", "apple.mp3", "mango.mp3"]);
        let collection = collection_over(music.path());

        fixture.save(&collection);
        let first = fixture.read_raw();
        fixture.save(&collection);

        assert_eq!(first, fixture.read_raw());
    }

    #[test]
    fn the_file_in_the_data_directory_lives_under_a_directory_of_its_own() {
        let Ok(file) = CollectionFile::in_data_directory() else {
            return;
        };
        assert!(
            file.path()
                .ends_with(PathBuf::from("jive").join("state.json"))
        );
    }

    #[test]
    fn scanning_names_every_track_below_the_root() {
        let music = directory_holding(&["b.mp3", "nested/a.flac", "cover.jpg"]);
        let mut collection = Collection::new(music.path());

        let tracks = collection.scan(None).expect("the directory can be read");

        assert_eq!(tracks.len(), 2);
        assert_eq!(
            names(&collection),
            [
                (String::from("b.mp3"), 0),
                (format!("nested{}a.flac", std::path::MAIN_SEPARATOR), 1),
            ]
        );
    }

    #[test]
    fn scanning_again_gives_every_track_the_name_it_had() {
        let music = directory_holding(&["a.mp3", "b.mp3"]);
        let mut collection = Collection::new(music.path());

        let first = collection.scan(None).expect("the directory can be read");
        let again = collection.scan(None).expect("the directory can be read");

        assert_eq!(first, again);
    }

    /// A track added later must not disturb the identifiers already assigned,
    /// or every history after it shifts onto a neighbouring track.
    #[test]
    fn a_track_added_later_is_named_after_the_ones_already_known() {
        let music = directory_holding(&["b.mp3", "c.mp3"]);
        let mut collection = Collection::new(music.path());
        collection.scan(None).expect("the directory can be read");

        add_track(music.path(), "a.mp3");
        collection.scan(None).expect("the directory can be read");

        assert_eq!(
            names(&collection),
            [
                (String::from("a.mp3"), 2),
                (String::from("b.mp3"), 0),
                (String::from("c.mp3"), 1),
            ]
        );
    }

    #[test]
    fn scanning_below_the_root_narrows_what_is_found_without_renaming_it() {
        let music = directory_holding(&["rock/a.mp3", "jazz/b.mp3"]);
        let mut collection = Collection::new(music.path());
        let all = collection.scan(None).expect("the directory can be read");

        let rock = collection
            .scan(Some(&music.path().join("rock")))
            .expect("the directory can be read");

        assert_eq!(all.len(), 2);
        assert_eq!(rock.len(), 1);
        let found = rock.first().expect("a track");
        assert_eq!(
            Some(found.identifier),
            all.iter()
                .find(|track| track.path == found.path)
                .map(|track| track.identifier),
            "narrowing the scan should not rename anything"
        );
    }

    #[test]
    fn a_directory_outside_the_root_is_refused() {
        let music = directory_holding(&["a.mp3"]);
        let elsewhere = directory_holding(&["b.mp3"]);
        let mut collection = Collection::new(music.path());

        assert!(collection.scan(Some(elsewhere.path())).is_err());
    }

    /// The point of keying tracks by relative path: the collection can be moved
    /// and every event still applies to the track it was recorded against.
    #[test]
    fn moving_the_music_leaves_every_track_named_as_it_was() {
        let music = directory_holding(&["rock/a.mp3", "b.mp3"]);
        let mut collection = collection_over(music.path());
        let before = names(&collection);

        let moved = directory_holding(&["rock/a.mp3", "b.mp3"]);
        collection.set_root(moved.path());
        let tracks = collection.scan(None).expect("the directory can be read");

        assert_eq!(
            names(&collection),
            before,
            "nothing should have been renamed"
        );
        // The catalog keeps its entries whatever the scan returned, so the
        // check below would hold vacuously over an empty list.
        assert_eq!(tracks.len(), 2, "both tracks should have been found again");
        assert!(
            tracks
                .iter()
                .all(|track| collection.history().events_for(track.identifier)
                    == Some(&finished_once())),
            "every track should have kept the events recorded against it"
        );
    }

    #[test]
    fn a_track_taken_out_of_the_directory_keeps_its_events() {
        let fixture = Fixture::new();
        let music = directory_holding(&["kept.mp3", "gone.mp3"]);
        let mut collection = collection_over(music.path());
        let gone = collection
            .catalog()
            .identifier_for(Path::new("gone.mp3"))
            .expect("a named track");

        std::fs::remove_file(music.path().join("gone.mp3")).expect("the file is removed");
        collection.scan(None).expect("the directory can be read");
        fixture.save(&collection);

        assert_eq!(
            fixture.stored().history().events_for(gone),
            Some(&finished_once())
        );
    }

    #[test]
    fn a_version_1_file_is_carried_over_onto_the_directory_it_remembered() {
        let collection = loaded_from(
            r#"{"version": 1, "default_directory": "/music", "tracks": [
                {"path": "/music/rock/a.mp3", "name": "a", "events": []},
                {"path": "/music/b.mp3", "name": "b", "events": []}
            ]}"#,
        );

        assert_eq!(collection.root(), Path::new("/music"));
        assert_eq!(
            names(&collection),
            [(String::from("b.mp3"), 0), (String::from("rock/a.mp3"), 1),]
        );
    }

    /// Without a remembered directory there is still one directory containing
    /// every track, and rebasing onto it preserves each track's history.
    #[test]
    fn a_version_1_file_without_a_directory_is_carried_over_onto_the_one_it_implies() {
        let collection = loaded_from(
            r#"{"version": 1, "tracks": [
                {"path": "/music/rock/a.mp3", "name": "a", "events": []},
                {"path": "/music/jazz/b.mp3", "name": "b", "events": []}
            ]}"#,
        );

        assert_eq!(collection.root(), Path::new("/music"));
        assert_eq!(collection.catalog().len(), 2);
    }

    #[test]
    fn a_version_1_track_from_outside_the_remembered_directory_is_left_behind() {
        let collection = loaded_from(
            r#"{"version": 1, "default_directory": "/music", "tracks": [
                {"path": "/music/a.mp3", "name": "a", "events": []},
                {"path": "/podcasts/b.mp3", "name": "b", "events": []}
            ]}"#,
        );

        assert_eq!(names(&collection), [(String::from("a.mp3"), 0)]);
    }

    /// Reading and writing a file written by a different version of jive.
    ///
    /// These state what an event or field added later costs: nothing that
    /// prevents startup, and nothing that the next save discards.
    mod compatibility {
        use crate::testing::Fixture;
        use crate::testing::finished_once;
        use crate::testing::loaded_from;
        use jive_core::TrackId;

        /// A file holding one event this version understands and one it does
        /// not, as a later jive that recorded ratings would have written.
        const FROM_A_LATER_VERSION: &str = r#"{
            "version": 2,
            "root": "/music",
            "next_id": 1,
            "tracks": [{"id": 0, "path": "a.mp3", "events": [
                {"at": 0, "event": {"event": "playback_outcome", "data": {"outcome": "finished"}}},
                {"at": 1500, "event": {"event": "rated", "data": {"stars": 4}}}
            ]}]
        }"#;

        /// The events written against the first track, as they reached the file.
        fn written_events(fixture: &Fixture) -> Vec<serde_json::Value> {
            let written: serde_json::Value =
                serde_json::from_str(&fixture.read_raw()).expect("the file is json");
            written["tracks"][0]["events"]
                .as_array()
                .expect("a track with events")
                .clone()
        }

        /// The written events that this version did not put there itself.
        fn kept_events(fixture: &Fixture) -> Vec<serde_json::Value> {
            written_events(fixture)
                .into_iter()
                .filter(|event| event["event"]["event"] == "rated")
                .collect()
        }

        #[test]
        fn an_event_this_version_cannot_read_does_not_stop_the_file_loading() {
            let loaded = Fixture::new().write_raw(FROM_A_LATER_VERSION).load();
            assert!(loaded.is_ok());
        }

        #[test]
        fn an_event_this_version_cannot_read_carries_no_weight() {
            let collection = loaded_from(FROM_A_LATER_VERSION);
            assert_eq!(
                collection.history().events_for(TrackId::new(0)),
                Some(&finished_once())
            );
        }

        #[test]
        fn an_event_this_version_cannot_read_is_written_back_unchanged() {
            let fixture = Fixture::after_a_session_over(FROM_A_LATER_VERSION);

            let kept = kept_events(&fixture);
            assert_eq!(kept.len(), 1, "the event should be written back once");
            assert_eq!(kept[0]["at"], 1_500);
            assert_eq!(kept[0]["event"]["data"]["stars"], 4);
        }

        #[test]
        fn an_event_this_version_cannot_read_is_written_after_the_ones_it_can() {
            let fixture = Fixture::after_a_session_over(FROM_A_LATER_VERSION);

            let events = written_events(&fixture);
            assert_eq!(events.len(), 2, "nothing should have been dropped");
            assert_eq!(events[0]["event"]["event"], "playback_outcome");
            assert_eq!(events[1]["event"]["event"], "rated");
        }

        /// Neither duplicated nor dropped: repeated sessions of an older jive
        /// must leave the file holding exactly what it found.
        #[test]
        fn an_event_this_version_cannot_read_survives_many_sessions() {
            let fixture = Fixture::new();
            let file = fixture.write_raw(FROM_A_LATER_VERSION);
            for _ in 0..3 {
                let collection = file.load().expect("the file loads").expect("a collection");
                file.save(&collection).expect("the collection is written");
            }

            assert_eq!(kept_events(&fixture).len(), 1);
        }

        /// The shape that once stopped jive starting: an event written before
        /// this format settled, as a bare name rather than a name and a
        /// payload. Such events exist in collection files already on disk.
        #[test]
        fn an_event_named_but_not_shaped_like_one_is_kept_rather_than_refused() {
            let fixture = Fixture::after_a_session_over(
                r#"{"version": 2, "root": "/music", "next_id": 1, "tracks": [
                    {"id": 0, "path": "a.mp3", "events": [
                        {"at": 0, "event": "added"},
                        {"at": 1000, "event": {"event": "playback_outcome",
                                               "data": {"outcome": "finished"}}}
                    ]}
                ]}"#,
            );

            let events = written_events(&fixture);
            assert_eq!(events.len(), 2, "nothing should have been dropped");
            assert_eq!(
                events[1],
                serde_json::json!({"at": 0, "event": "added"}),
                "the event nobody can read should be written back as it was"
            );
        }

        /// Text that is no event at all cannot be distinguished from an event
        /// added later, so it is carried the same way rather than rejected.
        #[test]
        fn text_that_is_no_event_at_all_is_kept_rather_than_refused() {
            let fixture = Fixture::after_a_session_over(
                r#"{"version": 2, "root": "/music", "next_id": 1,
                    "tracks": [{"id": 0, "path": "a.mp3", "events": ["added"]}]}"#,
            );

            assert_eq!(written_events(&fixture), [serde_json::json!("added")]);
        }

        /// A field added later is one this version has nowhere to put and no
        /// reason to reject the file over — at the root, on a track, or inside
        /// an event it otherwise understands.
        #[test]
        fn a_field_this_version_does_not_know_does_not_stop_the_file_loading() {
            let collection = loaded_from(
                r#"{"version": 2, "root": "/music", "next_id": 1, "listened_for": 900,
                    "tracks": [{"id": 0, "path": "a.mp3", "starred": true, "events": [
                        {"at": 0, "event": {"event": "playback_outcome",
                                            "data": {"outcome": "finished", "device": "phone"}}}
                    ]}]}"#,
            );

            assert_eq!(
                collection.history().events_for(TrackId::new(0)),
                Some(&finished_once()),
                "an event should still read once a field is added to it"
            );
        }

        #[test]
        fn an_event_this_version_cannot_read_is_carried_over_from_version_1() {
            let fixture = Fixture::after_a_session_over(
                r#"{"version": 1, "default_directory": "/music", "tracks": [
                    {"path": "/music/a.mp3", "name": "a", "events": [
                        {"at": 1500, "event": {"event": "rated", "data": {"stars": 4}}}
                    ]}
                ]}"#,
            );

            assert_eq!(kept_events(&fixture).len(), 1);
        }
    }

    #[test]
    fn a_version_1_file_keeps_the_events_of_each_track() {
        let collection = loaded_from(
            r#"{"version": 1, "default_directory": "/music", "tracks": [
                {"path": "/music/a.mp3", "name": "a",
                 "events": [{"at": 0, "event": {"event": "playback_outcome", "data": {"outcome": "finished"}}}]}
            ]}"#,
        );

        assert_eq!(
            collection.history().events_for(TrackId::new(0)),
            Some(&finished_once())
        );
    }
}
