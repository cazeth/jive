//! Track identity.
//!
//! A [`TrackId`] is assigned once, by whatever maintains the catalog, and
//! identifies that track from then on. [`TrackIds`] holds the identifiers
//! currently in play, and side tables such as [`TrackNames`] are keyed by
//! identifier.

use core::fmt;

/// The identifier of one track.
///
/// Meaningful only alongside the catalog that assigned it: the number carries
/// no information by itself, and two identifiers are comparable only if the
/// same catalog assigned both.
///
/// A [`History`](crate::History) is keyed by identifier, so a track given a new
/// one appears to have no history. Reassigning a track the identifier it held
/// previously is the responsibility of whatever maintains the catalog, as is
/// deciding what counts as the same track between runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TrackId(u32);

impl TrackId {
    /// The identifier numbered `number`.
    #[must_use]
    pub const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The number of this identifier.
    #[must_use]
    pub const fn number(self) -> u32 {
        self.0
    }

    fn slot(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "track {}", self.0)
    }
}

/// The identifiers currently in play, in the order they were found.
///
/// Only some of a catalog's identifiers are in play at any moment, so the list
/// is not contiguous and does not start at zero.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TrackIds {
    ids: Vec<TrackId>,
}

impl TrackIds {
    /// An empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self { ids: Vec::new() }
    }

    /// Appends an identifier.
    pub fn push(&mut self, identifier: TrackId) {
        self.ids.push(identifier);
    }

    /// How many identifiers are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether no identifier is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Whether `identifier` is present.
    #[must_use]
    pub fn contains(&self, identifier: TrackId) -> bool {
        self.ids.contains(&identifier)
    }

    /// The identifiers, in order.
    pub fn iter(&self) -> impl Iterator<Item = TrackId> + Clone + '_ {
        self.ids.iter().copied()
    }
}

impl FromIterator<TrackId> for TrackIds {
    fn from_iter<I: IntoIterator<Item = TrackId>>(identifiers: I) -> Self {
        Self {
            ids: identifiers.into_iter().collect(),
        }
    }
}

/// The name a track is displayed under.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TrackName(String);

impl TrackName {
    /// A name from any string.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name as an owned string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for TrackName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for TrackName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TrackName {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for TrackName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

/// Track names, looked up by identifier.
///
/// Identifiers are only as dense as the catalog that assigned them, so slots
/// for tracks not in play are empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackNames {
    names: Vec<Option<TrackName>>,
}

impl TrackNames {
    /// An empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self { names: Vec::new() }
    }

    /// Sets the name of a track, replacing whatever its slot held.
    pub fn insert(&mut self, identifier: TrackId, name: impl Into<TrackName>) {
        let slot = identifier.slot();
        if slot >= self.names.len() {
            self.names.resize(slot + 1, None);
        }
        self.names[slot] = Some(name.into());
    }

    /// The name of a track, or [`None`] if this table holds none for it.
    #[must_use]
    pub fn get(&self, identifier: TrackId) -> Option<&TrackName> {
        self.names.get(identifier.slot())?.as_ref()
    }

    /// How many names are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Whether no name is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// The names, in identifier order.
    pub fn iter(&self) -> core::iter::Flatten<core::slice::Iter<'_, Option<TrackName>>> {
        self.names.iter().flatten()
    }
}

impl<Name: Into<TrackName>> FromIterator<Name> for TrackNames {
    fn from_iter<I: IntoIterator<Item = Name>>(names: I) -> Self {
        Self {
            names: names.into_iter().map(|name| Some(name.into())).collect(),
        }
    }
}

impl<'names> IntoIterator for &'names TrackNames {
    type Item = &'names TrackName;
    type IntoIter = core::iter::Flatten<core::slice::Iter<'names, Option<TrackName>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::TrackId;
    use super::TrackIds;
    use super::TrackName;
    use super::TrackNames;

    /// A list of the first `count` identifiers.
    fn identifiers(count: u32) -> TrackIds {
        (0..count).map(TrackId::new).collect()
    }

    /// Identifiers and the names beside them.
    fn named(names: &[&str]) -> (TrackIds, TrackNames) {
        let identifiers = identifiers(u32::try_from(names.len()).expect("a small set"));
        let mut stored = TrackNames::new();
        for (identifier, name) in identifiers.iter().zip(names) {
            stored.insert(identifier, *name);
        }
        (identifiers, stored)
    }

    fn names_of(names: &TrackNames) -> Vec<String> {
        names.iter().map(TrackName::to_string).collect()
    }

    fn first_of(identifiers: &TrackIds) -> TrackId {
        identifiers.iter().next().expect("a first identifier")
    }

    fn last_of(identifiers: &TrackIds) -> TrackId {
        identifiers.iter().last().expect("a last identifier")
    }

    fn resolved(names: &[&str]) -> Vec<String> {
        let (identifiers, stored) = named(names);
        identifiers
            .iter()
            .filter_map(|identifier| stored.get(identifier))
            .map(TrackName::to_string)
            .collect()
    }

    /// A table holding one name, against an identifier far above zero.
    fn sparse() -> (TrackId, TrackNames) {
        let identifier = TrackId::new(5_000);
        let mut names = TrackNames::new();
        names.insert(identifier, "distant");
        (identifier, names)
    }

    #[test]
    fn an_empty_set_holds_nothing() {
        let identifiers = identifiers(0);
        assert_eq!(identifiers.len(), 0);
        assert!(identifiers.is_empty());
        assert_eq!(identifiers.iter().count(), 0);
    }

    #[test]
    fn a_set_holds_its_identifiers_in_the_order_they_were_added() {
        let identifiers = identifiers(3);
        assert_eq!(identifiers.len(), 3);
        assert!(!identifiers.is_empty());
        assert_eq!(
            identifiers.iter().collect::<Vec<TrackId>>(),
            [TrackId::new(0), TrackId::new(1), TrackId::new(2)]
        );
    }

    #[test]
    fn a_set_reports_which_identifiers_are_its_own() {
        let two = identifiers(2);
        assert!(two.contains(first_of(&two)));
        assert!(two.contains(last_of(&two)));
        assert_ne!(first_of(&two), last_of(&two), "identifiers are told apart");
        assert!(!identifiers(1).contains(last_of(&two)));
    }

    #[test]
    fn an_identifier_is_the_number_it_was_written_as() {
        assert_eq!(TrackId::new(7).number(), 7);
        assert_eq!(TrackId::new(7).to_string(), "track 7");
    }

    /// A name wraps the string it was built from, so every conversion in and
    /// out must return exactly that string, whatever it contains.
    #[test]
    fn a_name_is_the_text_it_was_built_from() {
        for text in ["a name", "", "東京の夜 · pt. 2"] {
            let name = TrackName::new(text);
            assert_eq!(name.as_str(), text);
            assert_eq!(name.to_string(), text);
            assert_eq!(AsRef::<str>::as_ref(&name), text);
            assert_eq!(name.clone().into_string(), String::from(text));
            assert_eq!(TrackName::from(String::from(text)), name);
        }
    }

    #[test]
    fn an_empty_table_holds_nothing() {
        let names = TrackNames::new();
        assert_eq!(names.len(), 0);
        assert!(names.is_empty());
    }

    #[test]
    fn a_table_resolves_the_names_it_holds_in_identifier_order() {
        let (_, stored) = named(&["one", "two"]);
        assert_eq!(stored.len(), 2);
        assert!(!stored.is_empty());
        assert_eq!(resolved(&["one", "two", "three"]), ["one", "two", "three"]);
        assert_eq!(
            stored.get(TrackId::new(4)),
            None,
            "an identifier with no name resolves to nothing"
        );
        assert_eq!((&stored).into_iter().count(), 2);
    }

    #[test]
    fn a_table_collects_from_an_iterator() {
        let collected: TrackNames = ["one", "two"].into_iter().collect();
        assert_eq!(names_of(&collected), ["one", "two"]);
    }

    /// A catalog assigns identifiers consecutively, but nothing guarantees it,
    /// so a table must hold one far above zero.
    #[test]
    fn a_distant_identifier_keeps_its_name() {
        let (identifier, names) = sparse();
        assert_eq!(
            names.get(identifier).map(TrackName::as_str),
            Some("distant")
        );
        assert_eq!(names.len(), 1);
        assert!(!names.is_empty());
    }

    #[test]
    fn every_identifier_of_a_set_resolves_to_its_own_name() {
        let names = ["first", "second", "third", "fourth"];
        let (identifiers, stored) = named(&names);
        for (identifier, expected) in identifiers.iter().zip(names) {
            assert_eq!(
                stored.get(identifier).map(TrackName::as_str),
                Some(expected)
            );
        }
    }

    #[test]
    fn a_table_keeps_duplicate_names_apart_by_identifier() {
        let (identifiers, stored) = named(&["same", "same"]);
        assert_eq!(stored.len(), 2);
        assert_eq!(names_of(&stored), ["same", "same"]);
        assert_ne!(first_of(&identifiers), last_of(&identifiers));
    }

    /// Gaps in the identifier range must not be reported as names.
    #[test]
    fn the_slots_between_two_distant_names_stay_empty() {
        let mut names = TrackNames::new();
        names.insert(TrackId::new(0), "first");
        names.insert(TrackId::new(9), "last");
        assert_eq!(names_of(&names), ["first", "last"]);
        assert!((1..9).all(|number| names.get(TrackId::new(number)).is_none()));
    }

    #[test]
    fn a_name_recorded_twice_keeps_the_later_one() {
        let identifier = TrackId::new(3);
        let mut names = TrackNames::new();
        names.insert(identifier, "as it was");
        names.insert(identifier, "as it is");
        assert_eq!(
            names.get(identifier).map(TrackName::as_str),
            Some("as it is")
        );
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn a_large_set_holds_distinct_identifiers_throughout() {
        let identifiers = identifiers(5_000);
        let seen: std::collections::HashSet<TrackId> = identifiers.iter().collect();
        assert_eq!(seen.len(), 5_000);
        assert!(identifiers.iter().all(|one| identifiers.contains(one)));
    }
}
