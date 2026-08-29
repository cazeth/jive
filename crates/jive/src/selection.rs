//! Which tracks are eligible for the next draw, and which of them is drawn.
//!
//! [`evaluations`] scores every track in the library. [`Shuffle`] then draws
//! one with probability proportional to its score.
//!
//! # The recent window
//!
//! The most recently played tracks are excluded from a draw, a window that
//! grows with the square root of the library: three tracks out of nine, ten out
//! of a hundred. Only tracks played within [`GONE_STALE_AFTER`] are counted as
//! recent, so the window shrinks as a session goes quiet.
//!
//! A window that leaves nothing eligible is relaxed one track at a time until
//! something can play, and only then is the track that just played allowed to
//! follow itself. That happens only when it is the sole track in the library
//! that will play.
//!
//! Tracks reported unavailable are never relaxed back in. They failed during
//! the player's current attempt to find something that plays, so drawing one
//! again would only fail again.

use crate::library::Library;
use crate::offer::Factors;
use crate::offer::GONE_STALE_AFTER;
use jive_core::Time;
use jive_core::TrackId;
use rand::SeedableRng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::rngs::Xoshiro256PlusPlus;
use std::cmp::Reverse;
use std::collections::HashSet;

/// Why a track is absent from the next draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// It is among the most recently played tracks, a window whose size grows
    /// with the square root of the library.
    Recent,
    /// It already failed during the player's current attempt to find a track
    /// that plays.
    Unavailable,
}

/// One track's evaluation for the next draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// The track being evaluated.
    pub identifier: TrackId,
    /// The independent scoring factors.
    pub factors: Factors,
    /// The factors multiplied together, or zero if the track is excluded.
    pub priority: f64,
    /// Why the track is excluded, if it is.
    pub exclusion: Option<Exclusion>,
}

/// The random source the next track is drawn from.
///
/// Draws come from xoshiro256++, named rather than reached through
/// [`rand::rngs::SmallRng`], whose algorithm may change between releases and
/// with it the sequence a seed stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shuffle {
    random: Xoshiro256PlusPlus,
}

impl Shuffle {
    /// A shuffle whose sequence of draws is fixed by `seed`.
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self {
            random: Xoshiro256PlusPlus::seed_from_u64(seed),
        }
    }

    /// A shuffle seeded from the clock, so that two runs draw differently.
    #[must_use]
    pub fn from_clock() -> Self {
        Self::seeded(Time::now().as_unix_milliseconds())
    }

    /// The next track to play, or [`None`] if nothing in the library can.
    ///
    /// Drawn in proportion to preference, staleness, and reliability, from the
    /// tracks outside the recent window: the most recently played tracks, a
    /// window whose size grows with the square root of the library. The window
    /// is relaxed if it leaves nothing eligible, and `just_played` may follow
    /// itself only when it is the sole track that will play.
    ///
    /// `unavailable` names the tracks that have already failed during the
    /// player's current attempt to find one that plays. None of them is drawn,
    /// and none is relaxed back in.
    pub fn next_track_excluding(
        &mut self,
        library: &Library,
        just_played: Option<TrackId>,
        now: Time,
        unavailable: &[TrackId],
    ) -> Option<TrackId> {
        let candidates = evaluations(library, just_played, now, unavailable);
        self.draw(&candidates)
    }

    /// One candidate, drawn with probability proportional to its priority.
    ///
    /// An excluded candidate weighs zero, so all-zero weights mean nothing can
    /// play and the draw is [`None`]. A weight that is negative or not a number
    /// cannot arise from a product of three positive factors, and is refused
    /// the same way.
    fn draw(&mut self, candidates: &[Candidate]) -> Option<TrackId> {
        let weights = candidates.iter().map(|candidate| candidate.priority);
        let drawn = WeightedIndex::new(weights).ok()?.sample(&mut self.random);
        candidates.get(drawn).map(|candidate| candidate.identifier)
    }
}

/// Every track's eligibility and score for the next draw.
///
/// This is what [`Shuffle::next_track_excluding`] draws from, exposed so that
/// `jive-debug` can report the same numbers the shuffle acts on.
#[must_use]
pub fn evaluations(
    library: &Library,
    just_played: Option<TrackId>,
    now: Time,
    unavailable: &[TrackId],
) -> Vec<Candidate> {
    let unavailable: HashSet<TrackId> = unavailable.iter().copied().collect();
    let recent = recent_tracks(library, now);
    let target = integer_sqrt(library.len()).min(library.len().saturating_sub(1));

    for kept_recent in (0..=target.min(recent.len())).rev() {
        let excluded_recent: HashSet<TrackId> = recent.iter().take(kept_recent).copied().collect();
        let evaluated = score(
            library,
            now,
            &unavailable,
            &excluded_recent,
            just_played,
            true,
        );
        if evaluated.iter().any(|candidate| candidate.priority > 0.0) {
            return evaluated;
        }
    }

    // Every window left nothing eligible, so allow the track that just played
    // to follow itself. Only a library with one usable track reaches here.
    score(
        library,
        now,
        &unavailable,
        &HashSet::new(),
        just_played,
        false,
    )
}

/// One [`Candidate`] per track, excluded tracks scoring zero.
fn score(
    library: &Library,
    now: Time,
    unavailable: &HashSet<TrackId>,
    recent: &HashSet<TrackId>,
    just_played: Option<TrackId>,
    exclude_just_played: bool,
) -> Vec<Candidate> {
    library
        .identifiers()
        .iter()
        .map(|identifier| {
            let factors = library.factors(identifier, now);
            let exclusion = if unavailable.contains(&identifier) {
                Some(Exclusion::Unavailable)
            } else if recent.contains(&identifier)
                || (exclude_just_played && Some(identifier) == just_played)
            {
                Some(Exclusion::Recent)
            } else {
                None
            };
            Candidate {
                identifier,
                factors,
                priority: if exclusion.is_none() {
                    factors.priority()
                } else {
                    0.0
                },
                exclusion,
            }
        })
        .collect()
}

/// Tracks played within [`GONE_STALE_AFTER`], most recent first.
fn recent_tracks(library: &Library, now: Time) -> Vec<TrackId> {
    let mut recent: Vec<(TrackId, Time)> = library
        .identifiers()
        .iter()
        .filter_map(|identifier| {
            let played = library.last_played(identifier)?;
            (now.duration_since(played) < GONE_STALE_AFTER).then_some((identifier, played))
        })
        .collect();
    recent.sort_by_key(|entry| Reverse(entry.1));
    recent
        .into_iter()
        .map(|(identifier, _)| identifier)
        .collect()
}

/// The largest integer whose square is at most `value`.
fn integer_sqrt(value: usize) -> usize {
    let mut root: usize = 0;
    while (root + 1).saturating_mul(root + 1) <= value {
        root += 1;
    }
    root
}

#[cfg(test)]
mod tests {
    use super::Exclusion;
    use super::Shuffle;
    use super::evaluations;
    use crate::library::Library;
    use crate::offer::GONE_STALE_AFTER;
    use crate::testing::every_track_finishes_in_turn;
    use crate::testing::finished;
    use crate::testing::identifier_named;
    use crate::testing::library_of;
    use crate::testing::library_with_history;
    use crate::testing::quick_skip;
    use crate::testing::repeated;
    use jive_core::Duration;
    use jive_core::Time;
    use jive_core::TrackId;
    use std::collections::HashSet;

    /// Fixed so that the share measurements are reproducible.
    const SEED: u64 = 7;

    /// How far a share must sit from the indifferent baseline to count as
    /// moved. Measured shares are 0.46 and 0.15 against a baseline of 0.34.
    const MARGIN: f64 = 0.05;

    fn draw_sequence(library: &Library, seed: u64, draws: usize) -> Vec<TrackId> {
        let mut shuffle = Shuffle::seeded(seed);
        let mut current = None;
        let mut sequence = Vec::new();
        let mut now = Time::EPOCH + GONE_STALE_AFTER;
        for _ in 0..draws {
            current = shuffle.next_track_excluding(library, current, now, &[]);
            if let Some(identifier) = current {
                sequence.push(identifier);
            }
            now += Duration::from_seconds(1);
        }
        sequence
    }

    #[test]
    fn empty_and_single_track_libraries_are_safe() {
        assert_eq!(
            Shuffle::seeded(1).next_track_excluding(&library_of(&[]), None, Time::EPOCH, &[]),
            None
        );
        assert_eq!(draw_sequence(&library_of(&["alone"]), 1, 5).len(), 5);
    }

    #[test]
    fn a_track_never_immediately_repeats_when_an_alternative_exists() {
        let sequence = draw_sequence(&library_of(&["one", "two", "three"]), 2, 500);
        assert!(sequence.windows(2).all(|pair| pair[0] != pair[1]));
    }

    /// The share of a long run of draws that one named track takes up.
    fn share_of(library: &Library, name: &str, seed: u64) -> f64 {
        let wanted = identifier_named(library, name);
        let sequence = draw_sequence(library, seed, 4_000);
        let drawn = sequence.iter().filter(|drawn| **drawn == wanted).count();
        assert!(!sequence.is_empty(), "nothing was drawn");
        #[allow(clippy::cast_precision_loss)]
        {
            drawn as f64 / sequence.len() as f64
        }
    }

    /// Shares are compared against the same three tracks with nothing recorded
    /// against them, rather than against a bare fraction. An indifferent
    /// shuffle already draws each of three tracks about a third of the time, so
    /// `liked > a third` would pass whether preference did anything or not.
    #[test]
    fn preference_changes_probability_without_making_selection_deterministic() {
        let opinionated = library_with_history(&[
            ("liked", repeated(&finished(), 20)),
            ("disliked", repeated(&quick_skip(), 20)),
            ("neutral", vec![]),
        ]);
        let indifferent = library_of(&["liked", "disliked", "neutral"]);
        let baseline = share_of(&indifferent, "liked", SEED);

        let liked = share_of(&opinionated, "liked", SEED);
        let disliked = share_of(&opinionated, "disliked", SEED);

        assert!(
            liked > baseline + MARGIN,
            "a liked track should come up more than an unrated one: \
             {liked:.3} against a baseline of {baseline:.3}"
        );
        assert!(
            disliked < baseline - MARGIN,
            "a disliked track should come up less than an unrated one: \
             {disliked:.3} against a baseline of {baseline:.3}"
        );
        assert!(
            disliked > 0.0,
            "a disliked track should still come up sometimes"
        );
    }

    #[test]
    fn unavailable_tracks_are_never_relaxed() {
        let library = library_of(&["one", "two"]);
        let one = identifier_named(&library, "one");
        let two = identifier_named(&library, "two");
        assert_eq!(
            Shuffle::seeded(3).next_track_excluding(&library, Some(two), Time::EPOCH, &[one]),
            Some(two)
        );
        assert_eq!(
            Shuffle::seeded(3).next_track_excluding(&library, Some(two), Time::EPOCH, &[one, two]),
            None
        );
    }

    #[test]
    fn adaptive_window_excludes_the_square_root_most_recent_tracks() {
        let mut library = library_of(&[
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ]);
        let identifiers = every_track_finishes_in_turn(&mut library);

        let now = Time::EPOCH + Duration::from_seconds(10);
        let excluded: Vec<TrackId> = evaluations(&library, None, now, &[])
            .iter()
            .filter(|candidate| candidate.exclusion == Some(Exclusion::Recent))
            .map(|candidate| candidate.identifier)
            .collect();

        assert_eq!(excluded.len(), 3);
        assert!(
            identifiers[6..]
                .iter()
                .all(|identifier| excluded.contains(identifier)),
            "the three most recently played should be the excluded ones"
        );
    }

    #[test]
    fn a_repeat_can_happen_before_every_track_has_played() {
        let library = library_of(&["one", "two", "three", "four", "five"]);
        let possible = (0..100).any(|seed| {
            let sequence = draw_sequence(&library, seed, 5);
            let distinct: HashSet<TrackId> = sequence.into_iter().collect();
            distinct.len() < library.len()
        });
        assert!(possible);
    }

    #[test]
    fn seed_reproduces_order() {
        let library = library_of(&["one", "two", "three"]);
        assert_eq!(
            draw_sequence(&library, 42, 80),
            draw_sequence(&library, 42, 80)
        );
    }
}
