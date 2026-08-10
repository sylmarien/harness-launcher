//! One spawn's screen, and the emulator that keeps it.
//!
//! The app is the terminal now. Bytes arrive from the control-mode client,
//! [`Screen::apply`] feeds them to an emulator, and the grid that comes out is
//! drawn into whatever part of the app's own screen the spawn is showing in.
//! **One grid per spawn, live whether or not it is the one on display** — which
//! is what makes changing which spawn you are looking at a re-render rather than
//! anything happening to a process.
//!
//! **No scrollback.** The grid is a screen, not a history: a spawn costs one
//! screenful of cells and nothing accumulates, which is what makes twenty of
//! them cost megabytes. It is coherent rather than lossy because the app starts
//! the harness on the alternate screen, where a transcript never scrolls off the
//! top in the first place.
//!
//! **Nothing is written back to the child from here.** A terminal answers
//! questions — where is the cursor, what are you — and the emulator behind this
//! grid answers none of them. It does not have to: tmux is the terminal the
//! child is actually connected to, and it replies to those queries itself before
//! ever passing the bytes on. This grid renders a copy of that conversation. An
//! answer of the app's own would not fill a gap; it would arrive at the child as
//! a second reply, which is to say as keystrokes nobody typed. The integration
//! test in [`crate::control`] is what pins that down.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// How big something on a terminal is, in cells.
///
/// Columns first, because that is the order a terminal is talked about in;
/// tmux's own arguments and the emulator's both want it the other way round,
/// and this type is where that is remembered so nowhere else has to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// How many cells across.
    pub columns: u16,
    /// How many cells down.
    pub rows: u16,
}

impl Size {
    /// The size of a region of the app's own screen.
    pub fn of(area: Rect) -> Self {
        Self {
            columns: area.width,
            rows: area.height,
        }
    }

    /// Whether there is any screen here at all.
    ///
    /// A terminal with no cells in it is not a small terminal: emulators and
    /// multiplexers alike take a zero as a refusal or a bad size, and the app
    /// can be given one by nothing worse than a window dragged very small.
    pub fn is_empty(self) -> bool {
        self.columns == 0 || self.rows == 0
    }
}

/// A spawn's screen, as far as the app has seen it.
pub struct Screen {
    emulator: vt100::Parser,
}

impl Screen {
    /// A blank screen of a given size.
    pub fn new(size: Size) -> Self {
        Self {
            emulator: vt100::Parser::new(size.rows, size.columns, NO_SCROLLBACK),
        }
    }

    /// Take in what the spawn drew.
    pub fn apply(&mut self, bytes: &[u8]) {
        self.emulator.process(bytes);
    }

    /// Become a different shape, when the app's own window becomes one.
    ///
    /// Told rather than asked: the child is resized by tmux, and this is the
    /// grid catching up with the size the child has already been given. A size
    /// with no cells in it is ignored rather than passed on.
    pub fn resize(&mut self, size: Size) {
        if size.is_empty() || size == self.size() {
            return;
        }

        self.emulator.screen_mut().set_size(size.rows, size.columns);
    }

    /// The shape the grid is currently in.
    pub fn size(&self) -> Size {
        let (rows, columns) = self.emulator.screen().size();

        Size { columns, rows }
    }

    /// Where the cursor is — column then row, relative to the grid's own top
    /// left — or nothing at all when the spawn has hidden it.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        let screen = self.emulator.screen();
        if screen.hide_cursor() {
            return None;
        }
        let (row, column) = screen.cursor_position();

        Some((column, row))
    }

    /// Whether the spawn asked for arrow keys in their application form.
    ///
    /// A terminal sends a different escape sequence for the same arrow key
    /// depending on a mode the program itself sets, so this is not a detail the
    /// keyboard can settle on its own — it has to be read off the screen the
    /// keystroke is going to.
    pub fn application_cursor(&self) -> bool {
        self.emulator.screen().application_cursor()
    }
}

/// What a spawn's grid holds of its own past: nothing.
const NO_SCROLLBACK: usize = 0;

impl Widget for &Screen {
    /// Draw the grid into a region of the app's screen.
    ///
    /// Cell by cell rather than through a widget that wraps the emulator. The
    /// grid is already the same shape as the region — it was made that way and
    /// resized with it — so this is a copy, and doing it here is what keeps the
    /// question about the emulator's fidelity rather than a wrapper's.
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let screen = self.emulator.screen();

        for row in 0..area.height {
            for column in 0..area.width {
                let Some(from) = screen.cell(row, column) else {
                    continue;
                };
                let Some(into) = buffer.cell_mut((area.x + column, area.y + row)) else {
                    continue;
                };

                // A wide character occupies the cell it is in and the one after
                // it. Giving the second cell a symbol of its own would push the
                // rest of the line along by one, so it is left empty and the
                // terminal's own idea of the first cell's width covers both.
                if from.is_wide_continuation() {
                    into.set_symbol("");
                    continue;
                }

                into.set_symbol(if from.has_contents() {
                    from.contents()
                } else {
                    " "
                });
                into.set_style(styled(from));
            }
        }
    }
}

/// How one cell reads.
fn styled(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    if let Some(colour) = coloured(cell.fgcolor()) {
        style = style.fg(colour);
    }
    if let Some(colour) = coloured(cell.bgcolor()) {
        style = style.bg(colour);
    }
    for (set, modifier) in [
        (cell.bold(), Modifier::BOLD),
        (cell.dim(), Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.underline(), Modifier::UNDERLINED),
        (cell.inverse(), Modifier::REVERSED),
    ] {
        if set {
            style = style.add_modifier(modifier);
        }
    }

    style
}

/// A colour the emulator read, in the terminal library's words.
///
/// The default is `None` rather than a colour of the app's choosing: a spawn
/// that has not said what colour it wants should inherit the user's terminal,
/// exactly as it would if they had started it themselves.
fn coloured(colour: vt100::Color) -> Option<Color> {
    match colour {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(Color::Indexed(index)),
        vt100::Color::Rgb(red, green, blue) => Some(Color::Rgb(red, green, blue)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::control::Output;

    /// A real control-mode recording of a real program drawing itself — see
    /// `captured/README.md`. Bytes, not text: what a spawn draws is not
    /// obliged to be valid UTF-8, and a reading of it that assumes otherwise
    /// would pass here and fail on the first half-written wide character.
    const CAPTURED: &[u8] = include_bytes!("../captured/tmux-control-mode.txt");

    /// The shape the recording was made at.
    const RECORDED: Size = Size {
        columns: 60,
        rows: 16,
    };

    /// The recording, played into a grid.
    fn recorded() -> Screen {
        let mut screen = Screen::new(RECORDED);
        for line in CAPTURED.split(|byte| *byte == b'\n') {
            if let Some(output) = Output::parse(line) {
                screen.apply(&output.bytes);
            }
        }

        screen
    }

    /// What the grid drew, one entry per cell of the app's own screen.
    ///
    /// The cells rather than what a terminal would make of them: whether a
    /// character lands in the right *cell* is the whole question here, and a row
    /// read back as one string has already thrown that away.
    fn cells(screen: &Screen) -> Vec<Vec<String>> {
        let size = screen.size();
        let area = Rect::new(0, 0, size.columns, size.rows);
        let mut buffer = Buffer::empty(area);
        screen.render(area, &mut buffer);

        (0..size.rows)
            .map(|row| {
                (0..size.columns)
                    .map(|column| buffer[(column, row)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// What the grid says, one string per row.
    ///
    /// Shared with the control-mode client's tests, which have the same
    /// question to ask of a grid and no business answering it a second way.
    pub fn shown(screen: &Screen) -> Vec<String> {
        cells(screen).iter().map(|row| row.concat()).collect()
    }

    /// Which cell something starts in.
    fn column_of(row: &[String], symbol: &str) -> usize {
        row.iter()
            .position(|cell| cell == symbol)
            .unwrap_or_else(|| panic!("{symbol} is not on the row: {row:?}"))
    }

    #[test]
    fn what_the_spawn_drew_is_what_the_grid_holds() {
        let shown = shown(&recorded());

        assert!(
            shown[0].contains("┌─ spawn ─ fix-worktree-cleanup ─┐"),
            "the box drawing did not survive: {shown:?}"
        );
        assert!(
            shown[1].contains("世界"),
            "the wide characters did not survive: {shown:?}"
        );
    }

    #[test]
    fn a_wide_character_takes_two_cells_and_moves_nothing_along() {
        let cells = cells(&recorded());

        // The cell after a wide character belongs to it, and holds nothing of
        // its own: anything written there would be drawn a column further along
        // than the terminal is going to put it.
        let wide = column_of(&cells[1], "世");
        assert_eq!(cells[1][wide + 1], "", "{:?}", cells[1]);

        // And the proof that it added up: the box the wide characters are inside
        // closes in the same column as the one above it.
        assert_eq!(
            cells[1].iter().rposition(|cell| cell == "│"),
            cells[0].iter().rposition(|cell| cell == "┐"),
            "the wide characters pushed the row out of line: {cells:?}"
        );
    }

    #[test]
    fn the_grid_is_the_shape_the_spawn_was_given() {
        let cells = cells(&recorded());

        assert_eq!(cells.len(), RECORDED.rows as usize);
        assert!(
            cells
                .iter()
                .all(|row| row.len() == RECORDED.columns as usize)
        );
    }

    #[test]
    fn a_spawn_that_says_nothing_leaves_a_blank_screen_rather_than_a_hole() {
        let screen = Screen::new(Size {
            columns: 8,
            rows: 2,
        });

        assert_eq!(shown(&screen), vec!["        ".to_string(); 2]);
    }

    #[test]
    fn colour_and_emphasis_reach_the_cell_they_were_asked_for() {
        let mut screen = Screen::new(Size {
            columns: 4,
            rows: 1,
        });
        screen.apply(b"\x1b[1;31mred\x1b[0m");

        let area = Rect::new(0, 0, 4, 1);
        let mut buffer = Buffer::empty(area);
        (&screen).render(area, &mut buffer);

        assert_eq!(buffer[(0, 0)].fg, Color::Indexed(1));
        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(
            buffer[(3, 0)].fg,
            Color::Reset,
            "a cell the spawn said nothing about took a colour anyway"
        );
    }

    #[test]
    fn the_cursor_is_where_the_spawn_left_it() {
        let mut screen = Screen::new(Size {
            columns: 20,
            rows: 4,
        });
        screen.apply(b"\x1b[3;7H");

        assert_eq!(screen.cursor(), Some((6, 2)));
    }

    #[test]
    fn a_hidden_cursor_is_not_drawn_somewhere_arbitrary_instead() {
        let mut screen = Screen::new(Size {
            columns: 20,
            rows: 4,
        });
        screen.apply(b"\x1b[?25l");

        assert_eq!(screen.cursor(), None);
    }

    #[test]
    fn a_spawn_can_ask_for_arrow_keys_in_their_application_form() {
        let mut screen = Screen::new(Size {
            columns: 20,
            rows: 4,
        });
        assert!(!screen.application_cursor());

        screen.apply(b"\x1b[?1h");

        assert!(screen.application_cursor());
    }

    #[test]
    fn a_resized_grid_is_the_new_shape() {
        let mut screen = Screen::new(RECORDED);

        screen.resize(Size {
            columns: 100,
            rows: 30,
        });

        assert_eq!(
            screen.size(),
            Size {
                columns: 100,
                rows: 30
            }
        );
    }

    #[test]
    fn a_window_dragged_shut_does_not_leave_a_grid_with_no_cells() {
        let mut screen = Screen::new(RECORDED);

        screen.resize(Size {
            columns: 0,
            rows: 0,
        });

        assert_eq!(screen.size(), RECORDED);
    }
}
