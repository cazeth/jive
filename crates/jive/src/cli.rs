//! The `jive` command line: `--root`, the usual `--help` and `--version`, and
//! one optional directory to play.
//!
//! Everything after a bare `--` is taken as the directory, however it is
//! spelled.

use crate::error::Error;
use crate::error::Result;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

/// What `--version` prints.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `--help` prints.
pub const HELP: &str = "\
jive — a music player with one button, and a shuffle that learns from it

Usage:
    jive [DIRECTORY]
    jive --root DIRECTORY

Arguments:
    DIRECTORY              A directory below the root to play instead of all of
                           it. Searched recursively.

Options:
    --root DIRECTORY       Where your music lives. Remembered between runs, and
                           required once before anything can play. Ratings are
                           kept against paths relative to this directory, so
                           moving your music and pointing --root at its new
                           location preserves them.
    -h, --help             Print this help
    -V, --version          Print the version

Keys:
    n                      Play another track
    q                      Leave, as does ctrl-c
";

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Play, with these arguments.
    Play(Arguments),
    /// Print this text and exit.
    Print(&'static str),
}

/// The arguments jive was given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Arguments {
    /// A directory below the root to play instead of all of it.
    pub directory: Option<PathBuf>,
    /// Where the music lives, to remember and play from here on.
    pub root: Option<PathBuf>,
}

impl Arguments {
    /// Reads the arguments this process was started with.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownArgument`] for anything unrecognized, and
    /// [`Error::MissingValue`] for a flag given without one.
    pub fn parse() -> Result<Request> {
        Self::parse_from(std::env::args_os().skip(1))
    }

    /// Reads arguments from any iterator, excluding the program name.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownArgument`] for anything unrecognized, and
    /// [`Error::MissingValue`] for a flag given without one.
    pub fn parse_from(arguments: impl IntoIterator<Item = impl Into<OsString>>) -> Result<Request> {
        let mut parsed = Self::default();
        let mut arguments = arguments.into_iter().map(Into::into);
        let mut flags_are_over = false;

        while let Some(argument) = arguments.next() {
            let flag = (!flags_are_over).then(|| Flag::of(&argument)).flatten();
            let Some(flag) = flag else {
                parsed.take_directory(argument)?;
                continue;
            };
            match parsed.take_flag(flag, &mut arguments)? {
                Taken::Carry => {}
                Taken::FlagsAreOver => flags_are_over = true,
                Taken::Print(text) => return Ok(Request::Print(text)),
            }
        }
        Ok(Request::Play(parsed))
    }

    /// Reads one flag, taking its value from `arguments` if it needs one.
    fn take_flag(
        &mut self,
        flag: Flag,
        arguments: &mut impl Iterator<Item = OsString>,
    ) -> Result<Taken> {
        match flag {
            Flag::Help => Ok(Taken::Print(HELP)),
            Flag::Version => Ok(Taken::Print(VERSION)),
            Flag::EndOfFlags => Ok(Taken::FlagsAreOver),
            Flag::Root => {
                self.root = Some(value_after(Flag::ROOT, arguments)?);
                Ok(Taken::Carry)
            }
            Flag::Unknown(argument) => Err(unknown(&argument)),
        }
    }

    fn take_directory(&mut self, argument: OsString) -> Result<()> {
        if self.directory.is_some() {
            return Err(unknown(&argument));
        }
        self.directory = Some(PathBuf::from(argument));
        Ok(())
    }

    /// Where the music lives: the root given on the command line, else the one
    /// `remembered` from an earlier run, else the directory to play, which is
    /// how a first run settles it.
    ///
    /// # Errors
    ///
    /// [`Error::NoDirectory`] when nothing says where the music is.
    pub fn root_of(&self, remembered: Option<&Path>) -> Result<PathBuf> {
        self.root
            .clone()
            .or_else(|| remembered.map(Path::to_path_buf))
            .or_else(|| self.directory.clone())
            .ok_or(Error::NoDirectory)
    }

    /// Which directory to play, or [`None`] for everything below the root.
    #[must_use]
    pub fn directory_to_play(&self) -> Option<&Path> {
        self.directory.as_deref()
    }
}

fn value_after(flag: &str, arguments: &mut impl Iterator<Item = OsString>) -> Result<PathBuf> {
    arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| Error::MissingValue {
            flag: flag.to_owned(),
        })
}

fn unknown(argument: &OsString) -> Error {
    Error::UnknownArgument {
        argument: argument.to_string_lossy().into_owned(),
    }
}

/// What reading a flag leaves the parser to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Taken {
    /// Read the next argument.
    Carry,
    /// Everything after this is a directory.
    FlagsAreOver,
    /// Print this text and exit.
    Print(&'static str),
}

/// The flag an argument stands for, as opposed to a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Flag {
    Help,
    Version,
    Root,
    /// Everything after this is a directory, however it is spelled.
    EndOfFlags,
    Unknown(OsString),
}

impl Flag {
    const ROOT: &'static str = "--root";

    /// The flag an argument stands for, or [`None`] if it is a directory.
    fn of(argument: &OsString) -> Option<Self> {
        let text = argument.to_str()?;
        if !text.starts_with('-') || text == "-" {
            return None;
        }
        Some(match text {
            "-h" | "--help" => Self::Help,
            "-V" | "--version" => Self::Version,
            Self::ROOT => Self::Root,
            "--" => Self::EndOfFlags,
            _ => Self::Unknown(argument.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Arguments;
    use super::HELP;
    use super::Request;
    use super::VERSION;
    use std::path::Path;
    use std::path::PathBuf;

    fn parse(command_line: &[&str]) -> Request {
        Arguments::parse_from(command_line.iter().copied()).expect("the command line parses")
    }

    fn arguments(command_line: &[&str]) -> Arguments {
        match parse(command_line) {
            Request::Play(arguments) => arguments,
            Request::Print(_) => panic!("expected arguments, not something to print"),
        }
    }

    /// Where the music would live, given a command line and what an earlier
    /// session remembered.
    fn root(command_line: &[&str], remembered: Option<&str>) -> Option<PathBuf> {
        arguments(command_line)
            .root_of(remembered.map(Path::new))
            .ok()
    }

    /// Which directory would play, given a command line.
    fn played(command_line: &[&str]) -> Option<PathBuf> {
        arguments(command_line)
            .directory_to_play()
            .map(Path::to_path_buf)
    }

    fn refused(command_line: &[&str]) -> bool {
        Arguments::parse_from(command_line.iter().copied()).is_err()
    }

    /// One test per `command line, remembered root => where the music lives`
    /// row.
    macro_rules! roots {
        ($($name:ident: $command_line:expr, $remembered:expr => $root:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(root(&$command_line, $remembered), $root.map(PathBuf::from));
                }
            )+
        };
    }

    /// One test per `command line => directory played` row.
    macro_rules! plays {
        ($($name:ident: $command_line:expr => $directory:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(played(&$command_line), $directory.map(PathBuf::from));
                }
            )+
        };
    }

    /// One test per command line that must be refused.
    macro_rules! refuses {
        ($($name:ident: $command_line:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert!(refused(&$command_line), "should have been refused");
                }
            )+
        };
    }

    roots! {
        a_first_run_takes_the_directory_it_is_pointed_at: ["/music"], None => Some("/music");
        a_remembered_root_is_used_when_none_is_given: [], Some("/music") => Some("/music");
        a_remembered_root_wins_over_a_directory_to_play:
            ["/music/rock"], Some("/music") => Some("/music");
        a_root_given_directly_wins_over_the_remembered_one:
            ["--root", "/mnt/music"], Some("/music") => Some("/mnt/music");
        a_root_given_directly_wins_over_a_directory_to_play:
            ["/music/rock", "--root", "/music"], None => Some("/music");
        a_root_beyond_ascii_is_taken_whole:
            ["--root", "/music/東京"], None => Some("/music/東京");
        a_root_with_spaces_is_taken_whole:
            ["--root", "/music/late night"], None => Some("/music/late night");
        nothing_at_all_is_reported: [], None => Option::<&str>::None;
    }

    plays! {
        a_directory_given_directly_is_what_plays: ["/music/rock"] => Some("/music/rock");
        a_relative_directory_is_taken_as_given: ["."] => Some(".");
        setting_a_root_plays_all_of_it: ["--root", "/music"] => Option::<&str>::None;
        nothing_given_plays_the_whole_root: [] => Option::<&str>::None;
        a_directory_spelled_like_a_flag_is_taken_after_a_bare_pair_of_dashes:
            ["--", "--music"] => Some("--music");
        a_lone_dash_is_a_directory: ["-"] => Some("-");
    }

    refuses! {
        an_unknown_flag_is_refused: ["--shuffle"];
        an_unknown_short_flag_is_refused: ["-x"];
        a_second_directory_is_refused: ["/music", "/other"];
        a_root_without_a_value_is_refused: ["--root"];
        a_second_directory_after_the_dashes_is_refused: ["--", "/music", "/other"];
    }

    /// Why a command line was refused, as the listener would read it.
    fn refusal(command_line: &[&str]) -> String {
        arguments(command_line)
            .root_of(None)
            .expect_err("there is nothing to play")
            .to_string()
    }

    /// One test per `command line => text printed instead of playing` row.
    macro_rules! prints {
        ($($name:ident: $command_line:expr => $text:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(parse(&$command_line), Request::Print($text));
                }
            )+
        };
    }

    prints! {
        help_is_asked_for_by_its_short_flag: ["-h"] => HELP;
        help_is_asked_for_by_its_long_flag: ["--help"] => HELP;
        help_wins_over_anything_else_on_the_line: ["/music", "--help"] => HELP;
        the_version_is_asked_for_by_its_short_flag: ["-V"] => VERSION;
        the_version_is_asked_for_by_its_long_flag: ["--version"] => VERSION;
    }

    /// Two separate settings: the root says where the collection lives, the
    /// directory says what to play out of it this time.
    #[test]
    fn a_directory_to_play_is_kept_apart_from_the_root() {
        let both = arguments(&["/music/rock", "--root", "/music"]);
        assert_eq!(both.directory, Some(PathBuf::from("/music/rock")));
        assert_eq!(both.root, Some(PathBuf::from("/music")));
        assert_eq!(
            arguments(&["/music"]).root,
            None,
            "playing a directory should move nothing"
        );
    }

    #[test]
    fn a_flag_after_the_dashes_is_a_directory_rather_than_a_flag() {
        let after = arguments(&["--", "--help"]);
        assert_eq!(after.directory, Some(PathBuf::from("--help")));
    }

    #[test]
    fn nothing_to_play_says_how_to_fix_it() {
        assert!(refusal(&[]).contains("--root"), "{}", refusal(&[]));
    }

    /// The panel names no key, so the help is the one place in the program
    /// every key is written down.
    #[test]
    fn the_help_covers_every_flag_and_every_key() {
        for mention in [
            "--root",
            "--help",
            "--version",
            "DIRECTORY",
            "n",
            "q",
            "ctrl-c",
        ] {
            assert!(HELP.contains(mention), "the help should mention {mention}");
        }
    }
}
