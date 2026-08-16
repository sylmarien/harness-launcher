//! The git the app drives.
//!
//! Everything shells out to `git` itself — no library: libgit2 has no
//! worktree-remove, and its prune deletes with no cleanliness check.
//! See docs/developers/components/worktrees-and-branches.md.

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

/// The branch a spawn starts from: the repository's default, read locally
/// from origin/HEAD — no fetch, spawning stays off the network.
///
/// Every way of not knowing refuses rather than guesses: choosing between
/// `main` and `master` picks wrong in a repository that has both.
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

/// The repository a worktree belongs to.
///
/// The path must be a worktree root of its own. `rev-parse --show-toplevel`
/// hands back the repository root for any other directory inside a
/// repository, and a row filed under that repository sits under the wrong
/// heading.
///
/// The repository itself is the main worktree, the first entry `worktree
/// list` prints. A linked worktree's toplevel names the worktree, not the
/// repository. A submodule's first entry is its git directory, which [`open`]
/// resolves back to the submodule's working tree.
pub fn worktree_repository(worktree: &Path) -> Result<Repository> {
    let at = path_argument(worktree)?;
    let toplevel = process::run("git", &["-C", at, "rev-parse", "--show-toplevel"])?;
    if !toplevel.ok || !same_directory(Path::new(&toplevel.stdout), worktree) {
        return Err(Error::new(format!(
            "{at} is not the root of a worktree of its own, so git names no repository for it"
        )));
    }

    let listed = process::run_ok("git", &["-C", at, "worktree", "list", "--porcelain"])?;

    let main = listed
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("worktree "))
        .ok_or_else(|| {
            Error::new(format!(
                "git listed no worktree around {}",
                worktree.display()
            ))
        })?;

    open(Path::new(main))
}

/// Whether two paths name the same directory. Both sides are resolved first:
/// a worktree root reached through a symlink is still that root.
fn same_directory(one: &Path, other: &Path) -> bool {
    match (one.canonicalize(), other.canonicalize()) {
        (Ok(one), Ok(other)) => one == other,
        _ => false,
    }
}

/// Create a worktree on a branch of its own.
///
/// Always `-b`, never the bare form: bare silently checks out a pre-existing
/// branch of that name. Always `--no-track`, for two reasons: tracking would
/// have `git status` report the work as up to date with `origin/main`, and
/// branching from a remote-tracking ref writes upstream config into
/// `.git/config` — the one file two concurrent creations contend for.
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

/// What is in a worktree that is not committed — empty when clean.
///
/// The flags are explicit on purpose: git's own `worktree remove` honours
/// `status.showUntrackedFiles=no` and goes blind to untracked work;
/// `--ignore-submodules=none` overrides the equivalent submodule settings.
/// Accepted cost: ignored files do not count and go with the worktree.
/// Stashes survive removal; `--assume-unchanged` files are invisible here.
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

/// Remove a worktree, leaving its branch — it holds committed work.
///
/// `--force` is deliberately not passed: git's own check is a second look at
/// the seconds between [`uncommitted`] passing and the removal. Accepted
/// cost: git refuses to remove a worktree containing submodules, so such a
/// spawn must be removed by hand with `git worktree remove --force`.
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

    /// Run a git command in a test repository, failing loudly if it does not
    /// work. Shared with every other module that needs a test repository.
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

    /// Every refusal must say what to do about it, not just "no".
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

    /// A repository carrying both `main` and `master` with nothing saying
    /// which is the default: refusing rather than guessing is the reason no
    /// fallback was ever written.
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

    #[test]
    fn a_worktree_says_which_repository_it_belongs_to() {
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

        assert_eq!(worktree_repository(&worktree).unwrap(), repository);
    }

    /// A directory under the worktree root that is no worktree of its own
    /// still sits inside a repository, and `worktree list` names that
    /// repository. The list groups rows by repository, so filing the spawn
    /// there puts its row under the wrong heading.
    #[test]
    fn a_directory_inside_a_repository_that_is_no_worktree_of_its_own_is_refused() {
        let root = repository_with_origin();
        let inside = clone_path(&root)
            .join("worktrees")
            .join("add-retry-logic-a7f3");
        fs::create_dir_all(&inside).unwrap();

        let named = worktree_repository(&inside);

        let refused = match named {
            Err(refused) => refused.to_string(),
            Ok(repository) => panic!(
                "a plain directory was filed under {:?}, the repository around it",
                repository.name()
            ),
        };
        assert!(
            refused.contains("worktree"),
            "the refusal does not say what was checked: {refused}"
        );
    }

    /// A submodule keeps its git directory in its superproject, and that
    /// directory is the first entry `worktree list` prints. It resolves back
    /// to the submodule's own working tree, so the repository is named and
    /// nothing is refused.
    #[test]
    fn a_submodule_is_named_by_its_own_working_tree() {
        let root = tempdir().unwrap();
        let library = root.path().join("lib");
        let library = library.to_str().unwrap();
        git(&["init", "-b", "main", library]);
        git(&["-C", library, "commit", "--allow-empty", "-m", "first"]);
        let superproject = root.path().join("super");
        git(&["init", "-b", "main", superproject.to_str().unwrap()]);
        git(&[
            "-c",
            "protocol.file.allow=always",
            "-C",
            superproject.to_str().unwrap(),
            "submodule",
            "add",
            library,
            "vendor/lib",
        ]);

        let named = worktree_repository(&superproject.join("vendor").join("lib")).unwrap();

        assert_eq!(named.name(), "lib");
    }

    /// `--separate-git-dir` puts a repository's git directory outside its
    /// working tree, and that directory is the first entry `worktree list`
    /// prints. It has no working tree of its own, so no repository can be
    /// named and the spawn is refused.
    #[test]
    fn a_worktree_whose_git_directory_sits_elsewhere_is_refused_rather_than_filed_under_a_neighbour()
     {
        let root = tempdir().unwrap();
        let elsewhere = root.path().join("elsewhere");
        git(&["init", "-b", "main", elsewhere.to_str().unwrap()]);
        let project = root.path().join("project");
        git(&[
            "init",
            "-b",
            "main",
            "--separate-git-dir",
            elsewhere.join("project.git").to_str().unwrap(),
            project.to_str().unwrap(),
        ]);
        let project = project.to_str().unwrap();
        git(&["-C", project, "commit", "--allow-empty", "-m", "first"]);
        let worktree = root.path().join("worktrees").join("add-retry-logic-a7f3");
        git(&[
            "-C",
            project,
            "worktree",
            "add",
            "--no-track",
            "-b",
            "spawn/add-retry-logic-a7f3",
            worktree.to_str().unwrap(),
            "main",
        ]);

        let named = worktree_repository(&worktree);

        assert!(
            named.is_err(),
            "the spawn was filed under {:?}, which is the repository around its git \
             directory rather than the one it works in",
            named.map(|repository| repository.name().to_string())
        );
    }

    #[test]
    fn a_directory_that_is_no_worktree_belongs_to_no_repository() {
        let nowhere = tempdir().unwrap();

        assert!(worktree_repository(nowhere.path()).is_err());
    }

    /// The guard: anything added to creation that writes the repository's
    /// config puts back the lock race two concurrent creations lose — and the
    /// `--no-track` test below would stay green while it did.
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

    // What is in a worktree, and what removing one does — run against a real
    // throwaway repository, because the rule is git's own answer.

    /// The branch every worktree below is on.
    pub(crate) const BRANCH: &str = "spawn/add-retry-logic-a7f3";

    /// A repository with a spawn's worktree on it, made the way the app makes
    /// one. Shared with the retirement tests.
    pub(crate) struct Spawned {
        /// Holds the repository and the worktree for the test's lifetime.
        root: TempDir,
        /// The worktree itself.
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

        /// Commit what is in the worktree.
        pub(crate) fn committed(&self, what: &str) {
            let worktree = self.worktree.to_str().unwrap();
            git(&["-C", worktree, "add", "--all"]);
            git(&["-C", worktree, "commit", "-m", what]);
        }

        /// Set a user-level setting the worktree inherits.
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

        /// What git says about the worktree without the app's flags.
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

    /// The reason the flags are passed at all: with `status.showUntrackedFiles`
    /// set to `no`, git's own check misses this file and `worktree remove`
    /// would delete it.
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

    /// The same rule for `--ignore-submodules=none`.
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

    /// Accepted cost: ignored files do not count, and go with the worktree.
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
