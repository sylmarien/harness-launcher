//! Running one external command and reading what it said.
//!
//! Arguments are passed as a vector, so nothing is ever quoted for a shell and
//! no shell is involved.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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

/// Run a command and return what it said, whether or not it succeeded; only
/// failing to start is an error here.
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

/// Run a command that is expected to succeed, and return its standard output;
/// a non-zero exit becomes an error carrying the command's own complaint.
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

/// A path as a command-line argument; a non-UTF-8 path is refused rather than
/// mangled into a different file name.
pub fn path_argument(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        Error::new(format!(
            "the path {} is not valid UTF-8, so it cannot be passed to git or tmux",
            path.display()
        ))
    })
}

/// Whether `program` is an executable file on `path`, looked for the way a
/// shell would. The `PATH` is passed in rather than read here, so the rule can
/// be tested independently of the machine.
pub fn runnable_on(path: Option<OsString>, program: &str) -> bool {
    let Some(path) = path else {
        return false;
    };

    env::split_paths(&path).any(|directory| executable(&directory.join(program)))
}

/// Whether this is a file something can execute; symlinks are followed, since
/// version managers install programs as symlinks.
fn executable(candidate: &Path) -> bool {
    fs::metadata(candidate)
        .is_ok_and(|about| about.is_file() && about.permissions().mode() & 0o111 != 0)
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::{TempDir, tempdir};

    /// A directory with one program in it, runnable or not.
    pub(crate) fn holding(program: &str, runnable: bool) -> TempDir {
        let directory = tempdir().unwrap();
        let file = directory.path().join(program);
        fs::write(&file, "#!/bin/sh\n").unwrap();
        fs::set_permissions(
            &file,
            fs::Permissions::from_mode(if runnable { 0o755 } else { 0o644 }),
        )
        .unwrap();

        directory
    }

    /// A `PATH` made of these directories, as the environment would carry it.
    pub(crate) fn path(directories: &[&TempDir]) -> OsString {
        env::join_paths(directories.iter().map(|directory| directory.path())).unwrap()
    }

    #[test]
    fn a_program_on_the_path_is_found_on_it() {
        let bin = holding("some-harness", true);

        assert!(runnable_on(Some(path(&[&bin])), "some-harness"));
    }

    #[test]
    fn a_program_no_directory_on_the_path_holds_is_not_found() {
        let bin = holding("something-else", true);

        assert!(!runnable_on(Some(path(&[&bin])), "some-harness"));
    }

    #[test]
    fn every_directory_on_the_path_is_looked_in() {
        let first = holding("something-else", true);
        let second = holding("some-harness", true);

        assert!(runnable_on(Some(path(&[&first, &second])), "some-harness"));
    }

    #[test]
    fn a_file_of_the_right_name_that_cannot_be_run_is_not_the_program() {
        let bin = holding("some-harness", false);

        assert!(!runnable_on(Some(path(&[&bin])), "some-harness"));
    }

    #[test]
    fn with_no_path_at_all_nothing_is_runnable() {
        assert!(!runnable_on(None, "some-harness"));
    }
}
