//! What the app leaves behind, and what it finds.
//!
//! Quitting kills nothing, on purpose: ending agents mid-turn because somebody
//! closed a viewer would be the most destructive thing this app could do. So
//! litter is accepted — invisible litter is not. The app reports what it found
//! at start-up and what it leaves at exit, and a report only states the world:
//! nothing is adopted, restored or recovered. The looking is
//! [`Litter::surveyed`]; everything else is pure.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::tmux::{self, Server};
use crate::worktrees;

/// What the app has running and on disk, at one moment.
pub struct Litter {
    /// The spawns still running, or `None` when there is no session at all.
    running: Option<Vec<String>>,
    /// Where the worktrees go.
    root: PathBuf,
    /// What is under it, by name.
    worktrees: Vec<String>,
}

impl Litter {
    /// Look at the world, and say what is there — the only thing in this
    /// module that touches anything, and it only reads.
    pub fn surveyed(server: &Server, root: &Path) -> Result<Self> {
        Ok(Self {
            running: server.running()?,
            root: root.to_path_buf(),
            worktrees: worktrees::under(root),
        })
    }

    /// What to say at exit: quitting stopped nothing, what is still running,
    /// and where the worktrees are.
    ///
    /// It claims only what quitting itself did — "nothing was stopped" would
    /// be false of any run where a spawn was deliberately retired.
    pub fn leaving(&self) -> String {
        let root = self.root.display();

        let Some(running) = &self.running else {
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

        format!(
            "quitting stopped nothing: {}, with worktrees under {root}",
            still_running(running.len())
        )
    }

    /// What to say at start-up, or `None` when there is nothing to say.
    ///
    /// Everything is named — the app remembers nothing, so a count would leave
    /// the reader unable to find any of it — and it ends by saying none of it
    /// is adopted, because a list of running agents would otherwise read as
    /// picked up.
    pub fn found(&self) -> Option<String> {
        let running: &[String] = self.running.as_deref().unwrap_or_default();
        if running.is_empty() && self.worktrees.is_empty() {
            return None;
        }

        let mut said = vec!["found from an earlier run, and left alone:".to_string()];
        if !running.is_empty() {
            said.push(format!(
                "  {}: {}",
                still_running(running.len()),
                running.join(", ")
            ));
        }
        if !self.worktrees.is_empty() {
            said.push(format!(
                "  {} under {}: {}",
                counted(self.worktrees.len(), "worktree", "worktrees"),
                self.root.display(),
                self.worktrees.join(", ")
            ));
        }
        said.push(
            "none of it is adopted — this run starts with an empty list, and \
             anything above is yours to deal with."
                .to_string(),
        );

        Some(said.join("\n"))
    }
}

/// So many spawns still running in the session, phrased the same way in both
/// reports.
fn still_running(how_many: usize) -> String {
    format!(
        "{} still running in the tmux session `{}`",
        counted(how_many, "spawn is", "spawns are"),
        tmux::SESSION
    )
}

/// So many of a thing, without "1 spawns". The caller supplies both readings
/// because agreement runs past the noun ("1 spawn is" / "2 spawns are").
fn counted(how_many: usize, one: &str, many: &str) -> String {
    let thing = if how_many == 1 { one } else { many };

    format!("{how_many} {thing}")
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
    use std::fs;
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

    /// A survey a test writes rather than takes, so the sentences can be
    /// pinned without a tmux or a filesystem.
    fn litter(running: Option<&[&str]>, worktrees: &[&str]) -> Litter {
        Litter {
            running: running.map(|names| names.iter().map(|n| (*n).to_string()).collect()),
            root: PathBuf::from("/data/harness-launcher/worktrees"),
            worktrees: worktrees.iter().map(|n| (*n).to_string()).collect(),
        }
    }

    #[test]
    fn the_way_in_names_what_it_found_and_says_none_of_it_is_taken_over() {
        let said = litter(
            Some(&["add-retry-logic-a7f3"]),
            &["add-retry-logic-a7f3", "work-1a2b"],
        )
        .found()
        .expect("something was found and not reported");

        assert!(said.contains("add-retry-logic-a7f3"), "{said}");
        assert!(
            said.contains("work-1a2b"),
            "a worktree with no session left was not named: {said}"
        );
        assert!(said.contains("spawns"), "the session is not named: {said}");
        assert!(
            said.contains("/data/harness-launcher/worktrees"),
            "the worktree root is not given: {said}"
        );
        assert!(
            said.contains("adopt"),
            "the report does not say that none of this is being taken over: {said}"
        );
    }

    #[test]
    fn a_survey_reads_the_session_and_the_root_as_they_really_are() {
        let tmux = PrivateTmux::start("litter-surveys-the-world");
        let session = tmux.server.session(SLOT).unwrap();
        let pane = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        tmux.server.start(&pane, &tmux.recipe("sleep 120")).unwrap();
        let somewhere = tempdir().unwrap();
        let root = somewhere.path().join("worktrees");
        fs::create_dir_all(root.join("left-from-a-reboot-c3d8")).unwrap();

        let said = Litter::surveyed(&tmux.server, &root)
            .unwrap()
            .found()
            .expect("a running spawn and a leftover worktree, and nothing said");

        assert!(
            said.contains("add-retry-logic-a7f3"),
            "the spawn that is really running was not found: {said}"
        );
        assert!(
            said.contains("left-from-a-reboot-c3d8"),
            "the worktree really on disk was not found: {said}"
        );
    }

    #[test]
    fn leaving_nothing_running_reads_as_nothing_rather_than_as_a_count_of_none() {
        let said = litter(Some(&[]), &[]).leaving();

        assert!(!said.contains('0'), "nothing was counted as none: {said}");
        assert!(said.contains("spawns"), "the session is not named: {said}");
        assert!(
            said.contains("/data/harness-launcher/worktrees"),
            "the worktree root is not given: {said}"
        );
    }

    #[test]
    fn a_machine_with_nothing_left_on_it_hears_nothing_on_the_way_in() {
        assert_eq!(
            litter(None, &[]).found(),
            None,
            "a machine that has never run this"
        );
        assert_eq!(
            litter(Some(&[]), &[]).found(),
            None,
            "a session standing empty was reported as something left behind"
        );
    }

    #[test]
    fn one_of_a_thing_reads_as_one_of_a_thing() {
        let one = litter(Some(&["add-retry-logic-a7f3"]), &["add-retry-logic-a7f3"]);

        assert!(one.leaving().contains("1 spawn is"), "{}", one.leaving());
        assert!(
            !one.leaving().contains("their"),
            "one spawn was given a plural pronoun: {}",
            one.leaving()
        );
        let found = one.found().unwrap();
        assert!(found.contains("1 spawn is"), "{found}");
        assert!(found.contains("1 worktree "), "{found}");
    }

    #[test]
    fn the_way_out_names_the_session_counts_the_spawns_and_says_where_the_worktrees_are() {
        let said = litter(
            Some(&["add-retry-logic-a7f3", "fix-the-flake-b2c9"]),
            &["add-retry-logic-a7f3", "fix-the-flake-b2c9"],
        )
        .leaving();

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
            litter(Some(&["add-retry-logic-a7f3"]), &[]).leaving(),
            litter(Some(&[]), &[]).leaving(),
            litter(None, &[]).leaving(),
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
