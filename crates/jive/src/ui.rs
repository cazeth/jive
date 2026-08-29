//! Rendering the player's state.
//!
//! A single bordered box, measured from what it holds: the track name and how
//! long it has played. Nothing sits outside the box.
//!
//! Only accents are colored and no background is set, so the panel reads
//! correctly on light and dark terminals alike.
//!
//! A track name too wide for the panel scrolls, so that all of it can be read
//! in turn. Anything else too wide is truncated with an ellipsis. Both are
//! measured in terminal columns, so a name in a double-width script neither
//! overflows nor is cut through the middle of a character.

use crate::player::ViewModel;
use jive_core::Duration;
use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Padding;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// The panel width, unless the terminal is narrower.
pub const PANEL_WIDTH: u16 = 48;

/// Columns of padding inside the border, on each side.
const BOX_SIDE_PADDING: u16 = 2;

/// The border and padded row above and below the content.
const BOX_MARGIN_ROWS: u16 = 4;

/// Rows the box holds: a heading, a blank row, and the line below it.
const CONTENT_ROWS: u16 = 3;

/// The height of the box, unless the terminal is shorter.
pub const PANEL_HEIGHT: u16 = BOX_MARGIN_ROWS + CONTENT_ROWS;

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const ELLIPSIS: char = '…';

/// The screen width of the ellipsis.
const ELLIPSIS_WIDTH: usize = 1;

/// The gap shown between the end of a scrolling title and its restart.
const SCROLL_GAP: &str = "   ";

/// How often the panel is redrawn.
///
/// The player waits this long for a key between draws, so it is also the rate
/// at which anything moving on the panel can move. [`crate::app`] takes its
/// poll timeout from here.
pub const REDRAW_INTERVAL: Duration = Duration::from_milliseconds(150);

/// Redraws that each step of a scrolling title lasts.
const REDRAWS_PER_STEP: u64 = 2;

/// How long each step of a scrolling title lasts.
///
/// A whole number of redraws, so every step lands on one and the title moves
/// at an even cadence. This is derived rather than written down because a step
/// that is not a whole number of redraws is sampled unevenly however regular
/// the redraws are: 300ms against a 200ms redraw can only land as move, move,
/// hold, so the title limps roughly twice a second.
const SCROLL_STEP: Duration =
    Duration::from_milliseconds(REDRAW_INTERVAL.as_milliseconds() * REDRAWS_PER_STEP);

/// Renders `view` into `frame`.
pub fn render(frame: &mut Frame, view: &ViewModel) {
    let panel = centered(frame.area(), PANEL_WIDTH, PANEL_HEIGHT);
    let block = panel_block();
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    frame.render_widget(
        Paragraph::new(Text::from(lines(view, inner.width))).alignment(Alignment::Center),
        inner,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn panel_block() -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
        .padding(Padding::new(BOX_SIDE_PADDING, BOX_SIDE_PADDING, 1, 1))
        .title(Span::styled(
            " jive ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
}

/// The three rows of the box: a heading, a blank row, and the line below it.
fn lines(view: &ViewModel, width: u16) -> Vec<Line<'static>> {
    match view {
        ViewModel::Playing { name, elapsed } => vec![
            title(rolled(name, width, *elapsed)),
            gap(),
            accented(fit(&format_elapsed(*elapsed), width)),
        ],
        ViewModel::Empty => message_lines("nothing to play", "no audio files here", width),
        ViewModel::Stalled => message_lines("nothing would play", "no file could be read", width),
    }
}

/// A blank row, setting one line apart from the next.
fn gap() -> Line<'static> {
    Line::default()
}

/// A statement, and the reason for it a blank row below.
fn message_lines(statement: &str, reason: &str, width: u16) -> Vec<Line<'static>> {
    vec![
        title(fit(statement, width)),
        gap(),
        muted(fit(reason, width)),
    ]
}

fn title(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::default().add_modifier(Modifier::BOLD))
}

fn accented(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::default().fg(ACCENT))
}

fn muted(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::default().fg(MUTED))
}

fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_whole_seconds();
    let (hours, minutes, seconds) = (total / 3_600, (total / 60) % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// `text` as it is shown after rolling for `elapsed`: whole if it fits, and a
/// window onto a rolling ticker if it does not.
///
/// Text too wide for the panel moves one character every [`SCROLL_STEP`],
/// coming round through [`SCROLL_GAP`] so that all of it can be read in turn.
///
/// The step comes from `elapsed` rather than from a count of frames, so the
/// text is where the clock says it should be even if a redraw is dropped. It
/// also makes the panel a function of the view alone, with no clock of its own.
///
/// The window is padded to exactly `width` columns. A window of the natural
/// width would change size as double-width characters entered and left it, and
/// the centering would shift the line about from one redraw to the next.
fn rolled(text: &str, width: u16, elapsed: Duration) -> String {
    let room = usize::from(width);
    if UnicodeWidthStr::width(text) <= room {
        return text.to_owned();
    }
    let ticker = ticker(text);
    let steps = usize::try_from(elapsed.as_milliseconds() / SCROLL_STEP.as_milliseconds())
        .unwrap_or(usize::MAX);
    let turned: String = ticker
        .iter()
        .cycle()
        .skip(steps % ticker.len())
        .take(ticker.len())
        .collect();
    padded(clipped_to(&turned, room), room)
}

/// The characters `text` rolls through before it comes round again: the text
/// itself, and the gap that separates its end from its start.
fn ticker(text: &str) -> Vec<char> {
    text.chars().chain(SCROLL_GAP.chars()).collect()
}

/// `text`, followed by the spaces that fill it out to `width` columns.
fn padded(text: String, width: usize) -> String {
    let mut padded = text;
    let room = width.saturating_sub(UnicodeWidthStr::width(padded.as_str()));
    padded.push_str(&" ".repeat(room));
    padded
}

/// `text`, or as much of it as fits `width` columns beside an ellipsis.
fn fit(text: &str, width: u16) -> String {
    let width = usize::from(width);
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    let Some(room) = width.checked_sub(ELLIPSIS_WIDTH) else {
        return ELLIPSIS.to_string();
    };
    let mut shortened = clipped_to(text, room);
    shortened.push(ELLIPSIS);
    shortened
}

/// As much of `text` as fits `width` terminal columns.
fn clipped_to(text: &str, width: usize) -> String {
    let mut clipped = String::new();
    let mut used = 0;
    for character in text.chars() {
        used += UnicodeWidthChar::width(character).unwrap_or(0);
        if used > width {
            break;
        }
        clipped.push(character);
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::PANEL_WIDTH;
    use super::REDRAW_INTERVAL;
    use super::SCROLL_GAP;
    use super::SCROLL_STEP;
    use super::render;
    use super::rolled;
    use crate::player::ViewModel;
    use jive_core::Duration;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use unicode_width::UnicodeWidthStr;

    /// The height of the box around a track: the border and its padded rows,
    /// plus the name, a blank row, and the time.
    ///
    /// [`render`] measures the box from what it holds, so this is the size a
    /// track happens to come to rather than a size it is given.
    const PANEL_HEIGHT: u16 = super::BOX_MARGIN_ROWS + 3;

    /// Terminal sizes worth drawing into: the panel's own size, sizes smaller
    /// than it in each direction, and a range of ordinary ones.
    const TERMINAL_SIZES: [(u16, u16); 11] = [
        (16, 7),
        (20, 8),
        (26, 9),
        (PANEL_WIDTH, PANEL_HEIGHT),
        (49, 10),
        (60, 20),
        (80, 24),
        (81, 25),
        (100, 30),
        (120, 40),
        (200, 60),
    ];

    /// Every state the panel can be in, with names that fit it.
    ///
    /// A name too wide to fit is scrolled, and a scrolling name is a full-width
    /// window onto a ticker rather than a centered line. The checks that assert
    /// centering therefore use these, and
    /// [`every_view_including_the_scrolling_ones`] covers the rest.
    fn every_view() -> Vec<ViewModel> {
        vec![
            playing("A Song", 0),
            playing("A Song", 3_671),
            ViewModel::Empty,
            ViewModel::Stalled,
        ]
    }

    /// The same, plus the names the panel scrolls.
    fn every_view_including_the_scrolling_ones() -> Vec<ViewModel> {
        let mut views = every_view();
        views.push(playing(TOO_LONG, 61));
        views.push(playing(TOO_WIDE, 61));
        views
    }

    fn playing(name: &str, seconds: u64) -> ViewModel {
        ViewModel::Playing {
            name: name.to_owned(),
            elapsed: Duration::from_seconds(seconds),
        }
    }

    /// What a terminal of the given size ends up showing.
    struct Screen {
        rows: Vec<Vec<String>>,
        colors: Vec<Color>,
    }

    impl Screen {
        fn of(view: &ViewModel, width: u16, height: u16) -> Self {
            let buffer = drawn(view, width, height);
            Self {
                rows: symbols_of(&buffer, width, height),
                colors: colors_of(&buffer, width, height),
            }
        }

        fn at(view: &ViewModel, size: (u16, u16)) -> Self {
            Self::of(view, size.0, size.1)
        }

        fn text(&self) -> String {
            self.rows
                .iter()
                .map(|row| row.concat())
                .collect::<Vec<String>>()
                .join("\n")
        }

        fn width(&self) -> usize {
            self.rows.first().map_or(0, Vec::len)
        }

        fn occupied(&self, row: usize) -> Option<(usize, usize)> {
            let columns = &self.rows[row];
            let first = columns.iter().position(|symbol| symbol.trim() != "")?;
            let last = columns.iter().rposition(|symbol| symbol.trim() != "")?;
            Some((first, last))
        }

        fn occupied_rows(&self) -> Vec<usize> {
            (0..self.rows.len())
                .filter(|row| self.occupied(*row).is_some())
                .collect()
        }

        /// The background of every cell that has something in it.
        fn backgrounds(&self) -> impl Iterator<Item = Color> + '_ {
            self.colors.iter().skip(1).step_by(2).copied()
        }

        /// The bordered box, which is all the panel draws.
        fn panel(&self) -> Panel {
            let rows = self.occupied_rows();
            let bounds = rows.iter().filter_map(|row| self.occupied(*row));
            Panel {
                top: *rows.first().expect("something was drawn"),
                bottom: *rows.last().expect("something was drawn"),
                left: bounds
                    .clone()
                    .map(|(first, _)| first)
                    .min()
                    .expect("a left edge"),
                right: bounds.map(|(_, last)| last).max().expect("a right edge"),
            }
        }
    }

    fn drawn(view: &ViewModel, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| render(frame, view))
            .expect("a drawn frame");
        terminal.backend().buffer().clone()
    }

    fn positions(width: u16, height: u16) -> impl Iterator<Item = (u16, u16)> {
        (0..height).flat_map(move |row| (0..width).map(move |column| (column, row)))
    }

    fn symbols_of(buffer: &Buffer, width: u16, height: u16) -> Vec<Vec<String>> {
        (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| buffer[(column, row)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    /// Every color the panel paints, foreground and background alike, taken
    /// only from the cells that have something in them.
    fn colors_of(buffer: &Buffer, width: u16, height: u16) -> Vec<Color> {
        positions(width, height)
            .filter(|position| buffer[*position].symbol().trim() != "")
            .flat_map(|position| {
                let cell = &buffer[position];
                [cell.fg, cell.bg]
            })
            .collect()
    }

    #[derive(Debug, Clone, Copy)]
    struct Panel {
        top: usize,
        bottom: usize,
        left: usize,
        right: usize,
    }

    fn assert_balanced(before: usize, after: usize, what: &str) {
        assert!(
            before.abs_diff(after) <= 1,
            "{what} is off center: {before} before, {after} after"
        );
    }

    fn assert_panel_centered(screen: &Screen) {
        let panel = screen.panel();
        assert_balanced(
            panel.left,
            screen.width() - 1 - panel.right,
            "the box horizontally",
        );
        assert_balanced(
            panel.top,
            screen.rows.len() - 1 - panel.bottom,
            "the box vertically",
        );
    }

    fn assert_contents_centered(screen: &Screen) {
        let panel = screen.panel();
        for row in (panel.top + 1)..panel.bottom {
            let interior = &screen.rows[row][(panel.left + 1)..panel.right];
            let Some(first) = interior.iter().position(|symbol| symbol.trim() != "") else {
                continue;
            };
            let last = interior
                .iter()
                .rposition(|symbol| symbol.trim() != "")
                .expect("a line with a start has an end");
            assert_balanced(first, interior.len() - 1 - last, "a line of the panel");
        }
    }

    fn assert_borders_line_up(screen: &Screen) {
        let panel = screen.panel();
        for row in panel.top..=panel.bottom {
            let (first, last) = screen.occupied(row).expect("a row of the panel");
            assert_eq!(first, panel.left, "row {row} starts in the wrong column");
            assert_eq!(last, panel.right, "row {row} ends in the wrong column");
        }
    }

    fn assert_inside_the_terminal(screen: &Screen) {
        let panel = screen.panel();
        assert!(panel.right < screen.width(), "the box ran off the side");
        assert!(
            panel.bottom < screen.rows.len(),
            "the box ran off the bottom"
        );
    }

    /// Everything the panel promises about its geometry, for one view at one
    /// terminal size.
    fn assert_well_formed(view: &ViewModel, width: u16, height: u16) -> Screen {
        let screen = Screen::of(view, width, height);
        assert_panel_centered(&screen);
        assert_contents_centered(&screen);
        assert_borders_line_up(&screen);
        assert_inside_the_terminal(&screen);
        screen
    }

    fn shown(view: &ViewModel) -> String {
        Screen::of(view, 80, 24).text()
    }

    /// Every row between the panel's borders, trimmed, blank rows included.
    ///
    /// Blank rows are the point: they are what the spacing is made of, so they
    /// are kept rather than filtered out.
    fn interior_rows(screen: &Screen) -> Vec<String> {
        let panel = screen.panel();
        ((panel.top + 1)..panel.bottom)
            .map(|row| {
                screen.rows[row][(panel.left + 1)..panel.right]
                    .concat()
                    .trim()
                    .to_owned()
            })
            .collect()
    }

    fn elapsed_shown(seconds: u64) -> String {
        let text = shown(&playing("A Song", seconds));
        text.split_whitespace()
            .find(|word| word.contains(':'))
            .unwrap_or_default()
            .to_owned()
    }

    /// One test per `seconds elapsed => how the panel renders them` row.
    macro_rules! counts_up {
        ($($name:ident: $seconds:expr => $shown:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(elapsed_shown($seconds), $shown);
                }
            )+
        };
    }

    counts_up! {
        a_minute_is_shown_as_minutes_and_seconds: 0 => "00:00";
        a_single_second_is_padded: 1 => "00:01";
        the_last_second_of_a_minute_is_shown: 59 => "00:59";
        a_whole_minute_rolls_over: 60 => "01:00";
        two_minutes_and_five_seconds_are_shown: 125 => "02:05";
        the_last_second_of_an_hour_is_shown: 3_599 => "59:59";
        a_whole_hour_grows_a_field: 3_600 => "1:00:00";
        an_hour_and_change_is_shown: 3_661 => "1:01:01";
        a_long_sitting_keeps_counting: 360_000 => "100:00:00";
    }

    /// One test per `view, word => whether the panel shows it` row.
    ///
    /// A failing row prints the whole screen.
    macro_rules! shows {
        ($($name:ident: $view:expr, $word:expr => $shown:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    let text = shown(&$view);
                    assert_eq!(
                        text.contains($word), $shown,
                        "looking for {:?} in:\n{text}", $word
                    );
                }
            )+
        };
    }

    /// Longer than the panel is wide, so the name has to be cut short.
    const TOO_LONG: &str = "An Extremely Long Track Name That Will Never Fit Inside The Panel";

    /// The same, in characters two columns wide, which are truncated by width
    /// rather than by count.
    const TOO_WIDE: &str = "東京の夜を歩きながら聴いている長い長い曲の名前と副題まで全部入り";

    shows! {
        the_panel_is_named: playing("A Song", 0), "jive" => true;
        a_playing_track_is_named: playing("Morning Rain", 0), "Morning Rain" => true;
        a_playing_track_is_not_labelled: playing("A Song", 0), "now playing" => false;
        a_playing_view_shows_no_key_hint: playing("A Song", 0), "next" => false;
        a_stalled_view_shows_no_key_hint: ViewModel::Stalled, "next" => false;
        a_playing_view_offers_no_quit_key: playing("A Song", 0), "quit" => false;
        a_stalled_view_says_nothing_would_play: ViewModel::Stalled, "nothing" => true;
        an_empty_view_offers_no_quit_key: ViewModel::Empty, "quit" => false;
        an_empty_view_says_there_is_nothing_to_play: ViewModel::Empty, "nothing" => true;
        a_long_name_scrolls_rather_than_being_shortened: playing(TOO_LONG, 0), '…' => false;
        a_wide_name_scrolls_rather_than_being_shortened: playing(TOO_WIDE, 0), '…' => false;
        a_name_that_fits_is_left_alone: playing("A Song", 0), '…' => false;
    }

    /// The box, blank rows and all.
    #[test]
    fn the_box_holds_the_track_and_nothing_beside_it() {
        let playing = Screen::of(&playing("A Song", 65), 80, 24);
        assert_eq!(interior_rows(&playing), ["", "A Song", "", "01:05", ""]);

        let stalled = Screen::of(&ViewModel::Stalled, 80, 24);
        assert_eq!(
            interior_rows(&stalled),
            ["", "nothing would play", "", "no file could be read", ""]
        );

        let empty = Screen::of(&ViewModel::Empty, 80, 24);
        assert_eq!(
            interior_rows(&empty),
            ["", "nothing to play", "", "no audio files here", ""]
        );
    }

    /// The rows of the box that hold something, by position.
    fn filled_rows(rows: &[String]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| !row.is_empty())
            .map(|(at, _)| at)
            .collect()
    }

    /// The box's spacing as a rule rather than as pictures of it.
    ///
    /// A view whose lines changed would keep the pictures above honest only by
    /// being edited into them. This holds whatever the box comes to hold, since
    /// [`render`] measures the box from its content.
    #[test]
    fn the_box_margins_whatever_it_holds_evenly() {
        for view in every_view() {
            let rows = interior_rows(&Screen::of(&view, 80, 24));
            let filled = filled_rows(&rows);
            let above = *filled.first().expect("a line");
            let below = rows.len() - 1 - filled.last().expect("a line");
            assert_eq!(above, below, "the margins should match: {rows:?}");
        }
    }

    /// A heading stands one blank row from the line below it.
    #[test]
    fn the_box_sets_a_heading_apart_from_the_line_below_it() {
        for view in every_view() {
            let rows = interior_rows(&Screen::of(&view, 80, 24));
            let filled = filled_rows(&rows);
            assert_eq!(filled.len(), 2, "two lines: {rows:?}");
            assert_eq!(filled[1] - filled[0], 2, "a row apart: {rows:?}");
        }
    }

    /// A scrolling title is a full-width window onto a ticker, so
    /// [`assert_contents_centered`] does not apply to it. Everything else the
    /// panel promises about its geometry still does.
    #[test]
    fn a_scrolling_title_stays_inside_the_panel_at_every_terminal_size() {
        for size in TERMINAL_SIZES {
            for view in [playing(TOO_LONG, 61), playing(TOO_WIDE, 61)] {
                let screen = Screen::at(&view, size);
                assert_panel_centered(&screen);
                assert_borders_line_up(&screen);
                assert_inside_the_terminal(&screen);
            }
        }
    }

    #[test]
    fn every_view_is_well_formed_at_every_terminal_size() {
        for size in TERMINAL_SIZES {
            for view in every_view() {
                assert_well_formed(&view, size.0, size.1);
            }
        }
    }

    #[test]
    fn a_terminal_far_too_small_for_the_panel_still_draws_something() {
        for size in [(1, 1), (2, 3), (4, 2), (8, 5)] {
            let screen = Screen::at(&playing("A Song", 0), size);
            assert_panel_centered(&screen);
            assert_inside_the_terminal(&screen);
        }
    }

    #[test]
    fn the_messages_fit_the_panel_at_its_own_width() {
        for view in [ViewModel::Empty, ViewModel::Stalled] {
            let text = Screen::of(&view, PANEL_WIDTH, PANEL_HEIGHT).text();
            assert!(!text.contains('…'), "the message should fit whole:\n{text}");
        }
    }

    #[test]
    fn a_message_too_wide_for_the_panel_is_shortened() {
        let text = Screen::of(&ViewModel::Empty, 20, 12).text();
        assert!(
            text.contains('…'),
            "the message should be shortened:\n{text}"
        );
    }

    #[test]
    fn the_panel_is_drawn_in_one_restrained_palette() {
        let allowed = [Color::Reset, Color::Cyan, Color::DarkGray];
        for size in TERMINAL_SIZES {
            for view in every_view_including_the_scrolling_ones() {
                for color in Screen::at(&view, size).colors {
                    assert!(
                        allowed.contains(&color),
                        "{color:?} is not part of the palette"
                    );
                }
            }
        }
    }

    #[test]
    fn nothing_is_drawn_on_a_background_of_its_own() {
        for view in every_view_including_the_scrolling_ones() {
            assert!(
                Screen::at(&view, (80, 24))
                    .backgrounds()
                    .all(|color| color == Color::Reset),
                "the panel should sit on the terminal's own background"
            );
        }
    }

    #[test]
    fn the_same_view_always_draws_the_same_screen() {
        for view in every_view_including_the_scrolling_ones() {
            assert_eq!(shown(&view), shown(&view));
        }
    }

    /// A name shown at `width` columns, `steps` scroll steps into the track.
    fn scrolled(name: &str, width: u16, steps: u64) -> String {
        rolled(
            name,
            width,
            Duration::from_milliseconds(steps * SCROLL_STEP.as_milliseconds()),
        )
    }

    /// How many steps a name takes to come round to its beginning again.
    fn cycle_of(name: &str) -> u64 {
        let characters = name.chars().count() + SCROLL_GAP.chars().count();
        u64::try_from(characters).expect("a name of a sane length")
    }

    /// The cadence rule, as a check rather than a comment.
    ///
    /// A step that is not a whole number of redraws is sampled unevenly however
    /// regular the redraws are, and the title limps. [`SCROLL_STEP`] is derived
    /// so that it cannot happen, and this fails if anyone writes it down again.
    #[test]
    fn a_scroll_step_is_a_whole_number_of_redraws() {
        assert_eq!(
            SCROLL_STEP.as_milliseconds() % REDRAW_INTERVAL.as_milliseconds(),
            0
        );
        assert!(
            SCROLL_STEP >= REDRAW_INTERVAL,
            "a step shorter than a redraw could not be seen"
        );
    }

    #[test]
    fn a_name_that_fits_is_shown_whole_and_stays_where_it_is() {
        assert_eq!(scrolled("A Song", 20, 0), "A Song");
        assert_eq!(scrolled("A Song", 20, 9), "A Song");
        assert_eq!(scrolled("A Song", 6, 9), "A Song", "a name that just fits");
    }

    #[test]
    fn a_name_too_wide_moves_one_character_at_a_time() {
        assert_eq!(scrolled(TOO_LONG, 20, 0), "An Extremely Long Tr");
        assert_eq!(scrolled(TOO_LONG, 20, 1), "n Extremely Long Tra");
        assert_eq!(scrolled(TOO_LONG, 20, 2), " Extremely Long Trac");
    }

    /// The point of scrolling: a name too long to show is readable in full if
    /// the listener waits, rather than being cut off for good.
    #[test]
    fn a_scrolling_name_brings_its_whole_length_into_view() {
        let seen: String = (0..cycle_of(TOO_LONG))
            .map(|step| scrolled(TOO_LONG, 20, step))
            .collect();
        for word in TOO_LONG.split(' ') {
            assert!(seen.contains(word), "{word} never came into view");
        }
    }

    #[test]
    fn a_scrolling_name_comes_round_to_where_it_started() {
        let cycle = cycle_of(TOO_LONG);
        for step in 0..3 {
            assert_eq!(
                scrolled(TOO_LONG, 20, step),
                scrolled(TOO_LONG, 20, step + cycle),
                "step {step} should repeat a cycle later"
            );
        }
    }

    /// Every window is the same width, or the centering would shift the line
    /// about from one redraw to the next. An odd width is used deliberately: a
    /// run of double-width characters cannot fill it exactly, so the last
    /// column has to be padded rather than half a character shown.
    #[test]
    fn every_window_of_a_scrolling_name_is_exactly_the_width_it_was_given() {
        for name in [TOO_LONG, TOO_WIDE] {
            for step in 0..cycle_of(name) {
                let shown = scrolled(name, 21, step);
                assert_eq!(
                    UnicodeWidthStr::width(shown.as_str()),
                    21,
                    "{shown:?} at step {step}"
                );
            }
        }
    }

    #[test]
    fn a_name_of_nothing_at_all_still_draws_a_panel() {
        assert_well_formed(&playing("", 0), 80, 24);
    }

    #[test]
    fn a_name_that_is_only_spaces_still_draws_a_panel() {
        assert_well_formed(&playing("     ", 0), 80, 24);
    }
}
