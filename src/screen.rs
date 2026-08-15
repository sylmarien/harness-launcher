//! One spawn's screen, and the emulator that keeps it.
//!
//! One grid per spawn, live whether or not it is on display, so switching
//! spawns is a re-render. No scrollback: the harness runs on the alternate
//! screen, so one screenful of cells per spawn loses nothing. There is no
//! write-back path on purpose — tmux is the child's real terminal and answers
//! its queries itself; a reply of the app's own would arrive as keystrokes
//! nobody typed, which the integration test in [`crate::control`] pins down.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// A size in cells, columns first; tmux and the emulator both want it the
/// other way round, and this type is where that is remembered.
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

    /// Whether there is any screen here at all; emulators and multiplexers
    /// treat a zero dimension as an error.
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

    /// Catch up with the size tmux has already given the child. An empty size
    /// is ignored.
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

    /// Whether the spawn asked for arrow keys in their application form; the
    /// program sets this mid-run, so it is read off the screen.
    pub fn application_cursor(&self) -> bool {
        self.emulator.screen().application_cursor()
    }
}

/// What a spawn's grid holds of its own past: nothing.
const NO_SCROLLBACK: usize = 0;

impl Widget for &Screen {
    /// Draw the grid into a region of the app's screen, cell by cell; the grid
    /// is already the region's shape.
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

                // A wide character owns the next cell too; giving that cell a
                // symbol of its own would push the rest of the line along.
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

/// A colour the emulator read, in the terminal library's words. Default maps
/// to `None` so an unstyled spawn inherits the user's terminal.
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

    /// A real control-mode recording — see `captured/README.md`. Bytes, not
    /// text: what a spawn draws need not be valid UTF-8.
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

    /// What the grid drew, one entry per cell — which cell a character lands
    /// in is the whole question here.
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

    /// What the grid says, one string per row; shared with the control-mode
    /// client's tests.
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

        let wide = column_of(&cells[1], "世");
        assert_eq!(cells[1][wide + 1], "", "{:?}", cells[1]);

        // The box closes in the same column as the row above, so the widths
        // added up.
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
