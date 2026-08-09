//! What every spawn is doing, worked out from what the world said.
//!
//! A ladder, not a single signal:
//!
//! 1. **Is the pane alive?** From tmux. A pane that stopped and was kept by
//!    `remain-on-exit` is **stopped**, immediately — the one signal that cannot
//!    go stale, and the one rung that needs nothing else to be working.
//! 2. **If alive, what does the harness's own record say?** `working` and
//!    `stopped` come straight back out as themselves.
//! 3. **If alive but the record will not resolve** — an older harness, a
//!    configuration directory somewhere else, a format that moved — the spawn
//!    is **unknown**, once it is old enough for that to mean anything.
//!
//! Everything here is a pure function over what the app read. It is the seam
//! the design named for exactly that reason: three statuses and a grace period
//! are the part most worth testing and the part least worth needing a
//! multiplexer, a filesystem or a clock to test.
//!
//! **Where the app must guess, it surfaces rather than hides.** A false
//! *stopped* costs a glance — you open the spawn, see it working, and you were
//! going to that pane anyway. A false *working* costs the product: a spawn
//! waits for you and never appears in the set that needs you. This is the
//! inverse of the prior art's bias, deliberately, because their false negative
//! destroys work and ours only shows a status.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::harness::Reading;
use crate::tmux::{Pane, Panes};

/// How long a spawn that has just started is given before *unknown* applies.
///
/// A spawn is alive before the harness has written anything about it. Without
/// this, every new spawn would spend its first seconds claiming to be broken —
/// firing the one status that is supposed to mean a real problem, routinely.
/// The length follows the prior art, which allows forty attempts at a two
/// hundred millisecond cadence before giving up on the same file.
///
/// It delays nothing else. A pane that is dead is stopped at once, however new
/// the spawn is.
pub const GRACE: Duration = Duration::from_secs(8);

/// What the list says about a spawn. Three, and they are about the agent —
/// except the third, which is about the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The agent is busy.
    Working,
    /// The agent has stopped. Finished, waiting on an answer, or dead: the app
    /// never infers which, because the response to all three is the same.
    Stopped,
    /// *The app* cannot tell. Not a kind of stopped: stopped means go and look
    /// at the spawn, unknown means something is wrong with the tooling.
    Unknown,
}

/// A spawn the supervisor watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    /// The spawn's name, which is also its row's.
    pub name: String,
    /// The pane holding it. Always known: the app split that pane itself.
    pub pane: String,
    /// When the app took charge of it, which is where the grace period counts
    /// from.
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

    /// The spawn's pane, if it is still running something.
    ///
    /// Said once here because the ladder and the tick both need it and must
    /// agree: were they to differ, the tick would gather evidence about a pane
    /// the ladder had already written off, or skip one it had not.
    pub fn alive_in<'a>(&self, panes: &'a Panes) -> Option<&'a Pane> {
        self.listed_in(panes).filter(|pane| !pane.dead)
    }
}

/// What the tie-breaker found holding a pane's terminal.
///
/// Asked only when the record has already failed, so it costs nothing on a tick
/// where everything resolves.
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

/// One spawn, and what it is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The spawn's name.
    pub name: String,
    /// What it is doing, as far as the app can tell.
    pub status: Status,
    /// Why the app cannot tell, when it cannot.
    ///
    /// Kept for display and never promoted to a status of its own: an old
    /// harness, a failed probe and a pane that vanished are one status and
    /// three sentences, not three statuses.
    pub reason: Option<String>,
}

/// What every spawn was doing, at one moment.
///
/// Built whole and then handed over, so the list is never read while it is
/// being written.
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

/// Work out what every watched spawn is doing.
///
/// Everything it needs has already been read: the panes tmux listed, and
/// whatever the app managed to find out about each spawn, keyed by name.
pub fn build(
    watched: &[Watched],
    panes: &Panes,
    evidence: &HashMap<String, Evidence>,
    at: Instant,
) -> Snapshot {
    Snapshot {
        rows: watched
            .iter()
            .map(|spawn| {
                let (status, reason) = climb(spawn, panes, evidence.get(&spawn.name), at);

                Row {
                    name: spawn.name.clone(),
                    status,
                    reason,
                }
            })
            .collect(),
    }
}

/// The ladder itself, for one spawn.
///
/// The grace period below covers the harness not having written anything yet.
/// It does not cover a pane the app cannot find: tmux knowing its own panes is
/// not something that takes a few seconds to become true, so a spawn whose pane
/// is not there is the app's own handle being wrong, from the first tick.
fn climb(
    spawn: &Watched,
    panes: &Panes,
    evidence: Option<&Evidence>,
    at: Instant,
) -> (Status, Option<String>) {
    let Some(pane) = spawn.listed_in(panes) else {
        return unknown("the app cannot find the pane it was running in".to_string());
    };
    if pane.dead {
        return (Status::Stopped, None);
    }

    let Some(evidence) = evidence else {
        return settle(
            spawn,
            at,
            "the app did not manage to read its status".to_string(),
        );
    };

    match &evidence.reading {
        Reading::Working => (Status::Working, None),
        Reading::Stopped => (Status::Stopped, None),
        Reading::Unresolved(why) => match &evidence.foreground {
            // The tie-breaker only ever settles it downwards. It cannot say a
            // spawn is working — a process holding a terminal is not an agent
            // with something to do — and a probe that failed must never read as
            // the agent being gone.
            Some(Foreground::SomethingElse) => (Status::Stopped, None),
            Some(Foreground::Unreadable(trouble)) => settle(
                spawn,
                at,
                format!("{why}, and the probe that would settle it failed: {trouble}"),
            ),
            Some(Foreground::Harness) | None => settle(spawn, at, why.clone()),
        },
    }
}

/// Whether a spawn has been watched for longer than the grace period.
///
/// Which is to say: whether silence from the harness has stopped meaning "it
/// has not started writing yet" and started meaning the app cannot tell. The
/// supervisor asks the same question to decide when a tie-breaker is worth
/// running, and it is defined here so there is one answer rather than two.
pub fn past_grace(spawn: &Watched, at: Instant) -> bool {
    at.saturating_duration_since(spawn.adopted) >= GRACE
}

/// What an unresolved spawn is, which depends on how long it has been one.
///
/// A spawn young enough to still be starting up is taken at its word: it was
/// handed work a moment ago, its pane is alive, and the harness has not written
/// anything yet. Past the grace period the same silence is the app admitting it
/// cannot tell.
///
/// This is the one place the app knowingly risks the *working* it says costs
/// the product, and it is bounded on both sides: it lasts seconds, and a spawn
/// that was handed work seconds ago cannot yet be waiting on an answer. The
/// alternative was every new spawn reporting the tooling as broken on the way
/// up, which spends the meaning of `unknown` on something that is not wrong.
fn settle(spawn: &Watched, at: Instant, why: String) -> (Status, Option<String>) {
    if past_grace(spawn, at) {
        return unknown(why);
    }

    (Status::Working, None)
}

/// Unknown, with the sentence that says why.
fn unknown(why: String) -> (Status, Option<String>) {
    (Status::Unknown, Some(why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::slice::from_ref;

    /// A real `list-panes` from a real tmux — see `captured/README.md`. In it,
    /// `%3` is a live pane and `%2` is one whose session stopped.
    const CAPTURED: &str = include_str!("../captured/tmux-list-panes.txt");
    const ALIVE: &str = "%3";
    const STOPPED: &str = "%2";
    const GONE: &str = "%99";

    /// A spawn that has been watched long enough for the grace period to be
    /// over.
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
        );

        snapshot
            .of(&spawn.name)
            .expect("a watched spawn has a row")
            .clone()
    }

    #[test]
    fn a_working_agent_in_a_live_pane_is_working() {
        let row = row(&watched(ALIVE), &found(Reading::Working, None));

        assert_eq!(row.status, Status::Working);
        assert_eq!(row.reason, None);
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
        assert!(row.reason.unwrap().contains("pane"));
    }

    #[test]
    fn a_record_that_will_not_resolve_is_unknown_once_the_grace_period_is_over() {
        let row = row(&watched(ALIVE), &found(unresolved(), None));

        assert_eq!(row.status, Status::Unknown);
        assert_eq!(
            row.reason,
            Some("its session record carries no status".to_string()),
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
        assert_eq!(row.reason, None);
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
        assert_eq!(row.reason, None);
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
        let why = row.reason.unwrap();
        assert!(why.contains("carries no status"), "{why}");
        assert!(why.contains("not on PATH"), "{why}");
    }

    #[test]
    fn a_spawn_the_app_read_nothing_about_at_all_is_unknown() {
        let row = row(&watched(ALIVE), &HashMap::new());

        assert_eq!(row.status, Status::Unknown);
        assert!(row.reason.is_some());
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
                );

                let row = &snapshot.rows[0];
                assert_eq!(
                    row.reason.is_some(),
                    row.status == Status::Unknown,
                    "a reason became a status of its own: {row:?}"
                );
            }
        }
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
        );

        let named: Vec<&str> = snapshot.rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(named, ["first-a1", "second-b2", "third-c3"]);
    }
}
