//! Making a spawn: everything between a description of work and a session
//! running in a worktree of its own.
//!
//! **Two roads lead here and they meet at [`Wanted`]** — a repository, what the
//! work is, and the ids of whatever the harness offered. The command line
//! produces one before the screen exists; a draft produces one when somebody
//! starts it. Everything after that is the same for both, which is the point of
//! the module: a spawn made from a form and a spawn made from an argument list
//! are the same spawn, worked out by the same code.
//!
//! **It comes in two halves, and where the line falls is not arbitrary.**
//! Resolving a repository and making a worktree take seconds and are where
//! nearly every refusal lives, so from a draft they run on a thread of their
//! own ([`making`]). Opening a window and starting the harness in it is tmux and
//! the control client, which belong to the one thread that holds them
//! ([`start`]) — and it is fast, because it is two commands to a server that is
//! already there.
//!
//! **Intent is written before action.** Every line a creation says is sent
//! before the step it describes is attempted, never after it succeeded. That is
//! the difference between a draft that dies half way leaving a record of the
//! worktree it made, and one leaving a mystery.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use crate::app::Spawn;
use crate::control::Client;
use crate::draft;
use crate::error::Result;
use crate::harness::{self, LaunchRecipe};
use crate::list::Entry;
use crate::screen::Size;
use crate::snapshot::Watched;
use crate::{git, names, process, tmux, worktrees};

/// What somebody asked for, before anything has been worked out about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The repository to work on — or any directory inside it.
    pub repository: PathBuf,
    /// The work to be done, in the user's own words.
    pub work: String,
    /// The ids of the options picked out of whatever the harness offered.
    ///
    /// Anonymous, in both directions: whoever collected them was told titles
    /// and labels and nothing else, and the harness recognises its own when it
    /// is handed them back. Nothing in between has to know which list is which.
    pub answers: Vec<String>,
}

/// One spawn, worked out but not yet made.
pub struct Plan {
    /// The repository it was started against.
    repository: git::Repository,
    /// The commit its branch is cut from.
    start_point: String,
    /// Where its worktree is to go.
    worktree: PathBuf,
    /// How to start the harness in it.
    recipe: LaunchRecipe,
    /// What the list will say about it.
    pub entry: Entry,
}

impl Plan {
    /// Make the worktree it calls for, on a branch of its own.
    pub fn create(&self) -> Result<()> {
        git::add_worktree(
            &self.repository,
            &self.worktree,
            &self.entry.branch,
            &self.start_point,
        )
    }
}

/// Work out everything about a spawn that can be settled without creating it.
///
/// Nothing here touches the repository being worked on. The one thing it does
/// leave behind is the app's own worktree root, which is a directory the app
/// owns and shares between every spawn there will ever be — not the worktree,
/// which is the next step and is announced before it is taken.
pub fn plan(wanted: &Wanted, root: &Path) -> Result<Plan> {
    let repository = git::open(&wanted.repository)?;
    let start_point = git::default_branch(&repository)?;

    let name = names::spawn_name(&wanted.work, names::fresh_seed());
    let branch = names::branch_name(&name);
    let worktree = worktrees::prepare(root, &name)?;

    let recipe = harness::launch_recipe(&harness::spec_from(
        name.clone(),
        wanted.work.clone(),
        worktree.clone(),
        &wanted.answers,
    ));
    let entry = Entry {
        repository: repository.name().to_string(),
        spawn: name,
        branch,
        worktree: process::path_argument(&worktree)?.to_string(),
    };

    Ok(Plan {
        repository,
        start_point,
        worktree,
        recipe,
        entry,
    })
}

/// A spawn that is running: what the app shows, and what the supervisor
/// watches.
///
/// Two views of one thing, handed over together because they are made together
/// and the pane in each of them has to be the same pane.
pub struct Started {
    /// The spawn, as the screen needs it.
    pub spawn: Spawn,
    /// The spawn, as the supervisor needs it.
    pub watched: Watched,
}

/// Open a window, put a grid behind it, and start the harness in it.
///
/// **The grid goes behind the pane before the harness is started.** A
/// control-mode client streams only what is produced while it is attached, so a
/// session that drew itself before anything was listening would leave a slot
/// that stays blank for ever — and nothing about it would look like an error.
///
/// The size is the slot's as it is now rather than as it was at start-up: a
/// spawn created into a window somebody has since resized would otherwise draw
/// at a shape nothing on screen has.
pub fn start(
    server: &tmux::Server,
    session: &str,
    client: &Client,
    slot: Size,
    plan: Plan,
) -> Result<Started> {
    let pane = server.open_window(session, &plan.entry.spawn)?;
    let grid = client.watch(&pane, slot);
    server.start(&pane, &plan.recipe)?;

    Ok(Started {
        watched: Watched::new(plan.entry.spawn.clone(), pane.clone()),
        spawn: Spawn {
            entry: plan.entry,
            pane,
            grid,
        },
    })
}

/// What a creation in flight has to say, and which draft it is about.
pub struct Report {
    /// The draft that was started, which is where this is shown.
    pub draft: draft::Id,
    /// What it has to say.
    pub said: Said,
}

/// One thing a creation has to say.
pub enum Said {
    /// What it is about to do, said before it does it.
    Doing(String),
    /// The worktree is there, and here is everything about the spawn that is to
    /// run in it.
    ///
    /// Boxed because a plan is far larger than a sentence, and every other
    /// thing said here is a sentence.
    Made(Box<Plan>),
    /// It stopped, and this is why. The draft is written again from here, with
    /// its text exactly as it was.
    Refused(String),
}

/// Make everything a spawn needs on disk, on a thread of its own.
///
/// **On a thread because `git worktree add` takes seconds**, and the app draws
/// sixty frames of a live session in that time. What comes back comes back as
/// reports, so a creation is something the user watches rather than something
/// the app disappears into.
///
/// A report nobody is listening for is the app having gone. The work carries on
/// regardless — there is nothing to undo, and a half-made worktree is exactly
/// what the app-owned root is for.
pub fn making(draft: draft::Id, wanted: Wanted, root: PathBuf, reporting: Sender<Report>) {
    thread::spawn(move || {
        let said = match made(&wanted, &root, &|doing| {
            tell(&reporting, draft, Said::Doing(doing));
        }) {
            Ok(plan) => Said::Made(Box::new(plan)),
            Err(refused) => Said::Refused(refused.to_string()),
        };

        tell(&reporting, draft, said);
    });
}

/// The half of a creation that is git, narrating itself as it goes.
///
/// Each line is said before the step it describes is attempted. The worktree
/// one is the load-bearing one: it names a path and a branch that do not exist
/// yet, so that if nothing is ever heard from this thread again, what is on
/// disk has already been written down.
fn made(wanted: &Wanted, root: &Path, say: &dyn Fn(String)) -> Result<Plan> {
    say(format!(
        "reading {} and resolving the branch to start from",
        wanted.repository.display()
    ));
    let plan = plan(wanted, root)?;

    say(format!(
        "creating the worktree {} on {}",
        plan.entry.worktree, plan.entry.branch
    ));
    plan.create()?;

    Ok(plan)
}

/// Say one thing about a creation, if anybody is still listening.
fn tell(reporting: &Sender<Report>, draft: draft::Id, said: Said) {
    let _ = reporting.send(Report { draft, said });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tempfile::{TempDir, tempdir};

    use crate::control::Grid;
    use crate::screen::tests::shown;
    use crate::tmux::tests::PrivateTmux;

    /// Run a git command in a test repository, failing loudly if it does not
    /// work.
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

    /// A repository with one commit, an `origin`, and a recorded default
    /// branch — the least a spawn can be started against.
    fn repository() -> TempDir {
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

    /// Somewhere for the app to put the worktrees this test makes, thrown away
    /// with the test rather than left in whoever is running it's home.
    fn root() -> TempDir {
        tempdir().unwrap()
    }

    /// A draft's identity, which is all a creation knows about the draft it is
    /// for. Taken from `Drafts` rather than made up, so nothing here needs a
    /// way of building one that the app does not have.
    fn drafts(how_many: usize) -> Vec<draft::Id> {
        let mut drafts = draft::Drafts::new(Vec::new());

        (0..how_many).map(|_| drafts.start()).collect()
    }

    /// What somebody asked for, against this repository.
    fn wanted(repository: &TempDir, work: &str) -> Wanted {
        Wanted {
            repository: repository.path().join("project"),
            work: work.to_string(),
            answers: Vec::new(),
        }
    }

    /// Everything one creation said, in the order it said it, run to the end.
    fn reported(wanted: Wanted, root: &TempDir) -> Vec<String> {
        let (reporting, reports) = mpsc::channel();
        making(drafts(1)[0], wanted, root.path().to_path_buf(), reporting);

        let mut said = Vec::new();
        while let Ok(report) = reports.recv() {
            said.push(match report.said {
                Said::Doing(doing) => doing,
                Said::Made(plan) => format!("made {}", plan.entry.spawn),
                Said::Refused(why) => format!("refused: {why}"),
            });
        }

        said
    }

    #[test]
    fn a_plan_makes_a_worktree_on_a_branch_of_its_own() {
        let root = root();
        let repository = repository();

        let plan = plan(&wanted(&repository, "add retry logic"), root.path()).unwrap();
        plan.create().unwrap();

        assert!(plan.entry.spawn.starts_with("add-retry-logic-"));
        assert_eq!(plan.entry.repository, "project");
        assert_eq!(plan.entry.branch, format!("spawn/{}", plan.entry.spawn));
        assert!(
            plan.entry
                .worktree
                .starts_with(root.path().to_str().unwrap()),
            "the worktree is not under the app's own root: {}",
            plan.entry.worktree
        );
        let branch = process::run_ok(
            "git",
            &[
                "-C",
                &plan.entry.worktree,
                "rev-parse",
                "--abbrev-ref",
                "HEAD",
            ],
        )
        .unwrap();
        assert_eq!(branch, plan.entry.branch);
    }

    #[test]
    fn the_work_and_the_answers_reach_the_command_the_spawn_runs() {
        let root = root();
        let repository = repository();
        let mut asked = wanted(&repository, "add retry logic");
        asked.answers = vec!["haiku".to_string()];

        let plan = plan(&asked, root.path()).unwrap();

        assert!(
            plan.recipe.args.contains(&"add retry logic".to_string()),
            "{:?}",
            plan.recipe.args
        );
        assert!(
            plan.recipe.args.contains(&"haiku".to_string()),
            "the answer the form collected did not reach the command line: {:?}",
            plan.recipe.args
        );
    }

    /// The rule stated as a test: **what is about to happen is said before it
    /// happens.** A creation that died the instant after the last thing it said
    /// has still said everything it made.
    #[test]
    fn what_it_is_about_to_do_is_said_before_it_is_done() {
        let root = root();
        let repository = repository();

        let said = reported(wanted(&repository, "add retry logic"), &root);

        let worktree = said
            .iter()
            .position(|line| line.starts_with("creating the worktree"))
            .unwrap_or_else(|| panic!("nothing said a worktree was being made: {said:?}"));
        let made = said
            .iter()
            .position(|line| line.starts_with("made "))
            .unwrap_or_else(|| panic!("nothing was made: {said:?}"));
        assert!(
            worktree < made,
            "the worktree was made before anything said it would be: {said:?}"
        );
        assert!(
            said[worktree].contains("spawn/add-retry-logic-"),
            "the record does not say which branch was made: {}",
            said[worktree]
        );
    }

    #[test]
    fn a_repository_that_is_not_one_is_a_refusal_rather_than_a_spawn() {
        let root = root();
        let nowhere = tempdir().unwrap();

        let said = reported(
            Wanted {
                repository: nowhere.path().to_path_buf(),
                work: "add retry logic".to_string(),
                answers: Vec::new(),
            },
            &root,
        );

        assert!(
            said.last().unwrap().starts_with("refused:"),
            "a directory that is not a repository was made into a spawn: {said:?}"
        );
        assert!(
            !said.iter().any(|line| line.starts_with("creating")),
            "it got as far as making a worktree: {said:?}"
        );
    }

    /// **No lock, and none needed.** Several creations run at once against one
    /// repository, and the reason none of them waits is that names carry a
    /// random suffix — so the paths they ask for were never going to be the same
    /// one.
    ///
    /// It earned its keep the day it was written: four at once found the one
    /// thing two creations really did contend for, which was the repository's
    /// `.git/config` and is now not written at all (see
    /// [`crate::git::add_worktree`]). Four rather than two because it is a race,
    /// and a race caught one run in five is a race that gets committed.
    #[test]
    fn drafts_started_at_once_do_not_wait_for_one_another() {
        const AT_ONCE: usize = 4;

        let root = root();
        let repository = repository();
        let started = drafts(AT_ONCE);
        let (reporting, reports) = mpsc::channel();

        for (which, draft) in started.iter().enumerate() {
            making(
                *draft,
                wanted(&repository, &format!("do the {which} piece of work")),
                root.path().to_path_buf(),
                reporting.clone(),
            );
        }
        drop(reporting);

        let mut made = Vec::new();
        while let Ok(report) = reports.recv() {
            match report.said {
                Said::Made(plan) => made.push(*plan),
                Said::Refused(why) => panic!("a concurrent creation was refused: {why}"),
                Said::Doing(_) => {}
            }
        }

        assert_eq!(made.len(), AT_ONCE);
        let worktrees: HashSet<&String> = made.iter().map(|plan| &plan.entry.worktree).collect();
        assert_eq!(
            worktrees.len(),
            AT_ONCE,
            "two spawns were given the same worktree"
        );
        for plan in &made {
            assert!(
                Path::new(&plan.entry.worktree).join(".git").exists(),
                "{} has no worktree",
                plan.entry.spawn
            );
        }
    }

    // The rest drives a real tmux on a socket of its own, because starting a
    // spawn is exactly the part a fake would have to pretend about.

    /// The shape of a slot in these tests.
    const SLOT: Size = Size {
        columns: 40,
        rows: 10,
    };

    /// Wait for a grid to say something, or give up and show what it did say.
    fn until(grid: &Grid, wanted: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let seen = shown(&grid.lock().unwrap()).join("\n");
            if seen.contains(wanted) {
                return seen;
            }
            assert!(
                Instant::now() < deadline,
                "gave up waiting for {wanted:?}; the grid says:\n{seen}"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn a_spawn_starts_in_a_window_of_its_own_with_a_grid_already_behind_it() {
        let root = root();
        let repository = repository();
        let tmux = PrivateTmux::start("creation-starts-a-spawn");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let mut plan = plan(&wanted(&repository, "add retry logic"), root.path()).unwrap();
        plan.create().unwrap();
        // No harness is ever really started in a test: the real one costs
        // tokens and needs credentials, so what runs is a stand-in that draws
        // something only it would draw.
        plan.recipe = tmux.recipe("printf 'the spawn is talking\\n'; sleep 120");

        let started = start(&tmux.server, &session, &client, SLOT, plan).unwrap();

        until(&started.spawn.grid, "the spawn is talking");
        assert_eq!(started.watched.pane, started.spawn.pane);
        assert_eq!(started.watched.name, started.spawn.entry.spawn);
        assert!(
            !tmux
                .server
                .panes()
                .unwrap()
                .get(&started.spawn.pane)
                .expect("the spawn's pane is not on the server")
                .dead
        );
    }
}
