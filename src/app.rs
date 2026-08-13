//! The whole screen.
//!
//! The list on the left, the slot on the right, and the line between them — all
//! of it drawn by the app in one pass. There is no mode in which two halves of
//! the screen belong to different programs and fail to line up, because there is
//! only one program drawing.
//!
//! What is in the slot is a spawn's own screen: a grid the control-mode client
//! keeps current whether or not it is the one being shown, copied here cell by
//! cell. Keystrokes go the other way. **The app types into a spawn only what the
//! user typed** — it has no keyboard of its own beyond the seven keys that
//! leave, move the selection, start a draft, throw one away, make one into a
//! spawn, and retire one.
//!
//! **Or the slot holds a draft**, which is a form the app draws and types into
//! itself. That is the one place an ordinary key means something to the app
//! rather than to a session, and which of the two it is comes from what the list
//! is on — settled once a frame, like everything else about the slot.
//!
//! **A draft that is started becomes a spawn here.** The making is somebody
//! else's ([`crate::creation`]); what this file owns is the moment it arrives:
//! the spawn joins the list under the repository it was started against, the
//! supervisor is told to watch it, the draft's row makes way, and the selection
//! follows only if it was still on that draft. Everything slow about it happened
//! on a thread, so no frame ever waits for a worktree.
//!
//! **Exactly one spawn is in the slot, and moving the selection changes which.**
//! That is the whole of switching: nothing is moved, resized or told anything,
//! because every spawn's grid was already current — the one that was off screen
//! was being drawn into all along, and the one arriving has nothing to catch up
//! on. **Nothing hides the list**, in this or any other state of the app.
//!
//! Nothing here is a fixed size. Every dimension comes from the real terminal on
//! every frame, so a maximised window is a bigger layout rather than a bigger
//! frame around a small one — and the slot growing is what tells tmux to grow
//! the panes behind it.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::crossterm::terminal;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::control::{Client, Grid, POISONED};
use crate::creation::{self, Report, Said, Started};
use crate::draft::{self, Draft, Drafts, Edit};
use crate::error::{Error, Result};
use crate::keys::{self, Modes};
use crate::list::{self, Cursor, Entry, Listing, Step};
use crate::retirement::{self, Retirement, Retirements};
use crate::scaffolding::{AMBER, DIM, HEADING, wrapped};
use crate::screen::{Screen, Size};
use crate::snapshot::{Snapshot, Unaccounted, Watched};
use crate::tmux::Server;

/// How long a frame waits for a keystroke before drawing itself again.
///
/// Short, because the slot is somebody else's screen now: what is on it changes
/// without anyone touching a key, and a frame that waited for one would show a
/// spawn thinking in steps.
const FRAME: Duration = Duration::from_millis(16);

/// How much of the screen the list takes.
///
/// A share rather than a size, so a maximised window is not a bigger frame
/// around the same small layout.
const LIST_SHARE: u16 = 33;

/// Leave, which kills nothing.
///
/// A function key because every ordinary one belongs to the spawn: a digit is
/// exactly what you need to send when a harness asks you to pick an option, and
/// `q` is a letter somebody is in the middle of typing. This and the keys below
/// are the whole of the app's keyboard so far, and how the rest of it is divided
/// is not settled yet — so the keys that move the selection sit well away from
/// the one that leaves, and nothing else is claimed.
const QUIT: event::KeyCode = event::KeyCode::F(10);
/// Take the selection one row up the list.
const UP: event::KeyCode = event::KeyCode::F(6);
/// Take it one row down.
const DOWN: event::KeyCode = event::KeyCode::F(7);
/// Throw the draft the list is on away.
///
/// **Beside the key that starts one, because it is the other end of the same
/// thing**: `F2` makes a draft and `F3` gets rid of one, and a hand reaching for
/// either is coming from the same place. That would be reckless anywhere else in
/// this block — it is one row of a mistyped `F2` from a paragraph that exists
/// nowhere else — and it is safe here for exactly one reason: **it asks first**.
/// The first press is a question, and any other key answers it *no*.
///
/// It is the only key in the app that asks, and the only one that needs to. A
/// retirement can refuse, because there is a worktree to look at and a branch
/// that outlives it; a draft is text in a slot and there is nothing to look at
/// afterwards, so the question is the only thing standing between a keystroke
/// and somebody's paragraph.
const DISCARD: event::KeyCode = event::KeyCode::F(3);

/// Start a draft, and put the selection on it.
///
/// A fourth function key, and it has to be one: composing is reached from a
/// spawn in the slot, where every ordinary key is that session's. It sits well
/// away from the two that move, because starting a draft by mistyping a
/// selection is a row appearing in the list nobody asked for.
const COMPOSE: event::KeyCode = event::KeyCode::F(2);

/// Retire the spawn the list is on.
///
/// The sixth key, and the second that cannot be taken back with another
/// keystroke — so it sits in the block with `F10` rather than beside the two
/// that move: the hand that walks the list is on `F6` and `F7` all day, and the
/// key that stops a session must not be one row of a mistyped selection away
/// from them. The mistyping it *is* exposed to is `F10` — leaving — which
/// costs nothing in the other direction, because leaving kills nothing.
///
/// *Accepted cost:* reaching for `F10` and landing here stops the selected
/// spawn. What it cannot do is destroy work: a worktree with anything
/// uncommitted in it refuses, and the branch is left alone either way.
const RETIRE: event::KeyCode = event::KeyCode::F(9);

/// Make the draft in the slot into a spawn.
///
/// The one key in the app that creates anything — a branch,
/// a worktree and a session, none of which can be taken back with another
/// keystroke. So it sits away from `F2`, which is otherwise the key a hand is
/// coming from, and away from the two that move: the two mistypings that would
/// cost something are *compose* becoming *start*, and a selection becoming one.
///
/// `Enter` was the other candidate and is worse. The form's own fields already
/// give it two meanings — a line break in a paragraph, moving on from a path —
/// and a third that creates a worktree would be settled by whichever control
/// the keyboard happened to be in.
const START: event::KeyCode = event::KeyCode::F(5);

/// A spawn, as the screen needs it: what to say about it, and what it drew.
pub struct Spawn {
    /// What the list says.
    pub entry: Entry,
    /// The pane it runs in, which is where keystrokes are addressed.
    pub pane: String,
    /// Its screen.
    pub grid: Grid,
}

impl Spawn {
    /// Its screen, for as long as the caller holds this.
    ///
    /// Held for a copy and no longer: the reader thread is waiting on the same
    /// lock with the next thing this spawn drew, and everything it draws while
    /// the lock is held is waiting behind it.
    fn screen(&self) -> MutexGuard<'_, Screen> {
        self.grid.lock().expect(POISONED)
    }

    /// The shape its screen is in.
    fn size(&self) -> Size {
        self.screen().size()
    }

    /// Catch its screen up with the shape the slot has become.
    fn resize(&self, slot: Size) {
        self.screen().resize(slot);
    }

    /// The modes its screen has put the keyboard in.
    fn modes(&self) -> Modes {
        Modes {
            application_cursor: self.screen().application_cursor(),
        }
    }
}

/// Every spawn there is, and the one the slot is showing.
///
/// **Switching is a re-render and nothing else.** There is no parking, no
/// holding session and no pane being moved: a spawn that is not in the slot is
/// running in its own window with its own grid, and the control-mode client
/// fills every grid whether or not the app is drawing it. Selecting another
/// spawn therefore shows something already current, and the one that just left
/// carries on exactly as it was — nothing was told anything, so nothing
/// redraws.
pub struct Spawns {
    /// In the order they were started, which is the order the list groups them
    /// from.
    all: Vec<Spawn>,
}

impl Spawns {
    /// The spawns the app is to show, of which there may be none.
    ///
    /// **None is an ordinary state now, at both ends of a run**: the app opened
    /// with nothing to start and a draft to write, or every spawn it had has
    /// been retired. What the screen is then is the list, with an empty slot
    /// beside it.
    pub fn new(all: Vec<Spawn>) -> Self {
        Self { all }
    }

    /// What the list says about all of them.
    ///
    /// Taken when the set of spawns changes rather than every frame: what the
    /// list says about a spawn is settled when it is created, and it is the
    /// snapshot beside it that changes.
    pub fn entries(&self) -> Vec<Entry> {
        self.all.iter().map(|spawn| spawn.entry.clone()).collect()
    }

    /// Take a spawn that did not exist when the app started.
    ///
    /// At the end, which is where the list wants it: repositories keep the
    /// order their first spawn arrived in, and within one the attention-first
    /// order settles where a row sits. A spawn nothing is yet known about sorts
    /// last of its group, which is exactly right for one that started a moment
    /// ago.
    fn add(&mut self, spawn: Spawn) {
        self.all.push(spawn);
    }

    /// The spawn of this name, if the app still has it.
    fn of(&self, name: &str) -> Option<&Spawn> {
        self.all.iter().find(|spawn| spawn.entry.spawn == name)
    }

    /// Let go of a spawn that has been retired, and hand it over.
    ///
    /// Handed back rather than dropped here, because there is one more thing to
    /// do with it: the pane it was running in is the pane whose grid the app is
    /// still holding.
    fn let_go_of(&mut self, name: &str) -> Option<Spawn> {
        let at = self
            .all
            .iter()
            .position(|spawn| spawn.entry.spawn == name)?;

        Some(self.all.remove(at))
    }

    /// The spawn in the slot, when there is one.
    ///
    /// A cursor on nothing — or on a spawn that is not here — shows the first
    /// that was started, because the slot is never empty while there is
    /// something to put in it. **Nothing at all is a state the app can now
    /// reach**, by retiring the last spawn there is: what it shows then is the
    /// list, with an empty slot beside it.
    fn showing(&self, cursor: &Cursor) -> Option<&Spawn> {
        cursor
            .spawn()
            .and_then(|on| self.of(on))
            .or_else(|| self.all.first())
    }

    /// The shape their screens are in.
    ///
    /// One answer for all of them: they are created at the slot's size and
    /// resized together, so a spawn whose grid was a different shape from its
    /// neighbours' would be a bug rather than a case to handle. Asked once,
    /// before the first frame, where there is always a spawn to ask.
    fn shape(&self) -> Option<Size> {
        self.all.first().map(Spawn::size)
    }

    /// Catch every spawn up with the shape the slot has become.
    ///
    /// Every one, not only the one on screen: a resize reaches the whole
    /// session at once, so a spawn left in the old shape would be drawing into
    /// a grid that clips it long before anybody selected it and noticed.
    fn resize(&self, slot: Size) {
        for spawn in &self.all {
            spawn.resize(slot);
        }
    }
}

/// How big the slot is when the terminal is this big.
///
/// Asked before the screen is taken over, because the panes tmux opens have to
/// be the shape of the region they will be drawn into — the app renders a
/// spawn's screen at the size the spawn thinks it has, and a disagreement shows
/// up as a child drawing off the edge.
pub fn slot(terminal: Size) -> Size {
    Size::of(regions(Rect::new(0, 0, terminal.columns, terminal.rows)).2)
}

/// How big the slot is on the terminal the app was started on.
///
/// The first thing that can refuse for a reason nothing else would catch: a
/// terminal too small to hold a slot has nothing to start a spawn at, and being
/// told so on a shell is better than a session opening at no size at all.
pub fn slot_now() -> Result<Size> {
    let (columns, rows) = terminal::size()
        .map_err(|trouble| Error::new(format!("the app has to run on a terminal: {trouble}")))?;

    let slot = slot(Size { columns, rows });
    if slot.is_empty() {
        return Err(Error::new(format!(
            "this terminal is {columns} by {rows}, which leaves no room for a session beside \
             the list — make the window bigger and run this again"
        )));
    }

    Ok(slot)
}

/// Everything outside the app's own screen that it can reach.
///
/// Gathered into one thing because they are one thing: the session the spawns
/// are windows of, the client their output arrives down and keystrokes leave
/// by, the supervisor's snapshots, and the way to tell it about a spawn that did
/// not exist when it started. Making another spawn while the app runs needs all
/// of them at once.
pub struct World<'a> {
    /// The tmux server the spawns live on.
    pub server: &'a Server,
    /// The session every spawn is a window of.
    pub session: &'a str,
    /// The client every spawn's output arrives down.
    pub client: &'a Client,
    /// What every spawn is doing, as of the supervisor's last tick.
    pub snapshots: Receiver<Snapshot>,
    /// How the supervisor is told to watch a spawn that has just been made.
    pub arriving: Sender<Watched>,
    /// How it is told to let go of one that has been retired.
    pub leaving: Sender<String>,
    /// Where the worktrees the app creates go. Resolved once, before the screen
    /// was taken over, so a machine with nowhere to put them said so on a shell.
    pub worktrees: PathBuf,
}

/// Draw everything until the user quits.
///
/// Snapshots are drained rather than queued: what the user wants to see is what
/// is true now, so a frame that arrives behind several ticks skips them. What a
/// creation or a retirement has to say is drained rather than sampled, for the
/// opposite reason: every line of it is a record of something that was about to
/// happen, and a skipped one is a worktree nobody wrote down.
///
/// **An empty slot is an ordinary state at both ends of a run** — nothing was
/// asked for and a draft is waiting, or everything there was has been retired.
/// The shape every grid is in is read off the first spawn there is; with none to
/// read it from it is asked of the terminal, which is the same answer the first
/// spawn will be given when somebody writes one.
pub fn run(held: &mut Held, world: &World) -> Result<()> {
    let mut latest = Snapshot::default();
    let mut shape = held.spawns.shape().map_or_else(slot_now, Ok)?;
    let (reporting, reports) = mpsc::channel();
    let (retiring, retirements) = mpsc::channel();

    ratatui::run(|terminal| -> io::Result<()> {
        loop {
            while let Ok(snapshot) = world.snapshots.try_recv() {
                latest = snapshot;
            }

            // Before anything is drawn or borrowed, because these are the two
            // things in a frame that can add or take away a spawn — and the
            // list, the slot and the keyboard all have to be looking at the
            // same set of them.
            while let Ok(report) = reports.try_recv() {
                reported(report, held, world, shape);
            }
            while let Ok(report) = retirements.try_recv() {
                retired(report, held, world, &latest);
            }

            // What is in the slot is settled once, at the top of the frame. The
            // screen drawn, the way the keyboard is read and the pane a
            // keystroke is addressed to are then the same spawn — or the same
            // draft — by construction; asking again further down would let a
            // selection that moved mid-frame send what was typed to the spawn
            // that left.
            let showing = in_the_slot(&held.spawns, &held.drafts, &held.cursor);
            let typing = showing.typing();
            // The pane's name rather than the pane, so that what is in the slot
            // is done being borrowed by the time an edit needs the drafts back.
            // A frame's worth of one short string, against a keystroke reaching
            // the wrong spawn.
            let addressed = showing.pane().map(str::to_string);
            let listing = Listing::new(
                held.drafts.all(),
                &held.entries,
                &latest,
                &held.retirements,
                &held.cursor,
            );

            terminal.draw(|frame| render(frame, listing, &showing))?;

            // A client that has gone leaves every grid exactly as it was, which
            // on screen is a session sitting there thinking. Asked here rather
            // than only when something is typed, because the user has no reason
            // to type at a spawn that looks busy.
            world.client.listening().map_err(io::Error::other)?;

            // The slot's shape is read off the frame that was just drawn rather
            // than worked out again from the terminal, so what the child is told
            // it has and what the app draws cannot come to differ.
            let wanted = slot(Size::of(terminal.get_frame().area()));
            if wanted != shape && !wanted.is_empty() {
                // The grids first: the resize reaches the children as a redraw,
                // and a grid still the old shape would clip it.
                held.spawns.resize(wanted);
                world.client.resize(wanted).map_err(io::Error::other)?;
                shape = wanted;
            }

            let asked = asked_for(typing)?;
            // Before the keystroke is acted on, because it is the same
            // keystroke: a question standing on a draft is answered *no* by
            // anything that is not the answer, wherever the key was aimed.
            if takes_the_question_back(&asked) {
                held.drafts.take_back_every_question();
            }

            match asked {
                Asked::Nothing => {}
                Asked::Quit => return Ok(()),
                Asked::Moved(step) => held.moved(step, &latest),
                Asked::Composed => held.cursor = Cursor::on_draft(held.drafts.start()),
                Asked::Started => {
                    held.start(world.worktrees.clone(), env::var_os("PATH"), &reporting);
                }
                Asked::Retired => held.retire(world.server, retiring.clone()),
                Asked::Discarded => held.discard(&latest),
                Asked::Edited(edit) => {
                    if let Some(draft) = held.cursor.draft() {
                        held.drafts.edit(draft, edit);
                    }
                }
                Asked::Typed(bytes) => {
                    if let Some(pane) = &addressed {
                        world.client.send(pane, &bytes).map_err(io::Error::other)?;
                    }
                }
            }
        }
    })
    .map_err(|error| Error::new(format!("the app stopped: {error}")))
}

/// Do what one thing a creation said calls for.
///
/// Everything a creation has to say lands on the draft it is about, which is
/// where it is read: the row in the list, and the form beside it. Only the last
/// thing it says is ever anything else — a plan, which is the app's cue to open
/// the window and start the harness.
///
/// **Starting the harness happens here rather than on the creation's own
/// thread**, because it is tmux and the control client, and those belong to the
/// thread that holds them. It is two commands to a server already running, so
/// the frame it costs is a frame.
fn reported(report: Report, held: &mut Held, world: &World, slot: Size) {
    match report.said {
        Said::Doing(step) => held.drafts.doing(report.draft, step),
        Said::Refused(why) => held.drafts.failed(report.draft, why),
        Said::Made(plan) => {
            // Said before it is done, like everything a creation says: a
            // harness that will not start leaves behind a draft that says the
            // worktree was made and what was about to run in it.
            held.drafts.doing(
                report.draft,
                format!("starting the harness in {}", plan.entry.worktree),
            );

            match creation::start(world.server, world.session, world.client, slot, *plan) {
                Ok(started) => held.adopt(started, report.draft, &world.arriving),
                // *Accepted cost:* the worktree and the branch stay, and are
                // not taken back. Retiring is what removes a worktree and it
                // acts on a **spawn** — there is none here, because the thing
                // that would have been one is exactly what failed to start.
                // The rule about litter is that it must not be *invisible*, and
                // the draft's own record is what makes it visible: it names the
                // worktree and the branch that were made, and it stays until
                // somebody deals with them.
                Err(refused) => held.drafts.failed(report.draft, refused.to_string()),
            }
        }
    }
}

/// Do what one thing a retirement said calls for.
///
/// Everything a retirement has to say lands on the spawn it is about, which is
/// the row in the list: somebody who asked for one and walked off to answer
/// another spawn is looking at the list, not at the slot.
///
/// Only the last thing it says is ever anything else — that there is nothing of
/// this spawn left, which is the app's cue to let go of every piece of it at
/// once.
fn retired(report: retirement::Report, held: &mut Held, world: &World, latest: &Snapshot) {
    match report.said {
        retirement::Said::Doing(step) => held.retirements.doing(&report.spawn, step),
        retirement::Said::Refused(why) => held.retirements.refused(&report.spawn, why),
        retirement::Said::Retired => {
            if let Some(pane) = held.let_go_of(&report.spawn, &world.leaving, latest) {
                world.client.forget(&pane);
            }
        }
    }
}

/// Everything the app itself is holding: the spawns, the drafts, and which row
/// the list is on.
///
/// The other half of a frame is [`World`] — everything the app can reach outside
/// its own screen. These two travel together through a frame because a frame is
/// exactly one meeting of them, and they are separate because only one of them
/// can be true without the app running.
pub struct Held {
    /// Every spawn there is.
    spawns: Spawns,
    /// Every draft being written or made.
    drafts: Drafts,
    /// Where the retirements in flight have got to.
    retirements: Retirements,
    /// Which row the list is on, and so what the slot holds.
    cursor: Cursor,
    /// What the list says about the spawns.
    ///
    /// Kept beside them rather than asked for every frame: what the list says
    /// about a spawn is settled when it is created, and the only thing that can
    /// change this is a spawn arriving — which is one place, below.
    entries: Vec<Entry>,
}

impl Held {
    /// What the app starts with: the spawns it was given, whatever drafts there
    /// are, and the selection on the first row of the list.
    ///
    /// **Which is a spawn when there is one, and a draft when there is not.**
    /// An app opened with nothing to start has one row and it is the form, so
    /// the first thing on screen is the thing there is to do; an app opened on
    /// sessions puts you on one of those, because the drafts are what you would
    /// have gone to `F2` for.
    pub fn new(spawns: Spawns, drafts: Drafts) -> Self {
        let entries = spawns.entries();
        let cursor = match (entries.first(), drafts.all().first()) {
            (Some(entry), _) => Cursor::on_spawn(&entry.spawn),
            (None, Some(draft)) => Cursor::on_draft(draft.id()),
            (None, None) => Cursor::default(),
        };

        Self {
            spawns,
            drafts,
            retirements: Retirements::default(),
            cursor,
            entries,
        }
    }

    /// Make the draft the list is on into a spawn.
    ///
    /// **Only a draft, and only what it says can start it.** A draft that has
    /// not said enough refuses on its own row rather than here, and one already
    /// being made is not started twice.
    ///
    /// **The harness is checked before the thread is started**, which is the
    /// last moment at which a refusal still costs nothing: everything past this
    /// line makes something — a directory, a branch, a window — and a machine
    /// with no harness on it was never going to run any of it. The refusal lands
    /// where every other one does, on the draft, which keeps its text and its
    /// choices and can be started again the moment the machine is fixed.
    ///
    /// The `PATH` comes in from the caller for the same reason it does
    /// everywhere else on this road ([`creation::harness_installed`]).
    fn start(&mut self, worktrees: PathBuf, path: Option<OsString>, reporting: &Sender<Report>) {
        let Some(draft) = self.cursor.draft() else {
            return;
        };
        let Some(wanted) = self.drafts.submit(draft) else {
            return;
        };

        if let Err(refused) = creation::harness_installed(path) {
            self.drafts.failed(draft, refused.to_string());

            return;
        }

        creation::making(draft, wanted, worktrees, reporting.clone());
    }

    /// Ask for the spawn the list is on to be retired.
    ///
    /// **Only a spawn, and only the one the selection is on.** A draft has
    /// nothing to retire — no session, no worktree, nothing on disk at all —
    /// and the key doing something to whichever spawn happened to be underneath
    /// would be the app choosing what to stop.
    ///
    /// The work goes on a thread, like a creation's: stopping a session takes
    /// as long as it takes, and every other spawn keeps drawing meanwhile.
    fn retire(&mut self, server: &Server, retiring: Sender<retirement::Report>) {
        let Some(name) = self.cursor.spawn().map(str::to_string) else {
            return;
        };
        let Some(spawn) = self.spawns.of(&name) else {
            return;
        };
        let pane = spawn.pane.clone();
        let worktree = PathBuf::from(&spawn.entry.worktree);

        if self.retirements.asked_for(&name) {
            retirement::retiring(name, pane, worktree, server.clone(), retiring);
        }
    }

    /// Throw the draft the list is on away, once it has said yes to being asked.
    ///
    /// **The whole of what it takes is the app's own state.** No server, no
    /// client, no channel and no thread: a draft owns no worktree, no branch and
    /// no process, so there is nothing outside the app to tell and nothing to
    /// take down in order. That is the difference from retiring, stated as a
    /// signature rather than as a comment.
    ///
    /// **Only a draft, and only the one the selection is on.** Which is also why
    /// there is no second way in from the list: the row the list is on and the
    /// thing in the slot are the same draft, so throwing it away from its own
    /// form and throwing it away from the list are one act.
    ///
    /// **The selection only moves if the draft it was on is the one that went**,
    /// and it moves the way it does after a retirement — onto the first row of
    /// what is left, which is where the drafts and the attention-first order
    /// have already put whatever most wants somebody.
    fn discard(&mut self, latest: &Snapshot) {
        let Some(draft) = self.cursor.draft() else {
            return;
        };
        if !self.drafts.discarded(draft) {
            return;
        }

        self.moved(Step::Down, latest);
    }

    /// Move the selection a step through the list as it stands.
    ///
    /// **The order is worked out here rather than handed in**, because the only
    /// order a step may be taken through is the one the list would draw *now*.
    /// Every caller is a moment when the rows have just moved under the
    /// selection — a draft thrown away, a spawn let go of, the keyboard asking
    /// for the next row — and a step through an order taken before that lands on
    /// a row nobody pointed at.
    fn moved(&mut self, step: Step, latest: &Snapshot) {
        let order = list::order(self.drafts.all(), &self.entries, latest);
        self.cursor.moved(&order, step);
    }

    /// Let go of a spawn that has been retired, and say which pane it was in.
    ///
    /// Everything the app was holding of it goes at once — the row, and the
    /// supervisor's interest in a pane that is no longer there. Anything left
    /// behind would be the app watching something it has itself removed, and
    /// then reporting that it cannot find it.
    ///
    /// **The pane comes back rather than being dealt with here**, because the
    /// grid behind it belongs to the control client, and the client is not
    /// something the app's own state holds — the same reason the supervisor is
    /// told down a channel rather than asked.
    ///
    /// **The selection only moves if it was on the spawn that went.** It lands
    /// on the first row of what is left, which is where the attention-first
    /// order puts whatever most needs somebody — and on nothing at all when
    /// there is nothing left to be on.
    fn let_go_of(
        &mut self,
        name: &str,
        leaving: &Sender<String>,
        latest: &Snapshot,
    ) -> Option<String> {
        let retired = self.spawns.let_go_of(name)?;

        self.entries = self.spawns.entries();
        self.retirements.finished(name);
        // A supervisor that has gone means the app is on its way out, and a
        // spawn it goes on watching is a row nothing draws.
        let _ = leaving.send(name.to_string());

        if self.cursor.spawn() == Some(name) {
            self.moved(Step::Down, latest);
        }

        Some(retired.pane)
    }

    /// Take a spawn that has just started.
    ///
    /// The order is the order it has to be. The supervisor is told first, so the
    /// tick that follows already knows about a spawn the list is about to draw;
    /// the draft goes last, because until it does the row is the only thing
    /// saying where this came from.
    ///
    /// **The selection follows the spawn only if it was still on the draft.**
    /// Somebody who walked off to answer another spawn while this one was being
    /// made asked to be there, and a creation finishing is no reason to move
    /// them.
    fn adopt(&mut self, started: Started, draft: draft::Id, arriving: &Sender<Watched>) {
        let arrived = started.spawn.entry.spawn.clone();

        // A supervisor that has gone means the app is on its way out, and a
        // spawn it never hears about is a row with no status rather than a
        // wrong one.
        let _ = arriving.send(started.watched);
        self.spawns.add(started.spawn);
        self.entries = self.spawns.entries();
        if self.cursor.draft() == Some(draft) {
            self.cursor = Cursor::on_spawn(&arrived);
        }
        self.drafts.finished(draft);
    }
}

/// What the slot is showing.
///
/// A draft when the list is on one, and the selected spawn otherwise. The list
/// is what settles it, which is what makes composing something you walk into and
/// out of rather than a mode the app is put into.
pub enum InTheSlot<'a> {
    /// A spawn, and the screen it drew.
    Session(&'a Spawn),
    /// A draft, and the form it is being written in.
    Composing(&'a Draft),
    /// Nothing: every spawn there was has been retired, and no draft has been
    /// started since. The list is still there — it is the only thing that is —
    /// and what it says the keyboard does is how there comes to be something in
    /// the slot again.
    Nothing,
}

impl InTheSlot<'_> {
    /// How the keyboard is read while this is what the slot holds.
    fn typing(&self) -> Typing {
        match self {
            InTheSlot::Session(spawn) => Typing::IntoTheSpawn(spawn.modes()),
            InTheSlot::Composing(_) => Typing::IntoTheDraft,
            InTheSlot::Nothing => Typing::Nowhere,
        }
    }

    /// The pane a keystroke is addressed to, when there is one. A draft has
    /// none: it is not a process, and nothing it is typed into leaves the app.
    fn pane(&self) -> Option<&str> {
        match self {
            InTheSlot::Session(spawn) => Some(&spawn.pane),
            InTheSlot::Composing(_) | InTheSlot::Nothing => None,
        }
    }
}

/// What the slot holds with the list where it is.
fn in_the_slot<'a>(spawns: &'a Spawns, drafts: &'a Drafts, cursor: &Cursor) -> InTheSlot<'a> {
    match cursor.draft().and_then(|draft| drafts.of(draft)) {
        Some(draft) => InTheSlot::Composing(draft),
        None => spawns
            .showing(cursor)
            .map_or(InTheSlot::Nothing, InTheSlot::Session),
    }
}

/// Where the ordinary keys are going this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typing {
    /// To the spawn in the slot, in the modes its own screen asked for.
    IntoTheSpawn(Modes),
    /// Into the draft in the slot, which never leaves the app.
    IntoTheDraft,
    /// Nowhere: the slot is empty, so an ordinary key has nothing to mean.
    Nowhere,
}

/// What the user did with the keyboard.
enum Asked {
    /// Nothing, in the time a frame waits.
    Nothing,
    /// To leave — which kills nothing.
    Quit,
    /// To go up or down the list.
    Moved(Step),
    /// To start a draft.
    Composed,
    /// To make the draft in the slot into a spawn.
    Started,
    /// To retire the spawn the list is on.
    Retired,
    /// To throw the draft the list is on away.
    Discarded,
    /// Something for the draft in the slot.
    Edited(Edit),
    /// Something for the spawn, already in the bytes a terminal would send.
    Typed(Vec<u8>),
}

/// Wait a frame's worth of time for the keyboard.
fn asked_for(typing: Typing) -> io::Result<Asked> {
    if !event::poll(FRAME)? {
        return Ok(Asked::Nothing);
    }

    let Event::Key(key) = event::read()? else {
        return Ok(Asked::Nothing);
    };

    Ok(what_it_means(key, typing))
}

/// What one keystroke means.
///
/// The whole of the split between the app's keyboard and whatever is in the
/// slot, in one place and with nothing else in it: the seven keys named here
/// are the app's wherever the selection is, and everything else belongs to what
/// the slot is holding — bytes for a session, an edit for a draft.
///
/// **The app's keys are the app's whatever is in the slot**, including the two
/// that only ever do anything to a draft. A key that meant one thing over a
/// form and went to the session otherwise would be a key nobody could learn: it
/// would reach a spawn as a keystroke the moment the selection moved.
fn what_it_means(key: KeyEvent, typing: Typing) -> Asked {
    // Terminals that report a key going back up send the same key twice, and a
    // spawn would be typed into twice.
    if key.kind != KeyEventKind::Press {
        return Asked::Nothing;
    }

    match key.code {
        QUIT => Asked::Quit,
        COMPOSE => Asked::Composed,
        START => Asked::Started,
        RETIRE => Asked::Retired,
        DISCARD => Asked::Discarded,
        UP => Asked::Moved(Step::Up),
        DOWN => Asked::Moved(Step::Down),
        _ => match typing {
            Typing::IntoTheSpawn(modes) => Asked::Typed(keys::typed(key, modes)),
            Typing::IntoTheDraft => edited(key).map_or(Asked::Nothing, Asked::Edited),
            Typing::Nowhere => Asked::Nothing,
        },
    }
}

/// Whether a keystroke is an answer of *no* to a question a draft is standing
/// on.
///
/// **Everything is, except two things.** The answer of *yes* is one key, and a
/// frame in which nothing was pressed is not an answer at all; everything else
/// the user could do — moving, starting, retiring, typing at a session — says
/// they were not asking for this. Naming a second key for *no* would be a second
/// thing to learn about the one question in the app, and somebody half-
/// remembering which key it was would press the other one.
///
/// An edit is left out because the draft itself takes the question back, and
/// swallows the keystroke while it does: a key that both said *don't* and typed
/// itself into the field would leave a character behind from a keystroke that
/// meant the opposite.
fn takes_the_question_back(asked: &Asked) -> bool {
    !matches!(asked, Asked::Discarded | Asked::Nothing | Asked::Edited(_))
}

/// What one keystroke means to a form.
///
/// A table of its own rather than a second reading of [`keys::typed`]: a form is
/// not a terminal, so what it wants is what the key meant rather than the bytes
/// a terminal would have sent for it. A key with nothing here does nothing — a
/// draft is text somebody is writing, and the app inventing an edit for a key it
/// does not know would be the one thing that costs them the paragraph.
fn edited(key: KeyEvent) -> Option<Edit> {
    use event::KeyCode;

    if key
        .modifiers
        .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT)
    {
        return None;
    }

    match key.code {
        KeyCode::Char(character) => Some(Edit::Typed(character)),
        KeyCode::Backspace => Some(Edit::Erased),
        KeyCode::Delete => Some(Edit::Deleted),
        KeyCode::Left => Some(Edit::Left),
        KeyCode::Right => Some(Edit::Right),
        KeyCode::Home => Some(Edit::Start),
        KeyCode::End => Some(Edit::End),
        KeyCode::Up => Some(Edit::Up),
        KeyCode::Down => Some(Edit::Down),
        KeyCode::Tab => Some(Edit::Next),
        KeyCode::BackTab => Some(Edit::Previous),
        KeyCode::Enter => Some(Edit::Entered),
        _ => None,
    }
}

/// How the screen is divided.
///
/// The list, the line, and the slot. The line is one cell because a line is one
/// cell; everything else is a share of whatever the terminal turned out to be.
fn regions(area: Rect) -> (Rect, Rect, Rect) {
    let [list, separator, slot] = Layout::horizontal([
        Constraint::Percentage(LIST_SHARE),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);

    (list, separator, slot)
}

/// Paint one frame.
///
/// The snapshot and the retirements are here for one thing beyond the list: what
/// the app has to say about a spawn in sentences is said **in the slot**, which
/// is where somebody who saw the mark on its row went to find out what is wrong.
pub fn render(frame: &mut Frame, listing: Listing, showing: &InTheSlot) {
    let (list, separator, slot) = regions(frame.area());
    // The snapshot, and the retirement asked for below: both come off the
    // listing rather than alongside it, because the list and the slot are two
    // halves of one frame and must be drawn out of one moment.
    let latest = listing.snapshot();

    // Where the terminal's own cursor goes, asked of whatever drew the slot.
    // Without it the app would have a screen that looks like a session, or a
    // form, and no sign of where what you type is going.
    let caret = match showing {
        InTheSlot::Session(spawn) => {
            let screen = spawn.screen();
            frame.render_widget(&*screen, slot);
            explain(
                frame,
                slot,
                said_about(
                    &spawn.entry.spawn,
                    latest
                        .of(&spawn.entry.spawn)
                        .and_then(|row| row.unaccounted.as_ref()),
                    listing.retirement(&spawn.entry.spawn),
                    usize::from(slot.width),
                ),
            );

            screen.cursor()
        }
        InTheSlot::Composing(draft) => {
            // Laid out once and asked twice: the form is what says where the
            // caret goes, and working it out a second way would put it a cell
            // from the character it belongs to.
            let form = draft.form(Size::of(slot));
            let caret = form.caret();
            frame.render_widget(form, slot);

            caret
        }
        // Nothing drawn and no caret: there is nowhere for what you type to go,
        // and a cursor sitting in an empty region would say there was.
        InTheSlot::Nothing => None,
    };

    // Painted after the slot rather than before it, because rendering the
    // listing consumes it and the arm above asks it about the retirement. The
    // three regions are disjoint, so the order they are painted in is not
    // something the screen can tell.
    frame.render_widget(listing, list);
    frame.render_widget(Block::new().borders(Borders::LEFT), separator);

    if let Some((column, row)) = caret
        && column < slot.width
        && row < slot.height
    {
        frame.set_cursor_position((slot.x + column, slot.y + row));
    }
}

/// Everything the app has to say about the spawn in the slot, in the order it is
/// read.
///
/// Two sentences can apply at once — the app cannot account for a spawn *and*
/// would not retire it — and **the retirement goes last**: it is the newest
/// thing to have happened to the spawn, and a refused retirement is what
/// somebody who pressed the key came back to read.
///
/// A retirement under way is not written in amber. Amber is the colour the app
/// admits things in, and a retirement doing what it was asked is not an
/// admission; a refused one is, and takes it.
fn said_about(
    name: &str,
    unaccounted: Option<&Unaccounted>,
    retiring: Option<&Retirement>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(unaccounted) = unaccounted {
        for (at, sentence) in unaccounted.explained(name).iter().enumerate() {
            let how = if at == 0 { HEADING } else { DIM };
            lines.extend(
                wrapped(sentence, width)
                    .into_iter()
                    .map(|line| Line::styled(line, how.fg(AMBER))),
            );
        }
    }

    if let Some(retiring) = retiring {
        let how = if retiring.refused() {
            HEADING.fg(AMBER)
        } else {
            HEADING
        };
        lines.extend(
            wrapped(retiring.said(), width)
                .into_iter()
                .map(|line| Line::styled(line, how)),
        );
    }

    lines
}

/// Say it over the top of what the spawn in the slot drew.
///
/// **Over the top rather than instead of.** An unaccountable spawn is very often
/// still running — the whole point of the status is that it is the *app's*
/// instrumentation that failed, not the agent — so taking its screen away would
/// hide a live session to complain about the app's own eyesight. A spawn being
/// retired is being stopped, so its screen is worth even less. The top is what
/// the band covers, because the bottom of a harness's screen is where it asks
/// you things.
///
/// It costs the rows it takes, and gives them back the moment the app has
/// nothing left to say — which is what makes this the app's own writing rather
/// than a part of the layout.
fn explain(frame: &mut Frame, slot: Rect, lines: Vec<Line<'static>>) {
    let Ok(rows) = u16::try_from(lines.len()) else {
        return;
    };
    if rows == 0 {
        return;
    }
    let band = Rect {
        height: rows.min(slot.height),
        ..slot
    };

    // Cleared first: what is underneath is a session's own screen, and prose
    // written over the top of it without clearing would read as a sentence of
    // the session's own.
    frame.render_widget(Clear, band);
    frame.render_widget(Paragraph::new(lines), band);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Row, Status};
    use std::sync::{Arc, Mutex};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    use crate::draft::tests::drafting;
    use crate::list::On;
    use crate::snapshot;

    /// What the list says about one spawn.
    fn entry(repository: &str, spawn: &str) -> Entry {
        Entry {
            repository: repository.to_string(),
            spawn: spawn.to_string(),
            branch: format!("spawn/{spawn}"),
            worktree: format!("/data/harness-launcher/worktrees/{spawn}"),
        }
    }

    /// One spawn, for the tests that draw a list rather than switch between
    /// what is in the slot.
    fn entries() -> Vec<Entry> {
        vec![entry("harness-launcher", "add-retry-logic-a7f3")]
    }

    /// What the supervisor would have said about the one spawn there is.
    fn saying(status: Status, reason: Option<&str>) -> Snapshot {
        Snapshot {
            rows: vec![Row {
                name: entries()[0].spawn.clone(),
                status,
                unaccounted: reason.map(|why| snapshot::cannot_account(why, None)),
                last_known: snapshot::last_read(status),
            }],
        }
    }

    /// A spawn's screen with something recognisable on it.
    fn drew(size: Size, what: &str) -> Screen {
        let mut screen = Screen::new(size);
        screen.apply(what.as_bytes());

        screen
    }

    fn rendered(width: u16, height: u16) -> String {
        drawn(width, height, &saying(Status::Working, None), "")
    }

    fn drawn(width: u16, height: u16, snapshot: &Snapshot, slot: &str) -> String {
        let terminal = Size {
            columns: width,
            rows: height,
        };
        let entries = entries();
        let spawn = spawn_of(
            &entries[0].repository,
            &entries[0].spawn,
            "%1",
            slot_size(terminal),
            slot,
        );
        let cursor = Cursor::on_spawn(&entries[0].spawn);

        painted(
            terminal,
            &[],
            &entries,
            snapshot,
            &Retirements::default(),
            &cursor,
            &InTheSlot::Session(&spawn),
        )
    }

    /// One frame, as the text it puts on the terminal.
    fn painted(
        terminal: Size,
        drafts: &[Draft],
        entries: &[Entry],
        snapshot: &Snapshot,
        retirements: &Retirements,
        cursor: &Cursor,
        showing: &InTheSlot,
    ) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(terminal.columns, terminal.rows)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    Listing::new(drafts, entries, snapshot, retirements, cursor),
                    showing,
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();

        (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|column| buffer[(column, row)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The slot's shape, which a spawn's grid is always in.
    fn slot_size(terminal: Size) -> Size {
        let size = slot(terminal);
        if size.is_empty() {
            Size {
                columns: 1,
                rows: 1,
            }
        } else {
            size
        }
    }

    #[test]
    fn the_list_names_the_spawn_under_its_repository() {
        let screen = rendered(90, 12);

        let repository = screen.find("harness-launcher").unwrap();
        let spawn = screen.find("add-retry-logic-a7f3").unwrap();
        assert!(
            repository < spawn,
            "the repository heads its spawns:\n{screen}"
        );
    }

    /// **Neither the branch nor the worktree is on the running screen.** Both
    /// are the spawn's own name under something fixed — a branch prefix, one
    /// worktree root — so drawing them costs lines to restate the name already
    /// there. What the screen keeps is that name, which is what somebody going
    /// to find the work goes back with.
    #[test]
    fn the_running_screen_says_neither_the_branch_nor_the_worktree() {
        let screen = rendered(160, 12);

        assert!(screen.contains("add-retry-logic-a7f3"), "{screen}");
        assert!(!screen.contains("spawn/add-retry-logic-a7f3"), "{screen}");
        assert!(
            !screen.contains("/data/harness-launcher/worktrees"),
            "{screen}"
        );
    }

    #[test]
    fn a_narrow_list_still_names_the_spawn_and_says_what_the_keyboard_does() {
        let screen = rendered(72, 24);

        assert!(screen.contains("add-retry-logic-a7f3"), "{screen}");
        assert!(screen.contains("F10 quits"), "{screen}");
    }

    #[test]
    fn a_wide_terminal_is_not_a_frame_around_a_narrow_one() {
        let narrow = rendered(90, 12);
        let wide = rendered(200, 12);

        assert!(
            separator(&wide) > separator(&narrow),
            "the list is the same width on a terminal twice the size:\n{wide}"
        );
    }

    /// Which column the line between the list and the slot is drawn in.
    fn separator(screen: &str) -> usize {
        screen
            .lines()
            .find_map(|line| line.find('│'))
            .unwrap_or_else(|| panic!("nothing divides the screen:\n{screen}"))
    }

    #[test]
    fn a_terminal_too_short_for_everything_still_draws_the_spawn() {
        let screen = rendered(90, 3);

        // Three rows are not enough for the word above the list *and* the
        // spawn, and the spawn is what the list is for.
        assert!(screen.contains("add-retry-logic-a7f3"), "{screen}");
    }

    #[test]
    fn the_spawns_own_screen_is_what_the_slot_shows() {
        let screen = drawn(
            90,
            12,
            &saying(Status::Working, None),
            "⏺ I'll start by reading how retirement is wired.",
        );

        assert!(
            screen.contains("I'll start by reading how retirement is wired."),
            "the spawn's screen is not in the slot:\n{screen}"
        );
    }

    #[test]
    fn the_list_and_the_slot_are_divided_by_a_line_the_app_draws() {
        let screen = drawn(90, 12, &saying(Status::Working, None), "in the slot");

        for line in screen.lines() {
            let separator = line
                .find('│')
                .unwrap_or_else(|| panic!("a row of the screen has no separator on it:\n{screen}"));
            let list = line[..separator].to_string();
            assert!(
                !list.contains("in the slot"),
                "the slot spilled into the list:\n{screen}"
            );
        }
    }

    #[test]
    fn the_slot_is_the_size_the_spawn_was_told_it_had() {
        let terminal = Size {
            columns: 120,
            rows: 40,
        };

        let slot = slot(terminal);

        assert_eq!(slot.rows, terminal.rows);
        assert!(
            slot.columns > terminal.columns / 2,
            "the slot is not the larger half: {slot:?}"
        );

        let (list, separator, drawn) = regions(Rect::new(0, 0, terminal.columns, terminal.rows));
        assert_eq!(
            list.width + separator.width + drawn.width,
            terminal.columns,
            "the list, the separator and the slot do not add up to the terminal"
        );
        assert_eq!(
            Size::of(drawn),
            slot,
            "the size a spawn is told it has is not the region it is drawn into"
        );
    }

    /// Where the terminal's own cursor ended up, or nothing at all when the
    /// frame left it hidden.
    fn cursor_after(terminal: Size, showing: &InTheSlot) -> Option<(u16, u16)> {
        let mut backend = Terminal::new(TestBackend::new(terminal.columns, terminal.rows)).unwrap();
        backend
            .draw(|frame| {
                render(
                    frame,
                    Listing::new(
                        &[],
                        &entries(),
                        &Snapshot::default(),
                        &Retirements::default(),
                        &Cursor::default(),
                    ),
                    showing,
                );
            })
            .unwrap();

        let at = backend.backend().cursor_position();

        backend.backend().cursor_visible().then_some((at.x, at.y))
    }

    #[test]
    fn the_cursor_is_put_where_the_spawn_left_it() {
        let spawn = spawn_of(
            "harness-launcher",
            "add-retry-logic-a7f3",
            "%1",
            slot(TERMINAL),
            "\x1b[3;7H",
        );

        let (_, _, slot) = regions(Rect::new(0, 0, TERMINAL.columns, TERMINAL.rows));

        assert_eq!(
            cursor_after(TERMINAL, &InTheSlot::Session(&spawn)),
            Some((slot.x + 6, slot.y + 2))
        );
    }

    #[test]
    fn a_spawn_that_hid_its_cursor_does_not_get_one_drawn_anyway() {
        let spawn = spawn_of(
            "harness-launcher",
            "add-retry-logic-a7f3",
            "%1",
            slot(TERMINAL),
            "\x1b[?25l",
        );

        assert_eq!(cursor_after(TERMINAL, &InTheSlot::Session(&spawn)), None);
    }

    /// What is in **the slot**, whatever the list beside it says.
    ///
    /// The other half of [`row`], and there for the same reason: the list and
    /// the slot can be about the same spawn at once, so a screen read whole
    /// cannot say which of them something was drawn in.
    fn in_the_slot_of(screen: &str) -> String {
        screen
            .lines()
            .filter_map(|line| line.split_once('│'))
            .map(|(_, slot)| slot)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The row the spawn is on in **the list**, whatever else moved around it.
    ///
    /// The list half of the line and not the whole of it: the slot beside it can
    /// name the same spawn — an unaccountable one explains itself there — and a
    /// row read across the separator would be asserting about both at once.
    fn row(screen: &str) -> String {
        screen
            .lines()
            .filter_map(|line| line.split('│').next())
            .find(|list| list.contains("add-retry-logic-a7f3"))
            .unwrap_or_else(|| panic!("the spawn is not in the list:\n{screen}"))
            .to_string()
    }

    #[test]
    fn each_status_is_a_mark_of_its_own_beside_the_spawn() {
        let working = row(&drawn(90, 12, &saying(Status::Working, None), ""));
        let stopped = row(&drawn(90, 12, &saying(Status::Stopped, None), ""));
        let unknown = row(&drawn(
            90,
            12,
            &saying(Status::Unknown, Some("no record")),
            "",
        ));

        assert!(working.contains('·'), "{working}");
        assert!(stopped.contains('●'), "{stopped}");
        assert!(unknown.contains('?'), "{unknown}");
        assert_ne!(working.trim(), stopped.trim());
        assert_ne!(stopped.trim(), unknown.trim());
    }

    #[test]
    fn a_spawn_the_app_cannot_tell_about_says_why_on_screen() {
        let screen = drawn(
            180,
            14,
            &saying(
                Status::Unknown,
                Some("its session record carries no status"),
            ),
            "",
        );

        assert!(screen.contains("carries no status"), "{screen}");
    }

    /// How many rows of the list have anything on them.
    fn written(screen: &str) -> usize {
        screen
            .lines()
            .filter_map(|line| line.split('│').next())
            .filter(|list| !list.trim().is_empty())
            .count()
    }

    /// **An unaccountable spawn explains itself where somebody went to look at
    /// it.** The row says *something is wrong with this one*; the slot is where
    /// they came to find out what, and what they need is what the app knows and
    /// they do not — the pane, the process whose status would not resolve, that
    /// it is alive all the same, and what the app could last tell.
    ///
    /// This is the design's one open question about `unknown` being answered:
    /// the reason reaches the slot, over the top of the session's own screen,
    /// rather than replacing it.
    #[test]
    fn the_slot_explains_a_spawn_the_app_cannot_account_for() {
        let screen = drawn(
            180,
            20,
            &saying(
                Status::Unknown,
                Some("its session record carries no status"),
            ),
            "\x1b[12;1Hthe spawn is still drawing",
        );

        let pid = crate::tmux::ALIVE_PANE_PID.to_string();
        for fact in [
            "cannot tell",
            crate::tmux::ALIVE_PANE,
            pid.as_str(),
            "alive",
            "carries no status",
        ] {
            assert!(
                screen.contains(fact),
                "nothing in the slot says {fact}:\n{screen}"
            );
        }
        assert!(
            screen.contains("the spawn is still drawing"),
            "the explanation took the whole slot rather than the top of it:\n{screen}"
        );
        assert!(
            screen.contains("SPAWNS"),
            "explaining a spawn hid the list:\n{screen}"
        );
    }

    /// The same slot, with the spawn in it being retired.
    fn while_retiring(width: u16, height: u16, retirements: &Retirements, slot: &str) -> String {
        let terminal = Size {
            columns: width,
            rows: height,
        };
        let entries = entries();
        let spawn = spawn_of(
            &entries[0].repository,
            &entries[0].spawn,
            "%1",
            slot_size(terminal),
            slot,
        );

        painted(
            terminal,
            &[],
            &entries,
            &saying(Status::Working, None),
            retirements,
            &Cursor::on_spawn(&entries[0].spawn),
            &InTheSlot::Session(&spawn),
        )
    }

    /// A retirement of the one spawn there is, at whatever step it has reached.
    fn being_retired(step: &str) -> Retirements {
        let mut retirements = Retirements::default();
        retirements.asked_for(&entries()[0].spawn);
        retirements.doing(&entries()[0].spawn, step.to_string());

        retirements
    }

    /// **A retirement writes in the slot for the reason the explanation does.**
    /// It is the app writing about itself rather than the agent; and a spawn
    /// somebody has said they are done with is being stopped, so drawing over
    /// its screen costs nothing anybody was going to read. The row keeps the
    /// mark, which is what a spawn being retired says from across the list.
    #[test]
    fn the_slot_says_what_a_retirement_is_doing() {
        let screen = while_retiring(
            180,
            20,
            &being_retired("stopping the session"),
            "the spawn is still drawing",
        );

        assert!(
            in_the_slot_of(&screen).contains("stopping the session"),
            "the slot does not say what is happening to the spawn:\n{screen}"
        );
    }

    /// A refusal is the one thing here somebody has to act on, and it is a
    /// sentence naming the work that stopped it — which is why it goes where
    /// there is room for a sentence.
    #[test]
    fn the_slot_says_why_a_retirement_was_refused() {
        let mut retirements = being_retired("removing the worktree");
        retirements.refused(
            &entries()[0].spawn,
            "/w/add-retry-logic-a7f3 has work in it that is not committed".to_string(),
        );

        let screen = while_retiring(180, 20, &retirements, "the spawn is still drawing");

        assert!(
            in_the_slot_of(&screen).contains("not committed"),
            "the slot does not say why the retirement stopped:\n{screen}"
        );
    }

    /// **Both, and the retirement last.** A spawn can be one the app cannot read
    /// *and* one that would not retire, and the retirement is the newer of the
    /// two — as well as the one somebody pressing the key came back to read.
    #[test]
    fn a_spawn_that_is_unaccounted_for_and_would_not_retire_says_both() {
        let mut retirements = being_retired("removing the worktree");
        retirements.refused(
            &entries()[0].spawn,
            "there is work in it that is not committed".to_string(),
        );
        let terminal = Size {
            columns: 180,
            rows: 20,
        };
        let entries = entries();
        let spawn = spawn_of(
            &entries[0].repository,
            &entries[0].spawn,
            "%1",
            slot_size(terminal),
            "",
        );

        let screen = painted(
            terminal,
            &[],
            &entries,
            &saying(
                Status::Unknown,
                Some("its session record carries no status"),
            ),
            &retirements,
            &Cursor::on_spawn(&entries[0].spawn),
            &InTheSlot::Session(&spawn),
        );

        let slot = in_the_slot_of(&screen);
        let explanation = slot
            .find("carries no status")
            .unwrap_or_else(|| panic!("the slot does not explain the spawn:\n{screen}"));
        let refusal = slot
            .find("not committed")
            .unwrap_or_else(|| panic!("the slot does not say why it would not retire:\n{screen}"));
        assert!(
            explanation < refusal,
            "the retirement is not the last thing said:\n{screen}"
        );
    }

    /// And a spawn the app *can* account for gets the whole slot: the band is
    /// the app admitting something, so a spawn it has nothing to admit about
    /// must not carry one.
    #[test]
    fn a_spawn_the_app_can_account_for_keeps_the_whole_slot() {
        let screen = drawn(
            180,
            20,
            &saying(Status::Working, None),
            "the spawn is talking",
        );

        assert!(!screen.contains("cannot tell"), "{screen}");
        assert!(screen.contains("the spawn is talking"), "{screen}");
    }

    /// **A spawn the app cannot account for is still one row of the list.** What
    /// it costs is a band in the slot, so a list of twenty is the same list of
    /// twenty whether or not the app can read one of them.
    #[test]
    fn a_row_is_one_line_whether_or_not_there_is_anything_to_explain() {
        let explained = drawn(180, 14, &saying(Status::Unknown, Some("no record")), "");
        let plain = drawn(180, 14, &saying(Status::Working, None), "");

        assert_eq!(
            written(&explained),
            written(&plain),
            "explaining a spawn cost the list a line:\n{explained}"
        );
        assert!(
            explained.contains("no record"),
            "the sentence is nowhere on the screen at all:\n{explained}"
        );
    }

    #[test]
    fn before_the_first_snapshot_the_row_claims_nothing() {
        let screen = drawn(90, 12, &Snapshot::default(), "");
        let row = row(&screen);

        assert!(!row.contains('·'), "{row}");
        assert!(!row.contains('●'), "{row}");
        assert!(!row.contains('?'), "{row}");
        assert!(row.contains("add-retry-logic-a7f3"), "{row}");
    }

    /// Which column something starts in — cells, not bytes, since a mark and a
    /// blank of the same width are not the same number of bytes.
    fn column_of(row: &str, text: &str) -> usize {
        let at = row
            .find(text)
            .unwrap_or_else(|| panic!("{text} is not on the row: {row}"));

        row[..at].chars().count()
    }

    #[test]
    fn a_spawn_moves_between_statuses_without_moving_on_screen() {
        let columns: Vec<usize> = [
            saying(Status::Working, None),
            saying(Status::Stopped, None),
            saying(Status::Unknown, Some("no record")),
            Snapshot::default(),
        ]
        .iter()
        .map(|snapshot| column_of(&row(&drawn(90, 12, snapshot, "")), "add-retry-logic-a7f3"))
        .collect();

        assert!(
            columns.windows(2).all(|pair| pair[0] == pair[1]),
            "the name shifted sideways as the status changed: {columns:?}"
        );
    }

    /// What the app makes of one key, with a spawn in the slot.
    fn read_as(key: KeyEvent) -> Asked {
        what_it_means(
            key,
            Typing::IntoTheSpawn(Modes {
                application_cursor: false,
            }),
        )
    }

    /// What it makes of one key being pressed and nothing else.
    fn pressed(code: KeyCode) -> Asked {
        read_as(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// What it makes of one key with a draft in the slot.
    fn typed_at_a_draft(code: KeyCode) -> Asked {
        what_it_means(
            KeyEvent::new(code, KeyModifiers::NONE),
            Typing::IntoTheDraft,
        )
    }

    #[test]
    fn the_app_keeps_seven_keys_and_the_spawn_gets_every_other() {
        assert!(matches!(pressed(QUIT), Asked::Quit));
        assert!(matches!(pressed(COMPOSE), Asked::Composed));
        assert!(matches!(pressed(START), Asked::Started));
        assert!(matches!(pressed(RETIRE), Asked::Retired));
        assert!(matches!(pressed(DISCARD), Asked::Discarded));
        assert!(matches!(pressed(UP), Asked::Moved(Step::Up)));
        assert!(matches!(pressed(DOWN), Asked::Moved(Step::Down)));
        assert!(matches!(pressed(KeyCode::Char('2')), Asked::Typed(bytes) if bytes == b"2"));
        assert!(matches!(pressed(KeyCode::Esc), Asked::Typed(bytes) if bytes == [0x1b]));
    }

    /// The whole of the difference a draft makes to the keyboard: the app's own
    /// keys are still the app's, and everything else is an edit rather than
    /// bytes on their way to a session.
    #[test]
    fn a_draft_in_the_slot_takes_the_ordinary_keys_and_leaves_the_apps_alone() {
        assert!(matches!(typed_at_a_draft(QUIT), Asked::Quit));
        assert!(matches!(typed_at_a_draft(UP), Asked::Moved(Step::Up)));
        assert!(matches!(typed_at_a_draft(DOWN), Asked::Moved(Step::Down)));
        assert!(matches!(
            typed_at_a_draft(KeyCode::Char('2')),
            Asked::Edited(Edit::Typed('2'))
        ));
        assert!(matches!(
            typed_at_a_draft(KeyCode::Tab),
            Asked::Edited(Edit::Next)
        ));
        assert!(matches!(
            typed_at_a_draft(KeyCode::Backspace),
            Asked::Edited(Edit::Erased)
        ));
        assert!(matches!(typed_at_a_draft(START), Asked::Started));
        assert!(matches!(typed_at_a_draft(RETIRE), Asked::Retired));
        assert!(matches!(typed_at_a_draft(DISCARD), Asked::Discarded));
    }

    #[test]
    fn a_key_a_form_has_nothing_to_do_with_does_nothing_rather_than_something() {
        assert!(matches!(typed_at_a_draft(KeyCode::Esc), Asked::Nothing));
        assert!(matches!(
            what_it_means(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Typing::IntoTheDraft
            ),
            Asked::Nothing
        ));
    }

    #[test]
    fn a_key_going_back_up_is_not_a_second_keystroke() {
        let released = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert!(matches!(read_as(released), Asked::Nothing));
    }

    // More than one spawn, and which of them the slot is showing.

    /// The terminal these tests draw on, and the shape a grid is therefore in.
    const TERMINAL: Size = Size {
        columns: 90,
        rows: 14,
    };

    /// A spawn the app could be holding: its own name, its own pane, its own
    /// screen with something only it would have drawn on it.
    fn spawn_of(repository: &str, name: &str, pane: &str, size: Size, said: &str) -> Spawn {
        Spawn {
            entry: entry(repository, name),
            pane: pane.to_string(),
            grid: Arc::new(Mutex::new(drew(size, said))),
        }
    }

    /// Three spawns across two repositories, each having drawn its own name.
    fn several() -> Spawns {
        Spawns::new(vec![
            spawn_of(
                "harness-launcher",
                "add-retry-logic-a7f3",
                "%1",
                slot(TERMINAL),
                "the first spawn is talking",
            ),
            spawn_of(
                "some-other-project",
                "fix-the-flake-b2c9",
                "%2",
                slot(TERMINAL),
                "the second spawn is talking",
            ),
            spawn_of(
                "harness-launcher",
                "drop-the-cache-d4e1",
                "%3",
                slot(TERMINAL),
                "the third spawn is talking",
            ),
        ])
    }

    /// The whole screen, with the list on the named spawn.
    fn with_the_list_on(spawns: &Spawns, on: &str) -> String {
        showing(spawns, &Cursor::on_spawn(on))
    }

    /// The whole screen, as this cursor leaves it, with nothing being drafted.
    fn showing(spawns: &Spawns, cursor: &Cursor) -> String {
        with_drafts(spawns, &Drafts::new(Vec::new()), cursor)
    }

    /// The whole screen, drafts and all — the slot holding whatever the cursor
    /// is on, which is the app's own rule about the slot rather than the test's.
    fn with_drafts(spawns: &Spawns, drafts: &Drafts, cursor: &Cursor) -> String {
        with_retirements(spawns, drafts, &Retirements::default(), cursor)
    }

    /// The same, with some spawns being retired.
    fn with_retirements(
        spawns: &Spawns,
        drafts: &Drafts,
        retirements: &Retirements,
        cursor: &Cursor,
    ) -> String {
        painted(
            TERMINAL,
            drafts.all(),
            &spawns.entries(),
            &Snapshot::default(),
            retirements,
            cursor,
            &in_the_slot(spawns, drafts, cursor),
        )
    }

    #[test]
    fn the_slot_shows_the_spawn_the_list_is_on() {
        let spawns = several();

        let second = with_the_list_on(&spawns, "fix-the-flake-b2c9");

        assert!(second.contains("the second spawn is talking"), "{second}");
        assert!(!second.contains("the first spawn is talking"), "{second}");
        assert!(!second.contains("the third spawn is talking"), "{second}");
    }

    #[test]
    fn moving_the_selection_is_the_whole_of_switching() {
        let spawns = several();

        let screens: Vec<String> = [
            "add-retry-logic-a7f3",
            "fix-the-flake-b2c9",
            "drop-the-cache-d4e1",
        ]
        .iter()
        .map(|on| with_the_list_on(&spawns, on))
        .collect();

        for (which, said) in ["first", "second", "third"].iter().enumerate() {
            assert!(
                screens[which].contains(&format!("the {said} spawn is talking")),
                "the {said} spawn is not in the slot when the list is on it:\n{}",
                screens[which]
            );
        }
    }

    /// The differentiator, stated as a test: **no state of the app hides the
    /// list.** Whichever spawn is in the slot, every other spawn is still named
    /// beside it, under the repository it was started against.
    #[test]
    fn nothing_that_can_be_in_the_slot_takes_the_list_off_the_screen() {
        let spawns = several();

        for on in [
            "add-retry-logic-a7f3",
            "fix-the-flake-b2c9",
            "drop-the-cache-d4e1",
        ] {
            let screen = with_the_list_on(&spawns, on);

            for named in [
                "add-retry-logic-a7f3",
                "fix-the-flake-b2c9",
                "drop-the-cache-d4e1",
                "harness-launcher",
                "some-other-project",
            ] {
                assert!(
                    screen.contains(named),
                    "with the list on {on}, {named} is not on screen:\n{screen}"
                );
            }
        }
    }

    /// What makes switching free, and what it costs nothing to prove: the
    /// spawn nobody is looking at is being drawn into all along, so when it is
    /// selected there is nothing to catch up on.
    #[test]
    fn a_spawn_the_slot_is_not_showing_is_still_being_drawn_into() {
        let spawns = several();
        let off_screen = &spawns.all[1];

        let looking_elsewhere = with_the_list_on(&spawns, "add-retry-logic-a7f3");
        off_screen
            .screen()
            .apply(b"\r\nand it kept going while you were away");
        let arriving = with_the_list_on(&spawns, "fix-the-flake-b2c9");

        assert!(
            !looking_elsewhere.contains("kept going"),
            "the wrong spawn was in the slot:\n{looking_elsewhere}"
        );
        assert!(
            arriving.contains("and it kept going while you were away"),
            "what the spawn drew off screen did not arrive with it:\n{arriving}"
        );
    }

    /// The path a keystroke really takes, with nothing between the pieces
    /// stubbed: what `Asked::Moved` runs is `Cursor::moved` over
    /// [`list::order`], and what the slot then holds is `Spawns::showing` of
    /// that cursor. Ordering the walk by the list's own order is the point —
    /// the row the eye moves to and the screen that arrives have to be the
    /// same spawn.
    #[test]
    fn moving_the_selection_walks_the_slot_down_the_list_and_stops_at_the_end() {
        let spawns = several();
        let entries = spawns.entries();
        let latest = Snapshot::default();
        let order = list::order(&[], &entries, &latest);
        let mut cursor = Cursor::default();

        // One step from nowhere lands on the first row, whichever spawn the
        // attention-first order put there.
        let mut visited = Vec::new();
        for _ in 0..=order.len() {
            cursor.moved(&order, Step::Down);
            visited.push(On::Spawn(
                spawns
                    .showing(&cursor)
                    .expect("a spawn in the slot")
                    .entry
                    .spawn
                    .clone(),
            ));
        }

        assert_eq!(
            visited[..order.len()],
            order[..],
            "the slot did not follow the order the list draws"
        );
        assert_eq!(
            visited.last(),
            visited.get(order.len() - 1),
            "the selection ran off the bottom of the list"
        );
    }

    #[test]
    fn what_is_typed_goes_to_the_spawn_in_the_slot() {
        let spawns = several();

        assert_eq!(
            spawns
                .showing(&Cursor::on_spawn("fix-the-flake-b2c9"))
                .expect("a spawn in the slot")
                .pane,
            "%2"
        );
        assert_eq!(
            spawns
                .showing(&Cursor::on_spawn("drop-the-cache-d4e1"))
                .expect("a spawn in the slot")
                .pane,
            "%3"
        );
    }

    #[test]
    fn a_selection_on_nothing_yet_still_leaves_something_in_the_slot() {
        let spawns = several();

        let screen = showing(&spawns, &Cursor::default());

        assert!(screen.contains("the first spawn is talking"), "{screen}");
    }

    /// A resize is one event about the app's window, not about any one spawn,
    /// so it reaches every spawn — including the ones that will not be drawn
    /// until somebody selects them.
    #[test]
    fn the_slot_changing_shape_reaches_every_spawn_and_not_just_the_one_on_screen() {
        let spawns = several();
        let bigger = Size {
            columns: 120,
            rows: 40,
        };

        spawns.resize(bigger);

        for spawn in &spawns.all {
            assert_eq!(
                spawn.size(),
                bigger,
                "{} was left in the old shape",
                spawn.entry.spawn
            );
        }
        assert_eq!(spawns.shape(), Some(bigger));
    }

    /// The other way in: the app opened with nothing to start, which is one row
    /// and it is the form. Somebody who ran it to *decide* what to work on is
    /// looking at the one thing there is to do, without having pressed a key.
    #[test]
    fn an_app_opened_with_nothing_to_start_opens_on_the_draft() {
        let held = Held::new(Spawns::new(Vec::new()), drafting(&[""]));

        assert_eq!(held.cursor.draft(), Some(held.drafts.all()[0].id()));
        assert!(matches!(
            in_the_slot(&held.spawns, &held.drafts, &held.cursor),
            InTheSlot::Composing(_)
        ));
        let screen = on_screen(&held);
        assert!(screen.contains("NEW SPAWN"), "{screen}");
        assert!(screen.contains("Repository"), "{screen}");
        assert!(
            screen.contains("SPAWNS"),
            "the list is not there:\n{screen}"
        );
    }

    /// And a draft does not take the selection when sessions were asked for:
    /// those are what the person who named them is there to look at.
    #[test]
    fn an_app_opened_on_sessions_opens_on_one_of_them() {
        let held = Held::new(several(), drafting(&["a draft as well"]));

        assert_eq!(held.cursor.spawn(), Some("add-retry-logic-a7f3"));
    }

    // Drafts: the other thing the slot can hold.

    #[test]
    fn starting_a_draft_is_what_puts_it_in_the_slot() {
        let spawns = several();
        let mut drafts = Drafts::new(Vec::new());

        let cursor = Cursor::on_draft(drafts.start());

        assert!(matches!(
            in_the_slot(&spawns, &drafts, &cursor),
            InTheSlot::Composing(_)
        ));
    }

    /// The differentiator again, and this is the state it was most at risk in:
    /// a form is exactly the thing other tools make modal.
    #[test]
    fn a_draft_in_the_slot_is_a_form_and_the_list_is_still_beside_it() {
        let spawns = several();
        let drafts = drafting(&["fix the worktree cleanup"]);
        let on_it = Cursor::on_draft(drafts.all()[0].id());

        let screen = with_drafts(&spawns, &drafts, &on_it);

        assert!(screen.contains("NEW SPAWN"), "{screen}");
        assert!(screen.contains("Repository"), "{screen}");
        for named in [
            "add-retry-logic-a7f3",
            "fix-the-flake-b2c9",
            "drop-the-cache-d4e1",
            "harness-launcher",
            "some-other-project",
        ] {
            assert!(
                screen.contains(named),
                "with a draft in the slot, {named} is not on screen:\n{screen}"
            );
        }
    }

    /// What the whole design of a draft is for: leave a half-written paragraph,
    /// go and deal with a spawn, come back to it exactly as it was.
    #[test]
    fn walking_away_from_a_half_written_draft_and_back_leaves_the_text_alone() {
        let spawns = several();
        let drafts = drafting(&["half a sentence and"]);
        let on_it = Cursor::on_draft(drafts.all()[0].id());

        let composing = with_drafts(&spawns, &drafts, &on_it);
        let away = with_drafts(&spawns, &drafts, &Cursor::on_spawn("fix-the-flake-b2c9"));
        let back = with_drafts(&spawns, &drafts, &on_it);

        assert!(composing.contains("half a sentence and"), "{composing}");
        assert!(
            away.contains("the second spawn is talking"),
            "the draft did not give the slot back:\n{away}"
        );
        assert!(
            !away.contains("NEW SPAWN"),
            "the form is still in the slot:\n{away}"
        );
        assert_eq!(composing, back, "coming back is not what was left");
    }

    #[test]
    fn several_drafts_are_in_flight_at_once_and_each_holds_its_own_text() {
        let spawns = several();
        let drafts = drafting(&["the first draft", "the second draft"]);

        let first = with_drafts(&spawns, &drafts, &Cursor::on_draft(drafts.all()[0].id()));
        let second = with_drafts(&spawns, &drafts, &Cursor::on_draft(drafts.all()[1].id()));

        // The slot alone: both drafts have a row in the list beside it, and it
        // is the form that has to be holding the right one's text.
        assert!(
            in_the_slot_of(&first).contains("the first draft"),
            "{first}"
        );
        assert!(
            !in_the_slot_of(&first).contains("the second draft"),
            "{first}"
        );
        assert!(
            in_the_slot_of(&second).contains("the second draft"),
            "{second}"
        );
    }

    /// The whole screen, on a terminal tall enough for everything a form has to
    /// say — including the record of a creation that stopped, which is the last
    /// thing on it and the first thing a short slot scrolls away from.
    fn on_a_screen_with_room_for_everything(held: &Held) -> String {
        painted(
            Size {
                columns: 90,
                rows: 30,
            },
            held.drafts.all(),
            &held.entries,
            &Snapshot::default(),
            &held.retirements,
            &held.cursor,
            &in_the_slot(&held.spawns, &held.drafts, &held.cursor),
        )
    }

    /// A draft with both fields filled in, which is one that can be started.
    fn ready(repository: &str, work: &str) -> Drafts {
        let mut drafts = drafting(&[]);
        let id = drafts.start();
        for character in repository.chars() {
            drafts.edit(id, Edit::Typed(character));
        }
        drafts.edit(id, Edit::Next);
        for character in work.chars() {
            drafts.edit(id, Edit::Typed(character));
        }

        drafts
    }

    /// **The refusal that costs nothing, in the one place it is reached from.**
    /// A machine without the harness is found out before a worktree, a branch or
    /// a pane exists — and the draft is still there afterwards with the
    /// paragraph and every choice it had, so fixing the machine and pressing the
    /// key again is the whole of what it costs.
    #[test]
    fn a_draft_started_without_the_harness_installed_keeps_everything_it_said() {
        let somewhere = tempfile::tempdir().unwrap();
        let worktrees = somewhere.path().join("worktrees");
        let nothing_installed = tempfile::tempdir().unwrap();
        let mut held = Held::new(
            Spawns::new(Vec::new()),
            ready("/code/project", "add retry logic"),
        );
        held.cursor = Cursor::on_draft(held.drafts.all()[0].id());
        let (reporting, _reports) = mpsc::channel();

        held.start(
            worktrees.clone(),
            Some(nothing_installed.path().as_os_str().to_owned()),
            &reporting,
        );

        assert!(
            !worktrees.exists(),
            "something was made on disk for a spawn that was never going to start"
        );
        let screen = on_a_screen_with_room_for_everything(&held);
        assert!(
            screen.contains("NOT STARTED"),
            "the draft does not say it did not start:\n{screen}"
        );
        assert!(
            screen.contains(crate::harness::requirement().program),
            "the draft does not say what is missing:\n{screen}"
        );
        assert!(
            screen.contains("add retry logic"),
            "the refusal cost the paragraph:\n{screen}"
        );
        assert!(
            screen.contains("Blue") && screen.contains("Small"),
            "the refusal cost the choices that were picked:\n{screen}"
        );
    }

    // A draft becoming a spawn, which is the one thing that changes what the
    // app is holding while it runs.

    /// A spawn that has just been started, as the two views of it that arrive
    /// together.
    fn just_started(repository: &str, name: &str) -> Started {
        let spawn = spawn_of(
            repository,
            name,
            "%9",
            slot(TERMINAL),
            "the new spawn is talking",
        );

        Started {
            watched: Watched::new(name.to_string(), spawn.pane.clone()),
            spawn,
        }
    }

    /// Everything the app is holding while a draft of this work is in flight,
    /// and which draft that is.
    fn about_to_arrive(work: &str) -> (Held, draft::Id) {
        let held = Held::new(several(), drafting(&[work]));
        let draft = held.drafts.all()[0].id();

        (held, draft)
    }

    /// The whole screen, as this app is holding it.
    fn on_screen(held: &Held) -> String {
        with_retirements(&held.spawns, &held.drafts, &held.retirements, &held.cursor)
    }

    #[test]
    fn a_draft_that_started_becomes_a_spawn_row_in_its_repository_group() {
        let (mut held, draft) = about_to_arrive("start the scheduler");
        held.cursor = Cursor::on_draft(draft);
        let (arriving, _arrivals) = mpsc::channel();

        held.adopt(
            just_started("harness-launcher", "start-the-scheduler-c8d2"),
            draft,
            &arriving,
        );

        assert!(
            held.drafts.all().is_empty(),
            "the draft's row is still there"
        );
        let screen = on_screen(&held);
        let repository = screen.find("harness-launcher").unwrap();
        let spawn = screen.find("start-the-scheduler-c8d2").unwrap();
        assert!(
            repository < spawn,
            "the new spawn is not under the repository it was started against:\n{screen}"
        );
        assert!(
            screen.contains("the new spawn is talking"),
            "the spawn that just started is not in the slot:\n{screen}"
        );
    }

    #[test]
    fn the_supervisor_is_told_to_watch_a_spawn_that_has_just_started() {
        let (mut held, draft) = about_to_arrive("start the scheduler");
        held.cursor = Cursor::on_draft(draft);
        let (arriving, arrivals) = mpsc::channel();

        held.adopt(
            just_started("harness-launcher", "start-the-scheduler-c8d2"),
            draft,
            &arriving,
        );

        let watched = arrivals.try_recv().expect("the supervisor was not told");
        assert_eq!(watched.name, "start-the-scheduler-c8d2");
        assert_eq!(watched.pane, "%9");
    }

    /// A newly arrived spawn has a row before anything is known about it, and
    /// that row claims nothing — the grace period the supervisor counts from
    /// adoption is what keeps it from claiming the tooling is broken, and until
    /// its first snapshot the list says nothing at all rather than `unknown`.
    #[test]
    fn a_spawn_that_has_just_arrived_does_not_claim_to_be_unaccounted_for() {
        let (mut held, draft) = about_to_arrive("start the scheduler");
        held.cursor = Cursor::on_draft(draft);
        let (arriving, _arrivals) = mpsc::channel();

        held.adopt(
            just_started("harness-launcher", "start-the-scheduler-c8d2"),
            draft,
            &arriving,
        );

        let arrived = on_screen(&held)
            .lines()
            .find(|line| line.contains("start-the-scheduler-c8d2"))
            .expect("the spawn that just started has no row")
            .to_string();
        assert!(
            !arrived.contains('?'),
            "a spawn that started a moment ago says the app cannot tell what it is doing: {arrived}"
        );
    }

    /// Somebody who walked off to answer another spawn while this one was being
    /// made asked to be there. A creation finishing is not a reason to move
    /// them, and the slot is what would move.
    #[test]
    fn a_spawn_arriving_does_not_take_the_selection_off_what_it_was_on() {
        let (mut held, draft) = about_to_arrive("start the scheduler");
        held.cursor = Cursor::on_spawn("fix-the-flake-b2c9");
        let (arriving, _arrivals) = mpsc::channel();

        held.adopt(
            just_started("harness-launcher", "start-the-scheduler-c8d2"),
            draft,
            &arriving,
        );

        assert_eq!(held.cursor.spawn(), Some("fix-the-flake-b2c9"));
        let screen = on_screen(&held);
        assert!(screen.contains("the second spawn is talking"), "{screen}");
        assert!(
            screen.contains("start-the-scheduler-c8d2"),
            "the spawn that arrived is not in the list:\n{screen}"
        );
    }

    /// What the list says about the spawns is kept beside them rather than
    /// asked for every frame, so the one thing that can change it has to.
    #[test]
    fn a_spawn_arriving_is_in_what_the_list_is_drawn_from() {
        let (mut held, draft) = about_to_arrive("start the scheduler");
        let (arriving, _arrivals) = mpsc::channel();

        held.adopt(
            just_started("harness-launcher", "start-the-scheduler-c8d2"),
            draft,
            &arriving,
        );

        assert_eq!(held.entries, held.spawns.entries());
        assert!(
            held.entries
                .iter()
                .any(|entry| entry.spawn == "start-the-scheduler-c8d2")
        );
    }

    // Retiring a spawn, which is the other thing that changes what the app is
    // holding while it runs — and the only one that takes something away.

    /// Everything the app is holding, with three spawns and nothing drafted.
    fn holding() -> Held {
        Held::new(several(), Drafts::new(Vec::new()))
    }

    #[test]
    fn a_spawn_that_has_been_retired_leaves_the_list_and_the_supervisor_lets_go_of_it() {
        let mut held = holding();
        let (leaving, left) = mpsc::channel();

        let pane = held.let_go_of("fix-the-flake-b2c9", &leaving, &Snapshot::default());

        assert_eq!(
            pane.as_deref(),
            Some("%2"),
            "the pane it was in was not said"
        );
        assert_eq!(
            left.try_recv().ok(),
            Some("fix-the-flake-b2c9".to_string()),
            "the supervisor is still watching a pane that has gone"
        );
        let screen = on_screen(&held);
        assert!(
            !screen.contains("fix-the-flake-b2c9"),
            "the retired spawn still has a row:\n{screen}"
        );
        assert!(
            !screen.contains("the second spawn is talking"),
            "the retired spawn is still in the slot:\n{screen}"
        );
        assert_eq!(held.entries, held.spawns.entries());
        for still_there in ["add-retry-logic-a7f3", "drop-the-cache-d4e1"] {
            assert!(
                screen.contains(still_there),
                "the list lost more than the spawn that went:\n{screen}"
            );
        }
    }

    #[test]
    fn the_selection_follows_a_retired_spawn_off_its_row_and_stays_put_otherwise() {
        let mut moved_from_it = holding();
        moved_from_it.cursor = Cursor::on_spawn("fix-the-flake-b2c9");
        let mut left_alone = holding();
        left_alone.cursor = Cursor::on_spawn("drop-the-cache-d4e1");
        let (leaving, _left) = mpsc::channel();

        moved_from_it.let_go_of("fix-the-flake-b2c9", &leaving, &Snapshot::default());
        left_alone.let_go_of("fix-the-flake-b2c9", &leaving, &Snapshot::default());

        assert_eq!(
            moved_from_it.cursor.spawn(),
            Some("add-retry-logic-a7f3"),
            "the selection was left on a row that is not there any more"
        );
        assert_eq!(
            left_alone.cursor.spawn(),
            Some("drop-the-cache-d4e1"),
            "retiring one spawn moved the selection off another"
        );
    }

    /// The state the app can now reach that it never could before: everything
    /// retired, and the list still there. **Nothing hides the list** — least of
    /// all having nothing to put beside it.
    #[test]
    fn retiring_the_last_spawn_leaves_an_empty_slot_with_the_list_beside_it() {
        let mut held = holding();
        let (leaving, _left) = mpsc::channel();

        for spawn in [
            "add-retry-logic-a7f3",
            "fix-the-flake-b2c9",
            "drop-the-cache-d4e1",
        ] {
            held.let_go_of(spawn, &leaving, &Snapshot::default());
        }

        assert!(matches!(
            in_the_slot(&held.spawns, &held.drafts, &held.cursor),
            InTheSlot::Nothing
        ));
        let screen = on_screen(&held);
        assert!(
            screen.contains("SPAWNS"),
            "the list has gone too:\n{screen}"
        );
        assert!(
            screen.contains("F2 starts a draft"),
            "nothing says how to start one again:\n{screen}"
        );
        for talking in [
            "the first spawn is talking",
            "the second spawn is talking",
            "the third spawn is talking",
        ] {
            assert!(
                !screen.contains(talking),
                "{talking} is still there:\n{screen}"
            );
        }
    }

    /// With nothing in the slot there is nowhere for an ordinary key to go, and
    /// the app's own keys are still the app's — which is how there comes to be
    /// something in the slot again.
    #[test]
    fn an_empty_slot_takes_the_apps_keys_and_swallows_everything_else() {
        assert!(matches!(
            what_it_means(
                KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
                Typing::Nowhere
            ),
            Asked::Nothing
        ));
        assert!(matches!(
            what_it_means(KeyEvent::new(COMPOSE, KeyModifiers::NONE), Typing::Nowhere),
            Asked::Composed
        ));
    }

    #[test]
    fn a_spawn_that_was_never_here_is_not_let_go_of_twice() {
        let mut held = holding();
        let (leaving, _left) = mpsc::channel();

        assert_eq!(
            held.let_go_of("retired-long-ago", &leaving, &Snapshot::default()),
            None
        );
        assert_eq!(held.entries.len(), 3);
    }

    /// The key acts on the row rather than on what is in the slot — and on a
    /// draft it does nothing at all, because a draft has no session and nothing
    /// on disk to release.
    #[test]
    fn asking_to_retire_says_so_on_the_spawns_row_and_does_nothing_to_a_draft() {
        let tmux = crate::tmux::tests::PrivateTmux::start("app-asks-for-a-retirement");
        let mut held = Held::new(several(), drafting(&["half a sentence and"]));
        let (retiring, _reports) = mpsc::channel();

        held.cursor = Cursor::on_draft(held.drafts.all()[0].id());
        held.retire(&tmux.server, retiring.clone());
        assert!(
            held.retirements.of("add-retry-logic-a7f3").is_none(),
            "a draft in the slot retired a spawn"
        );

        held.cursor = Cursor::on_spawn("add-retry-logic-a7f3");
        held.retire(&tmux.server, retiring);

        assert!(
            held.retirements.of("add-retry-logic-a7f3").is_some(),
            "the row says nothing about the retirement that was asked for"
        );
        let screen = on_screen(&held);
        assert!(
            screen.contains("▍-✻ add-retry-logic-a7f3"),
            "the row does not say it is being retired:\n{screen}"
        );
    }

    // Throwing a draft away, which is the third thing that changes what the app
    // is holding while it runs — and the only one that takes something away
    // without touching anything outside the app.

    /// **It asks first, and the question is on the screen somebody is looking
    /// at.** Then the row goes, the form goes with it, and nothing else on the
    /// screen moved: the other draft still holds its own text and every spawn is
    /// still listed under its own repository.
    #[test]
    fn discarding_the_draft_the_list_is_on_asks_first_and_then_takes_only_that_row() {
        let mut held = Held::new(
            several(),
            drafting(&["the first draft", "the second draft"]),
        );
        held.cursor = Cursor::on_draft(held.drafts.all()[1].id());

        held.discard(&Snapshot::default());
        let asking = on_screen(&held);
        held.discard(&Snapshot::default());

        assert!(
            asking.contains("DISCARD"),
            "the draft went without the app asking:\n{asking}"
        );
        assert!(
            asking.contains("the second draft"),
            "the question cost the typing it was asked about:\n{asking}"
        );
        let screen = on_screen(&held);
        assert!(
            !screen.contains("the second draft"),
            "the draft that was discarded still has a row:\n{screen}"
        );
        assert!(
            screen.contains("the first draft"),
            "discarding one draft took another with it:\n{screen}"
        );
        for still_there in [
            "add-retry-logic-a7f3",
            "fix-the-flake-b2c9",
            "drop-the-cache-d4e1",
        ] {
            assert!(
                screen.contains(still_there),
                "discarding a draft cost a spawn:\n{screen}"
            );
        }
    }

    /// The selection lands on the first row of what is left — and the slot
    /// follows it, which is what *the pane closes* means for a thing that never
    /// had one.
    #[test]
    fn the_selection_leaves_the_row_a_discarded_draft_was_on() {
        let mut held = Held::new(
            several(),
            drafting(&["the first draft", "the second draft"]),
        );
        held.cursor = Cursor::on_draft(held.drafts.all()[1].id());

        held.discard(&Snapshot::default());
        held.discard(&Snapshot::default());

        assert_eq!(held.cursor.draft(), Some(held.drafts.all()[0].id()));
        let screen = on_screen(&held);
        assert!(
            screen.contains("  the first draft"),
            "the slot is not showing the draft the selection landed on:\n{screen}"
        );
    }

    /// The last draft there was, on an app that has spawns: the slot goes back
    /// to a session, and the list is still the list.
    #[test]
    fn discarding_the_only_draft_leaves_the_slot_on_a_spawn() {
        let mut held = Held::new(several(), drafting(&["the only draft"]));
        held.cursor = Cursor::on_draft(held.drafts.all()[0].id());

        held.discard(&Snapshot::default());
        held.discard(&Snapshot::default());

        assert!(held.drafts.all().is_empty());
        let screen = on_screen(&held);
        assert!(
            !screen.contains("NEW SPAWN"),
            "the form is still in the slot:\n{screen}"
        );
        assert!(screen.contains("the first spawn is talking"), "{screen}");
        assert!(screen.contains("SPAWNS"), "the list has gone:\n{screen}");
    }

    /// **The key does nothing at all to a spawn.** A spawn has a session, a
    /// worktree and a branch, and getting rid of one is retiring — an ordered
    /// teardown that can refuse. Nothing here is allowed to be a second way into
    /// that.
    #[test]
    fn discarding_does_nothing_when_the_list_is_on_a_spawn() {
        let mut held = Held::new(several(), drafting(&["a draft as well"]));
        held.cursor = Cursor::on_spawn("fix-the-flake-b2c9");

        held.discard(&Snapshot::default());
        held.discard(&Snapshot::default());

        assert_eq!(held.entries.len(), 3);
        assert_eq!(held.drafts.all().len(), 1);
        assert_eq!(held.cursor.spawn(), Some("fix-the-flake-b2c9"));
    }

    /// The rule that makes one key enough: the *yes* is a key, and everything
    /// else is the *no*.
    #[test]
    fn anything_but_the_answer_takes_a_question_about_a_draft_back() {
        assert!(!takes_the_question_back(&Asked::Discarded));
        assert!(!takes_the_question_back(&Asked::Nothing));
        // The draft takes this one back itself, and swallows the key with it.
        assert!(!takes_the_question_back(&Asked::Edited(Edit::Typed('x'))));

        for anything_else in [
            Asked::Moved(Step::Down),
            Asked::Composed,
            Asked::Started,
            Asked::Retired,
            Asked::Typed(vec![b'a']),
        ] {
            assert!(takes_the_question_back(&anything_else));
        }
    }

    #[test]
    fn the_cursor_is_put_in_the_form_being_typed_into_rather_than_left_on_the_list() {
        let blank = drafting(&[""]);
        let six = drafting(&["typing"]);
        let (_, _, slot) = regions(Rect::new(0, 0, TERMINAL.columns, TERMINAL.rows));

        let empty_at = cursor_after(TERMINAL, &InTheSlot::Composing(&blank.all()[0]))
            .expect("a caret in the field being typed into");
        let typed_at = cursor_after(TERMINAL, &InTheSlot::Composing(&six.all()[0]))
            .expect("a caret in the field being typed into");

        assert!(
            empty_at.0 >= slot.x,
            "the caret is on the list rather than in the slot: {empty_at:?}"
        );
        assert_eq!(
            typed_at,
            (empty_at.0 + 6, empty_at.1),
            "the caret did not follow the six characters that were typed"
        );
    }
}
