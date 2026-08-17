//! Where the app puts the worktrees it creates.
//!
//! One app-owned root, outside every repository — inside one they would show
//! up as untracked files in the user's own `git status`. Deliberately not
//! configurable. Accepted cost: not where a git-literate user would think to
//! look.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::xdg;

/// The directory every worktree the app creates lives under.
pub fn root() -> Result<PathBuf> {
    root_from(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"))
}

/// Make the root, and return where a spawn's worktree should go. The worktree
/// directory itself is left for `git worktree add`, which refuses a path that
/// already exists — a refusal worth keeping.
pub fn prepare(root: &Path, spawn_name: &str) -> Result<PathBuf> {
    fs::create_dir_all(root).map_err(|error| {
        Error::new(format!(
            "could not create the worktree root {}: {error}",
            root.display()
        ))
    })?;

    Ok(root.join(spawn_name))
}

/// What the root is holding, by name, sorted.
///
/// A missing or unreadable root is nothing found, not a problem — this is a
/// report, not a check. Directories only: a stray file under the root is not a
/// worktree.
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
    let base = xdg::under(data_home, home, ".local/share").ok_or_else(|| {
        Error::new("neither $XDG_DATA_HOME nor $HOME is set, so there is nowhere to put worktrees")
    })?;

    Ok(base.join("worktrees"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_lands_under_the_data_directory() {
        let root = root_from(None, Some("/home/someone".into())).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/home/someone/.local/share/harness-launcher/worktrees")
        );
    }

    #[test]
    fn with_nowhere_to_put_worktrees_the_app_refuses() {
        assert!(root_from(None, None).is_err());
    }

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
