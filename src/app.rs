//! The whole screen: the list on the left, the slot on the right, drawn by the
//! app in one pass.
//!
//! The slot shows either the selected spawn's screen — a grid the control-mode
//! client keeps current for every spawn, shown or not — or a draft form the app
//! edits itself. Which one is settled once per frame from the list's selection.
//! The app sends a spawn only what the user typed, plus nothing: its own
//! keyboard is the seven function keys below. Switching spawns is a re-render
//! and nothing else; no state of the app hides the list; every dimension is
//! read from the real terminal each frame, and the slot's size is what tmux
//! panes are told they have.

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
/// Short, because the slot's content changes without any key being pressed.
const FRAME: Duration = Duration::from_millis(16);

/// How much of the screen the list takes — a share, so it scales with the
/// terminal.
const LIST_SHARE: u16 = 33;

/// Leave, which kills nothing.
///
/// A function key, like all the app's keys: every ordinary key belongs to the
/// spawn in the slot.
const QUIT: event::KeyCode = event::KeyCode::F(10);
/// Take the selection one row up the list.
const UP: event::KeyCode = event::KeyCode::F(6);
/// Take it one row down.
const DOWN: event::KeyCode = event::KeyCode::F(7);
/// Throw the draft the list is on away.
///
/// Safe beside `F2` (compose) only because it asks first: the first press is a
/// question, and any other key answers it *no*.
const DISCARD: event::KeyCode = event::KeyCode::F(3);

/// Start a draft, and put the selection on it.
const COMPOSE: event::KeyCode = event::KeyCode::F(2);

/// Retire the spawn the list is on. Kept away from the movement keys.
///
/// Accepted cost: reaching for `F10` and landing here stops the selected
/// spawn — but a worktree with uncommitted work refuses, and the branch stays.
const RETIRE: event::KeyCode = event::KeyCode::F(9);

/// Make the draft in the slot into a spawn.
///
/// Not `Enter`: the form's own fields already give `Enter` two meanings.
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
    /// Hold only for a copy: the reader thread waits on the same lock.
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
/// Switching is a re-render and nothing else: the control-mode client fills
/// every grid whether or not the app is drawing it, so a selected spawn is
/// already current and the one that left carries on untouched.
pub struct Spawns {
    /// In the order they were started, which the list groups them from.
    all: Vec<Spawn>,
}

impl Spawns {
    /// The spawns the app is to show; none is an ordinary state.
    pub fn new(all: Vec<Spawn>) -> Self {
        Self { all }
    }

    /// What the list says about all of them.
    /// Taken when the set of spawns changes rather than every frame.
    pub fn entries(&self) -> Vec<Entry> {
        self.all.iter().map(|spawn| spawn.entry.clone()).collect()
    }

    /// Take a spawn that did not exist when the app started.
    fn add(&mut self, spawn: Spawn) {
        self.all.push(spawn);
    }

    /// The spawn of this name, if the app still has it.
    fn of(&self, name: &str) -> Option<&Spawn> {
        self.all.iter().find(|spawn| spawn.entry.spawn == name)
    }

    /// Remove a retired spawn and hand it back; the caller still needs its
    /// pane.
    fn let_go_of(&mut self, name: &str) -> Option<Spawn> {
        let at = self
            .all
            .iter()
            .position(|spawn| spawn.entry.spawn == name)?;

        Some(self.all.remove(at))
    }

    /// The spawn in the slot, when there is one.
    /// A cursor on nothing, or on a spawn not here, shows the first started.
    fn showing(&self, cursor: &Cursor) -> Option<&Spawn> {
        cursor
            .spawn()
            .and_then(|on| self.of(on))
            .or_else(|| self.all.first())
    }

    /// The shape their screens are in.
    /// One answer for all: spawns are created at the slot's size and resized
    /// together.
    fn shape(&self) -> Option<Size> {
        self.all.first().map(Spawn::size)
    }

    /// Catch every spawn up with the shape the slot has become.
    /// Every one, not only the one on screen: an old-shape grid would clip its
    /// spawn long before anybody selected it.
    fn resize(&self, slot: Size) {
        for spawn in &self.all {
            spawn.resize(slot);
        }
    }
}

/// How big the slot is when the terminal is this big.
/// The panes tmux opens must be the shape of the region they are drawn into;
/// a disagreement shows up as a child drawing off the edge.
pub fn slot(terminal: Size) -> Size {
    Size::of(regions(Rect::new(0, 0, terminal.columns, terminal.rows)).2)
}

/// How big the slot is on the terminal the app was started on.
/// Refuses on a terminal too small to hold a slot, while there is still a
/// shell to say so on.
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
    /// Where the worktrees the app creates go; resolved before the screen was
    /// taken over.
    pub worktrees: PathBuf,
}

/// Draw everything until the user quits.
///
/// Snapshots collapse to the latest; creation and retirement reports are all
/// applied, since each one records something that happened. An empty slot is
/// an ordinary state at both ends of a run.
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

            // Applied before drawing: these can add or remove a spawn, and the
            // list, the slot and the keyboard must see the same set.
            while let Ok(report) = reports.try_recv() {
                reported(report, held, world, shape);
            }
            while let Ok(report) = retirements.try_recv() {
                retired(report, held, world, &latest);
            }

            // Settled once per frame, so the screen drawn, the keyboard and
            // the addressed pane are the same spawn or draft by construction.
            let showing = in_the_slot(&held.spawns, &held.drafts, &held.cursor);
            let typing = showing.typing();
            // The pane's name rather than the pane, so the borrow of the slot
            // ends before an edit needs the drafts back.
            let addressed = showing.pane().map(str::to_string);
            let listing = Listing::new(
                held.drafts.all(),
                &held.entries,
                &latest,
                &held.retirements,
                &held.cursor,
            );

            terminal.draw(|frame| render(frame, listing, &showing))?;

            // Checked every frame: a client that has gone leaves every grid
            // frozen, with no other symptom.
            world.client.listening().map_err(io::Error::other)?;

            // Read off the frame just drawn, so what the child is told and
            // what the app draws cannot differ.
            let wanted = slot(Size::of(terminal.get_frame().area()));
            if wanted != shape && !wanted.is_empty() {
                // The grids first: the resize reaches the children as a redraw,
                // and a grid still the old shape would clip it.
                held.spawns.resize(wanted);
                world.client.resize(wanted).map_err(io::Error::other)?;
                shape = wanted;
            }

            let asked = asked_for(typing)?;
            // A question standing on a draft is answered *no* by any key that
            // is not the answer, wherever the key was aimed.
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
/// Starting the harness happens here rather than on the creation's thread,
/// because tmux and the control client belong to the thread that holds them.
fn reported(report: Report, held: &mut Held, world: &World, slot: Size) {
    match report.said {
        Said::Doing(step) => held.drafts.doing(report.draft, step),
        Said::Refused(why) => held.drafts.failed(report.draft, why),
        Said::Made(plan) => {
            // Said before it is done, so a harness that fails to start leaves
            // a record of what was made.
            held.drafts.doing(
                report.draft,
                format!("starting the harness in {}", plan.entry.worktree),
            );

            match creation::start(world.server, world.session, world.client, slot, *plan) {
                Ok(started) => held.adopt(started, report.draft, &world.arriving),
                // Accepted cost: the worktree and branch stay. The draft's
                // record names them, so the litter is visible.
                Err(refused) => held.drafts.failed(report.draft, refused.to_string()),
            }
        }
    }
}

/// Do what one thing a retirement said calls for.
/// Progress lands on the spawn's row; the final word releases every piece of
/// the spawn at once.
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
/// the list is on. The other half of a frame is [`World`].
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
    /// Cached: only a spawn arriving or leaving can change it.
    entries: Vec<Entry>,
}

impl Held {
    /// What the app starts with, the selection on the first spawn when there
    /// is one and on the first draft otherwise.
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
    /// The harness is checked before the thread is started — the last moment a
    /// refusal costs nothing on disk. The refusal lands on the draft, which
    /// keeps its text. `PATH` comes from the caller
    /// ([`creation::harness_installed`]).
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
    /// Only a spawn, and only the selected one. The work goes on a thread so
    /// every other spawn keeps drawing meanwhile.
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

    /// Throw the draft the list is on away, once it has said yes to being
    /// asked.
    ///
    /// Touches only the app's own state: a draft owns no worktree, branch or
    /// process. The selection moves only if it was on the draft that went.
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
    /// The order is computed here, not handed in: every caller has just
    /// changed the rows.
    fn moved(&mut self, step: Step, latest: &Snapshot) {
        let order = list::order(self.drafts.all(), &self.entries, latest);
        self.cursor.moved(&order, step);
    }

    /// Let go of a spawn that has been retired, and say which pane it was in.
    ///
    /// The pane goes back to the caller because its grid belongs to the
    /// control client, which the app's own state does not hold. The selection
    /// moves only if it was on the spawn that went.
    fn let_go_of(
        &mut self,
        name: &str,
        leaving: &Sender<String>,
        latest: &Snapshot,
    ) -> Option<String> {
        let retired = self.spawns.let_go_of(name)?;

        self.entries = self.spawns.entries();
        self.retirements.finished(name);
        // A supervisor that has gone means the app is on its way out.
        let _ = leaving.send(name.to_string());

        if self.cursor.spawn() == Some(name) {
            self.moved(Step::Down, latest);
        }

        Some(retired.pane)
    }

    /// Take a spawn that has just started.
    ///
    /// Ordering constraint: the supervisor is told first, and the draft is
    /// removed last. The selection follows the spawn only if it was still on
    /// the draft.
    fn adopt(&mut self, started: Started, draft: draft::Id, arriving: &Sender<Watched>) {
        let arrived = started.spawn.entry.spawn.clone();

        // A supervisor that has gone means the app is on its way out.
        let _ = arriving.send(started.watched);
        self.spawns.add(started.spawn);
        self.entries = self.spawns.entries();
        if self.cursor.draft() == Some(draft) {
            self.cursor = Cursor::on_spawn(&arrived);
        }
        self.drafts.finished(draft);
    }
}

/// What the slot is showing: a draft when the list is on one, and the selected
/// spawn otherwise.
pub enum InTheSlot<'a> {
    /// A spawn, and the screen it drew.
    Session(&'a Spawn),
    /// A draft, and the form it is being written in.
    Composing(&'a Draft),
    /// Nothing: every spawn retired and no draft started; the list remains.
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

    /// The pane a keystroke is addressed to, when there is one; a draft has
    /// none.
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
/// The seven keys named here are the app's wherever the selection is;
/// everything else belongs to what the slot holds — bytes for a session, an
/// edit for a draft.
fn what_it_means(key: KeyEvent, typing: Typing) -> Asked {
    // Terminals that report key releases would otherwise type each key twice.
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

/// Whether a keystroke answers *no* to a question a draft is standing on.
///
/// Everything does, except the *yes* key and a frame with no key at all. An
/// edit is left out because the draft itself takes the question back and
/// swallows the keystroke while it does.
fn takes_the_question_back(asked: &Asked) -> bool {
    !matches!(asked, Asked::Discarded | Asked::Nothing | Asked::Edited(_))
}

/// What one keystroke means to a form.
/// A form wants what the key meant, not the bytes a terminal would send
/// ([`keys::typed`]); an unknown key does nothing rather than something.
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

/// How the screen is divided: the list, the line, and the slot.
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
/// What the app has to say about a spawn in sentences is said in the slot.
pub fn render(frame: &mut Frame, listing: Listing, showing: &InTheSlot) {
    let (list, separator, slot) = regions(frame.area());
    // Taken off the listing, so the list and the slot draw from one moment.
    let latest = listing.snapshot();

    // Where the terminal's own cursor goes, asked of whatever drew the slot.
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
            // The form says where the caret goes; recomputing it would drift.
            let form = draft.form(Size::of(slot));
            let caret = form.caret();
            frame.render_widget(form, slot);

            caret
        }
        InTheSlot::Nothing => None,
    };

    // After the slot: rendering the listing consumes it, and the arm above
    // still queries it. The regions are disjoint, so paint order is invisible.
    frame.render_widget(listing, list);
    frame.render_widget(Block::new().borders(Borders::LEFT), separator);

    if let Some((column, row)) = caret
        && column < slot.width
        && row < slot.height
    {
        frame.set_cursor_position((slot.x + column, slot.y + row));
    }
}

/// Everything the app has to say about the spawn in the slot, in the order it
/// is read.
///
/// Both sentences can apply at once, and the retirement goes last as the
/// newest. Amber marks only a refused retirement, not one under way.
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
/// Over the top rather than instead of: an unaccountable spawn is often still
/// running. The band covers the top because the bottom of a harness's screen
/// is where it asks you things.
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

    // Cleared first, or the prose would blend into the session's own screen.
    frame.render_widget(Clear, band);
    frame.render_widget(Paragraph::new(lines), band);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Row, Status};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

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

    /// One spawn, for the tests that draw a list.
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
                changed: Some(Instant::now()),
                age: Some(Duration::from_mins(31)),
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

        // Seventy-two columns leave the list twenty-four, which is not room
        // for the name and the age together. The age goes, the name stays.
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

    /// Where the terminal's own cursor ended up, or nothing when left hidden.
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

    /// What is in the slot, whatever the list beside it says.
    fn in_the_slot_of(screen: &str) -> String {
        screen
            .lines()
            .filter_map(|line| line.split_once('│'))
            .map(|(_, slot)| slot)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The row the spawn is on in the list — the list half of the line only,
    /// since the slot can name the same spawn.
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

    /// Which column something starts in — cells, not bytes.
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

    /// A spawn the app could be holding.
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

    /// The whole screen, drafts and all.
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

    #[test]
    fn moving_the_selection_walks_the_slot_down_the_list_and_stops_at_the_end() {
        let spawns = several();
        let entries = spawns.entries();
        let latest = Snapshot::default();
        let order = list::order(&[], &entries, &latest);
        let mut cursor = Cursor::default();

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

    /// The whole screen, on a terminal tall enough for everything a form says.
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

    // A draft becoming a spawn.

    /// A spawn that has just been started.
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

    /// Everything held while a draft of this work is in flight, and its id.
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

    // Retiring a spawn.

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

    // Throwing a draft away.

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
