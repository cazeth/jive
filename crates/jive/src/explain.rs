//! Reporting the numbers the shuffle draws from.
//!
//! [`rows`] takes the same library the player draws from and reports what each
//! factor contributes to a track's priority. [`table`] lays that out as text,
//! and [`legend`] describes the columns. This is what `jive-debug` prints.

use crate::library::Library;
use crate::offer;
use crate::rating;
use crate::selection;
use crate::selection::Exclusion;
use jive_core::Duration;
use jive_core::Time;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// What the shuffle computes for one track.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The name displayed.
    pub name: String,
    /// The measured listener preference.
    pub preference: f64,
    /// What the time since the track last played contributes.
    pub staleness: f64,
    /// The share of recorded plays that succeeded.
    pub reliability: f64,
    /// The factors multiplied together: the weight the track is drawn with.
    pub priority: f64,
    /// Why the track is excluded from the next draw, if it is.
    pub exclusion: Option<Exclusion>,
    /// The track's share of the next draw, from zero to one.
    pub share: f64,
    /// Time since the track last played, or [`None`] if it never has.
    pub idle_for: Option<Duration>,
    /// How many events are recorded against the track.
    pub events: usize,
}

/// One row per track, heaviest first.
#[must_use]
pub fn rows(library: &Library, now: Time) -> Vec<Row> {
    let mut rows: Vec<Row> = selection::evaluations(library, None, now, &[])
        .into_iter()
        .filter_map(|candidate| {
            let track = library.track(candidate.identifier)?;
            Some(Row {
                name: track.name.to_string(),
                preference: candidate.factors.preference,
                staleness: candidate.factors.staleness,
                reliability: candidate.factors.reliability,
                priority: candidate.priority,
                share: 0.0,
                idle_for: library
                    .last_played(candidate.identifier)
                    .map(|last| now.duration_since(last)),
                events: track.events.len(),
                exclusion: candidate.exclusion,
            })
        })
        .collect();
    let total: f64 = rows.iter().map(|row| row.priority).sum();
    for row in &mut rows {
        row.share = if total > 0.0 {
            row.priority / total
        } else {
            0.0
        };
    }
    rows.sort_by(|left, right| right.priority.total_cmp(&left.priority));
    rows
}

/// The rows as a table, one column per input.
#[must_use]
pub fn table(rows: &[Row]) -> String {
    if rows.is_empty() {
        return String::from("no tracks");
    }
    let columns = columns(rows);
    let mut lines = vec![heading(&columns), rule(&columns)];
    lines.extend((0..rows.len()).map(|row| line(&columns, row)));
    lines.join("\n")
}

/// What each column of [`table`] means, and the range each factor spans.
#[must_use]
pub fn legend() -> String {
    [
        format!(
            "taste     what the listener makes of a track, {:.2} to {:.2}",
            rating::MINIMUM_PREFERENCE,
            rating::MAXIMUM_PREFERENCE
        ),
        format!(
            "stale     {:.2} just after a track is played, rising to {:.2} once left for {}",
            offer::FRESH_MULTIPLIER,
            offer::STALEST_MULTIPLIER,
            span(offer::GONE_STALE_AFTER)
        ),
        String::from("reliable  share of recorded plays that succeeded"),
        String::from("eligible  yes, or why a track is outside this draw"),
        String::from("weight    taste × stale × reliable, or zero when excluded"),
        String::from(
            "share     that weight against every other track's; the bar draws it to scale",
        ),
    ]
    .join("\n")
}

/// The widest a track name may be, in terminal columns, before it is shortened.
const NAME_LIMIT: usize = 28;

/// The width of a bar standing for the whole of the next draw.
const BAR_WIDTH: u32 = 8;

/// The separator between two columns, and its counterpart in the rule.
const DIVIDER: &str = " | ";
const RULE_DIVIDER: &str = "-+-";

/// The edge a column's cells are aligned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
}

/// One column: a heading, and a cell for every row.
#[derive(Debug)]
struct Column {
    heading: &'static str,
    edge: Edge,
    cells: Vec<String>,
}

impl Column {
    /// The width of the widest cell, in terminal columns, heading included.
    fn width(&self) -> usize {
        self.cells
            .iter()
            .map(|cell| columns_of(cell))
            .chain(std::iter::once(columns_of(self.heading)))
            .max()
            .unwrap_or_default()
    }

    /// A cell padded to the column's width, and aligned to its edge.
    ///
    /// Padding is measured in terminal columns rather than characters: a name
    /// in a script drawn double width would otherwise push the columns after it
    /// out of line.
    fn pad(&self, text: &str) -> String {
        let padding = " ".repeat(self.width().saturating_sub(columns_of(text)));
        match self.edge {
            Edge::Left => format!("{text}{padding}"),
            Edge::Right => format!("{padding}{text}"),
        }
    }
}

/// The width of `text` in terminal columns.
fn columns_of(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn columns(rows: &[Row]) -> Vec<Column> {
    vec![
        Column {
            heading: "track",
            edge: Edge::Left,
            cells: rows.iter().map(|row| shortened(&row.name)).collect(),
        },
        Column {
            heading: "taste",
            edge: Edge::Right,
            cells: rows
                .iter()
                .map(|row| format!("{:.2}", row.preference))
                .collect(),
        },
        Column {
            heading: "stale",
            edge: Edge::Right,
            cells: rows
                .iter()
                .map(|row| format!("{:.2}", row.staleness))
                .collect(),
        },
        Column {
            heading: "reliable",
            edge: Edge::Right,
            cells: rows
                .iter()
                .map(|row| format!("{:.2}", row.reliability))
                .collect(),
        },
        Column {
            heading: "weight",
            edge: Edge::Right,
            cells: rows
                .iter()
                .map(|row| format!("{:.2}", row.priority))
                .collect(),
        },
        Column {
            heading: "eligible",
            edge: Edge::Left,
            cells: rows
                .iter()
                .map(|row| match row.exclusion {
                    None => String::from("yes"),
                    Some(Exclusion::Recent) => String::from("recent"),
                    Some(Exclusion::Unavailable) => String::from("unavailable"),
                })
                .collect(),
        },
        Column {
            heading: "last on",
            edge: Edge::Right,
            cells: rows.iter().map(|row| idle(row.idle_for)).collect(),
        },
        Column {
            heading: "share",
            edge: Edge::Left,
            cells: rows.iter().map(share).collect(),
        },
    ]
}

/// A row's share as a percentage, followed by a bar of the same size.
fn share(row: &Row) -> String {
    format!("{:>6}  {}", percentage(row.share), bar(row.share))
}

fn heading(columns: &[Column]) -> String {
    join(columns, DIVIDER, |column| column.pad(column.heading))
}

fn rule(columns: &[Column]) -> String {
    join(columns, RULE_DIVIDER, |column| "-".repeat(column.width()))
}

fn line(columns: &[Column], row: usize) -> String {
    join(columns, DIVIDER, |column| column.pad(&column.cells[row]))
}

/// Lays one cell per column side by side, trimming the padding off the last.
fn join(columns: &[Column], divider: &str, cell: impl Fn(&Column) -> String) -> String {
    let laid_out: Vec<String> = columns.iter().map(cell).collect();
    laid_out.join(divider).trim_end().to_owned()
}

/// A name, truncated to [`NAME_LIMIT`] columns so that one long name does not
/// widen every column after it.
fn shortened(name: &str) -> String {
    if columns_of(name) <= NAME_LIMIT {
        return name.to_owned();
    }
    let mut kept = String::new();
    let mut used = 0;
    for letter in name.chars() {
        used += UnicodeWidthChar::width(letter).unwrap_or(0);
        if used > NAME_LIMIT - 1 {
            break;
        }
        kept.push(letter);
    }
    format!("{kept}…")
}

fn percentage(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

/// How long ago, or `never` for a track that has not played.
fn idle(idle_for: Option<Duration>) -> String {
    idle_for.map_or_else(|| String::from("never"), span)
}

/// A duration, in the largest unit that remains meaningful.
fn span(length: Duration) -> String {
    let seconds = length.as_whole_seconds();
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3_599 => format!("{}m", seconds / 60),
        3_600..=86_399 => format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// A bar of up to [`BAR_WIDTH`] columns, proportional to `share`.
fn bar(share: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let filled = (share * f64::from(BAR_WIDTH)).round() as usize;
    "\u{2588}".repeat(filled.min(BAR_WIDTH as usize))
}

#[cfg(test)]
mod tests {
    use super::Row;
    use super::idle;
    use super::legend;
    use super::rows;
    use super::table;
    use crate::library::Library;
    use crate::testing::assert_close;
    use crate::testing::failed;
    use crate::testing::finished;
    use crate::testing::library_of;
    use crate::testing::library_where_every_track;
    use crate::testing::library_with_history;
    use crate::testing::quick_skip;
    use crate::testing::repeated;
    use jive_core::Duration;
    use jive_core::Time;
    use unicode_width::UnicodeWidthChar;
    use unicode_width::UnicodeWidthStr;

    fn now() -> Time {
        Time::EPOCH + crate::offer::GONE_STALE_AFTER
    }

    fn rows_of(library: &Library) -> Vec<Row> {
        rows(library, now())
    }

    fn named(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|row| row.name.clone()).collect()
    }

    /// How a track last played `seconds` ago is rendered.
    fn idle_after(seconds: u64) -> String {
        idle(Some(Duration::from_seconds(seconds)))
    }

    /// The one row a library of a single track produces.
    fn only_row(library: &Library) -> Row {
        let mut rows = rows_of(library);
        assert_eq!(rows.len(), 1, "one track should make one row");
        rows.pop().expect("a row")
    }

    /// One test per `time since a track played => how it is rendered` row.
    macro_rules! reads_as {
        ($($name:ident: $idle:expr => $shown:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!($idle, $shown);
                }
            )+
        };
    }

    reads_as! {
        a_track_never_on_says_so: idle(None) => "never";
        a_track_on_just_now_reads_in_seconds: idle_after(0) => "0s";
        a_track_on_a_minute_ago_reads_in_minutes: idle_after(60) => "1m";
        a_track_on_an_hour_ago_reads_in_hours: idle_after(3_600) => "1h 0m";
        a_track_on_yesterday_reads_in_days: idle_after(90_000) => "1d";
    }

    #[test]
    fn every_track_gets_a_row_and_an_empty_library_gets_none() {
        assert_eq!(rows_of(&library_of(&["one", "two", "three"])).len(), 3);
        assert!(rows_of(&library_of(&[])).is_empty());
        assert_eq!(table(&[]), "no tracks");
    }

    #[test]
    fn a_row_reports_what_is_recorded_against_its_track() {
        assert_eq!(only_row(&library_of(&["fresh"])).idle_for, None);

        let played = library_where_every_track(&["one"], &repeated(&finished(), 3));
        assert_eq!(only_row(&played).events, 3);
    }

    #[test]
    fn the_heaviest_track_comes_first() {
        let library = library_with_history(&[
            ("disliked", repeated(&quick_skip(), 4)),
            ("liked", repeated(&finished(), 4)),
        ]);
        assert_eq!(named(&rows_of(&library)), ["liked", "disliked"]);
    }

    #[test]
    fn the_shares_add_up_to_everything() {
        let library = library_of(&["one", "two", "three", "four"]);
        let total: f64 = rows_of(&library).iter().map(|row| row.share).sum();
        assert_close(total, 1.0);
    }

    /// Each factor must differ from one, or dropping it from the product would
    /// go unnoticed.
    #[test]
    fn a_row_reports_the_two_inputs_and_what_they_multiply_to() {
        let liked_but_unreliable = [repeated(&finished(), 5), repeated(&failed(), 3)].concat();
        let row = only_row(&library_where_every_track(&["one"], &liked_but_unreliable));

        for factor in [row.preference, row.staleness, row.reliability] {
            assert!(
                (factor - 1.0).abs() > 1e-9,
                "a factor of {factor} is invisible"
            );
        }
        assert_close(
            row.preference * row.staleness * row.reliability,
            row.priority,
        );
    }

    /// The width of each cell of a line, in terminal columns.
    ///
    /// Character counts will not do: a double-width character fills two
    /// columns, so measuring by count would report a misaligned table as
    /// aligned.
    fn cell_widths(line: &str) -> Vec<usize> {
        line.split(['|', '+']).map(UnicodeWidthStr::width).collect()
    }

    fn widths_of(rendered: &str) -> Vec<Vec<usize>> {
        rendered.lines().map(cell_widths).collect()
    }

    /// Where the dividers stand, counted in terminal columns.
    fn divider_places(line: &str) -> Vec<usize> {
        let mut places = Vec::new();
        let mut column = 0;
        for letter in line.chars() {
            if letter == '|' || letter == '+' {
                places.push(column);
            }
            column += UnicodeWidthChar::width(letter).unwrap_or(0);
        }
        places
    }

    /// The range of names a real library holds: plain, accented, and scripts
    /// whose characters are drawn twice as wide.
    const AWKWARD_NAMES: [&str; 5] = [
        "plain ascii name",
        "Björk - Jóga",
        "東京の夜",
        "日本語のタイトル",
        "a",
    ];

    #[test]
    fn every_column_is_the_same_width_on_every_line() {
        let rendered = table(&rows_of(&library_of(&AWKWARD_NAMES)));
        let widths = widths_of(&rendered);
        let heading = widths.first().expect("a heading").clone();

        for (line, width) in rendered.lines().zip(&widths) {
            assert_eq!(
                width.len(),
                heading.len(),
                "`{line}` has a different number of cells:\n{rendered}"
            );
            assert_eq!(
                width[..width.len() - 1],
                heading[..heading.len() - 1],
                "`{line}` does not line up:\n{rendered}"
            );
        }
    }

    #[test]
    fn every_divider_stands_in_the_same_place() {
        let rendered = table(&rows_of(&library_of(&AWKWARD_NAMES)));
        let places = divider_places(rendered.lines().next().expect("a heading"));
        assert_eq!(places.len(), 7, "eight columns need seven dividers");
        for line in rendered.lines() {
            assert_eq!(
                divider_places(line),
                places,
                "the dividers moved:\n{rendered}"
            );
        }
    }

    #[test]
    fn a_name_of_wide_characters_does_not_push_the_columns_out() {
        let wide = table(&rows_of(&library_of(&["東京の夜の長い名前"])));
        let plain = table(&rows_of(&library_of(&["a name of that same width"])));
        assert_eq!(
            widths_of(&wide)[0][1..],
            widths_of(&plain)[0][1..],
            "a wide name moved the other columns:\n{wide}\n{plain}"
        );
    }

    #[test]
    fn a_column_is_as_wide_as_the_widest_thing_in_it() {
        let widest = "the widest name here";
        let rendered = table(&rows_of(&library_of(&["a", widest])));
        assert_eq!(
            widths_of(&rendered)[0][0],
            UnicodeWidthStr::width(widest) + 1,
            "the name column should fit the widest name:\n{rendered}"
        );
    }

    #[test]
    fn a_name_of_wide_characters_is_shortened_by_the_room_it_takes() {
        let rendered = table(&rows_of(&library_of(&["東".repeat(40).as_str()])));
        assert!(
            rendered.contains('…'),
            "the name should be shortened:\n{rendered}"
        );
        assert!(
            widths_of(&rendered)[0][0] <= super::NAME_LIMIT + 1,
            "the name column ran past its limit:\n{rendered}"
        );
    }

    #[test]
    fn a_name_that_fits_is_shown_whole() {
        let name = "a much longer track name";
        let rendered = table(&rows_of(&library_of(&["a", name])));
        assert!(rendered.contains(name), "the name should be shown whole");
    }

    #[test]
    fn the_table_names_every_column() {
        let rendered = table(&rows_of(&library_of(&["one"])));
        for heading in [
            "track", "taste", "stale", "reliable", "weight", "eligible", "last on", "share",
        ] {
            assert!(rendered.contains(heading), "{heading} should be a column");
        }
    }

    #[test]
    fn a_name_too_long_for_its_column_is_shortened() {
        let long = "a track with a preposterously long name that would push the columns across";
        let rendered = table(&rows_of(&library_of(&[long])));
        assert!(
            rendered.contains('…'),
            "the name should be shortened:\n{rendered}"
        );
        assert!(
            !rendered.contains(long),
            "the whole name should not be shown"
        );
        let widest = rendered
            .lines()
            .map(|line| line.chars().count())
            .max()
            .expect("a table");
        assert!(
            widest < 110,
            "the table ran to {widest} columns:\n{rendered}"
        );
    }

    #[test]
    fn the_legend_explains_every_column() {
        let shown = legend();
        for column in ["taste", "stale", "reliable", "eligible", "weight", "share"] {
            assert!(shown.contains(column), "{column} should be explained");
        }
        assert!(
            shown.contains("2h 0m"),
            "the legend should say when a track is fully stale"
        );
        assert!(
            shown.contains("the bar draws it to scale"),
            "the bar should be explained"
        );
    }
}
