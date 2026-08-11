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
