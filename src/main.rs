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
//! **Run it with nothing at all and it opens on a blank form** — no session, a
//! draft in the slot, and the list beside it. Everything the command line can
//! say can be said there instead, so the shortest way in is to open the app and
//! write what you want done.
//!
//! **Another session is composed in the same place.** `F2` starts a draft: a
//! row of its own pinned above the repositories, and a form in the slot asking
//! for a repository, the work, and whatever the harness lets you choose. It is
//! not a dialog — walk away to a spawn that stopped and come back, and it is
//! where you left it. Several can be in flight at once.
//!
//! **`F5` makes one into a spawn**, and it says what it is doing as it does it:
//! the repository read, the worktree made, the harness started. On success the
//! draft's row makes way for a spawn row under its repository, and the new
//! session is in the slot. On failure it is a draft again, with the text exactly
//! as it was and a record of what had already been made.
//!
//! **And `F9` retires the spawn the list is on** — the one act that releases
//! what the app made, and never inferred from an agent falling silent. The
//! session is stopped, and only once it is confirmed gone is the worktree
//! checked and removed: a cleanliness check taken against a live agent is a
//! race, and losing it deletes the file the agent wrote on its way out. Anything
//! uncommitted and it refuses, saying so on the row. The branch is left alone
//! either way.

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
mod retirement;
mod scaffolding;
mod screen;
mod snapshot;
mod supervisor;
mod tmux;
mod worktrees;

use std::path::Path;
use std::process::ExitCode;

use crate::cli::Invocation;
use crate::creation::{Plan, Wanted};
use crate::error::{Error, Result};
use crate::litter::{Leaving, Litter};
use crate::tmux::Server;

/// What this app calls itself when it is talking to a shell.
///
/// Said once because it is said from three places — a refusal, the report on the
/// way in, the report on the way out — and they are the same voice. Three
/// literals would let the name this app is installed as drift away from the name
/// it answers to in the one place somebody would grep for it.
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
        // Nothing to make, and everything else the same: the session, the
        // client and the supervisor are what a spawn started from the form a
        // moment later will need, and they cost nothing standing empty.
        Invocation::Compose => spawn(&[]),
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
///
/// After this the app makes spawns the other way, from a draft — the same plan,
/// the same worktree, the same window, on a thread so the screen keeps drawing.
/// What is different here is only that a refusal has a shell to land on.
///
/// **Nothing to start is an ordinary way to run this**, and the only difference
/// it makes is a draft waiting in the slot instead of a session. Everything
/// around it is built the same: the session, the client and the supervisor are
/// what the first spawn will need whenever it is written.
fn spawn(wanted: &[Wanted]) -> Result<()> {
    // **Before anything else, and only when there is something to start.** A
    // machine without the harness on it has nothing to run, and finding that out
    // later would leave a worktree and a branch behind for a session that was
    // never going to exist. An app asked to start *nothing* is not a machine to
    // refuse over: it has created nothing, somebody may be about to install the
    // harness, and the same check runs again on the draft they write — where a
    // refusal costs them a sentence rather than the app.
    if !wanted.is_empty() {
        creation::harness_installed(std::env::var_os("PATH"))?;
    }

    let slot = app::slot_now()?;
    let worktrees = worktrees::root()?;
    let tmux = tmux::Server::app();

    say_what_was_found(&tmux, &worktrees)?;

    let plans: Vec<Plan> = wanted
        .iter()
        .map(|wanted| creation::plan(wanted, &worktrees))
        .collect::<Result<_>>()?;
    create(&plans)?;

    // **From here on this run has agents that outlive it**, so from here on the
    // way out is reported whichever way out it turns out to be. Every `?` below
    // is one — the third of four spawns refusing to start leaves two running and
    // used to say nothing about them — and so is the app falling over, which is
    // exactly when somebody most needs telling that twenty agents are still
    // going. Made before the session so that a session that cannot be opened is
    // still a survey of whatever the last run left.
    let _leaving = Leaving::saying(|| say_what_is_left(&tmux, &worktrees));

    let session = tmux.session(slot)?;
    let client = control::Client::attach(&tmux, &session, slot)?;

    let mut spawns = Vec::new();
    let mut watched = Vec::new();
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
    // A draft is started for somebody who asked for nothing, and only for them:
    // it is the whole of what they asked for, and starting one beside sessions
    // that were asked for would be a row nobody wanted.
    let mut drafts = draft::Drafts::new(harness::choices());
    if spawns.is_empty() {
        drafts.start();
    }
    let mut held = app::Held::new(app::Spawns::new(spawns), drafts);

    app::run(&mut held, &world)
}

/// Say what an earlier run left behind, once, before this one makes anything.
///
/// **Before anything**, and that is the whole of the placement: a moment later
/// the worktrees this run was asked for exist, and a report taken then would
/// name them as leftovers of a run that has not finished starting.
///
/// **Nothing here is adopted.** The report is a statement about the world — the
/// list this run carries on with is empty, and everything named stays exactly
/// where it is. Recovering any of it belongs to a later tranche, and reporting
/// it is not a step towards that; it is the alternative to it.
///
/// A refusal propagates rather than being swallowed. It can only be tmux failing
/// to run at all, which is the app's next move anyway, and refusing here means
/// refusing on the shell rather than on a screen that closes behind it.
///
/// *Accepted cost:* this lands on the shell a moment before the alternate screen
/// covers it, so it is read on the way out rather than on the way in — the
/// original screen comes back when the app leaves, with this still on it and the
/// leaving report printed under it. *Whether* it should be held back until it
/// can be read on the way in is deliberately left open —
/// `docs/developers/components/starting-and-leaving.md` records the question;
/// that both reports land on the shell is not, and the
/// shell is where every other thing the app says before it takes the screen
/// already lands.
fn say_what_was_found(tmux: &Server, worktrees: &Path) -> Result<()> {
    if let Some(report) = Litter::surveyed(tmux, worktrees)?.found() {
        println!("{SAID}: {report}");
    }

    Ok(())
}

/// Say what this run is leaving behind on its way out.
///
/// **Quitting kills nothing** — no signal, no `kill-pane`, no cleanup — and this
/// is the sentence that keeps that from being a silent surprise: the session to
/// attach to, how much is still running in it, and where the worktrees are.
///
/// Taken from the world rather than from what the app was holding, because by
/// now they can disagree: a spawn may have stopped on its own, and saying that
/// four agents are still working when one of them died an hour ago would be
/// worse than saying nothing.
///
/// **A failed survey is said out loud rather than swallowed.** The app is on its
/// way out and has nothing left to refuse with, but somebody has just been told
/// nothing about twenty agents that are still running, and silence would read as
/// "there was nothing to leave".
fn say_what_is_left(tmux: &Server, worktrees: &Path) {
    match Litter::surveyed(tmux, worktrees) {
        Ok(litter) => println!("{SAID}: {}", litter.leaving()),
        Err(why) => {
            eprintln!("{SAID}: could not say what is still running, and something is: {why}");
        }
    }
}

/// Make every worktree the plans call for, in order.
///
/// A refusal part-way through leaves the worktrees already made, and **says so
/// rather than exiting with only the reason the last one failed.** Litter is
/// accepted here; invisible litter is not, and this is the one place the app can
/// see what it left behind *while it still knows what it made*.
///
/// The next run rediscovers it instead — [`say_what_was_found`] names anything under the
/// worktree root that nothing is running for — but that report can only say what
/// the names on disk say. This one still has the plan in its hand, which is why
/// it is worth keeping rather than leaving to the start-up report.
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
