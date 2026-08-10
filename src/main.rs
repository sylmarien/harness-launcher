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
//! One spawn so far, and the list beside it says what that spawn is doing: a
//! supervisor thread works it out about five times a second and sends the list
//! an immutable snapshot. What it proves is the whole path — a worktree on a
//! branch of its own, a window nobody sees, a session running in it, its screen
//! in the slot, and a row that tells the truth about it.
//!
//! The list itself is already the real one: spawns under the repository they
//! were started against, attention-first, each repository header carrying a bar
//! of its spawns' statuses. It is one row long only because only one spawn is
//! started so far.

mod app;
mod cli;
mod control;
mod error;
mod git;
mod harness;
mod keys;
mod list;
mod names;
mod process;
mod screen;
mod snapshot;
mod supervisor;
mod tmux;
mod worktrees;

use std::process::ExitCode;

use crate::cli::{Invocation, Request};
use crate::error::Result;
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

/// Create the worktree, open a window nobody sees, and start the session in it.
///
/// The order matters twice over.
///
/// **Everything that can refuse does so before the screen is taken over**, so a
/// refusal lands on the shell the user is looking at rather than flashing past
/// on an alternate screen that closes behind it.
///
/// **The grid goes behind the pane before the harness is started in it.** A
/// control-mode client streams only what is produced while it is attached, so a
/// session that drew itself before anything was listening would leave a slot
/// that stays blank for ever — and nothing about it would look like an error.
fn spawn(request: Request) -> Result<()> {
    let slot = app::slot_now()?;

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
    let entry = list::Entry {
        repository: repository.name().to_string(),
        spawn: name,
        branch,
        worktree: process::path_argument(&worktree)?.to_string(),
    };

    let tmux = tmux::Server::app();
    let session = tmux.session(slot)?;
    let client = control::Client::attach(&tmux, &session, slot)?;
    let pane = tmux.open_window(&session, &entry.spawn)?;
    let grid = client.watch(&pane, slot);
    tmux.start(&pane, &recipe)?;

    let snapshots = supervisor::watch(tmux, vec![Watched::new(entry.spawn.clone(), pane.clone())]);

    app::run(&app::Spawn { entry, pane, grid }, &snapshots, &client)
}
