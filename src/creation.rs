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

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use crate::app::Spawn;
use crate::control::Client;
use crate::draft;
use crate::error::{Error, Result};
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

/// Refuse unless the harness this app starts is installed at all.
///
/// **Asked before anything is created**, which is the whole of why it is a
/// function of its own rather than a step of [`plan`]: a machine without the
/// harness has nothing to start, and finding that out from a pane that dies
/// would leave a worktree and a branch behind for a session that never ran. No
/// worktree, no branch, no pane, no litter — the refusal costs the user a
/// sentence and nothing else.
///
/// **The `PATH` is passed in** rather than read here, so this rule is one the
/// tests can state: read from the environment, the answer would depend on what
/// happened to be installed on the machine running them.
///
/// The two halves of what it says come from either side of the harness seam.
/// *What* is missing and *what to do about it* are the harness's own facts; that
/// a program has to be runnable to be started is the app's.
pub fn harness_installed(path: Option<OsString>) -> Result<()> {
    let required = harness::requirement();
    if process::runnable_on(path, required.program) {
        return Ok(());
    }

    Err(Error::new(format!(
        "`{}` is not on PATH, so there is nothing to start a spawn with — {}",
        required.program, required.fix
    )))
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
    use std::collections::{HashMap, HashSet};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tempfile::{TempDir, tempdir};

    use crate::control::Grid;
    use crate::git::tests::repository_with_origin as repository;
    // The same two helpers the module that resolves programs on a `PATH` tests
    // itself with: a directory holding a runnable program, and a `PATH` made of
    // directories. Written once there rather than near-copied here, so that a
    // test about *finding* the harness and a test about *refusing* over it
    // cannot come to disagree about what "installed" means on disk.
    use crate::process::tests::{holding, path};
    use crate::screen::tests::shown;
    use crate::snapshot::{self, Status};
    use crate::tmux::tests::PrivateTmux;

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

    /// **The refusal that has to come before every other one.** A machine
    /// without the harness has nothing to start, and finding that out after the
    /// worktree exists would leave a branch and a directory behind for a
    /// session that was never going to run.
    #[test]
    fn a_harness_that_is_not_installed_is_refused_and_says_what_to_do() {
        let nothing_installed = tempdir().unwrap();

        let refused = harness_installed(Some(path(&[&nothing_installed])))
            .expect_err("a machine with no harness on it started a spawn anyway");

        let said = refused.to_string();
        let required = harness::requirement();
        assert!(
            said.contains(required.program),
            "the refusal does not say what is missing: {said}"
        );
        assert!(
            said.contains(required.fix),
            "the refusal does not say what to do about it: {said}"
        );
    }

    #[test]
    fn a_harness_that_is_installed_is_nothing_to_refuse_over() {
        let installed = holding(harness::requirement().program, true);

        assert!(harness_installed(Some(path(&[&installed]))).is_ok());
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

    /// **A worktree that cannot be made says why, and leaves nothing behind
    /// that the app has not already written down.**
    ///
    /// The repository here is one git will plan a spawn from and then refuse to
    /// check out: origin's HEAD still names a branch whose ref has gone, which
    /// is what a clone looks like after somebody prunes it. That is the shape of
    /// the case worth testing — the refusal arrives *after* the app has said
    /// what it was about to make, which is what makes the difference between
    /// litter that is written down and litter that is a mystery.
    #[test]
    fn a_worktree_that_cannot_be_made_says_why_and_leaves_nothing_half_made() {
        let root = root();
        let repository = repository();
        let clone = repository.path().join("project");
        crate::git::tests::git(&[
            "-C",
            clone.to_str().unwrap(),
            "update-ref",
            "-d",
            "refs/remotes/origin/main",
        ]);

        let said = reported(wanted(&repository, "add retry logic"), &root);

        let refused = said.last().expect("a creation says something");
        assert!(
            refused.starts_with("refused:"),
            "a worktree git would not make was made anyway: {said:?}"
        );
        assert!(
            refused.contains("origin/main"),
            "the refusal does not carry git's own account of what went wrong: {refused}"
        );
        let about_to = said
            .iter()
            .find(|line| line.starts_with("creating the worktree"))
            .unwrap_or_else(|| panic!("nothing said what was about to be made: {said:?}"));
        assert!(
            about_to.contains("spawn/add-retry-logic-"),
            "the record does not name what it was about to make: {about_to}"
        );
        assert!(
            worktrees::under(root.path()).is_empty(),
            "something half-made was left under the app's own root: {:?}",
            worktrees::under(root.path())
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

    /// **A harness that dies the moment it starts is a spawn that stopped**, and
    /// what it said on its way out is still there to read.
    ///
    /// Three things at once, and they are one behaviour: `remain-on-exit` keeps
    /// the pane so the bytes it drew are still the grid's, the ladder reads a
    /// dead pane as **stopped** immediately, and there is **no fourth status**
    /// — a harness that would not start is a spawn that has stopped, which is
    /// the vocabulary the user already learned. A `failed` added here would be a
    /// state of the launch rather than of the agent.
    ///
    /// Without `remain-on-exit` this fails in a way worth naming: tmux reaps the
    /// pane, the error goes with it, and the ladder reports a spawn whose pane
    /// the app cannot find — the app calling its own instrumentation broken over
    /// something that merely exited.
    ///
    /// **The grid outlives tmux's own copy, and that is the point.** Measured
    /// against tmux 3.4 while writing this: when a pane's process exits, tmux
    /// *clears the pane* and draws `Pane is dead (status 1, …)` over it, so
    /// `capture-pane` afterwards no longer has the error at all. What the user
    /// reads is the app's grid, which is the app's own memory of what it
    /// observed and is never cleared by anything tmux does to its copy.
    ///
    /// **The stand-in lingers before exiting, and that is not padding.** Control
    /// mode carries only what tmux managed to read from the pty, and a child that
    /// writes and exits in the same breath races tmux's event loop: measured, a
    /// `printf` immediately followed by `exit` reaches the app **not at all**,
    /// and no priming can recover it because tmux has cleared its own copy by
    /// then. That race is a real limit of the transport rather than a property of
    /// this test — recorded here because it is invisible from the code and the
    /// obvious "fix" (priming from `capture-pane` on death) does not work.
    #[test]
    fn a_harness_that_dies_on_startup_stays_on_screen_and_reads_as_stopped() {
        let root = root();
        let repository = repository();
        let tmux = PrivateTmux::start("creation-harness-dies-at-once");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let mut plan = plan(&wanted(&repository, "add retry logic"), root.path()).unwrap();
        plan.create().unwrap();
        plan.recipe = tmux.recipe("printf 'not logged in\\n'; sleep 0.4; exit 1");

        let started = start(&tmux.server, &session, &client, SLOT, plan).unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        until(&started.spawn.grid, "not logged in");
        let snapshot = snapshot::build(
            std::slice::from_ref(&started.watched),
            &tmux.server.panes().unwrap(),
            &HashMap::new(),
            Instant::now(),
            &snapshot::Snapshot::default(),
        );
        let row = snapshot
            .of(&started.spawn.entry.spawn)
            .expect("a watched spawn has a row");
        assert_eq!(
            row.status,
            Status::Stopped,
            "a harness that would not start was given a status of its own: {row:?}"
        );
        assert_eq!(
            row.unaccounted, None,
            "a spawn that simply stopped was reported as something the app cannot account for"
        );
    }
}
