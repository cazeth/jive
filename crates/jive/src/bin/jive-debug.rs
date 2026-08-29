//! The `jive-debug` command: prints what the shuffle computes for a library.
//!
//! Reads the same directory and collection file `jive` does, and prints each
//! factor, each track's eligibility, and the priority they produce. Nothing is
//! written back, so running it does not affect a later session.

use jive::Error;
use jive::Library;
use jive::Result;
use jive::legend;
use jive::rows;
use jive::table;
use jive_core::Duration;
use jive_core::Time;
use jive_filesystem::Collection;
use jive_filesystem::CollectionFile;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "\
jive-debug — show the numbers the shuffle draws from

Usage:
    jive-debug [DIRECTORY]

Arguments:
    DIRECTORY          A directory below the root to read, as jive would.
                       Defaults to all of the music jive knows about.

Options:
    --state FILE       Read this collection file instead of the usual one
    --in MINUTES       Show the table as it will look this many minutes from now
    -h, --help         Print this help
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jive-debug: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let Some(arguments) = Arguments::parse(std::env::args_os().skip(1))? else {
        println!("{}", HELP.trim_end());
        return Ok(());
    };
    let mut stored = collection_file(arguments.state.as_deref())?
        .load()?
        .ok_or(Error::NoDirectory)?;
    let now = Time::now() + arguments.ahead;

    let directory = arguments
        .directory
        .clone()
        .unwrap_or_else(|| stored.root().to_path_buf());
    let library = library(&mut stored, arguments.directory.as_deref(), now)?;
    report(&library, &directory, now);
    Ok(())
}

fn report(library: &Library, directory: &Path, now: Time) {
    let rows = rows(library, now);
    println!("{} — {} tracks", directory.display(), rows.len());
    println!();
    println!("{}", table(&rows));
    println!();
    println!("{}", legend());
}

fn collection_file(given: Option<&Path>) -> Result<CollectionFile> {
    match given {
        Some(path) => Ok(CollectionFile::at(path)),
        None => Ok(CollectionFile::in_data_directory()?),
    }
}

/// The library as `jive` would build it.
///
/// The collection is not written back, so a track identified here for the first
/// time is identified again on the next run.
fn library(stored: &mut Collection, directory: Option<&Path>, now: Time) -> Result<Library> {
    let tracks = stored.scan(directory)?;
    if tracks.is_empty() {
        return Err(Error::NoTracks {
            path: directory.unwrap_or_else(|| stored.root()).to_path_buf(),
        });
    }
    Ok(Library::build(tracks, stored.history(), now))
}

/// What the command line asked for, or [`None`] if it asked for the help.
#[derive(Debug, Default, PartialEq, Eq)]
struct Arguments {
    directory: Option<PathBuf>,
    state: Option<PathBuf>,
    ahead: Duration,
}

impl Arguments {
    fn parse(arguments: impl IntoIterator<Item = impl Into<OsString>>) -> Result<Option<Self>> {
        let mut parsed = Self::default();
        let mut arguments = arguments.into_iter().map(Into::into);

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("-h" | "--help") => return Ok(None),
                Some("--state") => parsed.state = Some(value_after("--state", &mut arguments)?),
                Some("--in") => {
                    parsed.ahead = minutes(&value_after("--in", &mut arguments)?)?;
                }
                Some(text) if text.starts_with('-') && text != "-" => {
                    return Err(unknown(&argument));
                }
                _ if parsed.directory.is_some() => return Err(unknown(&argument)),
                _ => parsed.directory = Some(PathBuf::from(argument)),
            }
        }
        Ok(Some(parsed))
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

fn minutes(value: &Path) -> Result<Duration> {
    value
        .to_str()
        .and_then(|text| text.parse::<u64>().ok())
        .map(|minutes| Duration::from_seconds(minutes * 60))
        .ok_or_else(|| Error::NotANumber {
            value: value.display().to_string(),
            unit: "minutes",
        })
}

fn unknown(argument: &OsString) -> Error {
    Error::UnknownArgument {
        argument: argument.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::Arguments;
    use super::minutes;
    use jive_core::Duration;
    use std::path::Path;
    use std::path::PathBuf;

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

    fn parse(command_line: &[&str]) -> Option<Arguments> {
        Arguments::parse(command_line.iter().copied()).expect("the command line parses")
    }

    /// The arguments a command line parses to, given that it asks for work.
    fn asking(command_line: &[&str]) -> Arguments {
        parse(command_line).expect("a command line asking for work")
    }

    fn refused(command_line: &[&str]) -> bool {
        Arguments::parse(command_line.iter().copied()).is_err()
    }

    fn span(minutes: u64) -> Duration {
        Duration::from_seconds(minutes * 60)
    }

    #[test]
    fn each_setting_can_be_given_on_the_command_line() {
        assert_eq!(asking(&["/music"]).directory, Some(PathBuf::from("/music")));
        assert_eq!(
            asking(&["--state", "/tmp/state.json"]).state,
            Some(PathBuf::from("/tmp/state.json"))
        );
        assert_eq!(asking(&["--in", "90"]).ahead, span(90));
    }

    #[test]
    fn nothing_given_reads_what_jive_remembers() {
        assert_eq!(asking(&[]), Arguments::default());
    }

    #[test]
    fn asking_for_the_help_asks_for_no_work() {
        assert_eq!(parse(&["-h"]), None);
        assert_eq!(parse(&["--help"]), None);
    }

    #[test]
    fn a_span_is_read_as_whole_minutes_or_not_at_all() {
        assert_eq!(minutes(Path::new("90")).ok(), Some(span(90)));
        assert!(minutes(Path::new("half an hour")).is_err());
    }

    refuses! {
        an_unknown_flag_is_refused: ["--nonsense"];
        a_state_file_without_a_path_is_refused: ["--state"];
        a_span_without_a_number_is_refused: ["--in"];
        a_span_that_is_not_a_number_is_refused: ["--in", "soon"];
        a_second_directory_is_refused: ["/music", "/other"];
    }
}
