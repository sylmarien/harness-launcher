//! What the app leaves behind, and what it finds.
//!
//! **Quitting kills nothing.** tmux outlives the app, and that default is kept
//! deliberately: ending twenty agents mid-turn because somebody closed a viewer
//! would be the most destructive thing this app could do, and it would foreclose
//! the recovery a later tranche wants — there would be nothing left to recover.
//!
//! So litter is accepted. **Invisible litter is not**, and that is the whole of
//! what this module is for. On the way out the app says what it is leaving; on
//! the way in it says what it found. Neither is a feature: **nothing is adopted,
//! restored or recovered.** A report is a statement about the world, and the run
//! it belongs to carries on with an empty list either way.
//!
//! Both reports are the same look at the world, said two ways — which is why
//! there is one type here and two sentences. The looking is [`Litter::surveyed`];
//! everything else is pure.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::tmux::{self, Server};
use crate::worktrees;

/// What the app has running and on disk, at one moment.
pub struct Litter {
    /// The spawns still running in the session they are windows of, or nothing
    /// at all when there is no session — which is a machine that has not run
    /// this since it last started.
    running: Option<Vec<String>>,
    /// Where the worktrees go.
    root: PathBuf,
    /// What is under it, by name.
    worktrees: Vec<String>,
}

impl Litter {
    /// Look at the world, and say what is there.
    ///
    /// **The only thing in this module that touches anything.** It asks tmux
    /// what is running and reads what is under the worktree root, and it does
    /// nothing else — no session is made, no pane is opened, nothing is
    /// attached, nothing on disk is touched. That is what makes the reports
    /// statements rather than features: the same call serves the way in and the
    /// way out precisely because it has no side to take.
    pub fn surveyed(server: &Server, root: &Path) -> Result<Self> {
        Ok(Self {
            running: server.running()?,
            root: root.to_path_buf(),
            worktrees: worktrees::under(root),
        })
    }

    /// What to say on the way out.
    ///
    /// **The report exists because the app is about to leave all of this
    /// running**, so it says so first: quitting stopped nothing. Then the three
    /// things somebody would need to go and deal with any of it — the session to
    /// attach to, how much is in it, and where the worktrees are.
    ///
    /// **A claim about the act of leaving, not about the run.** "Nothing was
    /// stopped" is false of any run where a spawn was retired, and loudest in
    /// the run where every one of them was: the report would answer a screen
    /// full of deliberate retirements by insisting none of them happened. What
    /// quitting itself did is the promise being kept, it is true every time, and
    /// it is the only one of the two the reader is about to act on.
    ///
    /// The worktrees are given as the root rather than named one by one. On the
    /// way out the spawns are the ones this run started, so their names are
    /// already on the screen that is about to close; what is worth carrying away
    /// is where to look.
    ///
    /// **Leaving nothing running still says so**, and says it as "nothing"
    /// rather than as a count of none: every spawn retired is an ordinary way to
    /// finish, and "0 spawns are still running" reads like a tally where a
    /// sentence belongs.
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

    /// What to say on the way in, or nothing at all when there is nothing to
    /// say.
    ///
    /// **Everything is named, and a count would not do.** The app has just
    /// started and remembers nothing — that is the point of the report — so
    /// somebody reading it needs the strings that will let them find what it is
    /// talking about. A spawn, its branch and its worktree all carry the same
    /// name, which is what makes this possible at all.
    ///
    /// **It ends by saying that none of it is adopted**, because the natural
    /// reading of a list of running agents at start-up is that the app has
    /// picked them up. It has not, it will not, and the list beside it is about
    /// to be empty; leaving somebody to infer that from an empty list would be
    /// the report creating the confusion it exists to prevent.
    ///
    /// Nothing found says nothing. A first run on a clean machine has no reason
    /// to hear about the litter it has not made yet — and a session standing
    /// empty is the app's own furniture rather than something left behind.
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

/// So many spawns still running in the session, said the one way.
///
/// **Both reports open on this clause**, and it is one fact said twice rather
/// than two facts: how much is live, and which session it is live in. Written
/// out once because the two are read minutes apart by the same person — the way
/// in on the way in, the way out on the way out — and a count that pluralised
/// differently, or a session named differently, would read as two different
/// places.
fn still_running(how_many: usize) -> String {
    format!(
        "{} still running in the tmux session `{}`",
        counted(how_many, "spawn is", "spawns are"),
        tmux::SESSION
    )
}

/// So many of a thing, said so that one of them is not "1 spawns".
///
/// The caller supplies both readings because agreement runs past the noun: it
/// is "1 spawn **is** still running" against "2 spawns **are**", and a helper
/// that only pluralised the noun would leave the verb wrong at every call site
/// that needed one.
fn counted(how_many: usize, one: &str, many: &str) -> String {
    let thing = if how_many == 1 { one } else { many };

    format!("{how_many} {thing}")
}

/// Something said on the way out of the scope it is in, however that scope is
/// left.
///
/// **A value rather than a line at the end of a function**, because there is no
/// single end to put that line at. From the moment the app has a session it has
/// agents running that outlive it, and every way out from there leaves them
/// running: the app quitting on purpose, a refusal on the way up with three of
/// four spawns already started, or the app falling over. A line after the last
/// of those covers exactly one, and the two it misses are the two nobody is
/// expecting — which is precisely when a silent exit reads as *there was nothing
/// to leave*.
///
/// It holds the saying rather than the sentence: a report taken when the guard
/// was made would describe the world at the wrong moment, and the whole value of
/// this one is that it is a look at the world taken on the way out.
pub struct Leaving<R: FnOnce()> {
    /// Taken on the way out, which is the only time there is to use it.
    report: Option<R>,
}

impl<R: FnOnce()> Leaving<R> {
    /// Arrange for this to be said on the way out, whichever way out it is.
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

    /// The shape a spawn's pane is born in. Nothing here looks at a screen; it
    /// is only what a session has to be given to exist.
    const SLOT: Size = Size {
        columns: 61,
        rows: 17,
    };

    /// A look at the world that a test writes rather than takes, so the
    /// sentences can be pinned without a tmux or a filesystem.
    fn litter(running: Option<&[&str]>, worktrees: &[&str]) -> Litter {
        Litter {
            running: running.map(|names| names.iter().map(|n| (*n).to_string()).collect()),
            root: PathBuf::from("/data/harness-launcher/worktrees"),
            worktrees: worktrees.iter().map(|n| (*n).to_string()).collect(),
        }
    }

    /// The way in has a harder job than the way out: this app has just started
    /// and remembers nothing, so a count would leave somebody with no way to
    /// find what it is talking about. The artifacts name themselves — a spawn,
    /// its branch and its worktree are all the same string — so the report
    /// names them.
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

    /// The two halves of a survey, taken from the real world rather than
    /// written down: a real tmux holding a real running spawn, and a real
    /// directory holding a worktree from some earlier run that nothing is
    /// running for any more.
    ///
    /// The second one is the case that matters most on the way in — a spawn
    /// whose session died with a reboot leaves its worktree behind, and that is
    /// litter nothing else would ever mention.
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

    /// Quitting with nothing running — every spawn retired, or none ever
    /// started. It is a real way to leave and the sentence has to read like
    /// one: "0 spawns are still running" is a count where a plain "nothing is"
    /// belongs, and it sits oddly beside the promise it is there to keep.
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

    /// The commonest run there is, and it must be silent. **A session standing
    /// empty is not litter** — it is the app's own furniture, made by the last
    /// run and reused by this one, and reporting it would train somebody to
    /// ignore the report that matters.
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

    /// One of a thing is not "1 spawns". Cheap to get wrong and read by
    /// somebody every single time they quit.
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

    /// What the acceptance criteria ask the way out to say, in order: the
    /// holding session's name, how many spawns are live, and the worktree root.
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

    /// **The one thing the way out must not claim.** Retiring spawns with `F9`
    /// is an ordinary way to spend a run, and a report answering a screenful of
    /// deliberate retirements with "nothing was stopped" is false about the only
    /// thing the reader watched happen — loudest in the run that retired every
    /// one of them. What *quitting* did is the promise being kept, and it is the
    /// only half of it that is true every time.
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

    /// **The way out the report exists for is not the only way out there is.**
    /// A refusal with spawns already started — the third of four failing to
    /// start — leaves two agents running and a shell told nothing about them,
    /// which is exactly the surprise the report is there to prevent. So it is
    /// made on the way out of the scope rather than after the last thing in it.
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

    /// And falling over is when somebody most needs telling that twenty agents
    /// are still going: the app has just vanished from under them, and the one
    /// thing that is *not* true is that it took the work with it.
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

    /// Said once, not once per way out: an ordinary return is a way out too, and
    /// a report made twice would have somebody looking for a second session.
    #[test]
    fn a_scope_left_the_ordinary_way_says_it_once() {
        let said = Cell::new(0);
        {
            let _leaving = Leaving::saying(|| said.set(said.get() + 1));
        }

        assert_eq!(said.get(), 1);
    }
}
