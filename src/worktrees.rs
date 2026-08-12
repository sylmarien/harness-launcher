//! Where the app puts the worktrees it creates.
//!
//! One app-owned root, outside every repository. Inside a repository they would
//! show up as untracked files in the user's own `git status`, and the only fix
//! would be editing *their* `.gitignore` — writing into a project the app does
//! not own. A directory the app owns also makes leftovers findable.
//!
//! *Accepted cost:* the worktrees are not where a git-literate user would think
//! to look for them.
//!
//! The root is deliberately not configurable.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The directory every worktree the app creates lives under.
pub fn root() -> Result<PathBuf> {
    root_from(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"))
}

/// Make the root, and return where a spawn's worktree should go.
///
/// The directory itself is left for `git worktree add` to create: git refuses to
/// use a path that already exists, and that refusal is worth keeping.
///
/// The root is passed in rather than resolved here, because it is resolved once
/// — before the screen is taken over, where a machine with nowhere to put
/// worktrees can still be told so on a shell.
pub fn prepare(root: &Path, spawn_name: &str) -> Result<PathBuf> {
    fs::create_dir_all(root).map_err(|error| {
        Error::new(format!(
            "could not create the worktree root {}: {error}",
            root.display()
        ))
    })?;

    Ok(root.join(spawn_name))
}

/// What the root is holding, by name.
///
/// A worktree names itself — the same string as the spawn it belongs to and the
/// branch it is on — so this is enough for a report to say which piece of work
/// each leftover came from, rather than only how many there are.
///
/// **A root that is not there is nothing found, not a problem.** The app makes
/// it when it first needs one, so a machine that has never started a spawn has
/// nothing to report and no reason to hear about it.
///
/// **A root that cannot be read is also nothing found**, and that is the one
/// judgement call here. The alternative is refusing to start the app because a
/// directory listing failed, which trades a report nobody can act on for a
/// refusal that stops the work — and this is a report, not a check.
///
/// **Directories only, because a worktree is one.** The root is an ordinary
/// place on disk that anything may leave a file in — an editor's swap file, a
/// note, a tarball somebody dropped there — and a report that named one as a
/// worktree would send its reader looking for a checkout of work that was never
/// started. What cannot be read as a directory at all is skipped for the same
/// reason: this names leftovers, and a maybe is not a name.
///
/// Sorted, so the same leftovers read the same way twice: `read_dir` is in
/// whatever order the filesystem keeps.
pub fn under(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut found: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry.file_type().ok()?.is_dir().then_some(())?;

            Some(entry.file_name().to_str()?.to_string())
        })
        .collect();
    found.sort();

    found
}

/// Resolve the root from the environment, XDG first.
fn root_from(data_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    let base = match data_home {
        Some(data_home) if !data_home.is_empty() => PathBuf::from(data_home),
        _ => match home {
            Some(home) if !home.is_empty() => Path::new(&home).join(".local").join("share"),
            _ => {
                return Err(Error::new(
                    "neither $XDG_DATA_HOME nor $HOME is set, so there is nowhere to put worktrees",
                ));
            }
        },
    };

    Ok(base.join("harness-launcher").join("worktrees"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_follows_xdg_when_it_is_set() {
        let root = root_from(Some("/data".into()), Some("/home/someone".into())).unwrap();

        assert_eq!(root, PathBuf::from("/data/harness-launcher/worktrees"));
    }

    #[test]
    fn without_xdg_the_root_falls_back_to_the_home_directory() {
        let root = root_from(None, Some("/home/someone".into())).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/home/someone/.local/share/harness-launcher/worktrees")
        );
    }

    #[test]
    fn an_empty_xdg_variable_is_treated_as_unset() {
        let root = root_from(Some("".into()), Some("/home/someone".into())).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/home/someone/.local/share/harness-launcher/worktrees")
        );
    }

    #[test]
    fn with_nowhere_to_put_worktrees_the_app_refuses() {
        assert!(root_from(None, None).is_err());
    }

    /// The worktrees name themselves — the same string as the spawn and its
    /// branch — which is what lets a report say more than "there is stuff here".
    #[test]
    fn what_is_left_under_the_root_is_named() {
        let somewhere = tempfile::tempdir().unwrap();
        let root = somewhere.path().join("worktrees");
        fs::create_dir_all(root.join("fix-the-flake-b2c9")).unwrap();
        fs::create_dir_all(root.join("add-retry-logic-a7f3")).unwrap();

        assert_eq!(
            under(&root),
            ["add-retry-logic-a7f3", "fix-the-flake-b2c9"],
            "the leftovers are not named, in an order that reads the same twice"
        );
    }

    /// **A worktree is a directory**, and the root is somewhere anything may
    /// leave a file. Naming a stray one as a worktree sends the reader looking
    /// for a checkout of work nobody ever started — which is the report
    /// inventing exactly the confusion it exists to clear up.
    #[test]
    fn a_stray_file_under_the_root_is_not_read_as_a_worktree() {
        let somewhere = tempfile::tempdir().unwrap();
        let root = somewhere.path().join("worktrees");
        fs::create_dir_all(root.join("fix-the-flake-b2c9")).unwrap();
        fs::write(root.join("notes.txt"), "not a worktree").unwrap();

        assert_eq!(
            under(&root),
            ["fix-the-flake-b2c9"],
            "a file under the root was named as a worktree"
        );
    }

    /// The first run on a machine, where the root does not exist yet: nothing
    /// found, and nothing to say about it.
    #[test]
    fn a_root_that_was_never_made_is_holding_nothing() {
        let somewhere = tempfile::tempdir().unwrap();

        assert!(under(&somewhere.path().join("never-made")).is_empty());
    }

    #[test]
    fn preparing_makes_the_root_and_leaves_the_worktree_itself_to_git() {
        let somewhere = tempfile::tempdir().unwrap();
        let root = somewhere.path().join("harness-launcher").join("worktrees");

        let worktree = prepare(&root, "add-retry-logic-a7f3").unwrap();

        assert!(root.is_dir(), "the root was not made");
        assert_eq!(worktree, root.join("add-retry-logic-a7f3"));
        assert!(
            !worktree.exists(),
            "the worktree's own directory was made, which git would refuse to use"
        );
    }
}
