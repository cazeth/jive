//! The `jive` command: reads the command line and plays what it points at.
//!
//! Errors are printed to standard error and reported as a failing exit status.

use jive::Arguments;
use jive::Request;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jive: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> jive::Result<()> {
    match Arguments::parse()? {
        Request::Play(arguments) => jive::run(&arguments),
        Request::Print(text) => {
            println!("{}", text.trim_end());
            Ok(())
        }
    }
}
