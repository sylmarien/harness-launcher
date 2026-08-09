//! Running one external command and reading what it said.
//!
//! `git` and `tmux` are both driven this way: one process per call, arguments
//! passed as a vector so nothing is ever quoted for a shell, and no shell in the
//! path at all.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// What a command left behind.
pub struct Outcome {
    /// Whether it exited zero.
    pub ok: bool,
    /// Its standard output, trimmed of trailing whitespace.
    pub stdout: String,
    /// Its standard error, trimmed of trailing whitespace.
    pub stderr: String,
}

/// Run a command and return what it said, whether or not it succeeded.
///
/// Only the command failing to *start* is a refusal here — a non-zero exit is an
/// answer, and several callers ask questions where "no" is the useful reply.
pub fn run<A: AsRef<str>>(program: &str, args: &[A]) -> Result<Outcome> {
    let output = Command::new(program)
        .args(args.iter().map(AsRef::as_ref))
        .output()
        .map_err(|error| could_not_start(program, &error))?;

    Ok(Outcome {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
        stderr: String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_string(),
    })
}

/// Run a command that is expected to succeed, and return its standard output.
///
/// A non-zero exit becomes a refusal carrying the command's own complaint, which
/// is nearly always more specific than anything this app could say instead.
pub fn run_ok<A: AsRef<str>>(program: &str, args: &[A]) -> Result<String> {
    let outcome = run(program, args)?;
    if outcome.ok {
        return Ok(outcome.stdout);
    }

    let complaint = if outcome.stderr.is_empty() {
        outcome.stdout
    } else {
        outcome.stderr
    };
    Err(Error::new(format!(
        "`{}` failed: {complaint}",
        as_written(program, args)
    )))
}

/// A path on its way to becoming a command-line argument.
///
/// Paths reach `git` and `tmux` as text, so one that is not valid UTF-8 is
/// refused rather than mangled into something that would name a different file.
pub fn path_argument(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        Error::new(format!(
            "the path {} is not valid UTF-8, so it cannot be passed to git or tmux",
            path.display()
        ))
    })
}

/// A command that would not start at all.
fn could_not_start(program: &str, error: &std::io::Error) -> Error {
    Error::new(format!(
        "could not run `{program}`: {error} — is it installed and on PATH?"
    ))
}

/// A command as it would have been typed, for putting in a refusal.
fn as_written<A: AsRef<str>>(program: &str, args: &[A]) -> String {
    let mut written = program.to_string();
    for argument in args {
        written.push(' ');
        written.push_str(argument.as_ref());
    }

    written
}
