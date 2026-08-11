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
///
/// **And always `--no-track`**, which is two rules in one flag. A spawn's branch
/// starts *from* the default branch and is not a second copy of it: tracking
/// would have `git status` in the worktree report the work as being up to date
/// with `origin/main`, and `git pull` there merge main into it.
///
/// The other half is why it is here rather than a preference. Branching from a
/// remote-tracking ref makes git write the upstream configuration into the
/// repository's own `.git/config`, and **two spawns started at once against one
/// repository then race for that file's lock** — one of them losing with
/// `could not lock config file`, which is a refusal nobody can act on. The
/// design declines a creation lock on the grounds that names carry a random
/// suffix so nothing is ever contended; this is the one thing two creations
/// really did contend for, and not writing it is what makes that true rather
/// than nearly true. Found by the test below, which is the only reason it is
/// known at all: it is a race, so it fails about one run in five.
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
            "--no-track",
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

    /// **What two spawns created at once really contend for**, stated as the
    /// thing rather than as a consequence of it: creating a worktree does not
    /// write the repository's own config, so there is no lock to lose.
    ///
    /// The branch not tracking anything is what makes that true today, and has
    /// a test of its own below because it is worth keeping for its own sake.
    /// This one is the guard: anything added to creation that writes config —
    /// a `git config` call, a flag that sets one — puts the race back, and
    /// would leave that test green while doing it.
    #[test]
    fn creating_a_worktree_does_not_write_the_repositorys_config() {
        let root = repository_with_origin();
        let repository = open(&root.path().join("project")).unwrap();
        let config = repository.path().join(".git").join("config");
        let before = fs::read(&config).unwrap();

        add_worktree(
            &repository,
            &root.path().join("worktrees").join("add-retry-logic-a7f3"),
            "spawn/add-retry-logic-a7f3",
            "origin/main",
        )
        .unwrap();

        assert_eq!(
            fs::read(&config).unwrap(),
            before,
            "the repository's config was written, so two spawns created at once would \
             contend for its lock and one of them would refuse"
        );
    }

    /// A spawn's branch starts from the default branch without becoming a
    /// second copy of it: tracking would have `git status` in the worktree
    /// report the work as up to date with somebody else's branch, and `git
    /// pull` there merge that branch into it.
    #[test]
    fn a_spawns_branch_does_not_track_the_branch_it_started_from() {
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

        let upstream = process::run(
            "git",
            &[
                "-C",
                worktree.to_str().unwrap(),
                "rev-parse",
                "--abbrev-ref",
                "spawn/add-retry-logic-a7f3@{upstream}",
            ],
        )
        .unwrap();
        assert!(
            !upstream.ok,
            "the spawn's branch tracks {}, so its worktree reports the work as up to \
             date with somebody else's branch",
            upstream.stdout
        );
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
