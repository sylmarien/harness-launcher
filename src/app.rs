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
//! user typed** — it has no keyboard of its own beyond the four keys that leave,
//! move the selection and start a draft.
//!
//! **Or the slot holds a draft**, which is a form the app draws and types into
//! itself. That is the one place an ordinary key means something to the app
//! rather than to a session, and which of the two it is comes from what the list
//! is on — settled once a frame, like everything else about the slot.
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

use std::io;
use std::sync::MutexGuard;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::crossterm::terminal;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, Borders};

use crate::control::{Client, Grid, POISONED};
use crate::draft::{Draft, Drafts, Edit};
use crate::error::{Error, Result};
use crate::keys::{self, Modes};
use crate::list::{self, Cursor, Entry, Listing, Step};
use crate::screen::{Screen, Size};
use crate::snapshot::Snapshot;

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
/// `q` is a letter somebody is in the middle of typing. This and the two below
/// are the whole of the app's keyboard so far, and how the rest of it is divided
/// is not settled yet — so the keys that move the selection sit well away from
/// the one that leaves, and nothing else is claimed.
const QUIT: event::KeyCode = event::KeyCode::F(10);
/// Take the selection one row up the list.
const UP: event::KeyCode = event::KeyCode::F(6);
/// Take it one row down.
const DOWN: event::KeyCode = event::KeyCode::F(7);
/// Start a draft, and put the selection on it.
///
/// A fourth function key, and it has to be one: composing is reached from a
/// spawn in the slot, where every ordinary key is that session's. It sits well
/// away from the two that move, because starting a draft by mistyping a
/// selection is a row appearing in the list nobody asked for.
const COMPOSE: event::KeyCode = event::KeyCode::F(2);

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
    /// The spawns the app is to show. There has to be at least one.
    ///
    /// Not a state the app can reach today — the command line refuses to ask
    /// for nothing — but the slot is not designed to be empty, and an app
    /// drawing a screen with no session on it is not something to work out on
    /// the way past.
    pub fn new(all: Vec<Spawn>) -> Result<Self> {
        if all.is_empty() {
            return Err(Error::new("there is no spawn to show"));
        }

        Ok(Self { all })
    }

    /// What the list says about all of them.
    ///
    /// Taken once rather than every frame: what the list says about a spawn is
    /// settled when it is created, and it is the snapshot beside it that
    /// changes.
    pub fn entries(&self) -> Vec<Entry> {
        self.all.iter().map(|spawn| spawn.entry.clone()).collect()
    }

    /// The spawn in the slot.
    ///
    /// A cursor on nothing — or on a spawn that is not here — shows the first
    /// that was started, because the slot is never empty while there is
    /// something to put in it. Nothing the app does reaches that fallback: the
    /// selection starts on a spawn and every move lands on another, so this is
    /// what the type needs rather than a case with behaviour to defend.
    fn showing(&self, cursor: &Cursor) -> &Spawn {
        cursor
            .spawn()
            .and_then(|on| self.all.iter().find(|spawn| spawn.entry.spawn == on))
            .unwrap_or(&self.all[0])
    }

    /// The shape their screens are in.
    ///
    /// One answer for all of them: they are created at the slot's size and
    /// resized together, so a spawn whose grid was a different shape from its
    /// neighbours' would be a bug rather than a case to handle.
    fn shape(&self) -> Size {
        self.all[0].size()
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

/// Draw everything until the user quits.
///
/// Snapshots are drained rather than queued: what the user wants to see is what
/// is true now, so a frame that arrives behind several ticks skips them.
pub fn run(
    spawns: &Spawns,
    drafts: &mut Drafts,
    snapshots: &Receiver<Snapshot>,
    client: &Client,
) -> Result<()> {
    let mut latest = Snapshot::default();
    let mut shape = spawns.shape();
    let entries = spawns.entries();
    let mut cursor = Cursor::on_spawn(&entries[0].spawn);

    ratatui::run(|terminal| -> io::Result<()> {
        loop {
            while let Ok(snapshot) = snapshots.try_recv() {
                latest = snapshot;
            }

            // What is in the slot is settled once, at the top of the frame. The
            // screen drawn, the way the keyboard is read and the pane a
            // keystroke is addressed to are then the same spawn — or the same
            // draft — by construction; asking again further down would let a
            // selection that moved mid-frame send what was typed to the spawn
            // that left.
            let showing = in_the_slot(spawns, drafts, &cursor);
            let typing = showing.typing();
            // The pane's name rather than the pane, so that what is in the slot
            // is done being borrowed by the time an edit needs the drafts back.
            // A frame's worth of one short string, against a keystroke reaching
            // the wrong spawn.
            let addressed = showing.pane().map(str::to_string);
            let listing = Listing::new(drafts.all(), &entries, &latest, &cursor);

            terminal.draw(|frame| render(frame, listing, &showing))?;

            // A client that has gone leaves every grid exactly as it was, which
            // on screen is a session sitting there thinking. Asked here rather
            // than only when something is typed, because the user has no reason
            // to type at a spawn that looks busy.
            client.listening().map_err(io::Error::other)?;

            // The slot's shape is read off the frame that was just drawn rather
            // than worked out again from the terminal, so what the child is told
            // it has and what the app draws cannot come to differ.
            let wanted = slot(Size::of(terminal.get_frame().area()));
            if wanted != shape && !wanted.is_empty() {
                // The grids first: the resize reaches the children as a redraw,
                // and a grid still the old shape would clip it.
                spawns.resize(wanted);
                client.resize(wanted).map_err(io::Error::other)?;
                shape = wanted;
            }

            match asked_for(typing)? {
                Asked::Nothing => {}
                Asked::Quit => return Ok(()),
                Asked::Moved(step) => {
                    cursor.moved(&list::order(drafts.all(), &entries, &latest), step);
                }
                Asked::Composed => cursor = Cursor::on_draft(drafts.start()),
                Asked::Edited(edit) => {
                    if let Some(draft) = cursor.draft() {
                        drafts.edit(draft, edit);
                    }
                }
                Asked::Typed(bytes) => {
                    if let Some(pane) = &addressed {
                        client.send(pane, &bytes).map_err(io::Error::other)?;
                    }
                }
            }
        }
    })
    .map_err(|error| Error::new(format!("the app stopped: {error}")))
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
}

impl InTheSlot<'_> {
    /// How the keyboard is read while this is what the slot holds.
    fn typing(&self) -> Typing {
        match self {
            InTheSlot::Session(spawn) => Typing::IntoTheSpawn(spawn.modes()),
            InTheSlot::Composing(_) => Typing::IntoTheDraft,
        }
    }

    /// The pane a keystroke is addressed to, when there is one. A draft has
    /// none: it is not a process, and nothing it is typed into leaves the app.
    fn pane(&self) -> Option<&str> {
        match self {
            InTheSlot::Session(spawn) => Some(&spawn.pane),
            InTheSlot::Composing(_) => None,
        }
    }
}

/// What the slot holds with the list where it is.
fn in_the_slot<'a>(spawns: &'a Spawns, drafts: &'a Drafts, cursor: &Cursor) -> InTheSlot<'a> {
    match cursor.draft().and_then(|draft| drafts.of(draft)) {
        Some(draft) => InTheSlot::Composing(draft),
        None => InTheSlot::Session(spawns.showing(cursor)),
    }
}

/// Where the ordinary keys are going this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typing {
    /// To the spawn in the slot, in the modes its own screen asked for.
    IntoTheSpawn(Modes),
    /// Into the draft in the slot, which never leaves the app.
    IntoTheDraft,
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
/// slot, in one place and with nothing else in it: the four keys named here are
/// the app's wherever the selection is, and everything else belongs to what the
/// slot is holding — bytes for a session, an edit for a draft.
fn what_it_means(key: KeyEvent, typing: Typing) -> Asked {
    // Terminals that report a key going back up send the same key twice, and a
    // spawn would be typed into twice.
    if key.kind != KeyEventKind::Press {
        return Asked::Nothing;
    }

    match key.code {
        QUIT => Asked::Quit,
        COMPOSE => Asked::Composed,
        UP => Asked::Moved(Step::Up),
        DOWN => Asked::Moved(Step::Down),
        _ => match typing {
            Typing::IntoTheSpawn(modes) => Asked::Typed(keys::typed(key, modes)),
            Typing::IntoTheDraft => edited(key).map_or(Asked::Nothing, Asked::Edited),
        },
    }
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
pub fn render(frame: &mut Frame, listing: Listing, showing: &InTheSlot) {
    let (list, separator, slot) = regions(frame.area());

    frame.render_widget(listing, list);
    frame.render_widget(Block::new().borders(Borders::LEFT), separator);

    // Where the terminal's own cursor goes, asked of whatever drew the slot.
    // Without it the app would have a screen that looks like a session, or a
    // form, and no sign of where what you type is going.
    let caret = match showing {
        InTheSlot::Session(spawn) => {
            let screen = spawn.screen();
            frame.render_widget(&*screen, slot);

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
    };

    if let Some((column, row)) = caret
        && column < slot.width
        && row < slot.height
    {
        frame.set_cursor_position((slot.x + column, slot.y + row));
    }
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
                reason: reason.map(str::to_string),
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
        cursor: &Cursor,
        showing: &InTheSlot,
    ) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(terminal.columns, terminal.rows)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    Listing::new(drafts, entries, snapshot, cursor),
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
    fn the_list_says_what_the_app_created() {
        let screen = rendered(160, 12);

        assert!(screen.contains("spawn/add-retry-logic-a7f3"), "{screen}");
        assert!(
            screen.contains("/data/harness-launcher/worktrees"),
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
                    Listing::new(&[], &entries(), &Snapshot::default(), &Cursor::default()),
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

    /// The row the spawn is on, whatever else moved around it.
    fn row(screen: &str) -> String {
        screen
            .lines()
            .find(|line| line.contains("add-retry-logic-a7f3"))
            .unwrap_or_else(|| panic!("the spawn is not on screen:\n{screen}"))
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
    fn a_reason_takes_a_line_only_when_there_is_something_to_explain() {
        let explained = drawn(180, 14, &saying(Status::Unknown, Some("no record")), "");
        let plain = drawn(180, 14, &saying(Status::Working, None), "");

        assert_eq!(
            written(&explained),
            written(&plain) + 1,
            "an explained row and an unexplained one are the same height:\n{explained}"
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
    fn the_app_keeps_four_keys_and_the_spawn_gets_every_other() {
        assert!(matches!(pressed(QUIT), Asked::Quit));
        assert!(matches!(pressed(COMPOSE), Asked::Composed));
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
        .unwrap()
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
        painted(
            TERMINAL,
            drafts.all(),
            &spawns.entries(),
            &Snapshot::default(),
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
            visited.push(On::Spawn(spawns.showing(&cursor).entry.spawn.clone()));
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
            spawns.showing(&Cursor::on_spawn("fix-the-flake-b2c9")).pane,
            "%2"
        );
        assert_eq!(
            spawns
                .showing(&Cursor::on_spawn("drop-the-cache-d4e1"))
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
        assert_eq!(spawns.shape(), bigger);
    }

    #[test]
    fn the_app_will_not_run_with_nothing_to_put_in_the_slot() {
        assert!(Spawns::new(Vec::new()).is_err());
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

        assert!(first.contains("  the first draft"), "{first}");
        assert!(!first.contains("  the second draft"), "{first}");
        assert!(second.contains("  the second draft"), "{second}");
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
