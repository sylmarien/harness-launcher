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
//!    configuration directory somewhere else, a format that moved — one
//!    tie-breaker is asked before giving up: **what is holding the pane's
//!    terminal?** Something that is not the harness means the agent has gone,
//!    which is **stopped**. It only ever settles downwards — a process holding
//!    a terminal is not an agent with something to do, so the probe can never
//!    say *working*, and a probe that fails must never read as the agent being
//!    gone. **Asked only once the spawn is past the grace period below**, which
//!    is the same condition rung 4 waits for: until then an unresolved spawn is
//!    taken at its word, and no probe is run that could send one still starting
//!    up straight to *stopped*.
//! 4. **If it still will not resolve**, the spawn is **unknown**, once it is
//!    old enough for that to mean anything.
//!
//! The tie-breaker is not a fourth status and does not widen the vocabulary; it
//! is rung 3 refusing to spend `unknown` on a spawn whose pane can be shown to
//! have moved on. It costs nothing on a tick where every record resolves,
//! because it is only ever asked after one has failed.
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
#[cfg(test)]
use crate::tmux::{ALIVE_PANE, ALIVE_PANE_PID};
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

impl Status {
    /// What it is called in a sentence.
    ///
    /// The list says a status with a mark and a colour; prose has neither, and
    /// this is the one place the words are decided so that two of them cannot
    /// come to disagree.
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

/// Everything the app can say about a spawn it cannot account for.
///
/// **Kept for display and never promoted to a status of its own**: an old
/// harness, a failed probe and a pane that vanished are one status and three
/// sentences, not three statuses.
///
/// All of it is for the slot, where somebody who opened the spawn is diagnosing
/// *the app* — and what separates "the harness moved its records" from "this
/// spawn died and the app has not noticed" is exactly which process would not
/// resolve, whether its pane is still alive, and what the app could last tell.
/// The row it is about has one line and spends it on the spawn's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unaccounted {
    /// Why the app cannot tell, in one sentence.
    pub why: String,
    /// The pane the spawn was started in.
    pub pane: String,
    /// The process tmux says is in that pane — and nothing at all when the pane
    /// itself could not be found, because naming a process there would be the
    /// app inventing a fact about the thing it has just admitted it cannot see.
    pub pid: Option<u32>,
    /// The last status the app actually managed to read, if it ever did.
    pub last_known: Option<Status>,
}

impl Unaccounted {
    /// What to say about it where there is room to say it all — a line at a
    /// time, for whatever is drawing them.
    ///
    /// **The row says nothing and this says everything**, and the difference is
    /// what each is for: a row is one line saying *something is wrong with this
    /// one*, and the slot is where somebody who came to find out what goes about
    /// it. Naming the pane and the process is what makes the app diagnosable
    /// from outside itself — they are what `tmux list-panes` and `ps` are asked
    /// about next — and the last known status is what says whether this spawn was
    /// ever accounted for at all.
    ///
    /// Sentences rather than a table: they are read once, by somebody who did not
    /// come here to learn a layout.
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
    /// The last status the app worked out **from evidence**, if it ever managed
    /// one, carried across every tick that could not.
    ///
    /// **Not a copy of [`Row::status`]**, and the gap between them is the whole
    /// reason this is a field rather than a read of the status beside it. A
    /// spawn inside its grace period shows *working* on trust — nothing was
    /// read, the app is taking a spawn it started a moment ago at its word — so
    /// that row's status is not something the app can later quote back as
    /// having been read. Left implicit, every spawn would arrive at *unknown*
    /// claiming a last known status of *working*, because every spawn passes
    /// through its grace period on the way in, and the sentence saying nothing
    /// was ever read could never be printed.
    pub last_known: Option<Status>,
}

/// What the ladder would have said about a spawn it cannot account for.
///
/// Shared with the modules that *draw* one — the list and the slot — so that no
/// screen is ever asserted against a shape the ladder would not have produced.
/// The pane and the process are the captured listing's own, for the same reason.
/// The `last_known` the ladder would have put on a row of this status.
///
/// Shared with the modules that *draw* rows, for the same reason
/// [`cannot_account`] is: a status the app read is its own last known status,
/// and a status it could not read carries whatever came before — so a hand-built
/// row is never a shape the ladder would not have produced.
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
///
/// **`before` is the last snapshot there was**, and it is here for exactly one
/// thing: a spawn the app has stopped being able to read still knows what it
/// could last tell about it. That is a fact about the app's own history rather
/// than about the world, so it is carried forward from snapshot to snapshot
/// instead of being asked for — and passing it in keeps this a pure function of
/// what it was given.
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

/// Where a spawn lives, as far as the app can see — everything the sentences
/// about not being able to tell are written from.
struct Whereabouts<'a> {
    /// The spawn the sentences are about.
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

    /// What an unresolved spawn is, which depends on how long it has been one.
    ///
    /// A spawn young enough to still be starting up is taken at its word: it was
    /// handed work a moment ago, its pane is alive, and the harness has not
    /// written anything yet. Past the grace period the same silence is the app
    /// admitting it cannot tell.
    ///
    /// This is the one place the app knowingly risks the *working* it says costs
    /// the product, and it is bounded on both sides: it lasts seconds, and a
    /// spawn that was handed work seconds ago cannot yet be waiting on an
    /// answer. The alternative was every new spawn reporting the tooling as
    /// broken on the way up, which spends the meaning of `unknown` on something
    /// that is not wrong.
    ///
    /// **The word it takes the spawn at is not a reading**, so it carries
    /// `last_known` forward untouched rather than claiming this status as one.
    /// Every spawn passes through here on the way in; were the guess recorded as
    /// read, no spawn could ever say the app had read nothing about it.
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

/// A status the app worked out from evidence, and which is therefore also the
/// last status it could read.
fn read(spawn: &Watched, status: Status) -> Row {
    Row {
        name: spawn.name.clone(),
        status,
        unaccounted: None,
        last_known: Some(status),
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
    before: &Snapshot,
) -> Row {
    // Whatever the app had managed to read before this tick. A status it worked
    // out is the answer; one it could not is whatever that snapshot was already
    // carrying, so the fact survives however many ticks the app spends unable to
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
            // The tie-breaker only ever settles it downwards. It cannot say a
            // spawn is working — a process holding a terminal is not an agent
            // with something to do — and a probe that failed must never read as
            // the agent being gone.
            Some(Foreground::SomethingElse) => read(spawn, Status::Stopped),
            Some(Foreground::Unreadable(trouble)) => whereabouts.settle(
                at,
                format!("{why}, and the probe that would settle it failed: {trouble}"),
            ),
            Some(Foreground::Harness) | None => whereabouts.settle(at, why.clone()),
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
            &Snapshot::default(),
        );

        snapshot
            .of(&spawn.name)
            .expect("a watched spawn has a row")
            .clone()
    }

    /// The sentence the ladder wrote about not being able to tell, when it wrote
    /// one.
    ///
    /// The one part of an [`Unaccounted`] most of these tests are about: reaching
    /// through the field on every assertion would bury what each of them is
    /// checking. Nothing outside a test reads it on its own — the slot is drawn
    /// from [`Unaccounted::explained`], which is all of it at once.
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

    /// **What the app has to be able to say about its own ignorance**, and it
    /// is more than a sentence: the process whose status would not resolve, that
    /// the pane is alive all the same, and what it could last tell. Somebody
    /// reading this is diagnosing the app rather than the agent, and those are
    /// the three facts that separate "the harness moved its records" from "the
    /// spawn died and the app has not noticed".
    ///
    /// The pid is the captured listing's own — a real `list-panes` from a real
    /// tmux, so nothing here is a number this test made up.
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

    /// A pane the app cannot find has no process to name, and saying one anyway
    /// would be the app inventing a fact about the very thing it just admitted
    /// it cannot see.
    #[test]
    fn a_spawn_whose_pane_is_gone_names_no_process() {
        let row = row(&watched(GONE), &found(Reading::Working, None));

        let unaccounted = row
            .unaccounted
            .expect("a pane that is gone is unaccounted for");
        assert_eq!(unaccounted.pid, None);
        assert_eq!(unaccounted.pane, GONE);
    }

    /// **The last thing the app could tell, carried across the tick that lost
    /// it.** It is the difference between a spawn that was working a moment ago
    /// and one the app has never once managed to read — and only the first is a
    /// reason to go on waiting.
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

    /// And it keeps being carried while the app goes on not being able to tell,
    /// rather than surviving exactly one tick.
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

    /// A spawn nothing has ever resolved for says so, rather than claiming a
    /// last known status it never had.
    #[test]
    fn a_spawn_the_app_has_never_read_has_no_last_known_status() {
        let row = row(&watched(ALIVE), &found(unresolved(), None));

        assert_eq!(row.unaccounted.unwrap().last_known, None);
    }

    /// **The grace period's *working* is a courtesy, not a reading.** A spawn
    /// whose status has never once resolved spends its first seconds being taken
    /// at its word — and if it then turns unaccountable it has to say the app
    /// read nothing about it, rather than quoting its own guess back as a status
    /// it managed to read. Told "the last status it could read was working", a
    /// reader waits on a spawn the app never once saw.
    ///
    /// This is the tick sequence the running app actually produces, rather than
    /// a first tick handed an empty snapshot: every real spawn is watched from
    /// inside its grace period, so every real spawn has one of these guesses
    /// behind it.
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

    /// **What somebody who opens an unaccountable spawn is given to work with.**
    /// They are diagnosing the app, not the agent, so every fact that separates
    /// one cause from another has to be in front of them: the spawn, the pane,
    /// the process whose status would not resolve, that it is alive all the
    /// same, what went wrong, and what the app could last tell.
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

    /// A pane the app cannot find is not a pane it can call alive, and the
    /// explanation must not claim a process is running in it either.
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

    /// **What the supervisor hands the list, five times a second, at twenty.**
    ///
    /// A tick keeps the snapshot it built — it is where a spawn it can no longer
    /// read gets the last status it could — and sends a *copy* down the channel.
    /// That is the obvious thing rather than the clever one, and the obvious
    /// thing was chosen on the grounds that twenty rows are nothing to copy. This
    /// is that assumption written down as an assertion, because it is the one
    /// that would quietly stop being true if a row ever grew.
    ///
    /// Every row here carries the whole of what an unaccountable one does, which
    /// is the expensive shape: four heap-allocated strings rather than one.
    ///
    /// The bound is loose by three orders of magnitude on purpose. What it is
    /// aimed at is a row that has grown something a copy cannot afford; a bound
    /// tight enough to catch a slow machine would fail on one.
    ///
    /// **It prints what it measured**, because the evidence document quotes this
    /// cost and a number nothing emits cannot be checked against the test it is
    /// attributed to:
    ///
    /// ```text
    /// cargo test --release --bin harness-launcher -- --nocapture \
    ///     a_snapshot_of_twenty_rows
    /// ```
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
