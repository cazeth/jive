//! Running the player in a terminal.
//!
//! [`run`] loads the collection, builds a library from it, and runs the loop:
//! read a key, poll the backend, redraw, save anything new.

use crate::cli::Arguments;
use crate::error::Error;
use crate::error::Result;
use crate::library::Library;
use crate::player::Player;
use crate::selection::Shuffle;
use crate::ui;
use crossterm::cursor::Hide;
use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use jive_core::AudioBackend;
use jive_core::Time;
use jive_filesystem::Collection;
use jive_filesystem::CollectionFile;
use jive_mpv::MpvBackend;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::Stdout;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration as PollInterval;
use std::time::Instant;

/// How long a redraw is due after the last one.
///
/// Taken from [`ui::REDRAW_INTERVAL`], which the panel also scrolls a title by,
/// so the two cannot drift apart.
const REDRAW_INTERVAL: PollInterval =
    PollInterval::from_millis(ui::REDRAW_INTERVAL.as_milliseconds());

/// What a key press asks the player to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Skip to another track. Asked for by `n`, the one key the panel shows.
    Next,
    /// Leave. Asked for by `q` or the terminal's interrupt, neither of which
    /// the panel shows.
    Quit,
}

/// Runs the player until the listener leaves.
///
/// # Errors
///
/// If the directory cannot be played, the collection file cannot be used, or
/// the terminal or backend cannot be driven.
pub fn run(arguments: &Arguments) -> Result<()> {
    let mut store = Store::open(arguments)?;
    let mut player = open_player(&mut store, arguments)?;

    store.save(&player)?;
    let outcome = play(&mut player, &mut store);
    let stopped = player.stop(Time::now());
    let saved = store.save(&player);
    outcome.and(stopped).and(saved)
}

fn open_player(store: &mut Store, arguments: &Arguments) -> Result<Player<MpvBackend>> {
    let library = store.library(arguments.directory_to_play(), Time::now())?;
    Ok(Player::new(
        MpvBackend::new()?,
        library,
        Shuffle::from_clock(),
    ))
}

/// Draws and takes turns until the listener leaves.
///
/// Each draw is due a fixed [`REDRAW_INTERVAL`] after the last, rather than an
/// interval after the previous turn *finished*. Waiting a whole interval every
/// turn would add the time the turn itself took, so the draws would slide later
/// and later against the clock, and anything moving on the panel would move
/// unevenly.
fn play<Backend: AudioBackend>(player: &mut Player<Backend>, store: &mut Store) -> Result<()> {
    let mut session = Session::open()?;
    player.start(Time::now())?;
    let mut due = Instant::now();
    loop {
        session.draw(player)?;
        due = next_draw_due(due);
        if take_turn(player, store, due)? == Turn::Leave {
            return Ok(());
        }
    }
}

/// When the next draw is due, keeping to the fixed schedule `due` sits on.
///
/// A turn that ended early, because a key arrived, keeps the deadline it was
/// given. One that overran is advanced whole intervals at a time until it is in
/// the future, so a slow turn costs one draw rather than leaving the loop to
/// spin through every deadline it missed.
fn next_draw_due(due: Instant) -> Instant {
    let now = Instant::now();
    let mut next = due;
    while next <= now {
        next += REDRAW_INTERVAL;
    }
    next
}

/// Whether the loop continues after a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Continue,
    Leave,
}

/// One turn: read a key until `due`, poll the backend, save anything new.
fn take_turn<Backend: AudioBackend>(
    player: &mut Player<Backend>,
    store: &mut Store,
    due: Instant,
) -> Result<Turn> {
    let wait = due.saturating_duration_since(Instant::now());
    if let Some(command) = next_command(wait)?
        && obey(command, player, Time::now())? == Turn::Leave
    {
        return Ok(Turn::Leave);
    }
    player.poll(Time::now())?;
    save_what_is_new(player, store);
    Ok(Turn::Continue)
}

/// Acts on a command, returning whether the loop continues.
fn obey<Backend: AudioBackend>(
    command: Command,
    player: &mut Player<Backend>,
    now: Time,
) -> Result<Turn> {
    match command {
        Command::Next => {
            player.skip(now)?;
            Ok(Turn::Continue)
        }
        Command::Quit => Ok(Turn::Leave),
    }
}

/// Saves anything recorded since the last save.
///
/// A failed write is reported by the final save on exit rather than here, so a
/// full disk does not interrupt playback, and no further writes are attempted:
/// a turn comes round several times a second, and each retry would re-encode
/// the whole library. The previous contents survive either way, since a failed
/// save never replaces the file.
fn save_what_is_new<Backend: AudioBackend>(player: &mut Player<Backend>, store: &mut Store) {
    if player.has_unsaved_events() && store.may_save() && store.save(player).is_ok() {
        player.mark_saved();
    }
}

/// A collection file and the collection read from it.
struct Store {
    file: CollectionFile,
    collection: Collection,
    /// Whether a save has failed, after which no more are attempted.
    save_failed: bool,
}

impl Store {
    fn open(arguments: &Arguments) -> Result<Self> {
        Self::at(CollectionFile::in_data_directory()?, arguments)
    }

    /// Opens the store and settles where the music lives.
    ///
    /// The root is resolved to a single absolute path, so that two names for one
    /// directory do not produce two sets of identifiers. A root given on the
    /// command line replaces the one the file remembered.
    fn at(file: CollectionFile, arguments: &Arguments) -> Result<Self> {
        let stored = file.load()?;
        let root = resolve(&arguments.root_of(stored.as_ref().map(Collection::root))?);
        let mut collection = stored.unwrap_or_else(|| Collection::new(&root));
        collection.set_root(&root);
        Ok(Self {
            file,
            collection,
            save_failed: false,
        })
    }

    /// The tracks to play: everything below `directory`, or the whole root if
    /// none is given.
    fn library(&mut self, directory: Option<&Path>, now: Time) -> Result<Library> {
        let directory = directory.map(resolve);
        let tracks = self.collection.scan(directory.as_deref())?;
        if tracks.is_empty() {
            return Err(Error::NoTracks {
                path: directory.unwrap_or_else(|| self.collection.root().to_path_buf()),
            });
        }
        Ok(Library::build(tracks, self.collection.history(), now))
    }

    /// Whether a save is worth attempting. It is not, once one has failed.
    fn may_save(&self) -> bool {
        !self.save_failed
    }

    fn save<Backend: AudioBackend>(&mut self, player: &Player<Backend>) -> Result<()> {
        player.library().store_into(self.collection.history_mut());
        let outcome = self.file.save(&self.collection).map_err(Error::from);
        self.save_failed = outcome.is_err();
        outcome
    }
}

fn next_command(timeout: PollInterval) -> Result<Option<Command>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    Ok(command_of(&event::read()?))
}

fn command_of(event: &Event) -> Option<Command> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if is_interrupt(key) {
        return Some(Command::Quit);
    }
    match key.code {
        KeyCode::Char('n' | 'N') => Some(Command::Next),
        KeyCode::Char('q' | 'Q') => Some(Command::Quit),
        _ => None,
    }
}

fn is_interrupt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c' | 'C' | 'd' | 'D'))
}

/// The canonical absolute path of `directory`, so that one directory named two
/// ways rates the same tracks.
///
/// A path that cannot be resolved passes through unchanged, so the failure is
/// reported against what the listener typed.
fn resolve(directory: &Path) -> PathBuf {
    std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf())
}

/// The terminal, for as long as the player owns it. Restored on drop.
struct Session {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Session {
    fn open() -> Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        Self::take_over_screen().inspect_err(|_| drop(close_terminal()))
    }

    fn take_over_screen() -> Result<Self> {
        let mut output = std::io::stdout();
        execute!(output, EnterAlternateScreen, Hide)?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(output))?,
        })
    }

    fn draw<Backend: AudioBackend>(&mut self, player: &Player<Backend>) -> Result<()> {
        let view = player.view(Time::now());
        self.terminal.draw(|frame| ui::render(frame, &view))?;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        drop(close_terminal());
    }
}

/// Restores the terminal, attempting every step even if an earlier one fails,
/// so that nothing can leave the shell in raw mode.
fn close_terminal() -> std::io::Result<()> {
    let screen_restored = execute!(std::io::stdout(), Show, LeaveAlternateScreen);
    let raw_mode_ended = disable_raw_mode();
    screen_restored.and(raw_mode_ended)
}

fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |information| {
            drop(close_terminal());
            previous(information);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::Collection;
    use super::Command;
    use super::Library;
    use super::PollInterval;
    use super::REDRAW_INTERVAL;
    use super::Store;
    use super::Turn;
    use super::command_of;
    use super::next_draw_due;
    use super::obey;
    use super::resolve;
    use crate::testing::FakeBackend;
    use crate::testing::PlayerFixture;
    use crate::testing::StoreFixture;
    use crossterm::event::Event;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyEventKind;
    use crossterm::event::KeyEventState;
    use crossterm::event::KeyModifiers;
    use jive_core::Time;
    use std::path::Path;
    use std::time::Instant;

    fn press(code: KeyCode) -> Option<Command> {
        with_modifiers(code, KeyModifiers::NONE)
    }

    fn with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> Option<Command> {
        command_of(&Event::Key(KeyEvent::new(code, modifiers)))
    }

    fn of_kind(code: KeyCode, kind: KeyEventKind) -> Option<Command> {
        command_of(&Event::Key(KeyEvent::new_with_kind_and_state(
            code,
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        )))
    }

    fn typing(character: char) -> Option<Command> {
        press(KeyCode::Char(character))
    }

    /// One test per `key pressed => command it asks for` row.
    macro_rules! presses {
        ($($name:ident: $key:expr => $command:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!($key, $command);
                }
            )+
        };
    }

    presses! {
        the_next_key_asks_for_the_next_track: typing('n') => Some(Command::Next);
        the_next_key_answers_in_upper_case_too: typing('N') => Some(Command::Next);
        the_quit_key_asks_to_leave: typing('q') => Some(Command::Quit);
        the_quit_key_answers_in_upper_case_too: typing('Q') => Some(Command::Quit);
        escape_does_nothing: press(KeyCode::Esc) => None;
        an_interrupt_asks_to_leave:
            with_modifiers(KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Command::Quit);
        an_upper_case_interrupt_asks_to_leave:
            with_modifiers(KeyCode::Char('C'), KeyModifiers::CONTROL) => Some(Command::Quit);
        an_end_of_input_asks_to_leave:
            with_modifiers(KeyCode::Char('d'), KeyModifiers::CONTROL) => Some(Command::Quit);
        a_shifted_next_still_asks_for_the_next_track:
            with_modifiers(KeyCode::Char('N'), KeyModifiers::SHIFT) => Some(Command::Next);
        a_key_being_let_go_does_nothing:
            of_kind(KeyCode::Char('n'), KeyEventKind::Release) => None;
        a_key_being_held_down_does_nothing_either:
            of_kind(KeyCode::Char('n'), KeyEventKind::Repeat) => None;
    }

    /// A turn that overran must not leave the schedule behind it. Advancing to
    /// "now plus an interval" instead would fold the overrun into every later
    /// draw, and the panel would scroll a little more unevenly each time.
    #[test]
    fn a_draw_that_overran_stays_on_the_schedule() {
        let missed = Instant::now()
            .checked_sub(REDRAW_INTERVAL * 3 + PollInterval::from_millis(7))
            .expect("a moment in the past");
        let next = next_draw_due(missed);

        assert!(next > Instant::now(), "the next draw should be ahead");
        assert_eq!(
            next.duration_since(missed).as_millis() % REDRAW_INTERVAL.as_millis(),
            0,
            "the schedule should keep to whole intervals"
        );
    }

    /// A key arriving early ends the turn early. The draw it interrupted is
    /// still due when it was due, so the schedule does not shift forward.
    #[test]
    fn a_turn_that_ended_early_keeps_the_deadline_it_was_given() {
        let ahead = Instant::now() + REDRAW_INTERVAL * 5;
        assert_eq!(next_draw_due(ahead), ahead);
    }

    #[test]
    fn a_directory_that_cannot_be_resolved_is_left_as_given() {
        let unresolvable = Path::new("no/such/directory");
        assert_eq!(resolve(unresolvable), unresolvable);
    }

    /// The whole of the player's keyboard: two letters, and the interrupt every
    /// terminal program answers. Only `n` reaches the panel, which
    /// [`crate::ui`] covers.
    #[test]
    fn the_only_letters_that_do_anything_are_next_and_quit() {
        let acting: Vec<char> = ('a'..='z')
            .filter(|character| typing(*character).is_some())
            .collect();
        assert_eq!(acting, ['n', 'q']);
    }

    /// A store over a collection file and a music directory that are removed
    /// with the test.
    fn store_over(fixture: &StoreFixture) -> Store {
        Store::at(fixture.file(), &fixture.arguments()).expect("an empty collection file opens")
    }

    /// The library a store finds, so that what a player records lands on the
    /// identifiers the collection file holds.
    fn library_found_by(store: &mut Store) -> Library {
        store
            .library(None, Time::EPOCH)
            .expect("the music directory can be read")
    }

    /// A store over the fixture, and a started player over what it found.
    fn playing_over(fixture: &StoreFixture) -> (Store, PlayerFixture) {
        let mut store = store_over(fixture);
        let mut player = PlayerFixture::over(library_found_by(&mut store), FakeBackend::default());
        player.start();
        (store, player)
    }

    fn save(store: &mut Store, player: &PlayerFixture) {
        store
            .save(&player.player)
            .expect("the collection is written");
    }

    /// How many skips a library holds across all its tracks.
    fn skips_in(library: &Library) -> usize {
        library
            .tracks()
            .flat_map(|track| track.events.iter())
            .filter(|event| event.event.as_skipped().is_some())
            .count()
    }

    /// The collection stored in the fixture's file.
    fn stored(fixture: &StoreFixture) -> Collection {
        fixture
            .file()
            .load()
            .expect("the collection loads")
            .expect("a collection")
    }

    /// How many skips reached the file.
    fn skips_stored(fixture: &StoreFixture) -> usize {
        stored(fixture)
            .history()
            .tracks()
            .flat_map(|(_, events)| events.iter())
            .filter(|event| event.event.as_skipped().is_some())
            .count()
    }

    /// A turn comes round several times a second, so a store that has failed
    /// to write must not be asked again for the rest of the session.
    #[test]
    fn a_store_stops_saving_once_a_save_has_failed() {
        let fixture = StoreFixture::holding(&["one.mp3", "two.mp3"]);
        let (mut store, mut player) = playing_over(&fixture);
        player.wait(3).press_next();
        assert!(store.may_save(), "nothing has failed yet");

        std::fs::create_dir_all(fixture.file().path())
            .expect("a directory standing in for the file");

        assert!(store.save(&player.player).is_err());
        assert!(!store.may_save(), "a save that failed is not tried again");
    }

    #[test]
    fn saving_puts_what_the_player_recorded_into_the_file() {
        let fixture = StoreFixture::holding(&["one.mp3", "two.mp3"]);
        let (mut store, mut player) = playing_over(&fixture);
        player.wait(3).press_next();

        save(&mut store, &player);

        assert_eq!(
            skips_stored(&fixture),
            1,
            "the skip should have reached the file"
        );
    }

    #[test]
    fn saving_nothing_new_still_leaves_a_readable_file() {
        let fixture = StoreFixture::holding(&["one.mp3"]);
        let (mut store, player) = playing_over(&fixture);
        save(&mut store, &player);
        assert!(fixture.file().load().is_ok());
    }

    #[test]
    fn where_the_music_lives_is_kept_when_the_store_is_saved() {
        let fixture = StoreFixture::holding(&["one.mp3"]);
        let (mut store, player) = playing_over(&fixture);

        save(&mut store, &player);

        assert_eq!(stored(&fixture).root(), resolve(fixture.music()).as_path());
    }

    /// The point of the whole arrangement: the music moves, and every event
    /// recorded against it moves with it.
    #[test]
    fn music_moved_somewhere_else_keeps_its_events() {
        let fixture = StoreFixture::holding(&["one.mp3", "two.mp3"]);
        let (mut store, mut player) = playing_over(&fixture);
        player.wait(3).press_next();
        save(&mut store, &player);

        let moved = StoreFixture::holding(&["one.mp3", "two.mp3"]);
        let mut after_moving = Store::at(fixture.file(), &moved.arguments())
            .expect("the collection file opens against the new root");
        let library = library_found_by(&mut after_moving);

        assert_eq!(library.len(), 2);
        assert_eq!(
            skips_in(&library),
            1,
            "the skip should have followed the music"
        );
    }

    #[test]
    fn a_directory_outside_the_music_is_refused_rather_than_played() {
        let fixture = StoreFixture::holding(&["one.mp3"]);
        let elsewhere = StoreFixture::holding(&["other.mp3"]);
        let mut store = store_over(&fixture);

        assert!(store.library(Some(elsewhere.music()), Time::EPOCH).is_err());
    }

    #[test]
    fn the_next_key_plays_something_else_and_carries_on() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        let now = fixture.now();
        let turn =
            obey(Command::Next, &mut fixture.player, now).expect("the next track can be played");
        assert_eq!(turn, Turn::Continue);
        assert_eq!(fixture.tracks_played(), 2);
        assert_eq!(fixture.skips().len(), 1);
    }

    #[test]
    fn leaving_does_not_touch_the_track() {
        let mut fixture = PlayerFixture::playing(&["one", "two"]);
        let now = fixture.now();
        let turn = obey(Command::Quit, &mut fixture.player, now).expect("quitting always works");
        assert_eq!(turn, Turn::Leave);
        assert_eq!(fixture.tracks_played(), 1);
        assert!(fixture.skips().is_empty());
    }

    #[test]
    fn a_directory_named_two_ways_resolves_to_one_path() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let nested = directory.path().join("albums");
        std::fs::create_dir(&nested).expect("a directory");
        assert_eq!(resolve(&nested.join("..").join("albums")), resolve(&nested));
    }

    #[test]
    fn a_resolved_directory_is_absolute() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        assert!(resolve(directory.path()).is_absolute());
    }
}
