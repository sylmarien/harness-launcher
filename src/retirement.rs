//! Retiring a spawn: the one act that releases what the app created.
//!
//! **Never inferred.** An agent falling silent is not a retirement — the app
//! cannot tell a finished turn from a question waiting to be answered, and a
//! worktree removed on that guess is an hour of somebody's work. Retiring is
//! something a person says, about a spawn they are done with.
//!
//! **The order is strict, and the order is the point:**
//!
//! 1. stop the process
//! 2. confirm it is gone
//! 3. check the worktree is clean
//! 4. remove the worktree
//!
//! A cleanliness check run against a live agent is a race, and losing that race
//! deletes work: the agent writes a file in the moment between the check
//! passing and the directory going. Stopping first is the whole of what makes
//! the check mean anything — which is why the test that pins this order uses a
//! session that writes a file *as it is being stopped*.
//!
//! **A dirty worktree refuses, and there is no confirmation flow.** Clean it up
//! and retire again. *Accepted cost, and it is a real one:* the refusal lands
//! after the kill, so a spawn that turns out to be dirty ends up stopped and
//! needing dealing with by hand. An early check before stopping was considered
//! and declined — it would be a second answer about cleanliness, and the one
//! that decided anything would still be the one taken afterwards.
//!
//! **The branch stays.** The worktree is the app's to remove because the app
//! made it; the branch holds committed work, and deleting it is a different and
//! riskier act than removing a checkout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::tmux::Server;
use crate::{git, process};

/// How long a session is given to stop after being asked to.
///
/// Bounded rather than patient: an agent that will not go is not a reason for a
/// retirement to hang about for ever, and the backstop below is what the bound
/// is for. Long enough that a harness saving whatever it keeps on the way out
/// is not cut off mid-write.
const PATIENCE: Duration = Duration::from_secs(3);

/// How long the pane is given to go once it has been killed outright.
///
/// Short, because nothing is being waited *for* here — the pane is gone the
/// moment tmux takes it, and this only covers the round trip.
const CLOSING: Duration = Duration::from_secs(2);

/// How often the app looks to see whether it has gone yet.
const LOOKING: Duration = Duration::from_millis(25);

/// How much of what is in a worktree a refusal names.
///
/// Enough to recognise the work by, and not so much that a build directory
/// somebody forgot to ignore fills the list.
const NAMED: usize = 3;

/// What a retirement in flight has to say, and which spawn it is about.
pub struct Report {
    /// The spawn being retired, which is where this is shown.
    pub spawn: String,
    /// What it has to say.
    pub said: Said,
}

/// One thing a retirement has to say.
pub enum Said {
    /// What it is about to do, said before it does it.
    Doing(String),
    /// It is done: the session is stopped and the worktree is gone, so there is
    /// nothing of this spawn left for the app to hold.
    Retired,
    /// It stopped, and this is why. The spawn stays exactly where it was.
    Refused(String),
}

/// Where the retirements in flight have got to, one per spawn.
///
/// Held by the app rather than by the threads doing the work, because what a
/// retirement has to say is said on the row it is about — and the row is drawn
/// every frame, whichever spawn the slot is showing. Somebody who asked for a
/// retirement and walked off to answer another spawn is looking at the list.
#[derive(Default)]
pub struct Retirements {
    /// One per spawn that is being retired, or whose retirement was refused.
    of: HashMap<String, Retirement>,
}

/// Where one spawn's retirement has got to.
///
/// Two states rather than three: a retirement that finished leaves nothing
/// behind to be in a state, because the spawn itself is gone.
pub enum Retirement {
    /// Under way, and this is the step it is on.
    Doing(String),
    /// It stopped, and this is why. The spawn is still here, and still listed.
    Refused(String),
}

impl Retirement {
    /// What it has to say, as the sentence the row carries.
    pub fn said(&self) -> &str {
        match self {
            Retirement::Doing(step) => step,
            Retirement::Refused(why) => why,
        }
    }

    /// Whether it stopped rather than being under way.
    pub fn refused(&self) -> bool {
        matches!(self, Retirement::Refused(_))
    }
}

impl Retirements {
    /// Where one spawn's retirement has got to, if it has one at all.
    pub fn of(&self, spawn: &str) -> Option<&Retirement> {
        self.of.get(spawn)
    }

    /// Ask for a spawn to be retired.
    ///
    /// Nothing at all when one is already under way: the key is pressed on a
    /// row rather than aimed at a thread, and a second press while the first is
    /// still stopping a session must not stop it twice. A retirement that was
    /// *refused* is started again, which is what pressing it after cleaning the
    /// worktree up is for.
    pub fn asked_for(&mut self, spawn: &str) -> bool {
        if self.under_way(spawn) {
            return false;
        }
        self.of.insert(
            spawn.to_string(),
            Retirement::Doing("retiring the spawn".to_string()),
        );

        true
    }

    /// Whether a retirement of this spawn is already running.
    fn under_way(&self, spawn: &str) -> bool {
        matches!(self.of.get(spawn), Some(Retirement::Doing(_)))
    }

    /// Write down what a retirement is about to do, before it does it.
    pub fn doing(&mut self, spawn: &str, step: String) {
        self.of.insert(spawn.to_string(), Retirement::Doing(step));
    }

    /// Say that a retirement stopped, and leave the spawn where it was.
    pub fn refused(&mut self, spawn: &str, why: String) {
        self.of.insert(spawn.to_string(), Retirement::Refused(why));
    }

    /// Let go of a spawn that has been retired, or that is no longer here.
    pub fn finished(&mut self, spawn: &str) {
        self.of.remove(spawn);
    }
}

/// Retire a spawn, on a thread of its own.
///
/// **On a thread because stopping a session takes as long as the session takes
/// to stop**, and the app draws sixty frames a second of every other spawn
/// while it does. What comes back comes back as reports, so a retirement is
/// something the user watches on the row it is about rather than a screen that
/// stops moving.
///
/// A report nobody is listening for is not an error — it is the app on its way
/// out, and a retirement that is still going carries on until the process it is
/// a thread of does. **What that can leave behind is a session stopped and a
/// worktree still there**, which is litter of exactly the kind the design
/// accepts: it is on disk, under the spawn's own name, on the branch the spawn
/// was made for, and it is the same thing quitting with a spawn still running
/// leaves.
pub fn retiring(
    spawn: String,
    pane: String,
    worktree: PathBuf,
    server: Server,
    reporting: Sender<Report>,
) {
    thread::spawn(move || {
        let said = match retire(&server, &pane, &worktree, &|doing| {
            tell(&reporting, &spawn, Said::Doing(doing));
        }) {
            Ok(()) => Said::Retired,
            Err(refused) => Said::Refused(refused.to_string()),
        };

        tell(&reporting, &spawn, said);
    });
}

/// The whole of a retirement, in the order it has to happen in.
///
/// Each line is said before the step it describes is attempted, like a
/// creation's: a retirement that dies half way has already said what it was in
/// the middle of, which is the difference between a worktree that is gone and a
/// mystery.
fn retire(server: &Server, pane: &str, worktree: &Path, say: &dyn Fn(String)) -> Result<()> {
    say("stopping the session".to_string());
    stop(server, pane, PATIENCE, CLOSING)?;

    say("checking the worktree for work that is not committed".to_string());
    let uncommitted = git::uncommitted(worktree)?;
    if !uncommitted.is_empty() {
        return Err(Error::new(format!(
            "{} has work in it that is not committed ({}) — deal with it and retire the \
             spawn again. The session has been stopped",
            worktree.display(),
            named(&uncommitted)
        )));
    }

    say(format!("removing the worktree {}", worktree.display()));
    git::remove_worktree(worktree)?;

    // The window the app opened goes with it, and it goes last: until the
    // worktree is gone the pane is what a refusal leaves the user looking at.
    // A pane that has already gone — the backstop took it — is the outcome this
    // asked for, so its complaint is not one.
    let _ = server.close(pane);

    Ok(())
}

/// Stop what a pane is running, and do not come back until it is gone.
///
/// **`SIGTERM` first**, which is the signal that means *finish and go*: a
/// harness given it can put down whatever it was holding, and one that is
/// killed outright cannot. Then a bounded wait, and then `kill-pane` as the
/// backstop — because a retirement that hangs on a session ignoring signals is
/// a retirement that never happens.
///
/// **What settles it is never what `kill` said.** A signal sent to a process
/// that has just exited fails, and that failure is not a refusal — it is the
/// answer this was after.
///
/// What settles it is *tmux*, for as long as tmux has anything to say: a pane it
/// still holds is a pane it is still waiting on, so a pane it reports dead is a
/// process it has reaped. That is a better answer than any question the app
/// could ask about a process id.
///
/// **After the backstop there is no pane left to ask about**, and the `SIGHUP`
/// that went with it can be sat through exactly like the first signal — so there,
/// and only there, the process is asked about directly. Only there because a
/// process id is a name the system hands out again, and a retirement must not
/// refuse because something else has since been given the number.
///
/// The two waits are parameters so a test can pin the backstop, and what happens
/// when even that is outlived, without spending the real ones on every run.
fn stop(server: &Server, pane: &str, patience: Duration, closing: Duration) -> Result<()> {
    let Some(running) = running_in(server, pane)? else {
        return Ok(());
    };

    let asked = process::run("kill", &["-TERM", &running.to_string()])?;
    if within(patience, || Ok(running_in(server, pane)?.is_none()))? {
        return Ok(());
    }

    server.close(pane)?;
    if within(closing, || {
        Ok(running_in(server, pane)?.is_none() && !still_there(running)?)
    })? {
        return Ok(());
    }

    Err(Error::new(format!(
        "the session in {pane} is still running {} seconds after it was asked to stop and \
         then killed{}, so nothing was removed",
        (patience + closing).as_secs(),
        complaint(&asked)
    )))
}

/// The process a pane is running, if it is still running one.
fn running_in(server: &Server, pane: &str) -> Result<Option<u32>> {
    Ok(server
        .panes()?
        .get(pane)
        .filter(|pane| !pane.dead)
        .map(|pane| pane.pid))
}

/// Wait for something to become true, and say whether it did.
///
/// The one place a retirement waits at all, so the two things it waits for are
/// two conditions rather than two loops that could come to differ about what
/// giving up means.
fn within(patience: Duration, mut yet: impl FnMut() -> Result<bool>) -> Result<bool> {
    let deadline = Instant::now() + patience;
    loop {
        if yet()? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }

        thread::sleep(LOOKING);
    }
}

/// Whether a process is still there at all, asked without disturbing it.
///
/// Signal zero is the question rather than the instruction: it runs every check
/// a real signal would and then sends nothing.
///
/// **Recorded rather than fixed:** this is about the process the app started,
/// not its descendants. A harness that leaves a child of its own behind leaves
/// something no bounded check could see, and the worktree check that follows
/// would then be racing it.
fn still_there(pid: u32) -> Result<bool> {
    Ok(process::run("kill", &["-0", &pid.to_string()])?.ok)
}

/// What `kill` had to say, when it had something and it turned out to matter.
fn complaint(asked: &process::Outcome) -> String {
    if asked.ok || asked.stderr.is_empty() {
        return String::new();
    }

    format!(" (`kill` said: {})", asked.stderr)
}

/// What is in a worktree, as a phrase a refusal can carry.
fn named(uncommitted: &str) -> String {
    let lines: Vec<&str> = uncommitted.lines().collect();
    let named: Vec<String> = lines
        .iter()
        .take(NAMED)
        .map(|line| line.trim().to_string())
        .collect();

    match lines.len().saturating_sub(NAMED) {
        0 => named.join(", "),
        rest => format!("{}, and {rest} more", named.join(", ")),
    }
}

/// Say one thing about a retirement, if anybody is still listening.
fn tell(reporting: &Sender<Report>, spawn: &str, said: Said) {
    let _ = reporting.send(Report {
        spawn: spawn.to_string(),
        said,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Deref;
    use std::sync::{Mutex, mpsc};

    use crate::git::tests::{BRANCH, Spawned};
    use crate::harness::LaunchRecipe;
    use crate::screen::Size;
    use crate::tmux::tests::PrivateTmux;

    /// The shape of a slot in these tests. Nothing here draws, but a session
    /// has to be some size.
    const SLOT: Size = Size {
        columns: 40,
        rows: 10,
    };

    /// A spawn as retirement finds one: a session running in a worktree of its
    /// own, on a repository nothing else shares.
    struct Spawn {
        /// The tmux server the session runs on, which is this test's alone.
        tmux: PrivateTmux,
        /// The repository and the worktree, made the way the app makes them.
        spawned: Spawned,
        /// The pane the session is running in.
        pane: String,
    }

    /// Everything about the worktree reads through the spawn, because from
    /// retirement's side they are one thing: a session running in a checkout.
    impl Deref for Spawn {
        type Target = Spawned;

        fn deref(&self) -> &Spawned {
            &self.spawned
        }
    }

    impl Spawn {
        /// Start one, running a stand-in for the harness.
        ///
        /// No harness is ever really started in a test: the real one costs
        /// tokens and needs credentials. What runs is a shell script that does
        /// whatever the case being tested needs a session to do.
        fn running(named: &str, script: &str) -> Self {
            let spawned = Spawned::new();
            let tmux = PrivateTmux::start(named);
            let session = tmux.server.session(SLOT).unwrap();
            let pane = tmux
                .server
                .open_window(&session, "add-retry-logic-a7f3")
                .unwrap();
            tmux.server
                .start(
                    &pane,
                    &LaunchRecipe {
                        program: "sh".to_string(),
                        args: vec!["-c".to_string(), script.to_string()],
                        env: Vec::new(),
                        cwd: spawned.worktree.clone(),
                    },
                )
                .unwrap();
            // Nothing is retired until the session is really running: a script
            // signalled before its `trap` is set is a race in the test rather
            // than in the app.
            tmux.until("#{pane_id} #{pane_current_command}", |seen| {
                seen.contains(&format!("{pane} sh"))
            });

            Self {
                tmux,
                spawned,
                pane,
            }
        }

        /// Retire it, keeping every line it said on the way.
        fn retired(&self) -> (Result<()>, Vec<String>) {
            let said = Mutex::new(Vec::new());
            let outcome = retire(&self.tmux.server, &self.pane, &self.worktree, &|doing| {
                said.lock().unwrap().push(doing);
            });

            (outcome, said.into_inner().unwrap())
        }

        /// The process the session is running, while it is running one.
        fn process(&self) -> Option<u32> {
            running_in(&self.tmux.server, &self.pane).unwrap()
        }

        /// Whether the pane is still running something.
        fn still_running(&self) -> bool {
            self.process().is_some()
        }

        /// Whether the server still has the pane at all.
        fn pane_is_listed(&self) -> bool {
            self.tmux.server.panes().unwrap().get(&self.pane).is_some()
        }
    }

    /// The file a session leaves behind to say it was asked to stop rather than
    /// killed. Written outside the worktree, so saying so does not itself make
    /// the worktree dirty.
    const ASKED: &str = "asked-to-stop";

    /// A session that stays until it is told to go, and goes politely when it
    /// is — which is what a harness given `SIGTERM` does, and what one that was
    /// killed outright never gets the chance to.
    fn stops_when_asked(marker: &Path) -> String {
        format!(
            "trap 'printf asked > {}; exit 0' TERM; while :; do sleep 1; done",
            marker.display()
        )
    }

    #[test]
    fn retiring_stops_the_session_removes_the_worktree_and_leaves_the_branch() {
        let spawn = Spawn::running("retiring-a-clean-spawn", "sleep 120");

        let (outcome, said) = spawn.retired();

        outcome.unwrap();
        assert!(!spawn.worktree.exists(), "the worktree is still there");
        assert!(!spawn.still_running(), "the session is still running");
        assert!(!spawn.pane_is_listed(), "the window was left behind");
        assert!(
            spawn.branches().contains(BRANCH),
            "the branch went with the worktree: {}",
            spawn.branches()
        );
        assert!(
            said.iter().any(|line| line.contains("stopping")),
            "nothing said the session was being stopped: {said:?}"
        );
    }

    /// **The signal, stated as a test.** The session is *asked* to stop before
    /// anything is killed, which is what lets a harness put down whatever it
    /// was holding — and the mark it leaves is written outside the worktree, so
    /// saying so does not itself make the worktree dirty.
    #[test]
    fn the_session_is_asked_to_stop_before_anything_is_killed() {
        let elsewhere = tempfile::tempdir().unwrap();
        let marker = elsewhere.path().join(ASKED);
        let spawn = Spawn::running("retiring-asks-first", &stops_when_asked(&marker));

        spawn.retired().0.unwrap();

        assert!(
            marker.exists(),
            "the session was killed outright rather than asked to stop"
        );
    }

    /// **The race the order exists to lose, stated as a test.** The session
    /// writes a file as it is being stopped — which is exactly what an agent
    /// asked to finish does. A check taken before the kill would have found the
    /// worktree clean and deleted that file; taken afterwards, it finds it.
    #[test]
    fn what_the_session_writes_on_its_way_out_is_still_checked_for() {
        let spawn = Spawn::running(
            "retiring-checks-after-stopping",
            "trap 'printf notes > notes.md; exit 0' TERM; while :; do sleep 1; done",
        );

        let (outcome, _) = spawn.retired();

        let refused = outcome.expect_err("the work written on the way out was deleted");
        assert!(refused.to_string().contains("notes.md"), "{refused}");
        assert!(
            spawn.worktree.join("notes.md").exists(),
            "the file the session wrote as it was stopping is gone"
        );
        assert!(spawn.worktree.exists(), "the worktree is gone");
    }

    /// The accepted cost, stated as a test rather than left to be discovered: a
    /// spawn that turns out to be dirty ends up stopped all the same. Refusing
    /// is what the app does instead of choosing between somebody's work and
    /// their instruction.
    #[test]
    fn a_dirty_worktree_refuses_and_the_session_is_stopped_all_the_same() {
        let spawn = Spawn::running("retiring-a-dirty-spawn", "sleep 120");
        spawn.wrote("notes.md", "an hour of an agent's work\n");

        let (outcome, _) = spawn.retired();

        assert!(outcome.is_err(), "a dirty worktree was removed");
        assert!(spawn.worktree.join("notes.md").exists());
        assert!(
            !spawn.still_running(),
            "the refusal lands after the kill, which is the cost of checking a worktree \
             nothing is writing to"
        );
    }

    /// The whole of the case git's own removal would get wrong: a file the
    /// user's `status.showUntrackedFiles` hides, in a worktree the app is about
    /// to delete.
    #[test]
    fn an_untracked_file_the_users_own_git_would_not_see_still_refuses() {
        let spawn = Spawn::running("retiring-past-a-blind-config", "sleep 120");
        spawn.user_set("status.showUntrackedFiles", "no");
        spawn.wrote("notes.md", "never staged, and about to be deleted\n");

        let (outcome, _) = spawn.retired();

        assert!(
            outcome.is_err(),
            "the app deleted a file its own check could not see"
        );
        assert!(spawn.worktree.join("notes.md").exists());
    }

    #[test]
    fn work_the_spawn_committed_is_not_work_left_uncommitted() {
        let spawn = Spawn::running("retiring-a-spawn-that-committed", "sleep 120");
        spawn.wrote("notes.md", "the work, committed\n");
        spawn.committed("the work");

        spawn.retired().0.unwrap();

        assert!(!spawn.worktree.exists());
        assert!(spawn.branches().contains(BRANCH));
    }

    /// A session that has already stopped is retired like any other. Nothing to
    /// signal is not a special case — it is the state most retirements are in,
    /// because the reason you retire a spawn is usually that you watched it
    /// finish.
    #[test]
    fn a_session_that_has_already_stopped_is_retired_like_any_other() {
        let spawn = Spawn::running("retiring-a-stopped-spawn", "exit 0");
        spawn.tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        spawn.retired().0.unwrap();

        assert!(!spawn.worktree.exists());
        assert!(!spawn.pane_is_listed());
    }

    /// How long a test is willing to wait for a session that will not go. Short
    /// enough that the suite does not spend the real waits on every run, and
    /// the same shape as them: ask, then kill, then give up.
    const BRIEFLY: Duration = Duration::from_millis(250);

    /// The backstop. A session that ignores being asked is killed.
    #[test]
    fn a_session_that_ignores_being_asked_is_killed_anyway() {
        let spawn = Spawn::running(
            "retiring-past-a-deaf-session",
            "trap '' TERM; while :; do sleep 1; done",
        );

        stop(&spawn.tmux.server, &spawn.pane, BRIEFLY, BRIEFLY).unwrap();

        assert!(
            !spawn.still_running(),
            "a session that ignored SIGTERM outlived its retirement"
        );
    }

    /// **And what happens when the backstop is outlived too**, which is the one
    /// state where a worktree must not be touched: the app cannot say the agent
    /// has gone, so it says so and removes nothing. A pane taken away is not the
    /// process going — the signal `kill-pane` sends can be ignored exactly like
    /// the first one, and something still writing into a worktree is precisely
    /// what the order exists to rule out.
    #[test]
    fn a_session_that_outlives_even_the_kill_is_a_refusal_rather_than_a_removal() {
        let spawn = Spawn::running(
            "retiring-past-a-session-that-will-not-go",
            "trap '' TERM HUP; while :; do sleep 1; done",
        );
        let deaf = spawn.process().expect("a session to retire");

        let refused = stop(&spawn.tmux.server, &spawn.pane, BRIEFLY, BRIEFLY)
            .expect_err("a session that is still running was reported as gone");

        assert!(refused.to_string().contains("still running"), "{refused}");
        assert!(
            still_there(deaf).unwrap(),
            "the test's own premise failed: the session did go"
        );
        assert!(spawn.worktree.exists(), "the worktree was removed anyway");
        process::run("kill", &["-KILL", &deaf.to_string()]).unwrap();
    }

    /// The rule stated as a test: **what is about to happen is said before it
    /// happens.** A retirement that died the instant after the last thing it
    /// said has still said what it was in the middle of.
    #[test]
    fn what_it_is_about_to_do_is_said_before_it_is_done() {
        let spawn = Spawn::running("retiring-says-what-it-is-doing", "sleep 120");

        let (outcome, said) = spawn.retired();

        outcome.unwrap();
        let removing = said
            .iter()
            .position(|line| line.starts_with("removing the worktree"))
            .unwrap_or_else(|| panic!("nothing said the worktree was going: {said:?}"));
        assert_eq!(
            removing,
            said.len() - 1,
            "something was said after the last thing that was done: {said:?}"
        );
        assert!(
            said[removing].contains("add-retry-logic-a7f3"),
            "the record does not say which worktree went: {}",
            said[removing]
        );
    }

    /// What the app itself sees of a retirement: the same lines, down a
    /// channel, ending in the one that says there is nothing left to hold.
    #[test]
    fn a_retirement_on_a_thread_reports_what_it_did() {
        let spawn = Spawn::running("retiring-reports-back", "sleep 120");
        let (reporting, reports) = mpsc::channel();

        retiring(
            "add-retry-logic-a7f3".to_string(),
            spawn.pane.clone(),
            spawn.worktree.clone(),
            spawn.tmux.server.clone(),
            reporting,
        );

        let mut said = Vec::new();
        while let Ok(report) = reports.recv() {
            assert_eq!(report.spawn, "add-retry-logic-a7f3");
            said.push(match report.said {
                Said::Doing(doing) => doing,
                Said::Retired => "retired".to_string(),
                Said::Refused(why) => format!("refused: {why}"),
            });
        }

        assert_eq!(said.last().map(String::as_str), Some("retired"), "{said:?}");
        assert!(!spawn.worktree.exists());
    }

    // What the app holds about the retirements in flight, which is what its
    // rows are drawn from.

    /// One spawn, asked to be retired.
    fn asked_for() -> Retirements {
        let mut retirements = Retirements::default();
        assert!(retirements.asked_for("add-retry-logic-a7f3"));

        retirements
    }

    #[test]
    fn a_second_press_while_one_is_already_stopping_a_session_does_not_start_another() {
        let mut retirements = asked_for();
        retirements.doing("add-retry-logic-a7f3", "stopping the session".to_string());

        assert!(
            !retirements.asked_for("add-retry-logic-a7f3"),
            "a session was asked to stop twice"
        );
        assert_eq!(
            retirements.of("add-retry-logic-a7f3").map(Retirement::said),
            Some("stopping the session"),
            "the second press wrote over what the first one was doing"
        );
    }

    /// Which is what pressing it again after cleaning the worktree up is for.
    #[test]
    fn a_retirement_that_was_refused_can_be_asked_for_again() {
        let mut retirements = asked_for();
        retirements.refused("add-retry-logic-a7f3", "there is work in it".to_string());

        assert!(
            retirements
                .of("add-retry-logic-a7f3")
                .is_some_and(Retirement::refused)
        );
        assert!(retirements.asked_for("add-retry-logic-a7f3"));
        assert!(
            !retirements
                .of("add-retry-logic-a7f3")
                .is_some_and(Retirement::refused),
            "asking again left the row saying the last attempt had failed"
        );
    }

    #[test]
    fn a_spawn_that_has_gone_is_not_still_being_retired() {
        let mut retirements = asked_for();

        retirements.finished("add-retry-logic-a7f3");

        assert!(retirements.of("add-retry-logic-a7f3").is_none());
    }

    #[test]
    fn a_refusal_names_what_is_in_the_worktree_without_listing_all_of_it() {
        let all_of_it = (1..=10)
            .map(|which| format!("?? file-{which}"))
            .collect::<Vec<_>>()
            .join("\n");

        let named = named(&all_of_it);

        assert!(
            named.starts_with("?? file-1, ?? file-2, ?? file-3"),
            "{named}"
        );
        assert!(named.ends_with("and 7 more"), "{named}");
    }
}
