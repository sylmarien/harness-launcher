//! Retiring a spawn: the one act that releases what the app created.
//! Always explicit — never inferred from a silent agent.
//!
//! The order is strict: stop the process, confirm it is gone, check the
//! worktree is clean, remove the worktree. A cleanliness check against a live
//! agent races a file the agent writes on its way out, and losing that race
//! deletes work. A dirty worktree refuses; clean it up and retire again.
//! Accepted cost: the refusal lands after the kill, so a dirty spawn ends up
//! stopped. The branch stays — it holds committed work.
//! See docs/developers/components/retirement.md.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::tmux::Server;
use crate::{git, process};

/// How long a session is given to stop after being asked to: bounded so a
/// deaf session cannot hang a retirement, long enough for a harness to save
/// state on the way out.
const PATIENCE: Duration = Duration::from_secs(3);

/// How long the pane is given to go once killed outright — the round trip.
const CLOSING: Duration = Duration::from_secs(2);

/// How often the app looks to see whether it has gone yet.
const LOOKING: Duration = Duration::from_millis(25);

/// How many entries of a dirty worktree a refusal names.
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
    /// The session is stopped and the worktree is gone.
    Retired,
    /// It stopped, and this is why. The spawn stays exactly where it was.
    Refused(String),
}

/// Where the retirements in flight have got to, one per spawn.
///
/// Held by the app because progress is drawn on the spawn's row every frame.
#[derive(Default)]
pub struct Retirements {
    /// One per spawn that is being retired, or whose retirement was refused.
    of: HashMap<String, Retirement>,
}

/// Where one spawn's retirement has got to. Two states only: a finished
/// retirement leaves nothing behind to be in a state.
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
    /// A no-op while one is under way — a second press must not stop the
    /// session twice. A refused retirement is started again.
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
/// On a thread because stopping a session takes as long as the session takes
/// to stop; progress comes back as reports. A dropped receiver means the app
/// is exiting; at worst that leaves a stopped session with its worktree still
/// there — the same litter quitting with a spawn running leaves.
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
/// Each step is announced before it is attempted, so a retirement that dies
/// half way has said what it was in the middle of.
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

    // The window closes last: until the worktree is gone, the pane is what a
    // refusal leaves the user looking at. A pane the backstop already took is
    // the outcome asked for, so the error is ignored.
    let _ = server.close(pane);

    Ok(())
}

/// Stop what a pane is running, and do not return until it is gone.
///
/// `SIGTERM` first so the harness can save state, then a bounded wait, then
/// `kill-pane` as the backstop. Whether the process is gone is settled by
/// tmux, never by `kill`'s return — a signal to a just-exited process fails,
/// and that failure is the answer wanted. The process is asked about directly
/// (`kill -0`) only after the backstop, because a pid can be reused. The two
/// waits are parameters so tests can shorten them.
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

/// Whether a process is still there, asked without disturbing it (signal zero
/// checks and sends nothing). Accepted cost: this sees only the process the
/// app started, not any child a harness leaves behind.
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

    /// The shape of a slot in these tests.
    const SLOT: Size = Size {
        columns: 40,
        rows: 10,
    };

    /// A spawn as retirement finds one: a session running in a worktree of its
    /// own.
    struct Spawn {
        /// The tmux server the session runs on, which is this test's alone.
        tmux: PrivateTmux,
        /// The repository and the worktree, made the way the app makes them.
        spawned: Spawned,
        /// The pane the session is running in.
        pane: String,
    }

    impl Deref for Spawn {
        type Target = Spawned;

        fn deref(&self) -> &Spawned {
            &self.spawned
        }
    }

    impl Spawn {
        /// Start one, running a shell script as a stand-in for the harness
        /// (the real one costs tokens and needs credentials).
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
            // Wait until the script is really running: signalling it before
            // its `trap` is set would be a race in the test.
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

    /// The marker a session writes when asked to stop rather than killed —
    /// written outside the worktree, so writing it does not dirty the worktree.
    const ASKED: &str = "asked-to-stop";

    /// A session that exits politely on `SIGTERM`, leaving the marker.
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

    /// The session writes a file as it is being stopped, as an agent asked to
    /// finish does; a check taken before the kill would have missed it.
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

    /// Accepted cost: a spawn that turns out to be dirty ends up stopped all
    /// the same.
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

    /// The case git's own removal gets wrong: an untracked file hidden by
    /// `status.showUntrackedFiles=no`.
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

    #[test]
    fn a_session_that_has_already_stopped_is_retired_like_any_other() {
        let spawn = Spawn::running("retiring-a-stopped-spawn", "exit 0");
        spawn.tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        spawn.retired().0.unwrap();

        assert!(!spawn.worktree.exists());
        assert!(!spawn.pane_is_listed());
    }

    /// How long a test waits for a session that will not go, so the suite does
    /// not spend the real waits on every run.
    const BRIEFLY: Duration = Duration::from_millis(250);

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

    /// When even the backstop is outlived, nothing is removed: the `SIGHUP`
    /// from `kill-pane` can be ignored like `SIGTERM`, and the process may
    /// still be writing into the worktree.
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
