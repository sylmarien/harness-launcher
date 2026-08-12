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
            "{path} has no commits yet, so there is nothing to branch from — make a commit \
             in it and try again"
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
            "{path} has no `origin` remote, so its default branch cannot be resolved — add \
             one with `git remote add origin <url>`"
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

/// What is in a worktree that is not committed — nothing at all when it is
/// clean.
///
/// **The flags are the whole of this function, and they are passed explicitly
/// on purpose.** git's own `worktree remove` runs this same status *without*
/// `-u`, so it honours the user's `status.showUntrackedFiles` — and with that
/// set to `no`, which is a real setting on large repositories, git's check goes
/// blind to untracked files and removes an agent's never-staged work.
/// `--ignore-submodules=none` is the same rule for `diff.ignoreSubmodules` and
/// `submodule.<name>.ignore`: a setting of the user's must not be able to
/// decide what the app is allowed to delete.
///
/// **Ignored files do not count**, which matches git and is the only usable
/// answer: count them and no worktree in a project that builds is ever
/// retirable. The cost is real — a spawn's untracked-but-ignored configuration
/// goes with the worktree — and is recorded rather than hidden. **Stashes do
/// not count either, and that is genuinely safe**: a stash made in a worktree
/// lives in the repository's stash list and survives the worktree going.
///
/// **Known blind spot:** a file marked `--assume-unchanged` never appears in
/// status at all, so it is invisible to any check built on one.
pub fn uncommitted(worktree: &Path) -> Result<String> {
    process::run_ok(
        "git",
        &[
            "-C",
            path_argument(worktree)?,
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )
}

/// Remove a worktree, and leave its branch alone.
///
/// The branch holds committed work: removing a checkout and deleting the
/// history checked out in it are different acts, and only the first is the
/// app's to take. It also makes the names worth reading months later, which is
/// what they were built for.
///
/// This is git's own removal, with git's own refusals, and it is asked *after*
/// the app's explicit check in [`uncommitted`] has already passed — so what is
/// left for it to object to is not work in the worktree. **`--force` is not
/// passed**, deliberately: the app's check is the stricter of the two, and
/// leaving git's own in place is a second pair of eyes on the seconds between
/// the check and the removal.
///
/// *The consequence, recorded rather than hidden:* git refuses outright to
/// remove a worktree containing submodules, so **a spawn on a repository with
/// submodules cannot be retired by the app at all** — its session stops, and the
/// refusal the user reads is git's own sentence about it. Removing it by hand is
/// then a `git worktree remove --force` they can take responsibility for.
pub fn remove_worktree(worktree: &Path) -> Result<()> {
    let path = path_argument(worktree)?;
    process::run_ok("git", &["-C", path, "worktree", "remove", path])?;

    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    /// Run a git command in a test repository, failing loudly if it does not work.
    ///
    /// Shared with every other module that needs a repository to work against,
    /// for the same reason the tmux tests share one server: a second way of
    /// building a test repository is a second set of defaults to keep in step.
    pub(crate) fn git(arguments: &[&str]) {
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
    ///
    /// The repository itself is `project`, inside the directory handed back.
    pub(crate) fn repository_with_origin() -> TempDir {
        let root = tempdir().unwrap();
        let origin = root.path().join("origin.git");
        let clone = clone_path(&root);

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

        let repository = open(&clone_path(&root)).unwrap();

        assert_eq!(repository.name(), "project");
    }

    #[test]
    fn opening_a_subdirectory_finds_the_repository_it_belongs_to() {
        let root = repository_with_origin();
        let nested = clone_path(&root).join("src").join("deep");
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
        let repository = open(&clone_path(&root)).unwrap();

        assert_eq!(default_branch(&repository).unwrap(), "origin/main");
    }

    /// Why every one of these says more than "no": a refusal nobody can act on
    /// stops the work twice — once now, and again when they try the same thing
    /// having had nothing to go on.
    fn refusal(repository: &Repository) -> String {
        default_branch(repository)
            .expect_err("a base was worked out where there was nothing to work it out from")
            .to_string()
    }

    #[test]
    fn a_repository_with_no_commits_is_refused_and_told_what_to_do() {
        let root = tempdir().unwrap();
        let empty = root.path().join("empty");
        git(&["init", "-b", "main", empty.to_str().unwrap()]);
        let repository = open(&empty).unwrap();

        let refused = refusal(&repository);

        assert!(refused.contains("no commits"), "{refused}");
        assert!(
            refused.contains("make a commit"),
            "nothing says how to make this repository spawnable: {refused}"
        );
    }

    #[test]
    fn a_repository_with_no_origin_is_refused_and_told_what_to_do() {
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

        let refused = refusal(&repository);

        assert!(refused.contains("`origin`"), "{refused}");
        assert!(
            refused.contains("git remote add origin"),
            "nothing says how to give it one: {refused}"
        );
    }

    #[test]
    fn a_detached_head_is_refused_and_told_what_to_do() {
        let root = repository_with_origin();
        let clone = clone_path(&root);
        git(&["-C", clone.to_str().unwrap(), "checkout", "--detach"]);
        let repository = open(&clone).unwrap();

        let refused = refusal(&repository);

        assert!(refused.contains("detached HEAD"), "{refused}");
        assert!(
            refused.contains("check out a branch"),
            "nothing says how to get out of it: {refused}"
        );
    }

    #[test]
    fn an_origin_without_a_recorded_head_is_refused_and_told_what_to_do() {
        let root = repository_with_origin();
        let clone = clone_path(&root);
        git(&[
            "-C",
            clone.to_str().unwrap(),
            "remote",
            "set-head",
            "origin",
            "--delete",
        ]);
        let repository = open(&clone).unwrap();

        let refused = refusal(&repository);

        assert!(
            refused.contains("git remote set-head origin --auto"),
            "nothing says how to record what origin's HEAD points at: {refused}"
        );
    }

    /// **The refusal the whole rule exists for.** A repository carrying both
    /// `main` and `master` and nothing saying which is the default is exactly
    /// where a guess would be wrong half the time — and a wrong base means an
    /// agent working from the wrong code for an hour before anybody notices.
    ///
    /// Recorded as its own test rather than left to the one above, because the
    /// two failures look identical from the outside and only this one is the
    /// reason no fallback was ever written.
    #[test]
    fn a_repository_with_both_main_and_master_is_refused_rather_than_guessed_at() {
        let root = repository_with_origin();
        let clone = clone_path(&root);
        let at = clone.to_str().unwrap();
        git(&["-C", at, "branch", "master"]);
        git(&["-C", at, "push", "origin", "master"]);
        git(&["-C", at, "remote", "set-head", "origin", "--delete"]);
        let repository = open(&clone).unwrap();

        let refused = refusal(&repository);

        for guess in ["origin/main", "origin/master"] {
            assert!(
                !refused.contains(guess),
                "the app picked a base out of a repository that never said which: {refused}"
            );
        }
    }

    /// The working tree inside a [`repository_with_origin`].
    fn clone_path(root: &TempDir) -> PathBuf {
        root.path().join("project")
    }

    #[test]
    fn a_worktree_is_created_on_a_branch_of_its_own() {
        let root = repository_with_origin();
        let repository = open(&clone_path(&root)).unwrap();
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
        let repository = open(&clone_path(&root)).unwrap();
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
        let repository = open(&clone_path(&root)).unwrap();
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
        let clone = clone_path(&root);
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

    // What is in a worktree, and what removing one does. Every one of these
    // runs against a throwaway repository rather than a fake: the whole point
    // of the rule is that it is git's own answer, asked the app's own way.

    /// The branch every worktree below is on.
    pub(crate) const BRANCH: &str = "spawn/add-retry-logic-a7f3";

    /// A repository with a spawn's worktree on it, ready to be dirtied.
    ///
    /// Shared with the retirement tests, which need exactly this and a session
    /// running in it. The worktree is made the way the app makes one rather
    /// than by hand, so a test can never be working against a checkout the app
    /// would not have produced.
    pub(crate) struct Spawned {
        /// The directory the repository and the worktree both live under, kept
        /// so that both go when the test does.
        root: TempDir,
        /// The worktree itself, which is what everything here is about.
        pub(crate) worktree: PathBuf,
    }

    impl Spawned {
        /// A repository, and a spawn's worktree on a branch of its own.
        pub(crate) fn new() -> Self {
            let root = repository_with_origin();
            let repository = open(&clone_path(&root)).unwrap();
            let worktree = root.path().join("worktrees").join("add-retry-logic-a7f3");
            add_worktree(&repository, &worktree, BRANCH, "origin/main").unwrap();

            Self { root, worktree }
        }

        /// The repository the worktree belongs to.
        pub(crate) fn repository(&self) -> PathBuf {
            clone_path(&self.root)
        }

        /// Put a file in the worktree, as an agent working in it would.
        pub(crate) fn wrote(&self, name: &str, what: &str) {
            fs::write(self.worktree.join(name), what).unwrap();
        }

        /// Commit what is in the worktree, so there is something tracked to
        /// change afterwards — and, for the retirement tests, so there is work
        /// the branch has to be left holding.
        pub(crate) fn committed(&self, what: &str) {
            let worktree = self.worktree.to_str().unwrap();
            git(&["-C", worktree, "add", "--all"]);
            git(&["-C", worktree, "commit", "-m", what]);
        }

        /// Set something in the repository's own configuration — which is to
        /// say, the user's setting, which the worktree inherits.
        pub(crate) fn user_set(&self, setting: &str, value: &str) {
            git(&[
                "-C",
                self.repository().to_str().unwrap(),
                "config",
                setting,
                value,
            ]);
        }

        /// The branches the repository has.
        pub(crate) fn branches(&self) -> String {
            process::run_ok(
                "git",
                &[
                    "-C",
                    self.repository().to_str().unwrap(),
                    "branch",
                    "--list",
                ],
            )
            .unwrap()
        }

        /// What is uncommitted, as the app asks it.
        fn uncommitted(&self) -> String {
            uncommitted(&self.worktree).unwrap()
        }

        /// What git would say about the worktree without the app's flags —
        /// which is what the user's configuration is free to decide.
        fn as_the_user_configured_it(&self) -> String {
            process::run_ok(
                "git",
                &[
                    "-C",
                    self.worktree.to_str().unwrap(),
                    "status",
                    "--porcelain",
                ],
            )
            .unwrap()
        }
    }

    #[test]
    fn a_worktree_nobody_has_touched_has_nothing_uncommitted_in_it() {
        let spawned = Spawned::new();

        assert_eq!(spawned.uncommitted(), "");
    }

    #[test]
    fn a_tracked_file_an_agent_changed_is_uncommitted_work() {
        let spawned = Spawned::new();
        spawned.wrote("notes.md", "as committed\n");
        spawned.committed("notes");

        spawned.wrote("notes.md", "and then changed\n");

        assert!(spawned.uncommitted().contains("notes.md"));
    }

    /// **The flagship, and the reason the flags are passed at all.** With
    /// `status.showUntrackedFiles` set to `no` — a real setting on large
    /// repositories — git's own check goes blind to a file an agent wrote and
    /// never staged, and `git worktree remove` deletes it without a word. The
    /// app's check is the same command with the flag put back.
    #[test]
    fn an_untracked_file_counts_even_where_the_users_own_git_would_not_see_it() {
        let spawned = Spawned::new();
        spawned.user_set("status.showUntrackedFiles", "no");
        spawned.wrote("notes.md", "an hour of an agent's work, never staged\n");

        assert_eq!(
            spawned.as_the_user_configured_it(),
            "",
            "the setting this test is about is not in force, so it proves nothing"
        );
        assert!(
            spawned.uncommitted().contains("notes.md"),
            "a file the user's own git cannot see was about to be deleted with the worktree"
        );
    }

    /// The same rule for the other flag: a setting of the user's must not be
    /// able to decide what the app is allowed to delete.
    #[test]
    fn a_changed_submodule_counts_even_where_the_users_own_git_ignores_submodules() {
        let spawned = Spawned::new();
        let elsewhere = tempdir().unwrap();
        let library = elsewhere.path().join("library");
        git(&["init", "-b", "main", library.to_str().unwrap()]);
        fs::write(library.join("shipped.txt"), "as shipped\n").unwrap();
        git(&["-C", library.to_str().unwrap(), "add", "--all"]);
        git(&["-C", library.to_str().unwrap(), "commit", "-m", "shipped"]);

        let worktree = spawned.worktree.to_str().unwrap();
        git(&[
            "-c",
            "protocol.file.allow=always",
            "-C",
            worktree,
            "submodule",
            "add",
            library.to_str().unwrap(),
            "vendor",
        ]);
        spawned.committed("vendor");
        spawned.user_set("diff.ignoreSubmodules", "all");
        spawned.wrote("vendor/shipped.txt", "and then changed\n");

        assert_eq!(
            spawned.as_the_user_configured_it(),
            "",
            "the setting this test is about is not in force, so it proves nothing"
        );
        assert!(
            spawned.uncommitted().contains("vendor"),
            "work inside a submodule was about to be deleted with the worktree"
        );
    }

    /// Recorded rather than hidden: ignored files do not count, and they go
    /// with the worktree. Counting them is the alternative, and it is unusable
    /// — no worktree in a project that builds would ever be retirable.
    #[test]
    fn an_ignored_file_does_not_count_and_goes_with_the_worktree() {
        let spawned = Spawned::new();
        spawned.wrote(".gitignore", "secrets\n");
        spawned.committed("ignore secrets");
        spawned.wrote("secrets", "the spawn's own configuration\n");

        assert_eq!(spawned.uncommitted(), "");

        remove_worktree(&spawned.worktree).unwrap();

        assert!(!spawned.worktree.exists());
    }

    #[test]
    fn removing_a_worktree_takes_the_checkout_and_leaves_the_branch() {
        let spawned = Spawned::new();
        let repository = spawned.repository();
        let repository = repository.to_str().unwrap();

        remove_worktree(&spawned.worktree).unwrap();

        assert!(!spawned.worktree.exists(), "the checkout is still there");
        assert!(
            !process::run_ok("git", &["-C", repository, "worktree", "list"])
                .unwrap()
                .contains("add-retry-logic-a7f3"),
            "the worktree's own metadata was left behind"
        );
        assert!(
            spawned.branches().contains(BRANCH),
            "the branch went with the worktree, taking committed work with it: {}",
            spawned.branches()
        );
    }

    #[test]
    fn a_worktree_that_is_not_there_is_a_refusal_rather_than_a_removal() {
        let root = tempdir().unwrap();

        assert!(remove_worktree(&root.path().join("nowhere")).is_err());
    }
}
