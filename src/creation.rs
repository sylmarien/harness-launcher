//! Making a spawn: from a description of work to a session running in a
//! worktree of its own.
//!
//! Both entry paths — the command line and a started draft — produce a
//! [`Wanted`]; everything after that is shared. The git half ([`making`]) runs
//! on a thread of its own; the tmux half ([`start`]) runs on the one thread
//! that holds the control client. Intent is announced before each step, so a
//! creation that dies half way leaves a record of what it made.
//! See docs/developers/components/drafts-and-creation.md.

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
    /// Opaque to everything between the form and the harness.
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

/// Refuse unless the harness this app starts is installed.
///
/// Asked before anything is created, so a missing harness leaves no worktree
/// or branch behind. The `PATH` is passed in so tests do not depend on the
/// machine running them.
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
/// Touches nothing in the repository; the only thing left behind is the
/// app-owned worktree root shared by every spawn.
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
/// watches. Handed over together so both views name the same pane.
pub struct Started {
    /// The spawn, as the screen needs it.
    pub spawn: Spawn,
    /// The spawn, as the supervisor needs it.
    pub watched: Watched,
}

/// Open a window, put a grid behind it, and start the harness in it.
///
/// The grid attaches before the harness starts: a control-mode client streams
/// only what is produced while it is attached, so output drawn earlier would
/// leave the slot blank for ever. The size is the slot's current one, not the
/// start-up one.
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
    /// The worktree is there, with everything about the spawn to run in it.
    /// Boxed because a plan is far larger than the other variants.
    Made(Box<Plan>),
    /// It stopped, and this is why. The draft is restored with its text intact.
    Refused(String),
}

/// Make everything a spawn needs on disk, on a thread of its own.
///
/// On a thread because `git worktree add` takes seconds; progress comes back
/// as reports. A dropped receiver means the app has gone; the work carries on.
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

/// The git half of a creation.
///
/// Each line is said before the step it describes is attempted, so if this
/// thread dies, what is on disk has already been written down.
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
    use crate::process::tests::{holding, path};
    use crate::screen::tests::shown;
    use crate::snapshot::{self, Status};
    use crate::tmux::tests::PrivateTmux;

    /// A throwaway root for the worktrees a test makes.
    fn root() -> TempDir {
        tempdir().unwrap()
    }

    /// Draft ids, taken from `Drafts` rather than made up.
    fn drafts(how_many: usize) -> Vec<draft::Id> {
        let mut drafts = draft::Drafts::new(Vec::new(), Vec::new());

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

    /// The setup: origin's HEAD names a branch whose ref has gone, so git
    /// plans the worktree and then refuses to check it out — a refusal that
    /// arrives after the app has said what it was about to make.
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

    /// No creation lock: names carry a random suffix, so paths never collide.
    /// Four at once because this caught the `.git/config` race (see
    /// [`crate::git::add_worktree`]), which failed only about one run in five.
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

    // The rest drives a real tmux on a socket of its own.

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
        // A stand-in for the harness: the real one costs tokens and needs
        // credentials.
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

    /// A harness that dies at startup reads as stopped (no fourth status), and
    /// what it wrote stays readable: `remain-on-exit` keeps the pane, and the
    /// app's grid keeps the bytes even though tmux clears its own copy on pane
    /// death (measured on tmux 3.4 — `capture-pane` loses the error).
    ///
    /// The stand-in sleeps before exiting: a write immediately followed by
    /// exit races tmux's event loop and never reaches the app at all, and no
    /// priming from `capture-pane` can recover it.
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
