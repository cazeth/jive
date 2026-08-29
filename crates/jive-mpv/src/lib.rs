//! An [`AudioBackend`] that plays tracks with [mpv](https://mpv.io).
//!
//! One mpv process is run per track. When it exits on its own, the track is
//! over. Its exit status, together with anything it wrote to standard error,
//! becomes a [`PlaybackOutcome`]. When the listener moves on first, the process
//! is killed and no outcome is produced.
//!
//! mpv identifies files by content rather than by extension, so nothing is
//! rejected here on the strength of a file name. A file whose contents do not
//! match its name plays if mpv can decode it, and yields
//! [`PlaybackOutcome::Failed`] if it cannot. A path that is not a file yields
//! [`TrackFailure::FileNotFound`] without running mpv at all.
//!
//! [`MpvBackend::new`] runs `mpv --version` before returning, so a missing or
//! unusable program is reported as a [`BackendError`] up front rather than as
//! every track failing in turn.
//!
//! ```no_run
//! use jive_core::AudioBackend;
//! use jive_mpv::MpvBackend;
//! use std::path::Path;
//!
//! let mut backend = MpvBackend::new()?;
//! backend.play(Path::new("song.flac"))?;
//! while backend.poll_event()?.is_none() {
//!     std::thread::sleep(std::time::Duration::from_millis(100));
//! }
//! # Ok::<(), jive_core::BackendError>(())
//! ```

use jive_core::AudioBackend;
use jive_core::BackendError;
use jive_core::BackendResult;
use jive_core::track_events::PlaybackOutcome;
use jive_core::track_events::TrackFailure;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io;
use std::io::Read;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::thread::JoinHandle;

/// The program searched for on `PATH` when none is given.
pub const DEFAULT_PROGRAM: &str = "mpv";

/// The arguments every track is played with: no video, no terminal input, no
/// user configuration, errors only, and a trailing `--` so that a track named
/// like a flag is still playable.
const ARGUMENTS: [&str; 5] = [
    "--no-video",
    "--no-input-terminal",
    "--no-config",
    "--msg-level=all=error",
    "--",
];

/// An [`AudioBackend`] that plays one track per mpv process.
#[derive(Debug)]
pub struct MpvBackend {
    program: OsString,
    running: Option<RunningTrack>,
    pending_outcome: Option<PlaybackOutcome>,
}

#[derive(Debug)]
struct RunningTrack {
    process: Child,
    diagnostics: Option<JoinHandle<String>>,
}

impl MpvBackend {
    /// A backend running [`DEFAULT_PROGRAM`] from `PATH`.
    ///
    /// # Errors
    ///
    /// [`BackendError::Unavailable`] if mpv cannot be run.
    pub fn new() -> BackendResult<Self> {
        Self::with_program(DEFAULT_PROGRAM)
    }

    /// A backend running the mpv binary at `program`.
    ///
    /// # Errors
    ///
    /// [`BackendError::Unavailable`] if it cannot be run.
    pub fn with_program(program: impl AsRef<OsStr>) -> BackendResult<Self> {
        let program = program.as_ref().to_os_string();
        verify_program(&program)?;
        Ok(Self::unverified(program))
    }

    fn unverified(program: OsString) -> Self {
        Self {
            program,
            running: None,
            pending_outcome: None,
        }
    }

    fn spawn(&mut self, path: &Path) -> BackendResult<()> {
        let mut process = self
            .command_for(path)
            .spawn()
            .map_err(|error| BackendError::play(path, error))?;
        let diagnostics = process.stderr.take().map(read_in_background);
        self.running = Some(RunningTrack {
            process,
            diagnostics,
        });
        Ok(())
    }

    fn command_for(&self, path: &Path) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(ARGUMENTS)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command
    }

    fn take_outcome(&mut self) -> BackendResult<Option<PlaybackOutcome>> {
        let Some(status) = self.exit_status()? else {
            return Ok(None);
        };
        let diagnostics = self.take_diagnostics();
        Ok(Some(classify_exit(status.code(), &diagnostics)))
    }

    /// The exit status, or [`None`] if the process is still running or there is
    /// none.
    fn exit_status(&mut self) -> BackendResult<Option<ExitStatus>> {
        let Some(running) = self.running.as_mut() else {
            return Ok(None);
        };
        running.process.try_wait().map_err(BackendError::poll)
    }

    /// What the finished process wrote to standard error, consuming the running
    /// track.
    fn take_diagnostics(&mut self) -> String {
        self.running
            .take()
            .and_then(|running| running.diagnostics)
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default()
    }
}

impl AudioBackend for MpvBackend {
    fn play(&mut self, path: &Path) -> BackendResult<()> {
        self.stop()?;
        if path.is_file() {
            self.spawn(path)
        } else {
            self.pending_outcome = Some(TrackFailure::FileNotFound.into());
            Ok(())
        }
    }

    fn stop(&mut self) -> BackendResult<()> {
        self.pending_outcome = None;
        let Some(mut running) = self.running.take() else {
            return Ok(());
        };
        let stopped = running
            .process
            .kill()
            .and_then(|()| running.process.wait().map(|_| ()));
        drop(running.diagnostics.map(JoinHandle::join));
        stopped.map_err(BackendError::stop)
    }

    fn poll_event(&mut self) -> BackendResult<Option<PlaybackOutcome>> {
        if let Some(outcome) = self.pending_outcome.take() {
            return Ok(Some(outcome));
        }
        self.take_outcome()
    }
}

impl Drop for MpvBackend {
    fn drop(&mut self) {
        drop(self.stop());
    }
}

fn verify_program(program: &OsStr) -> BackendResult<()> {
    match report_version(program) {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(BackendError::unavailable(format!(
            "`{} --version` exited with {status}",
            program.to_string_lossy()
        ))),
        Err(error) => Err(BackendError::unavailable(io::Error::new(
            error.kind(),
            format!("`{}` could not be run: {error}", program.to_string_lossy()),
        ))),
    }
}

fn report_version(program: &OsStr) -> io::Result<ExitStatus> {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

fn read_in_background(mut output: impl Read + Send + 'static) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut collected = String::new();
        drop(output.read_to_string(&mut collected));
        collected
    })
}

/// The outcome an exit status and its diagnostics amount to.
///
/// A clean exit is a finish, whatever was written. Otherwise the diagnostics
/// decide. When they name no failure, the exit status decides:
///
/// * 2 is a missing file.
/// * 3 is a decoder failure.
/// * Anything else is the backend giving up, including a process that was
///   killed, which reports no code at all.
fn classify_exit(code: Option<i32>, diagnostics: &str) -> PlaybackOutcome {
    if code == Some(0) {
        return PlaybackOutcome::Finished;
    }
    if let Some(failure) = reported_failure(diagnostics) {
        return failure.into();
    }
    match code {
        Some(2) => TrackFailure::FileNotFound.into(),
        Some(3) => TrackFailure::DecoderError.into(),
        _ => TrackFailure::BackendExited.into(),
    }
}

/// Substrings of mpv's diagnostics that identify a failure, most specific
/// first.
///
/// Order matters: a file mpv cannot parse produces complaints about both
/// opening the file and its format, and the format is the more informative of
/// the two.
const FAILURE_SIGNS: [(TrackFailure, &[&str]); 4] = [
    (
        TrackFailure::UnsupportedFormat,
        &[
            "recognize file format",
            "No audio or video streams",
            "Unrecognized file format",
        ],
    ),
    (
        TrackFailure::FileNotFound,
        &["No such file", "does not exist", "Failed to open"],
    ),
    (
        TrackFailure::DecoderError,
        &[
            "Could not open codec",
            "Failed to initialize a decoder",
            "Error decoding",
        ],
    ),
    (
        TrackFailure::BackendExited,
        &["Could not open/initialize audio device"],
    ),
];

/// The failure `diagnostics` names, or [`None`] if they name none.
fn reported_failure(diagnostics: &str) -> Option<TrackFailure> {
    FAILURE_SIGNS
        .iter()
        .find(|(_, signs)| mentions_any(diagnostics, signs))
        .map(|(failure, _)| *failure)
}

fn mentions_any(diagnostics: &str, signs: &[&str]) -> bool {
    signs.iter().any(|sign| diagnostics.contains(sign))
}

#[cfg(test)]
mod tests {
    use super::ARGUMENTS;
    use super::DEFAULT_PROGRAM;
    use super::MpvBackend;
    use super::classify_exit;
    use jive_core::AudioBackend;
    use jive_core::track_events::PlaybackOutcome;
    use jive_core::track_events::TrackFailure;
    use std::path::Path;

    /// The exit statuses mpv is known to use.
    const CLEAN: i32 = 0;
    const GENERAL_FAILURE: i32 = 1;
    const NOTHING_TO_PLAY: i32 = 2;
    const NOTHING_PLAYED: i32 = 3;

    fn outcome_of(code: i32, diagnostics: &str) -> PlaybackOutcome {
        classify_exit(Some(code), diagnostics)
    }

    fn failure_of(code: i32, diagnostics: &str) -> Option<TrackFailure> {
        outcome_of(code, diagnostics).failure()
    }

    /// A backend whose program does not exist, for tests about what happens
    /// before a process is started.
    fn offline_backend() -> MpvBackend {
        MpvBackend::unverified("mpv-that-is-not-installed".into())
    }

    fn missing_track() -> &'static Path {
        Path::new("no-such-directory/no-such-song.flac")
    }

    /// A backend asked to play a file that does not exist, so that an outcome
    /// is waiting to be reported without any process having run.
    fn with_an_outcome_waiting() -> MpvBackend {
        let mut backend = offline_backend();
        backend
            .play(missing_track())
            .expect("a missing file is not a backend error");
        backend
    }

    fn polled(backend: &mut MpvBackend) -> Option<PlaybackOutcome> {
        backend.poll_event().expect("polling succeeds")
    }

    /// One test per `exit status, diagnostics => failure` row.
    macro_rules! classifications {
        ($($name:ident: $code:expr, $diagnostics:expr => $failure:expr;)+) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(failure_of($code, $diagnostics), $failure);
                }
            )+
        };
    }

    // One diagnostic message at a time, naming the failure by itself.
    classifications! {
        a_clean_exit_is_no_failure: CLEAN, "" => None;
        a_clean_exit_outweighs_anything_written: CLEAN, "Error decoding" => None;
        a_format_nobody_recognizes_is_reported:
            NOTHING_PLAYED, "Failed to recognize file format."
            => Some(TrackFailure::UnsupportedFormat);
        a_file_without_audio_is_reported:
            NOTHING_PLAYED, "No audio or video streams selected."
            => Some(TrackFailure::UnsupportedFormat);
        a_file_that_is_not_there_is_reported:
            NOTHING_TO_PLAY, "Failed to open song.flac."
            => Some(TrackFailure::FileNotFound);
        a_file_that_cannot_be_opened_is_reported:
            NOTHING_TO_PLAY, "No such file or directory"
            => Some(TrackFailure::FileNotFound);
        a_codec_that_will_not_open_is_reported:
            NOTHING_PLAYED, "Could not open codec."
            => Some(TrackFailure::DecoderError);
        a_decoder_that_will_not_start_is_reported:
            NOTHING_PLAYED, "Failed to initialize a decoder"
            => Some(TrackFailure::DecoderError);
        a_machine_without_sound_is_reported_as_the_backend:
            NOTHING_TO_PLAY, "Could not open/initialize audio device -> no sound."
            => Some(TrackFailure::BackendExited);
    }

    // Two messages at once: the more specific one must win, or every missing
    // file mentioned below a decoder warning would be blamed on the decoder.
    classifications! {
        a_missing_file_beats_a_decoder_message:
            NOTHING_TO_PLAY, "Failed to open song.flac. Error decoding"
            => Some(TrackFailure::FileNotFound);
        an_unreadable_format_beats_a_missing_file_message:
            NOTHING_PLAYED, "Failed to open x. Failed to recognize file format."
            => Some(TrackFailure::UnsupportedFormat);
    }

    // Nothing written at all, leaving the exit status to decide.
    classifications! {
        a_silent_status_of_two_means_a_missing_file:
            NOTHING_TO_PLAY, "" => Some(TrackFailure::FileNotFound);
        a_silent_status_of_three_means_a_decoder:
            NOTHING_PLAYED, "" => Some(TrackFailure::DecoderError);
        a_silent_failure_means_the_backend_gave_up:
            GENERAL_FAILURE, "" => Some(TrackFailure::BackendExited);
        an_unknown_status_means_the_backend_gave_up:
            42, "" => Some(TrackFailure::BackendExited);
    }

    #[test]
    fn a_process_that_was_killed_means_the_backend_gave_up() {
        assert_eq!(
            classify_exit(None, "").failure(),
            Some(TrackFailure::BackendExited)
        );
    }

    #[test]
    fn every_way_a_track_can_fail_is_a_failure_rather_than_a_finish() {
        for (code, diagnostics) in [
            (GENERAL_FAILURE, ""),
            (NOTHING_TO_PLAY, ""),
            (NOTHING_PLAYED, ""),
            (NOTHING_PLAYED, "Failed to recognize file format."),
        ] {
            assert!(!outcome_of(code, diagnostics).is_finished());
        }
    }

    #[test]
    fn a_missing_file_is_reported_without_starting_a_process() {
        assert_eq!(
            polled(&mut with_an_outcome_waiting()),
            Some(TrackFailure::FileNotFound.into())
        );
    }

    #[test]
    fn an_outcome_is_reported_once_and_then_forgotten() {
        let mut backend = with_an_outcome_waiting();
        assert!(polled(&mut backend).is_some());
        for _ in 0..3 {
            assert_eq!(polled(&mut backend), None);
        }
    }

    #[test]
    fn stopping_discards_the_outcome_of_the_track_that_was_stopped() {
        let mut backend = with_an_outcome_waiting();
        backend.stop().expect("stopping succeeds");
        assert_eq!(polled(&mut backend), None);
    }

    #[test]
    fn stopping_an_idle_backend_is_allowed() {
        let mut backend = offline_backend();
        for _ in 0..3 {
            backend.stop().expect("stopping an idle backend succeeds");
        }
        assert_eq!(polled(&mut backend), None);
    }

    #[test]
    fn playing_again_replaces_the_outcome_that_was_waiting() {
        let mut backend = with_an_outcome_waiting();
        backend.play(missing_track()).expect("play again");
        assert!(polled(&mut backend).is_some());
        assert_eq!(polled(&mut backend), None);
    }

    #[test]
    fn a_backend_with_no_program_is_reported_when_it_is_created() {
        let error = MpvBackend::with_program("mpv-that-is-not-installed")
            .expect_err("a missing program is an error");
        assert!(error.to_string().contains("mpv-that-is-not-installed"));
    }

    #[test]
    fn mpv_is_asked_for_by_name_and_kept_out_of_the_way() {
        assert_eq!(DEFAULT_PROGRAM, "mpv");
        for argument in ["--no-video", "--no-input-terminal", "--no-config"] {
            assert!(ARGUMENTS.contains(&argument), "{argument} should be passed");
        }
        assert_eq!(
            ARGUMENTS.last(),
            Some(&"--"),
            "a track named like a flag must still be playable"
        );
    }
}
