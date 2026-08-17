//! Start a coding session on a worktree of its own, beside a list of the
//! others.
//!
//! One screen, drawn end to end by the app: the list on the left, a real
//! session in the slot on the right. tmux runs headless — it owns the session
//! processes and draws nothing — so quitting the app kills nothing. The README
//! and `docs/` carry the full picture.

mod adoption;
mod app;
mod cli;
mod control;
mod creation;
mod draft;
mod error;
mod git;
mod harness;
mod keys;
mod list;
mod litter;
mod names;
mod process;
mod projects;
mod retirement;
mod scaffolding;
mod screen;
mod snapshot;
mod supervisor;
mod tmux;
mod worktrees;
mod xdg;

use std::path::Path;
use std::process::ExitCode;

use crate::cli::Invocation;
use crate::creation::{Plan, Wanted};
use crate::error::{Error, Result};
use crate::litter::Leaving;
use crate::tmux::Server;

/// What this app calls itself when it is talking to a shell; said from three
/// places, so it is one constant.
const SAID: &str = "harness-launcher";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{SAID}: {error}");
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
        Invocation::Spawn(wanted) => spawn(&wanted),
        // Nothing to make, and everything else the same: a spawn started from
        // the form a moment later needs the session, client and supervisor.
        Invocation::Compose => spawn(&[]),
    }
}

/// Create the worktrees, open windows nobody sees, and start the sessions in
/// them.
///
/// The order matters: everything that can refuse does so before the screen is
/// taken over; every repository is resolved before any worktree is made; and
/// the control-mode client is attached before any harness starts, because it
/// streams only what is produced while attached — a session that drew first
/// would leave a slot that stays blank forever.
///
/// Adoption runs before this run's own spawns start, so the list reads in the
/// order the spawns were made.
fn spawn(wanted: &[Wanted]) -> Result<()> {
    // Only when there is something to start: an app asked to start nothing has
    // created nothing to leave behind, and the same check runs on any draft.
    if !wanted.is_empty() {
        creation::harness_installed(std::env::var_os("PATH"))?;
    }

    let slot = app::slot_now()?;
    let worktrees = worktrees::root()?;
    let projects = projects::saved()?;
    let tmux = tmux::Server::app();

    let plans: Vec<Plan> = wanted
        .iter()
        .map(|wanted| creation::plan(wanted, &worktrees))
        .collect::<Result<_>>()?;
    let created: Vec<String> = plans.iter().map(|plan| plan.entry.spawn.clone()).collect();
    create(&plans)?;

    // From here on this run has agents that outlive it, so every way out —
    // refusal and crash included — is reported. Installed before the session,
    // so a session that cannot open still surveys what the last run left.
    let _leaving = Leaving::saying(|| say_what_is_left(&tmux, &worktrees));

    let session = tmux.session(slot)?;
    let client = control::Client::attach(&tmux, &session, slot)?;

    let adopted = adoption::adopt(&tmux, &client, &worktrees, slot)?;
    say_what_was_found(&adopted, &worktrees, &created);

    let mut spawns = Vec::new();
    let mut watched = Vec::new();
    for started in adopted.started {
        watched.push(started.watched);
        spawns.push(started.spawn);
    }
    for plan in plans {
        let started = creation::start(&tmux, &session, &client, slot, plan)?;

        watched.push(started.watched);
        spawns.push(started.spawn);
    }

    let watching = supervisor::watch(tmux.clone(), watched);
    let world = app::World {
        server: &tmux,
        session: &session,
        client: &client,
        snapshots: watching.snapshots,
        arriving: watching.arriving,
        leaving: watching.leaving,
        worktrees: worktrees.clone(),
    };
    // A draft only for an empty list. Somebody who asked for nothing but has
    // spawns from an earlier run lands on those instead.
    let mut drafts = draft::Drafts::new(harness::choices(), projects);
    if spawns.is_empty() {
        drafts.start();
    }
    let mut held = app::Held::new(app::Spawns::new(spawns), drafts);

    app::run(&mut held, &world)
}

/// Say what an earlier run left behind, and what this one made of it.
///
/// Accepted cost: it lands on the shell just before the alternate screen
/// covers it, so it is read at exit. Whether to hold it back is left open in
/// `docs/developers/components/starting-and-leaving.md`.
fn say_what_was_found(adopted: &adoption::Adopted, worktrees: &Path, created: &[String]) {
    if let Some(report) = adopted.found(worktrees, created) {
        println!("{SAID}: {report}");
    }
}

/// Say what this run is leaving behind on its way out.
///
/// Quitting kills nothing, and this sentence keeps that from being a silent
/// surprise. Taken from the world rather than the app's own list, which can be
/// stale by now. A failed survey is said out loud rather than swallowed.
fn say_what_is_left(tmux: &Server, worktrees: &Path) {
    match litter::surveyed(tmux) {
        Ok(running) => println!("{SAID}: {}", litter::leaving(running.as_deref(), worktrees)),
        Err(why) => {
            eprintln!("{SAID}: could not say what is still running, and something is: {why}");
        }
    }
}

/// Make every worktree the plans call for, in order.
///
/// A refusal part-way through names the worktrees already made: litter is
/// accepted, invisible litter is not, and only here does the app still hold
/// the plan.
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
