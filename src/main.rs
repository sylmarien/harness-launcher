//! Start a coding session on a worktree of its own, beside a list of the others.
//!
//! One screen, drawn end to end by the app: the list on the left, the slot on
//! the right. The slot holds a real session — the actual program, which you type
//! into as if you had started it yourself. **The app never types into it
//! anything but what you typed.**
//!
//! **It is an ordinary terminal program.** Run it from a shell, from inside tmux,
//! from anywhere; it makes no difference to anything. tmux is here, but headless:
//! it owns the session's process and draws nothing, and the app reads what that
//! process draws over a control-mode client and renders every cell itself. What
//! the multiplexer buys is the one thing an owned terminal could not — **quitting
//! the app kills nothing.** The session is still running afterwards.
//!
//! **As many spawns as you ask for**, each on whatever repository you name, each
//! in a worktree and a window of its own — and the list beside them says what
//! every one of them is doing: a supervisor thread works it out about five
//! times a second and sends the list an immutable snapshot. Exactly one of them
//! is in the slot, and moving the selection changes which; the rest carry on
//! working off screen, because the slot is a region the app draws rather than
//! somewhere a session has to be moved to.
//!
//! The list is the real one: spawns under the repository they were started
//! against, attention-first, each repository header carrying a bar of its
//! spawns' statuses. **Nothing hides it** — that is the point of the whole
//! layout.
//!
//! **Another session is composed in the same place.** `F2` starts a draft: a
//! row of its own pinned above the repositories, and a form in the slot asking
//! for a repository, the work, and whatever the harness lets you choose. It is
//! not a dialog — walk away to a spawn that stopped and come back, and it is
//! where you left it. Several can be in flight at once. **Nothing is created
//! from one yet**; that is the next piece of work.

mod app;
mod cli;
mod control;
mod draft;
mod error;
mod git;
mod harness;
mod keys;
mod list;
mod names;
mod process;
mod scaffolding;
mod screen;
mod snapshot;
mod supervisor;
mod tmux;
mod worktrees;

use std::path::PathBuf;
use std::process::ExitCode;

use crate::cli::{Invocation, Request};
use crate::error::{Error, Result};
use crate::harness::LaunchRecipe;
use crate::list::Entry;
use crate::snapshot::Watched;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("harness-launcher: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match cli::parse(std::env::args().collect())? {
        Invocation::Help => {
            print!("{}", cli::usage());
            Ok(())
        }
        Invocation::Spawn(requests) => spawn(requests),
    }
}

/// Create the worktrees, open windows nobody sees, and start the sessions in
/// them.
///
/// The order matters three times over.
///
/// **Everything that can refuse does so before the screen is taken over**, so a
/// refusal lands on the shell the user is looking at rather than flashing past
/// on an alternate screen that closes behind it.
///
/// **Every repository is resolved before any worktree is made.** A line naming
/// four repositories, one of which is not one, should say so while there is
/// still nothing on disk to explain — and the commonest way to write that line
/// wrong is a description that needed quoting, which turns into a repository
/// that does not exist.
///
/// **The grid goes behind each pane before the harness is started in it.** A
/// control-mode client streams only what is produced while it is attached, so a
/// session that drew itself before anything was listening would leave a slot
/// that stays blank for ever — and nothing about it would look like an error.
fn spawn(requests: Vec<Request>) -> Result<()> {
    let slot = app::slot_now()?;

    let plans: Vec<Plan> = requests.into_iter().map(plan).collect::<Result<_>>()?;
    create(&plans)?;

    let tmux = tmux::Server::app();
    let session = tmux.session(slot)?;
    let client = control::Client::attach(&tmux, &session, slot)?;

    let mut spawns = Vec::new();
    let mut watched = Vec::new();
    for plan in plans {
        let pane = tmux.open_window(&session, &plan.entry.spawn)?;
        let grid = client.watch(&pane, slot);
        tmux.start(&pane, &plan.recipe)?;

        watched.push(Watched::new(plan.entry.spawn.clone(), pane.clone()));
        spawns.push(app::Spawn {
            entry: plan.entry,
            pane,
            grid,
        });
    }

    let snapshots = supervisor::watch(tmux, watched);
    let mut drafts = draft::Drafts::new(harness::choices());

    app::run(&app::Spawns::new(spawns)?, &mut drafts, &snapshots, &client)
}

/// One spawn, worked out but not yet made.
struct Plan {
    /// The repository it was started against.
    repository: git::Repository,
    /// The commit its branch is cut from.
    start_point: String,
    /// Where its worktree is to go.
    worktree: PathBuf,
    /// How to start the harness in it.
    recipe: LaunchRecipe,
    /// What the list will say about it.
    entry: Entry,
}

impl Plan {
    /// Make the worktree it calls for, on a branch of its own.
    fn create(&self) -> Result<()> {
        git::add_worktree(
            &self.repository,
            &self.worktree,
            &self.entry.branch,
            &self.start_point,
        )
    }
}

/// Work out everything about a spawn that can be settled without creating it.
fn plan(request: Request) -> Result<Plan> {
    let repository = git::open(&request.repository)?;
    let start_point = git::default_branch(&repository)?;

    let name = names::spawn_name(&request.work, names::fresh_seed());
    let branch = names::branch_name(&name);
    let worktree = worktrees::prepare(&name)?;

    let recipe = harness::launch_recipe(&harness::SpawnSpec {
        name: name.clone(),
        work: request.work,
        model: request.model,
        effort: request.effort,
        worktree: worktree.clone(),
    });
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

/// Make every worktree the plans call for, in order.
///
/// A refusal part-way through leaves the worktrees already made, and **says so
/// rather than exiting with only the reason the last one failed.** Litter is
/// accepted here; invisible litter is not, and this is the one place the app can
/// see what it left behind while it still knows what it made — a start-up report
/// that rediscovers orphans is a different job, and it is not built yet.
fn create(plans: &[Plan]) -> Result<()> {
    for (at, plan) in plans.iter().enumerate() {
        if let Err(refused) = plan.create() {
            return Err(match left_behind(&plans[..at]) {
                None => refused,
                Some(behind) => Error::new(format!("{refused}\nleft behind on disk: {behind}")),
            });
        }
    }

    Ok(())
}

/// What the spawns made before a refusal left on disk, as a sentence.
fn left_behind(made: &[Plan]) -> Option<String> {
    if made.is_empty() {
        return None;
    }

    let listed: Vec<String> = made
        .iter()
        .map(|plan| format!("{} on {}", plan.entry.worktree, plan.entry.branch))
        .collect();

    Some(listed.join(", "))
}
