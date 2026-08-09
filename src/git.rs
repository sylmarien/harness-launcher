//! The git the app drives.
//!
//! Everything here shells out to `git` itself. That is a decision, not a
//! shortcut: libgit2 — which nearly every language binds — has no
//! worktree-remove at all, and its prune deletes the directory with no
//! cleanliness checking whatsoever. One tool, git's own semantics, and no second
//! definition of "clean" to keep in sync.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::process::{self, path_argument};

/// A repository a spawn can be started against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    path: PathBuf,
    name: String,
}

impl Repository {
    /// Where the repository's working tree is rooted.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the list calls it: the directory the working tree sits in.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Find the repository containing `path`, or refuse.
pub fn open(path: &Path) -> Result<Repository> {
    if !path.exists() {
        return Err(Error::new(format!("{} does not exist", path.display())));
    }

    let path = path_argument(path)?;
    let bare = process::run("git", &["-C", path, "rev-parse", "--is-bare-repository"])?;
    if !bare.ok {
        return Err(Error::new(format!("{path} is not a git repository")));
    }
    if bare.stdout == "true" {
        return Err(Error::new(format!(
            "{path} is a bare repository, which has no working tree to branch a spawn from"
        )));
    }

    let toplevel = process::run_ok("git", &["-C", path, "rev-parse", "--show-toplevel"])?;
    let toplevel = PathBuf::from(toplevel);
    let name = toplevel
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| {
            Error::new(format!(
                "cannot work out what to call the repository at {}",
                toplevel.display()
            ))
        })?;

    Ok(Repository {
        path: toplevel,
        name,
    })
}

/// The branch a spawn starts from: the repository's default, read locally.
///
/// **No fetch.** Spawning stays off the network, and if the local default is
/// stale that is the state of the repository — exactly as it would be had you
/// branched by hand.
///
/// Every way of not knowing is a refusal rather than a guess. Choosing between
/// `main` and `master` picks wrong in a repository that has both, and a wrong
/// base means an agent works from the wrong code for an hour.
pub fn default_branch(repository: &Repository) -> Result<String> {
    let path = path_argument(repository.path())?;

    if !process::run(
        "git",
        &["-C", path, "rev-parse", "--verify", "--quiet", "HEAD"],
    )?
    .ok
    {
        return Err(Error::new(format!(
            "{path} has no commits yet, so there is nothing to branch from"
        )));
    }

    if !process::run("git", &["-C", path, "symbolic-ref", "--quiet", "HEAD"])?.ok {
        return Err(Error::new(format!(
            "{path} has a detached HEAD — check out a branch before starting a spawn"
        )));
    }

    let remotes = process::run_ok("git", &["-C", path, "remote"])?;
    if !remotes.lines().any(|remote| remote == "origin") {
        return Err(Error::new(format!(
            "{path} has no `origin` remote, so its default branch cannot be resolved"
        )));
    }

    let head = process::run(
        "git",
        &[
            "-C",
            path,
            "symbolic-ref",
            "--quiet",
            "refs/remotes/origin/HEAD",
        ],
    )?;
    if !head.ok {
        return Err(Error::new(format!(
            "{path} does not record which branch origin's HEAD points at — run \
             `git remote set-head origin --auto` in it and try again"
        )));
    }

    head.stdout
        .strip_prefix("refs/remotes/")
        .map(str::to_string)
        .ok_or_else(|| {
            Error::new(format!(
                "origin's HEAD in {path} resolved to {}, which is not a remote branch",
                head.stdout
            ))
        })
}

/// Create a worktree on a branch of its own.
///
/// **Always `-b`, never the bare form.** This is a safety rule rather than a
/// style one: without `-b`, git silently checks out a *pre-existing* branch of
/// that name instead of creating one, which would drop a fresh agent onto
/// somebody else's in-progress work.
pub fn add_worktree(
    repository: &Repository,
    worktree: &Path,
    branch: &str,
    start_point: &str,
) -> Result<()> {
    process::run_ok(
        "git",
        &[
            "-C",
            path_argument(repository.path())?,
            "worktree",
            "add",
            "-b",
            branch,
            path_argument(worktree)?,
            start_point,
        ],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    /// Run a git command in a test repository, failing loudly if it does not work.
    fn git(arguments: &[&str]) {
        let mut full = vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ];
        full.extend_from_slice(arguments);

        let outcome = process::run("git", &full).unwrap();
        assert!(outcome.ok, "git {arguments:?} failed: {}", outcome.stderr);
    }

    /// A repository with one commit, an `origin`, and a recorded default branch.
    fn repository_with_origin() -> TempDir {
        let root = tempdir().unwrap();
        let origin = root.path().join("origin.git");
        let clone = root.path().join("project");

        git(&["init", "--bare", "-b", "main", origin.to_str().unwrap()]);
        git(&["init", "-b", "main", clone.to_str().unwrap()]);

        let clone = clone.to_str().unwrap();
        git(&["-C", clone, "commit", "--allow-empty", "-m", "first"]);
        git(&[
            "-C",
            clone,
            "remote",
            "add",
            "origin",
            origin.to_str().unwrap(),
        ]);
        git(&["-C", clone, "push", "-u", "origin", "main"]);
        git(&["-C", clone, "remote", "set-head", "origin", "--auto"]);

        root
    }

    #[test]
    fn a_working_tree_opens() {
        let root = repository_with_origin();

        let repository = open(&root.path().join("project")).unwrap();

        assert_eq!(repository.name(), "project");
    }

    #[test]
    fn opening_a_subdirectory_finds_the_repository_it_belongs_to() {
        let root = repository_with_origin();
        let nested = root.path().join("project").join("src").join("deep");
        fs::create_dir_all(&nested).unwrap();

        let repository = open(&nested).unwrap();

        assert_eq!(repository.name(), "project");
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_refused() {
        let root = tempdir().unwrap();

        assert!(open(root.path()).is_err());
    }

    #[test]
    fn a_path_that_does_not_exist_is_refused() {
        let root = tempdir().unwrap();

        assert!(open(&root.path().join("nowhere")).is_err());
    }

    #[test]
    fn a_bare_repository_is_refused() {
        let root = repository_with_origin();

        assert!(open(&root.path().join("origin.git")).is_err());
    }

    #[test]
    fn the_default_branch_comes_from_origins_head() {
        let root = repository_with_origin();
        let repository = open(&root.path().join("project")).unwrap();

        assert_eq!(default_branch(&repository).unwrap(), "origin/main");
    }

    #[test]
    fn a_repository_with_no_commits_is_refused() {
        let root = tempdir().unwrap();
        let empty = root.path().join("empty");
        git(&["init", "-b", "main", empty.to_str().unwrap()]);
        let repository = open(&empty).unwrap();

        assert!(default_branch(&repository).is_err());
    }

    #[test]
    fn a_repository_with_no_origin_is_refused() {
        let root = tempdir().unwrap();
        let lonely = root.path().join("lonely");
        git(&["init", "-b", "main", lonely.to_str().unwrap()]);
        git(&[
            "-C",
            lonely.to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-m",
            "first",
        ]);
        let repository = open(&lonely).unwrap();

        assert!(default_branch(&repository).is_err());
    }

    #[test]
    fn a_detached_head_is_refused() {
        let root = repository_with_origin();
        let clone = root.path().join("project");
        git(&["-C", clone.to_str().unwrap(), "checkout", "--detach"]);
        let repository = open(&clone).unwrap();

        assert!(default_branch(&repository).is_err());
    }

    #[test]
    fn an_origin_without_a_recorded_head_is_refused() {
        let root = repository_with_origin();
        let clone = root.path().join("project");
        git(&[
            "-C",
            clone.to_str().unwrap(),
            "remote",
            "set-head",
            "origin",
            "--delete",
        ]);
        let repository = open(&clone).unwrap();

        assert!(default_branch(&repository).is_err());
    }

    #[test]
    fn a_worktree_is_created_on_a_branch_of_its_own() {
        let root = repository_with_origin();
        let repository = open(&root.path().join("project")).unwrap();
        let worktree = root.path().join("worktrees").join("add-retry-logic-a7f3");

        add_worktree(
            &repository,
            &worktree,
            "spawn/add-retry-logic-a7f3",
            "origin/main",
        )
        .unwrap();

        assert!(worktree.join(".git").exists());
        let branch = process::run_ok(
            "git",
            &[
                "-C",
                worktree.to_str().unwrap(),
                "rev-parse",
                "--abbrev-ref",
                "HEAD",
            ],
        )
        .unwrap();
        assert_eq!(branch, "spawn/add-retry-logic-a7f3");
    }

    #[test]
    fn a_worktree_is_never_dropped_onto_an_existing_branch() {
        let root = repository_with_origin();
        let clone = root.path().join("project");
        git(&["-C", clone.to_str().unwrap(), "branch", "spawn/taken"]);
        let repository = open(&clone).unwrap();

        let refused = add_worktree(
            &repository,
            &root.path().join("worktrees").join("taken"),
            "spawn/taken",
            "origin/main",
        );

        assert!(refused.is_err());
    }
}
