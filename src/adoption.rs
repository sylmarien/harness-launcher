//! Taking over the spawns an earlier run left running.
//!
//! Quitting kills nothing, so a session outlives the run that made it. This
//! module reads that session at start-up and puts what it finds into the
//! list. It closes the panes whose spawns have already stopped, and it
//! removes no worktree: one can hold work that exists nowhere else.
//! See docs/developers/components/starting-and-leaving.md.

use std::path::Path;

use crate::app::Spawn;
use crate::control::{Client, POISONED};
use crate::creation::Started;
use crate::error::Result;
use crate::list::Entry;
use crate::process::path_argument;
use crate::screen::Size;
use crate::snapshot::Watched;
use crate::tmux::Server;
use crate::{git, names, worktrees};

/// What an adopted spawn's screen says before anything arrives.
///
/// The grid starts blank: a control client streams only what is produced
/// while it is attached, and this one attached after the spawn drew its
/// screen. A blank slot would read as an agent that has produced nothing, so
/// the app writes down what it does not have. A full-screen redraw by the
/// spawn covers it. A spawn that only streams lines leaves it on screen above
/// them, and an idle agent redraws nothing at all.
const JOINED: &[u8] = b"The app started after this spawn did.\r\n\
                        This screen holds only what the spawn has drawn since.\r\n";

/// What this run made of the session an earlier run left behind.
#[derive(Default)]
pub struct Adopted {
    /// The spawns taken over, for the list and for the supervisor.
    pub started: Vec<Started>,
    /// The spawns whose pane was closed because they had already stopped.
    closed: Vec<String>,
    /// The spawns whose pane had already stopped and would not close, and why
    /// each close failed.
    unclosed: Vec<Unadopted>,
    /// The spawns left running, and why each one could not be taken over.
    left: Vec<Unadopted>,
}

/// A spawn in the session that this run did not take into the list.
struct Unadopted {
    /// The spawn's name.
    name: String,
    /// What went wrong, in the words of whatever refused.
    why: String,
}

impl Adopted {
    /// What to say at start-up, or `None` when an earlier run left nothing.
    ///
    /// Everything is named. The app remembers nothing between runs, so a
    /// count would leave the reader unable to find any of it. `created` names
    /// the spawns this run made, whose worktrees are not leftovers.
    pub fn found(&self, root: &Path, created: &[String]) -> Option<String> {
        let mut said = Vec::new();

        if let Some(taken) = named(
            self.started
                .iter()
                .map(|started| &started.spawn.entry.spawn),
        ) {
            said.push(format!("  taken into the list: {taken}"));
        }
        if let Some(closed) = named(self.closed.iter()) {
            said.push(format!(
                "  closed, because the spawn had already stopped: {closed}"
            ));
        }
        said.extend(explained(
            "already stopped, and the pane would not close",
            &self.unclosed,
        ));
        said.extend(explained(
            "still running, and left out of the list because the app cannot say what they are",
            &self.left,
        ));
        if let Some(leftover) = named(self.leftover(root, created).iter()) {
            said.push(format!(
                "  left on disk under {}: {leftover}",
                root.display()
            ));
        }

        if said.is_empty() {
            return None;
        }

        said.insert(0, "from an earlier run:".to_string());

        Some(said.join("\n"))
    }

    /// The worktrees under the root that no spawn in this run's list works in.
    /// Reported, never removed — see this module's own note.
    fn leftover(&self, root: &Path, created: &[String]) -> Vec<String> {
        let mine: Vec<&str> = self
            .started
            .iter()
            .map(|started| started.spawn.entry.spawn.as_str())
            .chain(self.left.iter().map(|left| left.name.as_str()))
            .chain(created.iter().map(String::as_str))
            .collect();

        worktrees::under(root)
            .into_iter()
            .filter(|worktree| !mine.contains(&worktree.as_str()))
            .collect()
    }
}

/// Names in one line, or `None` when there are none.
fn named<'a>(names: impl Iterator<Item = &'a String>) -> Option<String> {
    let listed: Vec<&str> = names.map(String::as_str).collect();

    (!listed.is_empty()).then(|| listed.join(", "))
}

/// One heading, and a line per spawn under it saying what went wrong. Nothing
/// at all when there are none.
fn explained(heading: &str, spawns: &[Unadopted]) -> Vec<String> {
    if spawns.is_empty() {
        return Vec::new();
    }

    let mut said = vec![format!("  {heading}:")];
    said.extend(
        spawns
            .iter()
            .map(|spawn| format!("    {}: {}", spawn.name, spawn.why)),
    );

    said
}

/// Take over every spawn the session is still running.
///
/// Call after the control client is attached: a spawn with no grid behind its
/// pane loses everything it produces.
pub fn adopt(server: &Server, client: &Client, root: &Path, slot: Size) -> Result<Adopted> {
    let mut adopted = Adopted::default();
    let Some(windows) = server.windows()? else {
        return Ok(adopted);
    };

    for window in windows {
        // A dead pane holds nothing and shows up in every listing, so it is
        // closed. The worktree beside it is left alone: it can hold work. One
        // pane that will not close is reported, never a start-up refused.
        if window.dead {
            match server.close(&window.pane) {
                Ok(()) => adopted.closed.push(window.name),
                Err(why) => adopted.unclosed.push(Unadopted {
                    name: window.name,
                    why: why.to_string(),
                }),
            }
            continue;
        }

        // Left running, not stopped: a spawn the app cannot describe is still
        // an agent doing work.
        let entry = match described(&window.name, root) {
            Ok(entry) => entry,
            Err(why) => {
                adopted.left.push(Unadopted {
                    name: window.name,
                    why: why.to_string(),
                });
                continue;
            }
        };

        let grid = client.watch(&window.pane, slot);
        grid.lock().expect(POISONED).apply(JOINED);
        adopted.started.push(Started {
            watched: Watched::already_running(window.name.clone(), window.pane.clone()),
            spawn: Spawn {
                entry,
                pane: window.pane,
                grid,
            },
        });
    }

    Ok(adopted)
}

/// What the list has to say about a spawn only tmux knew about.
///
/// The name gives the worktree and the branch, the way it does for a spawn
/// this run makes. Only the repository is read from the world, and a worktree
/// git will not name a repository for is a refusal.
fn described(name: &str, root: &Path) -> Result<Entry> {
    let worktree = root.join(name);

    Ok(Entry {
        repository: git::worktree_repository(&worktree)?.name().to_string(),
        spawn: name.to_string(),
        branch: names::branch_name(name),
        worktree: path_argument(&worktree)?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::time::Instant;
    use tempfile::{TempDir, tempdir};

    use crate::control::Client;
    use crate::creation::{self, Wanted};
    use crate::git::tests::{git, repository_with_origin};
    use crate::process;
    use crate::screen::Size;
    use crate::screen::tests::shown;
    use crate::snapshot::{self, Snapshot, Status};
    use crate::tmux::tests::PrivateTmux;

    /// The shape of a slot in these tests.
    const SLOT: Size = Size {
        columns: 40,
        rows: 10,
    };

    /// An earlier run: a worktree on its own branch, and a window running in
    /// it. Hands back the spawn's name.
    fn earlier_run(
        tmux: &PrivateTmux,
        session: &str,
        root: &TempDir,
        repository: &TempDir,
    ) -> String {
        left_running(tmux, session, root, repository, "sleep 120")
    }

    /// The same, with a say in what the spawn runs.
    fn left_running(
        tmux: &PrivateTmux,
        session: &str,
        root: &TempDir,
        repository: &TempDir,
        script: &str,
    ) -> String {
        let plan = creation::plan(
            &Wanted {
                repository: repository.path().join("project"),
                work: "add retry logic".to_string(),
                answers: Vec::new(),
            },
            root.path(),
        )
        .unwrap();
        plan.create().unwrap();
        let name = plan.entry.spawn.clone();
        let pane = tmux.server.open_window(session, &name).unwrap();
        tmux.server.start(&pane, &tmux.recipe(script)).unwrap();

        name
    }

    #[test]
    fn a_spawn_an_earlier_run_left_running_is_in_this_runs_list() {
        let tmux = PrivateTmux::start("adoption-takes-over");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        let name = earlier_run(&tmux, &session, &root, &repository);
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        assert_eq!(adopted.started.len(), 1, "the spawn was not taken over");
        let entry = &adopted.started[0].spawn.entry;
        assert_eq!(entry.spawn, name);
        assert_eq!(entry.repository, "project");
        assert_eq!(entry.branch, format!("spawn/{name}"));
        assert_eq!(
            entry.worktree,
            root.path().join(&name).to_str().unwrap(),
            "the spawn was not given the worktree its name says it works in"
        );
        assert_eq!(adopted.started[0].watched.name, name);
        assert_eq!(
            adopted.started[0].watched.pane,
            adopted.started[0].spawn.pane
        );
    }

    /// The measured limit from tranche 1 §4.9: a control client streams only
    /// what is produced while it is attached, so an adopted spawn's grid
    /// starts with nothing in it. The app says so rather than showing a blank
    /// screen that reads as an agent with nothing to say.
    #[test]
    fn an_adopted_spawns_screen_says_the_app_only_has_what_arrived_since() {
        let tmux = PrivateTmux::start("adoption-says-it-joined-late");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        earlier_run(&tmux, &session, &root, &repository);
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        let screen = adopted.started[0].spawn.grid.lock().unwrap();
        let said = shown(&screen).join("\n");
        assert!(
            said.contains("started after this spawn did"),
            "an adopted spawn's screen does not say the app joined it mid-run: {said}"
        );
        // A fragment, not the sentence: the emulator wraps it in a slot this
        // narrow, and where it wraps is not what this test is about.
        assert!(
            said.contains("This screen holds only"),
            "an adopted spawn's screen does not say what is missing from it: {said}"
        );
    }

    /// `remain-on-exit` leaves a pane behind when its process stops. There is
    /// nothing in one, and it shows up in every listing until somebody closes
    /// it.
    #[test]
    fn a_pane_whose_spawn_had_already_stopped_is_closed_and_its_worktree_kept() {
        let tmux = PrivateTmux::start("adoption-closes-dead-panes");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        let name = left_running(&tmux, &session, &root, &repository, "exit 3");
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        assert!(
            adopted.started.is_empty(),
            "a spawn that had already stopped was put in the list"
        );
        assert_eq!(
            tmux.server.windows().unwrap(),
            Some(Vec::new()),
            "the dead pane is still on the server"
        );
        assert!(
            root.path().join(&name).is_dir(),
            "the stopped spawn's worktree was removed, and it can hold work"
        );
    }

    /// A worktree an agent left on a detached HEAD still names its spawn, and
    /// the spawn is still an agent doing work. The branch comes from the name,
    /// which is where every branch this app makes comes from.
    #[test]
    fn a_spawn_whose_worktree_is_on_no_branch_is_still_taken_into_the_list() {
        let tmux = PrivateTmux::start("adoption-takes-a-detached-head");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        let name = earlier_run(&tmux, &session, &root, &repository);
        git(&[
            "-C",
            root.path().join(&name).to_str().unwrap(),
            "checkout",
            "--detach",
        ]);
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        assert_eq!(
            adopted.started.len(),
            1,
            "an agent still doing work was dropped out of the list because git would not \
             name the branch its worktree is on"
        );
        assert_eq!(
            adopted.started[0].spawn.entry.branch,
            format!("spawn/{name}")
        );
    }

    /// A spawn with no worktree under the root cannot be given a repository or
    /// a branch. The list groups by repository, so a guess would file it under
    /// the wrong heading.
    #[test]
    fn a_running_spawn_the_app_cannot_describe_is_left_running_rather_than_guessed_at() {
        let tmux = PrivateTmux::start("adoption-refuses-to-guess");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let pane = tmux.server.open_window(&session, "work-1a2b").unwrap();
        tmux.server.start(&pane, &tmux.recipe("sleep 120")).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        assert!(
            adopted.started.is_empty(),
            "a spawn the app knows nothing about was put in the list anyway"
        );
        assert!(
            !tmux
                .server
                .panes()
                .unwrap()
                .get(&pane)
                .expect("the spawn's pane was taken off the server")
                .dead,
            "a spawn the app could not describe was stopped"
        );
    }

    #[test]
    fn the_report_names_what_was_taken_what_was_closed_and_what_is_left_on_disk() {
        let tmux = PrivateTmux::start("adoption-reports-what-it-did");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        let taken = earlier_run(&tmux, &session, &root, &repository);
        let stopped = left_running(&tmux, &session, &root, &repository, "exit 3");
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));
        let undescribable = tmux.server.open_window(&session, "work-1a2b").unwrap();
        tmux.server
            .start(&undescribable, &tmux.recipe("sleep 120"))
            .unwrap();
        fs::create_dir_all(root.path().join("old-thing-9f2a")).unwrap();
        fs::create_dir_all(root.path().join("made-this-run-c4d5")).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();
        let said = adopted
            .found(root.path(), &["made-this-run-c4d5".to_string()])
            .expect("an earlier run left four things behind, and nothing was said");

        assert!(
            said.contains(&format!("taken into the list: {taken}")),
            "{said}"
        );
        assert!(
            said.contains(&format!("had already stopped: {stopped}")),
            "{said}"
        );
        assert!(said.contains("work-1a2b: "), "{said}");
        assert!(said.contains("old-thing-9f2a"), "{said}");
        assert!(
            !said.contains("made-this-run-c4d5"),
            "a worktree this run made was reported as an earlier run's leftover: {said}"
        );
        assert!(
            said.contains(root.path().to_str().unwrap()),
            "the report does not say where the leftover worktrees are: {said}"
        );
        assert!(
            root.path().join("old-thing-9f2a").is_dir(),
            "a leftover worktree was removed instead of reported"
        );
    }

    /// The grace period covers a record a fresh spawn has not written yet. A
    /// spawn running since an earlier run wrote its record long ago, so a
    /// reading the app cannot resolve is unknown from the first tick.
    #[test]
    fn an_adopted_spawn_is_past_the_grace_period_from_the_first_tick() {
        let tmux = PrivateTmux::start("adoption-grants-no-grace-period");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        earlier_run(&tmux, &session, &root, &repository);
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        let snapshot = snapshot::build(
            std::slice::from_ref(&adopted.started[0].watched),
            &tmux.server.panes().unwrap(),
            &HashMap::new(),
            Instant::now(),
            &Snapshot::default(),
        );
        assert_eq!(
            snapshot.rows[0].status,
            Status::Unknown,
            "a spawn the app could read nothing about read as working, because it was \
             taken for one that had only just started"
        );
    }

    /// One pane that will not close is one pane, not a start-up. The spawn it
    /// belonged to had already stopped, and the report names it.
    #[test]
    fn a_pane_that_would_not_close_is_named_rather_than_taking_the_start_up_with_it() {
        let mut adopted = Adopted::default();
        adopted.unclosed.push(Unadopted {
            name: "drop-the-cache-d4e1".to_string(),
            why: "`tmux kill-pane -t %7` failed: can't find pane %7".to_string(),
        });

        let said = adopted
            .found(Path::new("/data/harness-launcher/worktrees"), &[])
            .expect("a pane the app could not close, and nothing was said");

        assert!(said.contains("would not close"), "{said}");
        assert!(said.contains("drop-the-cache-d4e1: "), "{said}");
        assert!(said.contains("can't find pane %7"), "{said}");
    }

    /// The same over a real tmux. `after-kill-pane` closes a second dead pane
    /// as a side effect of the first close, so the app asks for a pane that
    /// has already gone. Start-up carries on: the live spawn is still adopted.
    #[test]
    fn a_close_that_fails_costs_one_line_of_the_report_and_not_the_start_up() {
        let tmux = PrivateTmux::start("adoption-outlives-a-close-that-fails");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let repository = repository_with_origin();
        let first = left_running(&tmux, &session, &root, &repository, "exit 3");
        let second = left_running(&tmux, &session, &root, &repository, "exit 3");
        tmux.until("#{pane_dead}", |seen| {
            seen.lines().filter(|line| *line == "1").count() == 2
        });
        let doomed = pane_of(&tmux, &second);
        process::run(
            "tmux",
            &[
                "-L",
                tmux.server.socket(),
                "set-hook",
                "-g",
                "after-kill-pane",
                &format!("kill-pane -t {doomed}"),
            ],
        )
        .unwrap();
        let live = earlier_run(&tmux, &session, &root, &repository);
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        assert_eq!(adopted.closed, vec![first]);
        let names: Vec<&str> = adopted
            .unclosed
            .iter()
            .map(|spawn| spawn.name.as_str())
            .collect();
        assert_eq!(names, [second.as_str()]);
        let taken: Vec<&str> = adopted
            .started
            .iter()
            .map(|started| started.spawn.entry.spawn.as_str())
            .collect();
        assert_eq!(
            taken,
            [live.as_str()],
            "a pane that would not close took the running spawn with it"
        );
        let said = adopted
            .found(root.path(), &[])
            .expect("a pane the app could not close, and nothing was said");
        assert!(said.contains("would not close"), "{said}");
        assert!(said.contains(&format!("{second}: ")), "{said}");
    }

    /// The pane a named spawn is in, off the session itself.
    fn pane_of(tmux: &PrivateTmux, name: &str) -> String {
        tmux.server
            .windows()
            .unwrap()
            .unwrap()
            .into_iter()
            .find(|window| window.name == name)
            .unwrap_or_else(|| panic!("the session is not holding {name}"))
            .pane
    }

    #[test]
    fn a_first_run_on_a_clean_machine_has_nothing_to_report() {
        let tmux = PrivateTmux::start("adoption-finds-nothing");
        let session = tmux.server.session(SLOT).unwrap();
        let root = tempdir().unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let adopted = adopt(&tmux.server, &client, root.path(), SLOT).unwrap();

        assert!(adopted.found(root.path(), &[]).is_none());
    }
}
