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
//! user typed** — it has no keyboard of its own beyond the three keys that leave
//! and move the selection.
//!
//! Nothing here is a fixed size. Every dimension comes from the real terminal on
//! every frame, so a maximised window is a bigger layout rather than a bigger
//! frame around a small one — and the slot growing is what tells tmux to grow
//! the panes behind it.

use std::io;
use std::slice::from_ref;
use std::sync::MutexGuard;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::crossterm::terminal;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, Borders};

use crate::control::{Client, Grid, POISONED};
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
    /// lock with the next thing the spawn drew, so anything done under it is
    /// done to every spawn at once.
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
pub fn run(spawn: &Spawn, snapshots: &Receiver<Snapshot>, client: &Client) -> Result<()> {
    let mut latest = Snapshot::default();
    let mut showing = spawn.size();
    let entries = from_ref(&spawn.entry);
    let mut cursor = Cursor::on(&spawn.entry.spawn);

    ratatui::run(|terminal| -> io::Result<()> {
        loop {
            while let Ok(snapshot) = snapshots.try_recv() {
                latest = snapshot;
            }

            terminal.draw(|frame| render(frame, entries, &latest, &cursor, &spawn.screen()))?;

            // A client that has gone leaves every grid exactly as it was, which
            // on screen is a session sitting there thinking. Asked here rather
            // than only when something is typed, because the user has no reason
            // to type at a spawn that looks busy.
            client.listening().map_err(io::Error::other)?;

            // The slot's shape is read off the frame that was just drawn rather
            // than worked out again from the terminal, so what the child is told
            // it has and what the app draws cannot come to differ.
            let wanted = slot(Size::of(terminal.get_frame().area()));
            if wanted != showing && !wanted.is_empty() {
                // The grid first: the resize reaches the child as a redraw, and
                // a grid still the old shape would clip it.
                spawn.resize(wanted);
                client.resize(wanted).map_err(io::Error::other)?;
                showing = wanted;
            }

            match asked_for(spawn.modes())? {
                Asked::Nothing => {}
                Asked::Quit => return Ok(()),
                Asked::Moved(step) => cursor.moved(&list::order(entries, &latest), step),
                Asked::Typed(bytes) => {
                    client.send(&spawn.pane, &bytes).map_err(io::Error::other)?;
                }
            }
        }
    })
    .map_err(|error| Error::new(format!("the app stopped: {error}")))
}

/// What the user did with the keyboard.
enum Asked {
    /// Nothing, in the time a frame waits.
    Nothing,
    /// To leave — which kills nothing.
    Quit,
    /// To go up or down the list.
    Moved(Step),
    /// Something for the spawn, already in the bytes a terminal would send.
    Typed(Vec<u8>),
}

/// Wait a frame's worth of time for the keyboard.
fn asked_for(modes: Modes) -> io::Result<Asked> {
    if !event::poll(FRAME)? {
        return Ok(Asked::Nothing);
    }

    let Event::Key(key) = event::read()? else {
        return Ok(Asked::Nothing);
    };

    Ok(what_it_means(key, modes))
}

/// What one keystroke means.
///
/// The whole of the split between the app's keyboard and the spawn's, in one
/// place and with nothing else in it: everything not named here is on its way to
/// the session, unchanged.
fn what_it_means(key: KeyEvent, modes: Modes) -> Asked {
    // Terminals that report a key going back up send the same key twice, and a
    // spawn would be typed into twice.
    if key.kind != KeyEventKind::Press {
        return Asked::Nothing;
    }

    match key.code {
        QUIT => Asked::Quit,
        UP => Asked::Moved(Step::Up),
        DOWN => Asked::Moved(Step::Down),
        _ => Asked::Typed(keys::typed(key, modes)),
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
pub fn render(
    frame: &mut Frame,
    entries: &[Entry],
    snapshot: &Snapshot,
    cursor: &Cursor,
    screen: &Screen,
) {
    let (list, separator, slot) = regions(frame.area());

    frame.render_widget(Listing::new(entries, snapshot, cursor), list);
    frame.render_widget(Block::new().borders(Borders::LEFT), separator);
    frame.render_widget(screen, slot);

    // The cursor is the terminal's own, put where the spawn left it. Without
    // this the app would have a screen that looks like a session and no sign of
    // where what you type is going.
    if let Some((column, row)) = screen.cursor()
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    /// The one spawn the app has so far, and what the list says about it.
    fn entries() -> Vec<Entry> {
        vec![Entry {
            repository: "harness-launcher".to_string(),
            spawn: "add-retry-logic-a7f3".to_string(),
            branch: "spawn/add-retry-logic-a7f3".to_string(),
            worktree: "/data/harness-launcher/worktrees/add-retry-logic-a7f3".to_string(),
        }]
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
        let screen = drew(slot_size(terminal), slot);
        let entries = entries();
        let cursor = Cursor::on(&entries[0].spawn);

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &entries, snapshot, &cursor, &screen))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

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

    #[test]
    fn the_cursor_is_put_where_the_spawn_left_it() {
        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        let size = slot(Size {
            columns: 90,
            rows: 12,
        });
        let screen = drew(size, "\x1b[3;7H");

        let (_, _, slot) = regions(Rect::new(0, 0, 90, 12));
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &entries(),
                    &Snapshot::default(),
                    &Cursor::default(),
                    &screen,
                );
            })
            .unwrap();

        assert!(terminal.backend().cursor_visible());
        assert_eq!(
            terminal.backend().cursor_position(),
            (slot.x + 6, slot.y + 2).into()
        );
    }

    #[test]
    fn a_spawn_that_hid_its_cursor_does_not_get_one_drawn_anyway() {
        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        let size = slot(Size {
            columns: 90,
            rows: 12,
        });
        let screen = drew(size, "\x1b[?25l");

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &entries(),
                    &Snapshot::default(),
                    &Cursor::default(),
                    &screen,
                );
            })
            .unwrap();

        assert!(!terminal.backend().cursor_visible());
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

    /// What the app makes of one key.
    fn read_as(key: KeyEvent) -> Asked {
        what_it_means(
            key,
            Modes {
                application_cursor: false,
            },
        )
    }

    /// What it makes of one key being pressed and nothing else.
    fn pressed(code: KeyCode) -> Asked {
        read_as(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn the_app_keeps_three_keys_and_the_spawn_gets_every_other() {
        assert!(matches!(pressed(QUIT), Asked::Quit));
        assert!(matches!(pressed(UP), Asked::Moved(Step::Up)));
        assert!(matches!(pressed(DOWN), Asked::Moved(Step::Down)));
        assert!(matches!(pressed(KeyCode::Char('2')), Asked::Typed(bytes) if bytes == b"2"));
        assert!(matches!(pressed(KeyCode::Esc), Asked::Typed(bytes) if bytes == [0x1b]));
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
}
