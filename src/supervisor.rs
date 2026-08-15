//! The thread that watches, and the channel it reports down.
//!
//! A main thread that draws, one blocking thread that looks, a channel
//! between them — no task per spawn, no async runtime. A tick is one
//! `list-panes` plus a stat per live spawn; the `ps` probe runs only for a
//! spawn the ladder would otherwise call unknown, and its answer stands for
//! a few seconds. See docs/developers/concurrency.md.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::harness::{self, Reading, StatusFiles};
use crate::process;
use crate::snapshot::{self, Evidence, Foreground, Snapshot, Watched};
use crate::tmux::{Pane, Server};

/// How often the supervisor looks.
const TICK: Duration = Duration::from_millis(200);

/// What the supervisor hands back.
pub struct Watching {
    /// What every spawn is doing, one snapshot per tick.
    pub snapshots: Receiver<Snapshot>,
    /// Spawns made since it started, which the next tick picks up.
    pub arriving: Sender<Watched>,
    /// Spawns retired since it started, which the next tick lets go of.
    pub leaving: Sender<String>,
}

/// Start watching, and hand back the snapshots. Everything moves whole down
/// a channel, so no two threads share state; the thread exits when the
/// receiving end goes.
pub fn watch(server: Server, watched: Vec<Watched>) -> Watching {
    let (sending, snapshots) = mpsc::channel();
    let (arriving, arrivals) = mpsc::channel();
    let (leaving, departures) = mpsc::channel();

    thread::spawn(move || {
        let mut supervisor = Supervisor::new(server, watched, arrivals, departures);
        while sending.send(supervisor.tick()).is_ok() {
            thread::sleep(TICK);
        }
    });

    Watching {
        snapshots,
        arriving,
        leaving,
    }
}

/// One thread's worth of state: what to watch, and what it saw last time.
struct Supervisor {
    /// The tmux server the spawns live on.
    server: Server,
    /// The spawns being watched.
    watched: Vec<Watched>,
    /// Spawns made since it started looking, waiting to be taken up.
    arrivals: Receiver<Watched>,
    /// Spawns retired since it started looking, waiting to be let go of.
    departures: Receiver<String>,
    /// What the harness's records said.
    records: Records,
    /// What the tie-breaker found, for the spawns it has been run for.
    probes: Probes,
    /// What it said last time; where a lost spawn's last status comes from.
    said: Snapshot,
}

impl Supervisor {
    fn new(
        server: Server,
        watched: Vec<Watched>,
        arrivals: Receiver<Watched>,
        departures: Receiver<String>,
    ) -> Self {
        Self {
            server,
            watched,
            arrivals,
            departures,
            records: Records::new(),
            probes: Probes::default(),
            said: Snapshot::default(),
        }
    }

    /// Look once, and say what everything is doing. Arrivals are taken up
    /// first, so a new spawn is in this snapshot; retirements are let go of
    /// before anything is asked, so a retired spawn is not reported as a
    /// missing pane. A tmux that will not answer reads as no panes, which the
    /// ladder already handles.
    fn tick(&mut self) -> Snapshot {
        let at = Instant::now();
        while let Ok(arrival) = self.arrivals.try_recv() {
            self.watched.push(arrival);
        }
        while let Ok(retired) = self.departures.try_recv() {
            self.watched.retain(|spawn| spawn.name != retired);
        }
        let panes = self.server.panes().unwrap_or_default();

        let mut evidence = HashMap::new();
        let mut live = HashSet::new();
        for spawn in &self.watched {
            let Some(pane) = spawn.alive_in(&panes) else {
                continue;
            };
            live.insert(pane.pid);

            // The tie-breaker is asked exactly when the ladder would otherwise
            // call this spawn unknown. Keying on the spawn rather than the
            // last tick's conclusion stops the row flickering.
            let reading = self.records.of(pane.pid);
            let unresolved = matches!(reading, Reading::Unresolved(_));
            let foreground = (unresolved && snapshot::past_grace(spawn, at))
                .then(|| self.probes.of(&spawn.name, pane, at));

            evidence.insert(
                spawn.name.clone(),
                Evidence {
                    reading,
                    foreground,
                },
            );
        }
        self.records.forget_all_but(&live);
        self.probes.forget_all_but(&live);

        // The previous snapshot carries each spawn's last readable status; the
        // ladder is pure and is handed it.
        self.said = snapshot::build(&self.watched, &panes, &evidence, at, &self.said);

        self.said.clone()
    }
}

/// What the harness's records said, and where they live.
struct Records {
    /// Where to look, if anywhere.
    files: Option<StatusFiles>,
    /// What each record said, kept until the file moves underneath it.
    read: HashMap<u32, (SystemTime, Reading)>,
}

impl Records {
    /// Where the harness keeps its records on this machine.
    fn new() -> Self {
        Self {
            files: harness::status_files(
                env::var_os(harness::config_directory_variable()),
                env::var_os("HOME"),
            ),
            read: HashMap::new(),
        }
    }

    /// What the record for the session running as `pid` says. Opened only
    /// when the file has changed; an unresolved reading is cached the same
    /// way, so it is not re-read five times a second either.
    fn of(&mut self, pid: u32) -> Reading {
        let Some(files) = &self.files else {
            return Reading::Unresolved(
                "the app has nowhere to look for what it is doing: neither the harness's \
                 configuration directory nor $HOME is set"
                    .to_string(),
            );
        };

        let file = files.of(pid);
        let changed = match fs::metadata(&file).and_then(|about| about.modified()) {
            Ok(changed) => changed,
            Err(trouble) => {
                self.read.remove(&pid);

                return unreadable(&trouble);
            }
        };

        if let Some((last, reading)) = self.read.get(&pid)
            && *last == changed
        {
            return reading.clone();
        }

        let reading = match fs::read_to_string(&file) {
            Ok(record) => harness::read_status(&record, pid),
            Err(trouble) => unreadable(&trouble),
        };
        self.read.insert(pid, (changed, reading.clone()));

        reading
    }

    /// Let go of what was read about processes that are no longer there.
    fn forget_all_but(&mut self, live: &HashSet<u32>) {
        self.read.retain(|pid, _| live.contains(pid));
    }
}

/// A record that could not be opened at all, said the way the ladder expects.
fn unreadable(trouble: &std::io::Error) -> Reading {
    Reading::Unresolved(format!("the app cannot read what it is doing: {trouble}"))
}

/// How long a tie-breaker's answer is good for. Caching keeps the probe off
/// the per-tick path; the bounded lifetime keeps a failed or transient answer
/// from pinning a spawn for ever.
const AN_ANSWER_LASTS: Duration = Duration::from_secs(5);

/// Probe answers, keyed by spawn. An answer is about one process at one
/// moment, so a new pid or a stale time is a new question.
#[derive(Default)]
struct Probes {
    found: HashMap<String, Answer>,
}

/// One answer, and what it was an answer about.
struct Answer {
    /// The process holding the pane when the question was asked.
    about: u32,
    /// When it was asked.
    at: Instant,
    /// What was found.
    found: Foreground,
}

impl Probes {
    /// What holds this spawn's terminal, asked afresh only when the last
    /// answer is stale or about a session that has gone.
    fn of(&mut self, spawn: &str, pane: &Pane, at: Instant) -> Foreground {
        if let Some(answer) = self.found.get(spawn)
            && answer.about == pane.pid
            && at.saturating_duration_since(answer.at) < AN_ANSWER_LASTS
        {
            return answer.found.clone();
        }

        let found = foreground(&pane.tty);
        self.found.insert(
            spawn.to_string(),
            Answer {
                about: pane.pid,
                at,
                found: found.clone(),
            },
        );

        found
    }

    /// Let go of answers about processes that are no longer there.
    fn forget_all_but(&mut self, live: &HashSet<u32>) {
        self.found.retain(|_, answer| live.contains(&answer.about));
    }
}

/// Ask what holds a pane's terminal — the only per-spawn subprocess, which is
/// why it runs only for unresolved spawns.
fn foreground(tty: &str) -> Foreground {
    let outcome = match process::run("ps", &["-t", tty, "-o", "pgid=,tpgid=,comm=,args="]) {
        Ok(outcome) => outcome,
        Err(trouble) => return Foreground::Unreadable(trouble.to_string()),
    };
    if outcome.ok {
        return holding(&outcome.stdout);
    }

    let complaint = if outcome.stderr.is_empty() {
        "it listed no processes at all"
    } else {
        &outcome.stderr
    };

    Foreground::Unreadable(format!("`ps` had nothing to say about {tty}: {complaint}"))
}

/// Read what `ps` printed about one terminal. The foreground group is the one
/// whose id equals the terminal's, and it can hold several processes. A
/// listing with no foreground group is unreadable, never empty: a failed read
/// must never count as the agent being gone.
fn holding(listing: &str) -> Foreground {
    let mut looked_at_something = false;

    for line in listing.lines() {
        let mut fields = line.split_whitespace();
        let (Some(group), Some(terminals), Some(command), Some(argv0)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if group != terminals {
            continue;
        }

        looked_at_something = true;
        if harness::names_the_harness(command, argv0) {
            return Foreground::Harness;
        }
    }

    if looked_at_something {
        Foreground::SomethingElse
    } else {
        Foreground::Unreadable("nothing at all holds its terminal".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::tempdir;

    use crate::screen::Size;
    use crate::snapshot::Status;
    use crate::tmux::tests::PrivateTmux;

    /// Real `ps` output — see `captured/README.md`. In the first, the harness
    /// has gone and a stand-in holds the terminal; in the second it is there.
    const WITHOUT: &str = include_str!("../captured/ps-foreground.txt");
    const WITH: &str = include_str!("../captured/ps-harness.txt");

    #[test]
    fn a_pane_the_harness_still_holds_is_read_as_such() {
        assert_eq!(holding(WITH), Foreground::Harness);
    }

    #[test]
    fn a_pane_holding_something_else_says_the_agent_has_gone() {
        assert_eq!(holding(WITHOUT), Foreground::SomethingElse);
    }

    #[test]
    fn the_processes_that_are_not_in_the_foreground_are_not_asked_about() {
        // The first row of the recording is the starting shell, outside the
        // foreground group.
        let shell = WITHOUT.lines().next().unwrap();

        assert_eq!(
            holding(shell),
            Foreground::Unreadable("nothing at all holds its terminal".to_string())
        );
    }

    #[test]
    fn a_listing_that_says_nothing_is_unreadable_rather_than_empty() {
        assert!(matches!(holding(""), Foreground::Unreadable(_)));
        assert!(matches!(holding("garbage\n"), Foreground::Unreadable(_)));
    }

    /// The process the captured record belongs to.
    const PID: u32 = harness::RECORDED_PID;

    /// A directory the harness could be keeping its records in.
    struct Kept {
        directory: tempfile::TempDir,
    }

    impl Kept {
        fn new() -> Self {
            Self {
                directory: tempdir().unwrap(),
            }
        }

        /// Records read from this directory, remembering nothing yet.
        fn records(&self) -> Records {
            Records {
                files: harness::status_files(Some(self.directory.path().into()), None),
                read: HashMap::new(),
            }
        }

        /// Write a record saying one thing, where the harness would write it.
        fn writes(&self, status: &str) {
            let file = self.records().files.unwrap().of(PID);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut written = fs::File::create(file).unwrap();
            written
                .write_all(harness::recorded(status).as_bytes())
                .unwrap();
            written.sync_all().unwrap();
        }

        fn file(&self) -> PathBuf {
            self.records().files.unwrap().of(PID)
        }
    }

    #[test]
    fn a_record_is_read_from_where_the_harness_keeps_it() {
        let kept = Kept::new();
        kept.writes("busy");

        assert_eq!(kept.records().of(PID), Reading::Working);
    }

    #[test]
    fn a_record_that_has_not_moved_is_not_read_again() {
        let kept = Kept::new();
        kept.writes("busy");
        let mut records = kept.records();
        records.of(PID);

        // Changed underneath, keeping the same modification time.
        let when = fs::metadata(kept.file()).unwrap().modified().unwrap();
        kept.writes("idle");
        fs::File::options()
            .write(true)
            .open(kept.file())
            .unwrap()
            .set_modified(when)
            .unwrap();

        assert_eq!(
            records.of(PID),
            Reading::Working,
            "the file was read again although nothing said it had changed"
        );
    }

    #[test]
    fn a_record_that_has_moved_is_read_again() {
        let kept = Kept::new();
        kept.writes("busy");
        let mut records = kept.records();
        records.of(PID);

        kept.writes("idle");
        fs::File::options()
            .write(true)
            .open(kept.file())
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(1))
            .unwrap();

        assert_eq!(records.of(PID), Reading::Stopped);
    }

    #[test]
    fn a_record_that_is_not_there_is_unresolved_rather_than_a_refusal() {
        let reading = Kept::new().records().of(PID);

        assert!(matches!(reading, Reading::Unresolved(_)), "{reading:?}");
    }

    #[test]
    fn with_nowhere_to_look_a_spawn_is_unresolved_rather_than_a_refusal() {
        let mut nowhere = Records {
            files: None,
            read: HashMap::new(),
        };

        let reading = nowhere.of(PID);

        assert!(matches!(reading, Reading::Unresolved(_)), "{reading:?}");
    }

    #[test]
    fn a_record_that_goes_away_is_forgotten_rather_than_remembered_stale() {
        let kept = Kept::new();
        kept.writes("busy");
        let mut records = kept.records();
        records.of(PID);

        fs::remove_file(kept.file()).unwrap();
        let reading = records.of(PID);

        assert!(matches!(reading, Reading::Unresolved(_)), "{reading:?}");
        assert!(
            records.read.is_empty(),
            "a record was kept for a file that is gone"
        );
    }

    /// A live pane, holding a terminal nothing will be found on.
    fn pane(pid: u32) -> Pane {
        Pane {
            dead: false,
            pid,
            tty: "/dev/pts/nothing-of-the-sort".to_string(),
        }
    }

    /// A moment that is not now, so a test can say when it asked.
    fn moment() -> Instant {
        Instant::now()
            .checked_sub(AN_ANSWER_LASTS * 4)
            .expect("a machine that has been up for longer than a few answers")
    }

    /// What an answer says, without caring which answer it was.
    fn kept(probes: &mut Probes, pane: &Pane, at: Instant) -> Foreground {
        probes.of("add-retry-logic-a7f3", pane, at)
    }

    #[test]
    fn an_answer_stands_until_it_goes_stale() {
        let mut probes = Probes::default();
        let asked = moment();

        let first = kept(&mut probes, &pane(4321), asked);
        let again = kept(&mut probes, &pane(4321), asked + AN_ANSWER_LASTS / 2);

        assert_eq!(first, again);
        assert_eq!(probes.found.len(), 1, "the answer was not kept at all");
    }

    #[test]
    fn a_stale_answer_is_asked_again_rather_than_believed_for_ever() {
        let mut probes = Probes::default();
        let asked = moment();
        kept(&mut probes, &pane(4321), asked);

        kept(&mut probes, &pane(4321), asked + AN_ANSWER_LASTS);

        let answer = &probes.found["add-retry-logic-a7f3"];
        assert_eq!(
            answer.at,
            asked + AN_ANSWER_LASTS,
            "a probe that failed once would have pinned the spawn for ever"
        );
    }

    #[test]
    fn a_new_session_in_the_same_pane_is_a_new_question() {
        let mut probes = Probes::default();
        let asked = moment();
        kept(&mut probes, &pane(4321), asked);

        kept(&mut probes, &pane(9999), asked);

        assert_eq!(
            probes.found["add-retry-logic-a7f3"].about, 9999,
            "an answer about a process that has gone was kept"
        );
    }

    #[test]
    fn a_spawn_made_while_the_app_runs_is_watched_from_the_next_tick() {
        let tmux = PrivateTmux::start("supervisor-adopts-a-new-spawn");
        let session = tmux
            .server
            .session(Size {
                columns: 40,
                rows: 10,
            })
            .unwrap();
        let watching = watch(tmux.server.clone(), Vec::new());
        let pane = tmux
            .server
            .open_window(&session, "start-the-scheduler-c8d2")
            .unwrap();

        watching
            .arriving
            .send(Watched::new("start-the-scheduler-c8d2".to_string(), pane))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = watching.snapshots.recv().unwrap();
            if let Some(row) = snapshot.of("start-the-scheduler-c8d2") {
                assert_eq!(
                    row.status,
                    Status::Working,
                    "a spawn adopted a moment ago: {row:?}"
                );

                return;
            }
            assert!(
                Instant::now() < deadline,
                "the supervisor never picked up the spawn it was told about"
            );
        }
    }

    /// A tick is one listing plus stats, never per-spawn subprocesses;
    /// `tests/twenty_spawns.rs` counts the subprocesses under `strace`, this
    /// pins the consequence. The bound is loose on purpose so a slow machine
    /// passes. It prints its timings because
    /// `docs/evidence/scale-at-twenty.md` quotes them.
    #[test]
    fn a_tick_at_twenty_spawns_stays_comfortably_inside_the_period_between_ticks() {
        const SPAWNS: usize = 20;

        let tmux = PrivateTmux::start("tick-at-twenty-spawns");
        let session = tmux
            .server
            .session(Size {
                columns: 80,
                rows: 24,
            })
            .unwrap();
        let watched: Vec<Watched> = (0..SPAWNS)
            .map(|number| {
                let name = format!("spawn-number-{number:02}");
                let pane = tmux.server.open_window(&session, &name).unwrap();

                Watched::new(name, pane)
            })
            .collect();
        let (_arriving, arrivals) = mpsc::channel();
        let (_leaving, departures) = mpsc::channel();
        let mut supervisor = Supervisor::new(tmux.server.clone(), watched, arrivals, departures);

        let mut ticks = Vec::new();
        for _ in 0..10 {
            let taken = Instant::now();
            let snapshot = supervisor.tick();
            ticks.push(taken.elapsed());
            assert_eq!(
                snapshot.rows.len(),
                SPAWNS,
                "a tick did not say something about every spawn"
            );
        }

        let worst = ticks.iter().max().copied().unwrap_or_default();
        let best = ticks.iter().min().copied().unwrap_or_default();
        println!(
            "a tick over {SPAWNS} spawns: {best:?} to {worst:?} over {} ticks, \
             against the {TICK:?} between them",
            ticks.len()
        );

        assert!(
            worst < TICK / 2,
            "a tick over {SPAWNS} spawns took {worst:?}, which is not comfortably inside the \
             {TICK:?} between them: {ticks:?}"
        );
    }

    #[test]
    fn a_spawn_that_has_been_retired_is_let_go_of() {
        let tmux = PrivateTmux::start("supervisor-lets-go-of-a-retired-spawn");
        let session = tmux
            .server
            .session(Size {
                columns: 40,
                rows: 10,
            })
            .unwrap();
        let pane = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        let watching = watch(
            tmux.server.clone(),
            vec![Watched::new("add-retry-logic-a7f3".to_string(), pane)],
        );
        until(&watching, |snapshot| {
            snapshot.of("add-retry-logic-a7f3").is_some()
        });

        watching
            .leaving
            .send("add-retry-logic-a7f3".to_string())
            .unwrap();

        until(&watching, |snapshot| {
            snapshot.of("add-retry-logic-a7f3").is_none()
        });
    }

    /// Wait for the supervisor to say something, or give up.
    fn until(watching: &Watching, said: impl Fn(&Snapshot) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = watching.snapshots.recv().unwrap();
            if said(&snapshot) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the supervisor never said it: {snapshot:?}"
            );
        }
    }

    #[test]
    fn answers_about_processes_that_have_gone_are_let_go_of() {
        let mut probes = Probes::default();
        kept(&mut probes, &pane(4321), moment());

        probes.forget_all_but(&HashSet::from([9999]));

        assert!(probes.found.is_empty());
    }
}
