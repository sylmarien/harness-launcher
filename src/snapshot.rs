//! Works out each spawn's status; a pure function over what the tick read.
//!
//! The ladder, in order: a dead pane is stopped at once, whatever the record
//! says; a live pane's status is what the harness's record says; a record
//! that will not resolve may be settled to stopped by the foreground probe;
//! otherwise the spawn is unknown, once past the grace period.
//!
//! The bias is deliberate: a false stopped costs a glance, a false working
//! hides a spawn that is waiting for the user.
//! See docs/developers/knowing-what-a-spawn-is-doing.md.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::harness::Reading;
#[cfg(test)]
use crate::tmux::{ALIVE_PANE, ALIVE_PANE_PID};
use crate::tmux::{Pane, Panes};

/// How long before *unknown* applies: a fresh spawn has not written its
/// record yet. The length follows the prior art (forty attempts at 200ms).
/// A dead pane is stopped at once, however new the spawn is.
pub const GRACE: Duration = Duration::from_secs(8);

/// What the list says about a spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The agent is busy.
    Working,
    /// The agent has stopped: finished, waiting, or dead — the app never
    /// infers which.
    Stopped,
    /// The app cannot tell. Not a kind of stopped: it means the tooling is
    /// wrong, not the spawn.
    Unknown,
}

impl Status {
    /// The status's name in prose, decided in one place.
    fn named(self) -> &'static str {
        match self {
            Status::Working => "working",
            Status::Stopped => "stopped",
            Status::Unknown => "unaccounted for",
        }
    }
}

/// A spawn the supervisor watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    /// The spawn's name, which is also its row's.
    pub name: String,
    /// The pane holding it. Always known: the app split that pane itself.
    pub pane: String,
    /// When the app took charge of it; the grace period counts from here.
    pub adopted: Instant,
}

impl Watched {
    /// Start watching a spawn, from now.
    pub fn new(name: String, pane: String) -> Self {
        Self {
            name,
            pane,
            adopted: Instant::now(),
        }
    }

    /// The spawn's pane, if the server still has it.
    pub fn listed_in<'a>(&self, panes: &'a Panes) -> Option<&'a Pane> {
        panes.get(&self.pane)
    }

    /// The spawn's pane, if it is still running something. Defined once so
    /// the ladder and the tick agree.
    pub fn alive_in<'a>(&self, panes: &'a Panes) -> Option<&'a Pane> {
        self.listed_in(panes).filter(|pane| !pane.dead)
    }
}

/// What the tie-breaker found holding a pane's terminal. Asked only when the
/// record has already failed to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Foreground {
    /// The harness is still what the pane is running.
    Harness,
    /// Something else holds the terminal, so the agent is not there any more.
    SomethingElse,
    /// The probe did not answer, and this is why.
    Unreadable(String),
}

/// What the app found out about one spawn this tick, beyond what tmux said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// What reading the harness's record produced.
    pub reading: Reading,
    /// The tie-breaker, on the ticks where one was worth running.
    pub foreground: Option<Foreground>,
}

/// Diagnostic detail for a spawn the app cannot account for. Kept for
/// display; never promoted to a status of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unaccounted {
    /// Why the app cannot tell, in one sentence.
    pub why: String,
    /// The pane the spawn was started in.
    pub pane: String,
    /// The process tmux says is in that pane; `None` when the pane itself
    /// could not be found.
    pub pid: Option<u32>,
    /// The last status the app actually managed to read, if it ever did.
    pub last_known: Option<Status>,
}

impl Unaccounted {
    /// The slot's full explanation, a sentence per line. Names the pane and
    /// the process so the app can be diagnosed from `tmux list-panes` and `ps`.
    pub fn explained(&self, name: &str) -> Vec<String> {
        vec![
            format!("the app cannot tell what {name} is doing."),
            match self.pid {
                Some(pid) => format!("its pane {} is alive, running process {pid}.", self.pane),
                None => format!(
                    "its pane {} is not one the tmux server still has.",
                    self.pane
                ),
            },
            format!("what went wrong: {}", self.why),
            match self.last_known {
                Some(status) => format!("the last status it could read was {}.", status.named()),
                None => "it has not read one since this spawn started.".to_string(),
            },
        ]
    }
}

/// One spawn, and what it is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The spawn's name.
    pub name: String,
    /// What it is doing, as far as the app can tell.
    pub status: Status,
    /// What it has to say when it cannot tell, and nothing when it can.
    pub unaccounted: Option<Unaccounted>,
    /// The last status worked out from evidence, carried across ticks that
    /// could not read one. Not a copy of [`Row::status`]: the grace period's
    /// *working* is a guess, not a reading, so it never lands here.
    pub last_known: Option<Status>,
}

/// The `last_known` the ladder would put on a row of this status, so a
/// hand-built test row matches the ladder's shape.
#[cfg(test)]
pub(crate) fn last_read(status: Status) -> Option<Status> {
    match status {
        Status::Unknown => None,
        read => Some(read),
    }
}

#[cfg(test)]
pub(crate) fn cannot_account(why: &str, last_known: Option<Status>) -> Unaccounted {
    Unaccounted {
        why: why.to_string(),
        pane: ALIVE_PANE.to_string(),
        pid: Some(ALIVE_PANE_PID),
        last_known,
    }
}

/// What every spawn was doing, at one moment. Built whole and then handed
/// over, so the list is never read while it is being written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// One row per watched spawn, in the order they are watched.
    pub rows: Vec<Row>,
}

impl Snapshot {
    /// What this snapshot says about one spawn, if it holds it at all.
    pub fn of(&self, name: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.name == name)
    }
}

/// Work out what every watched spawn is doing, from what was already read.
/// `before` is the last snapshot: it carries the last status the app could
/// read for a spawn it can no longer read, and passing it in keeps this pure.
pub fn build(
    watched: &[Watched],
    panes: &Panes,
    evidence: &HashMap<String, Evidence>,
    at: Instant,
    before: &Snapshot,
) -> Snapshot {
    Snapshot {
        rows: watched
            .iter()
            .map(|spawn| climb(spawn, panes, evidence.get(&spawn.name), at, before))
            .collect(),
    }
}

/// Where a spawn lives, as far as the app can see.
struct Whereabouts<'a> {
    spawn: &'a Watched,
    /// The process tmux says holds its pane, when the pane was found at all.
    pid: Option<u32>,
    /// The last status the app could read about it.
    last_known: Option<Status>,
}

impl Whereabouts<'_> {
    /// Unknown, with the sentence that says why.
    fn unknown(&self, why: String) -> Row {
        Row {
            name: self.spawn.name.clone(),
            status: Status::Unknown,
            unaccounted: Some(Unaccounted {
                why,
                pane: self.spawn.pane.clone(),
                pid: self.pid,
                last_known: self.last_known,
            }),
            last_known: self.last_known,
        }
    }

    /// An unresolved spawn is *working* inside the grace period — a fresh
    /// spawn has not written its record yet — and *unknown* after it. The
    /// grace-period working is a guess, not a reading, so `last_known` is
    /// carried forward untouched.
    fn settle(&self, at: Instant, why: String) -> Row {
        if past_grace(self.spawn, at) {
            return self.unknown(why);
        }

        Row {
            name: self.spawn.name.clone(),
            status: Status::Working,
            unaccounted: None,
            last_known: self.last_known,
        }
    }
}

/// A status worked out from evidence, which is therefore also the last one
/// read.
fn read(spawn: &Watched, status: Status) -> Row {
    Row {
        name: spawn.name.clone(),
        status,
        unaccounted: None,
        last_known: Some(status),
    }
}

/// The ladder itself, for one spawn. A missing pane is unknown from the first
/// tick: the grace period covers an unwritten record, not tmux losing a pane.
fn climb(
    spawn: &Watched,
    panes: &Panes,
    evidence: Option<&Evidence>,
    at: Instant,
    before: &Snapshot,
) -> Row {
    // What the app had read before this tick, surviving ticks that could not
    // tell.
    let last_known = before.of(&spawn.name).and_then(|row| row.last_known);
    let Some(pane) = spawn.listed_in(panes) else {
        return Whereabouts {
            spawn,
            pid: None,
            last_known,
        }
        .unknown("the app cannot find the pane it was running in".to_string());
    };
    if pane.dead {
        return read(spawn, Status::Stopped);
    }
    let whereabouts = Whereabouts {
        spawn,
        pid: Some(pane.pid),
        last_known,
    };

    let Some(evidence) = evidence else {
        return whereabouts.settle(at, "the app did not manage to read its status".to_string());
    };

    match &evidence.reading {
        Reading::Working => read(spawn, Status::Working),
        Reading::Stopped => read(spawn, Status::Stopped),
        Reading::Unresolved(why) => match &evidence.foreground {
            // The tie-breaker can only change the answer to stopped, never to
            // working — and a failed probe never counts as the agent being
            // gone.
            Some(Foreground::SomethingElse) => read(spawn, Status::Stopped),
            Some(Foreground::Unreadable(trouble)) => whereabouts.settle(
                at,
                format!("{why}, and the probe that would settle it failed: {trouble}"),
            ),
            Some(Foreground::Harness) | None => whereabouts.settle(at, why.clone()),
        },
    }
}

/// Whether the grace period is over. Defined once: the supervisor asks the
/// same question to decide when the tie-breaker is worth running.
pub fn past_grace(spawn: &Watched, at: Instant) -> bool {
    at.saturating_duration_since(spawn.adopted) >= GRACE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::slice::from_ref;

    /// A real `list-panes` from a real tmux — see `captured/README.md`. In it,
    /// `%3` is a live pane and `%2` is one whose session stopped.
    const CAPTURED: &str = include_str!("../captured/tmux-list-panes.txt");
    const ALIVE: &str = ALIVE_PANE;
    const STOPPED: &str = "%2";
    const GONE: &str = "%99";

    /// A spawn watched long enough for the grace period to be over.
    fn watched(pane: &str) -> Watched {
        Watched {
            name: "add-retry-logic-a7f3".to_string(),
            pane: pane.to_string(),
            adopted: Instant::now()
                .checked_sub(GRACE * 2)
                .expect("a machine that has been up for longer than a grace period"),
        }
    }

    fn found(reading: Reading, foreground: Option<Foreground>) -> HashMap<String, Evidence> {
        HashMap::from([(
            "add-retry-logic-a7f3".to_string(),
            Evidence {
                reading,
                foreground,
            },
        )])
    }

    fn unresolved() -> Reading {
        Reading::Unresolved("its session record carries no status".to_string())
    }

    /// What the ladder makes of one spawn.
    fn row(spawn: &Watched, evidence: &HashMap<String, Evidence>) -> Row {
        let snapshot = build(
            from_ref(spawn),
            &Panes::parse(CAPTURED),
            evidence,
            Instant::now(),
            &Snapshot::default(),
        );

        snapshot
            .of(&spawn.name)
            .expect("a watched spawn has a row")
            .clone()
    }

    /// The unaccounted reason, when the ladder wrote one.
    fn why(row: &Row) -> Option<&str> {
        row.unaccounted
            .as_ref()
            .map(|unaccounted| unaccounted.why.as_str())
    }

    #[test]
    fn a_working_agent_in_a_live_pane_is_working() {
        let row = row(&watched(ALIVE), &found(Reading::Working, None));

        assert_eq!(row.status, Status::Working);
        assert_eq!(why(&row), None);
    }

    #[test]
    fn a_stopped_agent_in_a_live_pane_is_stopped() {
        let row = row(&watched(ALIVE), &found(Reading::Stopped, None));

        assert_eq!(row.status, Status::Stopped);
    }

    #[test]
    fn a_pane_that_stopped_is_stopped_whatever_the_record_still_says() {
        let row = row(&watched(STOPPED), &found(Reading::Working, None));

        assert_eq!(
            row.status,
            Status::Stopped,
            "a stale record outranked the one signal that cannot go stale"
        );
    }

    #[test]
    fn a_pane_that_is_gone_is_unknown_rather_than_stopped() {
        let row = row(&watched(GONE), &found(Reading::Working, None));

        assert_eq!(row.status, Status::Unknown);
        assert!(why(&row).unwrap().contains("pane"));
    }

    #[test]
    fn a_record_that_will_not_resolve_is_unknown_once_the_grace_period_is_over() {
        let row = row(&watched(ALIVE), &found(unresolved(), None));

        assert_eq!(row.status, Status::Unknown);
        assert_eq!(
            why(&row),
            Some("its session record carries no status"),
            "the reason the app cannot tell was dropped"
        );
    }

    #[test]
    fn a_spawn_that_has_only_just_started_is_not_called_broken_yet() {
        let fresh = Watched::new("add-retry-logic-a7f3".to_string(), ALIVE.to_string());

        let row = row(&fresh, &found(unresolved(), None));

        assert_eq!(
            row.status,
            Status::Working,
            "a spawn started a moment ago reported the tooling as broken"
        );
        assert_eq!(why(&row), None);
    }

    #[test]
    fn a_new_spawn_whose_pane_died_is_stopped_at_once_all_the_same() {
        let fresh = Watched::new("add-retry-logic-a7f3".to_string(), STOPPED.to_string());

        let row = row(&fresh, &found(unresolved(), None));

        assert_eq!(row.status, Status::Stopped);
    }

    #[test]
    fn the_probe_settles_an_unresolvable_spawn_when_the_harness_has_gone() {
        let row = row(
            &watched(ALIVE),
            &found(unresolved(), Some(Foreground::SomethingElse)),
        );

        assert_eq!(row.status, Status::Stopped);
        assert_eq!(why(&row), None);
    }

    #[test]
    fn the_probe_finding_the_harness_still_there_settles_nothing() {
        let row = row(
            &watched(ALIVE),
            &found(unresolved(), Some(Foreground::Harness)),
        );

        assert_eq!(row.status, Status::Unknown);
    }

    #[test]
    fn a_probe_that_failed_is_unknown_and_says_both_things_that_went_wrong() {
        let row = row(
            &watched(ALIVE),
            &found(
                unresolved(),
                Some(Foreground::Unreadable("`ps` is not on PATH".to_string())),
            ),
        );

        assert_eq!(
            row.status,
            Status::Unknown,
            "a failed probe was read as the agent being gone"
        );
        let said = why(&row).unwrap_or_default();
        assert!(said.contains("carries no status"), "{said}");
        assert!(said.contains("not on PATH"), "{said}");
    }

    #[test]
    fn an_unaccountable_spawn_says_which_process_would_not_resolve_and_that_it_is_alive() {
        let row = row(&watched(ALIVE), &found(unresolved(), None));

        let unaccounted = row
            .unaccounted
            .expect("a spawn the app cannot account for says so");
        assert_eq!(unaccounted.pane, ALIVE);
        assert_eq!(
            unaccounted.pid,
            Some(ALIVE_PANE_PID),
            "the process whose status would not resolve is not named"
        );
        assert!(
            unaccounted.why.contains("carries no status"),
            "{unaccounted:?}"
        );
    }

    #[test]
    fn a_spawn_whose_pane_is_gone_names_no_process() {
        let row = row(&watched(GONE), &found(Reading::Working, None));

        let unaccounted = row
            .unaccounted
            .expect("a pane that is gone is unaccounted for");
        assert_eq!(unaccounted.pid, None);
        assert_eq!(unaccounted.pane, GONE);
    }

    #[test]
    fn an_unaccountable_spawn_carries_the_last_status_the_app_could_read() {
        let spawn = watched(ALIVE);
        let panes = Panes::parse(CAPTURED);
        let working = build(
            from_ref(&spawn),
            &panes,
            &found(Reading::Working, None),
            Instant::now(),
            &Snapshot::default(),
        );

        let lost_it = build(
            from_ref(&spawn),
            &panes,
            &found(unresolved(), None),
            Instant::now(),
            &working,
        );

        let unaccounted = lost_it.of(&spawn.name).unwrap().unaccounted.clone();
        assert_eq!(
            unaccounted
                .expect("a spawn the app cannot account for")
                .last_known,
            Some(Status::Working),
            "what the app could tell a tick ago was thrown away"
        );
    }

    #[test]
    fn the_last_status_survives_more_than_the_first_tick_that_lost_it() {
        let spawn = watched(ALIVE);
        let panes = Panes::parse(CAPTURED);
        let mut latest = build(
            from_ref(&spawn),
            &panes,
            &found(Reading::Stopped, None),
            Instant::now(),
            &Snapshot::default(),
        );

        for _ in 0..3 {
            latest = build(
                from_ref(&spawn),
                &panes,
                &found(unresolved(), None),
                Instant::now(),
                &latest,
            );
        }

        assert_eq!(
            latest
                .of(&spawn.name)
                .unwrap()
                .unaccounted
                .as_ref()
                .unwrap()
                .last_known,
            Some(Status::Stopped)
        );
    }

    #[test]
    fn a_spawn_the_app_has_never_read_has_no_last_known_status() {
        let row = row(&watched(ALIVE), &found(unresolved(), None));

        assert_eq!(row.unaccounted.unwrap().last_known, None);
    }

    #[test]
    fn the_word_a_starting_spawn_is_taken_at_is_not_a_status_the_app_read() {
        let spawn = Watched::new("add-retry-logic-a7f3".to_string(), ALIVE.to_string());
        let panes = Panes::parse(CAPTURED);

        let starting = build(
            from_ref(&spawn),
            &panes,
            &found(unresolved(), None),
            spawn.adopted,
            &Snapshot::default(),
        );
        assert_eq!(
            starting.of(&spawn.name).unwrap().status,
            Status::Working,
            "a spawn inside its grace period is still taken at its word"
        );

        let gave_up = build(
            from_ref(&spawn),
            &panes,
            &found(unresolved(), None),
            spawn.adopted + GRACE,
            &starting,
        );

        assert_eq!(
            gave_up
                .of(&spawn.name)
                .unwrap()
                .unaccounted
                .as_ref()
                .expect("a spawn the app cannot account for")
                .last_known,
            None,
            "the grace period's guess was quoted back as a status the app had read"
        );
    }

    #[test]
    fn an_unaccountable_spawn_explains_itself_in_full() {
        let unaccounted = cannot_account(
            "its session record carries no status",
            Some(Status::Working),
        );

        let said = unaccounted.explained("add-retry-logic-a7f3").join("\n");

        for fact in [
            "add-retry-logic-a7f3",
            ALIVE,
            &ALIVE_PANE_PID.to_string(),
            "alive",
            "carries no status",
            "working",
        ] {
            assert!(said.contains(fact), "nothing says {fact}:\n{said}");
        }
    }

    #[test]
    fn a_spawn_whose_pane_is_gone_does_not_claim_it_is_alive() {
        let unaccounted = Unaccounted {
            why: "the app cannot find the pane it was running in".to_string(),
            pane: ALIVE.to_string(),
            pid: None,
            last_known: Some(Status::Working),
        };

        let said = unaccounted.explained("add-retry-logic-a7f3").join("\n");

        assert!(!said.contains("alive"), "{said}");
        assert!(said.contains(ALIVE), "{said}");
    }

    #[test]
    fn a_spawn_nothing_was_ever_read_about_says_so_rather_than_naming_a_status() {
        let said = cannot_account("its session record carries no status", None)
            .explained("add-retry-logic-a7f3")
            .join("\n");

        for never_read in ["working", "stopped"] {
            assert!(
                !said.contains(never_read),
                "a status the app never read was claimed as the last one:\n{said}"
            );
        }
    }

    #[test]
    fn a_spawn_the_app_read_nothing_about_at_all_is_unknown() {
        let row = row(&watched(ALIVE), &HashMap::new());

        assert_eq!(row.status, Status::Unknown);
        assert!(why(&row).is_some());
    }

    #[test]
    fn only_an_unknown_spawn_carries_a_reason() {
        let panes = Panes::parse(CAPTURED);
        let every_pane = [ALIVE, STOPPED, GONE].map(watched);

        for spawn in &every_pane {
            for reading in [Reading::Working, Reading::Stopped, unresolved()] {
                let snapshot = build(
                    from_ref(spawn),
                    &panes,
                    &found(reading, None),
                    Instant::now(),
                    &Snapshot::default(),
                );

                let row = &snapshot.rows[0];
                assert_eq!(
                    why(row).is_some(),
                    row.status == Status::Unknown,
                    "a reason became a status of its own: {row:?}"
                );
            }
        }
    }

    /// Every tick clones the snapshot down the channel; this pins that a copy
    /// stays cheap. The bound is loose on purpose so a slow machine passes.
    /// It prints what it measured because docs/evidence quotes the number.
    #[test]
    fn a_snapshot_of_twenty_rows_is_cheap_enough_to_copy_every_tick() {
        const COPIES: u32 = 1_000;

        let snapshot = Snapshot {
            rows: (0..20)
                .map(|number| Row {
                    name: format!("some-piece-of-work-{number:02}-a7f3"),
                    status: Status::Unknown,
                    unaccounted: Some(cannot_account(
                        "its session record carries no status — the harness may be older \
                         than the version that writes one",
                        Some(Status::Working),
                    )),
                    last_known: Some(Status::Working),
                })
                .collect(),
        };

        let taken = Instant::now();
        for _ in 0..COPIES {
            drop(std::hint::black_box(snapshot.clone()));
        }
        let each = taken.elapsed() / COPIES;
        println!("copying a snapshot of twenty unaccountable rows: {each:?} each");

        assert!(
            each < Duration::from_micros(100),
            "copying a snapshot of twenty rows took {each:?}, which is no longer nothing \
             beside the {:?} between ticks",
            Duration::from_millis(200)
        );
    }

    #[test]
    fn every_watched_spawn_gets_a_row_in_the_order_it_is_watched() {
        let spawns = vec![
            Watched::new("first-a1".to_string(), ALIVE.to_string()),
            Watched::new("second-b2".to_string(), STOPPED.to_string()),
            Watched::new("third-c3".to_string(), GONE.to_string()),
        ];

        let snapshot = build(
            &spawns,
            &Panes::parse(CAPTURED),
            &HashMap::new(),
            Instant::now(),
            &Snapshot::default(),
        );

        let named: Vec<&str> = snapshot.rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(named, ["first-a1", "second-b2", "third-c3"]);
    }
}
