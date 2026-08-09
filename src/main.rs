//! Start a coding session on a worktree of its own, beside a list of the others.
//!
//! One window: the app's list pane on the left, the slot on the right. The slot
//! holds a real session — the actual program, in a real pane, which you type
//! into as if you had started it yourself. **The app never types into it.**
//!
//! This is the walking skeleton: one spawn, and the list beside it is static
//! text. What it proves is the whole path — a worktree on a branch of its own, a
//! window composed around it, and a session running in it.

mod app;
mod cli;
mod error;
mod git;
mod harness;
mod names;
mod process;
mod tmux;
mod worktrees;

use std::process::ExitCode;

use crate::cli::{Invocation, Request};
use crate::error::Result;

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
        Invocation::ListPane(view) => app::run(&view),
        Invocation::Spawn(request) => spawn(request),
    }
}

/// Create the worktree, compose the window, and start the session in it.
///
/// The order matters: everything that can refuse does so before tmux is
/// involved, so a refusal lands on the shell the user is looking at rather than
/// flashing past inside a session that closes behind it.
fn spawn(request: Request) -> Result<()> {
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

    // Inside tmux the app is already a pane, so the window it is in becomes the
    // window: the slot is split off the app's own pane, and whatever else the
    // user had in that window is left alone rather than killed to make room.
    // Outside, there is no window yet — so the app builds one and starts itself
    // again inside it, which is what being a pane requires.
    let tmux = tmux::Server::inherited();
    if tmux::inside_session() {
        let slot = tmux.open_slot(&tmux::current_pane()?, &recipe)?;
        tmux.select_pane(&slot)?;
        app::run(&view)
    } else {
        tmux.open_window(
            &format!("harness-launcher-{}", view.spawn),
            &cli::list_pane_command(&view)?,
            &recipe,
            app::terminal_size(),
        )
    }
}
