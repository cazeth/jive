//! A stable identifier for every track below a root directory.
//!
//! A [`Catalog`] maps the path of each track, relative to the root, to a
//! [`TrackId`]. Identifiers are assigned in the order tracks are first seen.
//! Because the key is a relative path, the root may be moved or renamed without
//! affecting any of them. Renaming a track or one of its parent directories
//! produces a path the catalog has not seen, and therefore a new identifier.
//!
//! Identifiers are never reused. Nothing here removes an entry, so a track
//! whose file is gone keeps its identifier and reappears with its history
//! intact. [`Catalog::next`] is stored rather than derived from the entries
//! present, so an entry removed by other means does not free its identifier for
//! a later track, which would otherwise inherit its history.

use jive_core::TrackId;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

/// A root directory, and the identifier assigned to every track ever seen below
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    root: PathBuf,
    next: u32,
    tracks: BTreeMap<PathBuf, TrackId>,
}

impl Catalog {
    /// An empty catalog for the music below `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            next: 0,
            tracks: BTreeMap::new(),
        }
    }

    /// A catalog rebuilt from the parts a stored one is written as.
    ///
    /// # Errors
    ///
    /// A description of why the parts are unusable: two tracks share an
    /// identifier, or an identifier is at or beyond the next one to assign.
    pub(crate) fn restore(
        root: PathBuf,
        next: u32,
        tracks: impl IntoIterator<Item = (PathBuf, TrackId)>,
    ) -> Result<Self, String> {
        let mut catalog = Self {
            root,
            next,
            tracks: BTreeMap::new(),
        };
        let mut taken: BTreeMap<TrackId, PathBuf> = BTreeMap::new();
        for (path, identifier) in tracks {
            if identifier.number() >= next {
                return Err(format!(
                    "`{}` is {identifier}, which has not been assigned yet",
                    path.display()
                ));
            }
            if let Some(other) = taken.insert(identifier, path.clone()) {
                return Err(format!(
                    "`{}` and `{}` are both {identifier}",
                    other.display(),
                    path.display()
                ));
            }
            catalog.tracks.insert(path, identifier);
        }
        Ok(catalog)
    }

    /// The root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Changes the root directory.
    ///
    /// Identifiers are unaffected: each is keyed by a path relative to the
    /// root, not by a path on this machine.
    pub fn set_root(&mut self, root: impl Into<PathBuf>) {
        self.root = root.into();
    }

    /// The next identifier to assign.
    #[must_use]
    pub(crate) fn next(&self) -> u32 {
        self.next
    }

    /// The identifier of the track at `path`, if it has been seen before.
    ///
    /// `path` is relative to the root.
    #[must_use]
    pub fn identifier_for(&self, path: &Path) -> Option<TrackId> {
        self.tracks.get(path).copied()
    }

    /// The path of a track relative to the root, if it is in the catalog.
    #[must_use]
    pub fn path_of(&self, identifier: TrackId) -> Option<&Path> {
        self.tracks
            .iter()
            .find(|(_, known)| **known == identifier)
            .map(|(path, _)| path.as_path())
    }

    /// Every track, as an identifier and a path relative to the root, in path
    /// order.
    pub fn tracks(&self) -> impl Iterator<Item = (TrackId, &Path)> + '_ {
        self.tracks
            .iter()
            .map(|(path, identifier)| (*identifier, path.as_path()))
    }

    /// How many tracks have ever been seen.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether no track has been seen.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The identifier of the track at `path`, assigning a new one if it has not
    /// been seen before.
    ///
    /// # Panics
    ///
    /// If more than [`u32::MAX`] tracks have ever been seen.
    pub(crate) fn identify(&mut self, path: &Path) -> TrackId {
        if let Some(identifier) = self.tracks.get(path) {
            return *identifier;
        }
        let identifier = TrackId::new(self.next);
        self.next = self
            .next
            .checked_add(1)
            .expect("a catalog holds fewer tracks than u32::MAX");
        self.tracks.insert(path.to_path_buf(), identifier);
        identifier
    }
}

#[cfg(test)]
mod tests {
    use super::Catalog;
    use jive_core::TrackId;
    use std::path::Path;
    use std::path::PathBuf;

    fn catalog() -> Catalog {
        Catalog::new("/music")
    }

    /// A catalog that has seen each of `tracks`, in the order given.
    fn having_seen(tracks: &[&str]) -> Catalog {
        let mut catalog = catalog();
        for path in tracks {
            catalog.identify(Path::new(path));
        }
        catalog
    }

    fn identifier_of(catalog: &Catalog, path: &str) -> Option<u32> {
        catalog.identifier_for(Path::new(path)).map(TrackId::number)
    }

    /// The paths a catalog returns, in the order it returns them.
    fn paths_of(catalog: &Catalog) -> Vec<&Path> {
        catalog.tracks().map(|(_, path)| path).collect()
    }

    /// The identifiers a catalog returns, in the order it returns them.
    fn numbers_of(catalog: &Catalog) -> Vec<u32> {
        catalog
            .tracks()
            .map(|(identifier, _)| identifier.number())
            .collect()
    }

    /// A catalog rebuilt from the parts a stored one is written as, with the
    /// next identifier overridable so that a file claiming a stale one can be
    /// tried too.
    fn restored(catalog: &Catalog, next: u32) -> Result<Catalog, String> {
        let tracks: Vec<(PathBuf, TrackId)> = catalog
            .tracks()
            .map(|(identifier, path)| (path.to_path_buf(), identifier))
            .collect();
        Catalog::restore(PathBuf::from("/music"), next, tracks)
    }

    #[test]
    fn a_fresh_catalog_holds_no_tracks() {
        let catalog = catalog();
        assert!(catalog.is_empty());
        assert_eq!(identifier_of(&catalog, "a.mp3"), None);
        assert_eq!(catalog.path_of(TrackId::new(0)), None);
    }

    #[test]
    fn a_track_is_named_the_moment_it_is_seen() {
        let catalog = having_seen(&["nested/a.mp3"]);
        assert!(!catalog.is_empty());
        assert_eq!(identifier_of(&catalog, "nested/a.mp3"), Some(0));
        assert_eq!(
            catalog.path_of(TrackId::new(0)),
            Some(Path::new("nested/a.mp3"))
        );
    }

    #[test]
    fn tracks_come_back_in_path_order() {
        let catalog = having_seen(&["z.mp3", "a.mp3", "m/b.mp3"]);
        assert_eq!(
            paths_of(&catalog),
            [Path::new("a.mp3"), Path::new("m/b.mp3"), Path::new("z.mp3")]
        );
    }

    /// A stored `next` at or below an identifier already in use would assign
    /// that identifier to a second track, silently merging their histories.
    #[test]
    fn an_identifier_that_would_be_assigned_again_is_refused() {
        assert!(restored(&having_seen(&["a.mp3"]), 0).is_err());
    }

    #[test]
    fn a_catalog_survives_being_stored() {
        let catalog = having_seen(&["a.mp3", "nested/b.mp3"]);
        let next = catalog.next();
        assert_eq!(restored(&catalog, next), Ok(catalog.clone()));
    }

    #[test]
    fn identifiers_are_assigned_in_the_order_tracks_are_seen() {
        let catalog = having_seen(&["b.mp3", "a.mp3", "c.mp3"]);
        assert_eq!(identifier_of(&catalog, "b.mp3"), Some(0));
        assert_eq!(identifier_of(&catalog, "a.mp3"), Some(1));
        assert_eq!(identifier_of(&catalog, "c.mp3"), Some(2));
    }

    #[test]
    fn a_track_seen_again_keeps_the_identifier_it_had() {
        let mut catalog = having_seen(&["a.mp3", "b.mp3"]);
        assert_eq!(catalog.identify(Path::new("a.mp3")), TrackId::new(0));
        assert_eq!(catalog.len(), 2, "nothing new should have been identified");
    }

    /// Reusing the identifier of a deleted track would give its history to a
    /// new one.
    #[test]
    fn an_identifier_is_not_assigned_again_once_its_track_is_gone() {
        let mut catalog = having_seen(&["gone.mp3", "kept.mp3"]);
        catalog.tracks.remove(Path::new("gone.mp3"));

        assert_eq!(catalog.identify(Path::new("fresh.mp3")), TrackId::new(2));
    }

    #[test]
    fn moving_the_root_leaves_every_identifier_alone() {
        let mut catalog = having_seen(&["rock/a.mp3", "jazz/b.mp3"]);
        let before = numbers_of(&catalog);

        catalog.set_root("/mnt/elsewhere");

        assert_eq!(catalog.root(), Path::new("/mnt/elsewhere"));
        assert_eq!(numbers_of(&catalog), before);
    }

    #[test]
    fn two_places_sharing_an_identifier_are_refused() {
        let tracks = [
            (PathBuf::from("a.mp3"), TrackId::new(0)),
            (PathBuf::from("b.mp3"), TrackId::new(0)),
        ];
        let refused = Catalog::restore(PathBuf::from("/music"), 1, tracks)
            .expect_err("two tracks cannot be one track");
        assert!(refused.contains("both track 0"), "{refused}");
    }
}
