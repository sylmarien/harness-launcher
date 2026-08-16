//! What the app leaves behind at exit.
//!
//! Quitting kills nothing, on purpose: ending agents mid-turn because somebody
//! closed a viewer would be the most destructive thing this app could do. So
//! litter is accepted, and invisible litter is not. The report counts what is
//! still running and claims only what quitting itself did. What the next run
//! makes of the same litter is [`crate::adoption`]. The looking is
//! [`surveyed`]; everything else is pure.

use std::path::Path;

use crate::error::Result;
use crate::tmux::{self, Server};

/// The spawns still running, or `None` when there is no session at all.
///
/// The only thing in this module that touches anything, and it only reads.
pub fn surveyed(server: &Server) -> Result<Option<Vec<String>>> {
    Ok(server.windows()?.map(|windows| {
        windows
            .into_iter()
            .filter(|window| !window.dead)
            .map(|window| window.name)
            .collect()
    }))
}

/// What to say at exit: quitting stopped nothing, what is still running, and
/// where the worktrees are.
///
/// It claims only what quitting itself did — "nothing was stopped" would be
/// false of any run where a spawn was deliberately retired.
pub fn leaving(running: Option<&[String]>, root: &Path) -> String {
    let root = root.display();

    let Some(running) = running else {
        return format!(
            "quitting stopped nothing, and the tmux session `{}` is no longer there — \
             whatever was in it has gone with it. Worktrees are under {root}",
            tmux::SESSION
        );
    };

    if running.is_empty() {
        return format!(
            "quitting stopped nothing, and nothing was left running — the tmux session \
             `{}` is standing empty. Worktrees are under {root}",
            tmux::SESSION
        );
    }

    // Agreement runs past the noun, so both readings are written out.
    format!(
        "quitting stopped nothing: {} {} still running in the tmux session `{}`, with \
         worktrees under {root}",
        running.len(),
        if running.len() == 1 {
            "spawn is"
        } else {
            "spawns are"
        },
        tmux::SESSION
    )
}

/// Something said when its scope is left, however it is left — an ordinary
/// return, a refusal part-way, or a panic; a line at the end of a function
/// covers only the first. It holds the saying rather than the sentence, so the
/// report describes the world at the moment of leaving.
pub struct Leaving<R: FnOnce()> {
    report: Option<R>,
}

impl<R: FnOnce()> Leaving<R> {
    /// Arrange for this to be said when the scope is left, however it is left.
    pub fn saying(report: R) -> Self {
        Self {
            report: Some(report),
        }
    }
}

impl<R: FnOnce()> Drop for Leaving<R> {
    fn drop(&mut self) {
        if let Some(report) = self.report.take() {
            report();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::panic::{self, AssertUnwindSafe};

    use tempfile::tempdir;

    use crate::error::Error;

    use crate::screen::Size;
    use crate::tmux::tests::PrivateTmux;

    /// What a session has to be given to exist; nothing here looks at a screen.
    const SLOT: Size = Size {
        columns: 61,
        rows: 17,
    };

    /// The exit sentence for a survey a test writes rather than takes, so the
    /// wording can be pinned without a tmux.
    fn said(running: Option<&[&str]>) -> String {
        let running: Option<Vec<String>> =
            running.map(|names| names.iter().map(|name| (*name).to_string()).collect());

        leaving(
            running.as_deref(),
            Path::new("/data/harness-launcher/worktrees"),
        )
    }

    #[test]
    fn a_survey_counts_the_spawns_that_are_really_still_running() {
        let tmux = PrivateTmux::start("litter-surveys-the-world");
        let session = tmux.server.session(SLOT).unwrap();
        let going = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        tmux.server
            .start(&going, &tmux.recipe("sleep 120"))
            .unwrap();
        let stopped = tmux
            .server
            .open_window(&session, "drop-the-cache-d4e1")
            .unwrap();
        tmux.server.start(&stopped, &tmux.recipe("exit 3")).unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));
        let root = tempdir().unwrap();

        let running = surveyed(&tmux.server).unwrap();
        let said = leaving(running.as_deref(), root.path());

        assert!(
            said.contains("1 spawn is"),
            "the spawn that is really running was not counted, or the stopped one was: {said}"
        );
    }

    #[test]
    fn leaving_nothing_running_reads_as_nothing_rather_than_as_a_count_of_none() {
        let said = said(Some(&[]));

        assert!(!said.contains('0'), "nothing was counted as none: {said}");
        assert!(said.contains("spawns"), "the session is not named: {said}");
        assert!(
            said.contains("/data/harness-launcher/worktrees"),
            "the worktree root is not given: {said}"
        );
    }

    #[test]
    fn one_of_a_thing_reads_as_one_of_a_thing() {
        let one = said(Some(&["add-retry-logic-a7f3"]));

        assert!(one.contains("1 spawn is"), "{one}");
        assert!(
            !one.contains("their"),
            "one spawn was given a plural pronoun: {one}"
        );
    }

    #[test]
    fn the_way_out_names_the_session_counts_the_spawns_and_says_where_the_worktrees_are() {
        let said = said(Some(&["add-retry-logic-a7f3", "fix-the-flake-b2c9"]));

        assert!(said.contains("spawns"), "the session is not named: {said}");
        assert!(
            said.contains('2'),
            "the live spawns are not counted: {said}"
        );
        assert!(
            said.contains("/data/harness-launcher/worktrees"),
            "the worktree root is not given: {said}"
        );
    }

    #[test]
    fn the_way_out_claims_only_that_quitting_stopped_nothing() {
        for said in [
            said(Some(&["add-retry-logic-a7f3"])),
            said(Some(&[])),
            said(None),
        ] {
            assert!(
                !said.contains("nothing was stopped"),
                "a run that retired every spawn is told none of it happened: {said}"
            );
            assert!(
                said.contains("quitting stopped nothing"),
                "the promise that quitting kills nothing is not kept: {said}"
            );
        }
    }

    #[test]
    fn a_scope_left_by_a_refusal_still_says_what_is_being_left_behind() {
        let said = Cell::new(0);

        let refused = || -> Result<()> {
            let _leaving = Leaving::saying(|| said.set(said.get() + 1));
            Err(Error::new(
                "the third of four spawns would not start".to_string(),
            ))?;

            unreachable!("the refusal above is the way out of this scope")
        };

        assert!(refused().is_err());
        assert_eq!(
            said.get(),
            1,
            "a refusal left with agents still running and said nothing about them"
        );
    }

    #[test]
    fn a_scope_left_by_a_crash_still_says_what_is_being_left_behind() {
        let said = Cell::new(0);

        let fell_over = panic::catch_unwind(AssertUnwindSafe(|| {
            let _leaving = Leaving::saying(|| said.set(said.get() + 1));

            panic!("the app falling over, which is what this test is about");
        }));

        assert!(fell_over.is_err(), "the panic did not happen");
        assert_eq!(
            said.get(),
            1,
            "the app fell over and said nothing about what it left running"
        );
    }

    #[test]
    fn a_scope_left_the_ordinary_way_says_it_once() {
        let said = Cell::new(0);
        {
            let _leaving = Leaving::saying(|| said.set(said.get() + 1));
        }

        assert_eq!(said.get(), 1);
    }
}
