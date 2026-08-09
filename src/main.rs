//! Start a coding session on a worktree of its own, beside a list of the others.
//!
//! One window: the app's list pane on the left, the slot on the right. The slot
//! holds a real session — the actual program, in a real pane, which you type
//! into as if you had started it yourself. **The app never types into it.**
//!
//! **The app runs inside tmux, and refuses to run anywhere else.** It composes
//! a window around the session it starts, and to do that it has to already be a
//! pane in that window — which, started from a bare shell, it is not. It could
//! build a window and start itself again inside it, and did; that meant a
//! second process learning the slot's pane id across a boundary, for a case the
//! user can settle by typing `tmux` first.
//!
//! One spawn so far, and the list beside it says what that spawn is doing: a
//! supervisor thread works it out about five times a second and sends the list
//! an immutable snapshot. What it proves is the whole path — a worktree on a
//! branch of its own, a window composed around it, a session running in it, and
//! a row that tells the truth about it.

mod app;
mod cli;
mod error;
mod git;
mod harness;
mod names;
mod process;
mod snapshot;
mod supervisor;
mod tmux;
mod worktrees;

use std::process::ExitCode;

use crate::cli::{Invocation, Request};
use crate::error::{Error, Result};
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
        Invocation::Spawn(request) => spawn(request),
    }
}

/// Create the worktree, compose the window, and start the session in it.
///
/// The order matters: everything that can refuse does so before tmux is
/// involved, so a refusal lands on the shell the user is looking at rather than
/// flashing past inside a session that closes behind it. Not being in tmux is
/// refused first of all, because it is the one refusal that costs nothing to
/// check and would otherwise land after a worktree had been created.
fn spawn(request: Request) -> Result<()> {
    if !tmux::inside_session() {
        return Err(Error::new(
            "this has to run inside tmux. It composes a window around the session it \
             starts — the list on the left, the session on the right — and to do that it \
             has to be a pane in that window itself. Start `tmux`, then run this again \
             from inside it.",
        ));
    }

    let repository = git::open(&request.repository)?;
    let start_point = git::default_branch(&repository)?;

    let name = names::spawn_name(&request.work, names::fresh_seed());
    let branch = names::branch_name(&name);
    let worktree = worktrees::prepare(&name)?;
    git::add_worktree(&repository, &worktree, &branch, &start_point)?;

    let recipe = harness::launch_recipe(&harness::SpawnSpec {
        name: name.clone(),
        work: request.work,
        model: request.model,
        effort: request.effort,
        worktree: worktree.clone(),
    });
    let view = app::View {
        repository: repository.name().to_string(),
        spawn: name,
        branch,
        worktree: process::path_argument(&worktree)?.to_string(),
    };

    // The app is already a pane, so the window it is in becomes the window: the
    // slot is split off the app's own pane, and whatever else the user had in
    // that window is left alone rather than killed to make room.
    let tmux = tmux::Server::inherited();
    let slot = tmux.open_slot(&tmux::current_pane()?, &recipe)?;
    tmux.select_pane(&slot)?;

    let snapshots = supervisor::watch(tmux, vec![Watched::new(view.spawn.clone(), slot)]);

    app::run(&view, &snapshots)
}
