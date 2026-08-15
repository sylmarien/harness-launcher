//! The scaffolding the app draws its own screen with: marks, styles, text
//! layout, and column behaviour (eliding, scrolling, footers).
//!
//! Shared between the list and the draft form so the two cannot drift.
//! Nothing a spawn drew passes through here — this is only the app writing
//! about itself.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The mark in the gutter of the row the keyboard is on.
pub const SELECTED: &str = "▍";

/// The mark appended to text that was cut, spending one cell of the cut.
pub const ELLIPSIS: char = '…';

/// The style for headings: repository names, section titles, form controls.
pub const HEADING: Style = Style::new().add_modifier(Modifier::BOLD);

/// The style for de-emphasised text: details and keyboard hints.
pub const DIM: Style = Style::new().add_modifier(Modifier::DIM);

/// The selection colour, defined once so the gutter mark and the band can
/// never disagree about what being selected looks like.
const SELECTION_COLOUR: Color = Color::Cyan;

/// How the mark in the gutter reads where the row itself is not painted.
pub const SELECTION: Style = Style::new().fg(SELECTION_COLOUR);

/// The style of a selected row: black on a full-width band.
///
/// Naming black is safe only here, because the app paints the background
/// itself; everywhere else the user's theme is unknown and left alone.
/// An `alarmed` row's band is [`AMBER`] so the warning survives selection.
pub fn band(alarmed: bool) -> Style {
    Style::new()
        .bg(if alarmed { AMBER } else { SELECTION_COLOUR })
        .fg(Color::Black)
}

/// The colour reserved for the app reporting a problem of its own: a spawn it
/// cannot account for, or something it was asked to do and could not.
pub const AMBER: Color = Color::Yellow;

/// The gutter cell: the selection mark, or a space so text beside it does not
/// shift as the selection moves.
///
/// The style is the caller's: a cyan mark on a cyan [`band`] would vanish, so
/// a painted row passes its own style; columns with no band pass [`SELECTION`].
pub fn gutter(on_it: bool, how_it_reads: Style) -> Span<'static> {
    Span::styled(if on_it { SELECTED } else { " " }, how_it_reads)
}

/// `text`, cut to `cells`, ending in [`ELLIPSIS`] when it was cut.
///
/// Counted in characters, not display cells: text with wide characters may
/// overrun its cut by a cell.
pub fn elided(text: &str, cells: usize) -> String {
    if text.chars().count() <= cells {
        return text.to_string();
    }
    let Some(kept) = cells.checked_sub(1) else {
        return String::new();
    };

    text.chars()
        .take(kept)
        .chain(std::iter::once(ELLIPSIS))
        .collect()
}

/// How far down a column has to be scrolled for `selected` to be on it: only
/// far enough, following the selection one line at a time.
///
/// This is not general scrolling — reaching content the selection is not on,
/// on a column longer than the screen, is the open question recorded in
/// `docs/developers/components/the-screen.md`.
///
/// `selected` is the first and last line the selected thing occupies (the
/// same line twice for a one-line row). When even the whole selection will
/// not fit, its first line wins.
pub fn scroll_offset(selected: Option<(usize, usize)>, height: usize) -> u16 {
    let Some((first, last)) = selected else {
        return 0;
    };

    let past = last.saturating_sub(height.saturating_sub(1));
    let offset = past.min(first);

    u16::try_from(offset).unwrap_or(u16::MAX)
}

/// `text`, wrapped on its spaces into lines of at most `cells` — for prose.
/// A word too long for a line of its own is cut. Zero width yields no lines
/// rather than a blank line per word.
pub fn wrapped(text: &str, cells: usize) -> Vec<String> {
    laid_out(text, cells, |word, cells| vec![elided(word, cells)])
}

/// Like [`wrapped`], but a word too long for a line is split across lines
/// rather than cut — for paths, which must survive whole to be followed.
pub fn broken(text: &str, cells: usize) -> Vec<String> {
    laid_out(text, cells, pieces)
}

/// Words onto lines of at most `cells`; `too_long` decides what becomes of a
/// word that will not fit on a line of its own.
fn laid_out(
    text: &str,
    cells: usize,
    too_long: impl Fn(&str, usize) -> Vec<String>,
) -> Vec<String> {
    if cells == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();

    for word in text.split_whitespace() {
        for (at, piece) in whole(word, cells, &too_long).into_iter().enumerate() {
            match lines.last_mut() {
                Some(line)
                    if at == 0 && line.chars().count() + 1 + piece.chars().count() <= cells =>
                {
                    line.push(' ');
                    line.push_str(&piece);
                }
                _ => lines.push(piece),
            }
        }
    }

    lines
}

/// One word as its line pieces: itself when it fits, `too_long`'s result
/// otherwise.
fn whole(word: &str, cells: usize, too_long: &impl Fn(&str, usize) -> Vec<String>) -> Vec<String> {
    if word.chars().count() <= cells {
        return vec![word.to_string()];
    }

    too_long(word, cells)
}

/// `word`, split into pieces of at most `cells` characters.
fn pieces(word: &str, cells: usize) -> Vec<String> {
    let characters: Vec<char> = word.chars().collect();

    characters
        .chunks(cells)
        .map(|piece| piece.iter().collect())
        .collect()
}

/// A column's keyboard hints: dim, anchored to the bottom, cut rather than
/// wrapped, and shown all-or-nothing.
pub struct Footer {
    /// What it says, one line per row.
    says: &'static [&'static str],
    /// The shortest column that can still spare the rows.
    room: u16,
}

impl Footer {
    /// A footer saying this, on any column at least `room` rows tall.
    pub const fn new(says: &'static [&'static str], room: u16) -> Self {
        Self { says, room }
    }

    /// How many rows it takes on a column this tall: all of them, or none.
    pub fn rows(&self, height: u16) -> u16 {
        if height < self.room {
            return 0;
        }

        u16::try_from(self.says.len()).unwrap_or(u16::MAX)
    }

    /// Its lines, cut to whatever width the column turned out to have.
    pub fn lines(&self, width: usize) -> Vec<Line<'static>> {
        self.says
            .iter()
            .map(|line| Line::styled(elided(line, width), DIM))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A footer of two lines, on a column that has to be six rows to take them.
    const FOOTER: Footer = Footer::new(&["first line", "second line"], 6);

    #[test]
    fn a_footer_takes_one_row_per_line_of_it() {
        assert_eq!(FOOTER.rows(20), 2);
        assert_eq!(FOOTER.lines(20).len(), 2);
    }

    #[test]
    fn a_column_with_no_room_to_spare_gets_its_rows_back_rather_than_half_a_footer() {
        assert_eq!(FOOTER.rows(6), 2);
        assert_eq!(FOOTER.rows(5), 0);
    }

    #[test]
    fn a_footer_too_wide_for_its_column_is_cut_rather_than_clipped() {
        let lines = FOOTER.lines(6);

        assert_eq!(lines[0].to_string(), "first…");
    }

    #[test]
    fn text_that_fits_is_left_exactly_as_it_is() {
        assert_eq!(elided("first line", 10), "first line");
        assert_eq!(elided("first line", 20), "first line");
    }

    #[test]
    fn a_column_with_no_width_at_all_is_given_no_text_rather_than_a_bare_mark() {
        assert_eq!(elided("first line", 0), "");
    }

    #[test]
    fn a_selection_that_fits_scrolls_nothing() {
        assert_eq!(scroll_offset(Some((0, 2)), 10), 0);
        assert_eq!(scroll_offset(Some((7, 9)), 10), 0);
        assert_eq!(scroll_offset(None, 10), 0);
    }

    #[test]
    fn a_selection_past_the_bottom_is_followed_one_line_at_a_time() {
        assert_eq!(scroll_offset(Some((10, 10)), 10), 1);
        assert_eq!(scroll_offset(Some((11, 11)), 10), 2);
    }

    #[test]
    fn what_is_selected_wins_over_the_detail_hanging_off_it() {
        assert_eq!(scroll_offset(Some((5, 8)), 3), 5);
    }

    #[test]
    fn prose_is_broken_on_its_spaces_rather_than_wherever_it_runs_out() {
        assert_eq!(
            wrapped("its session record carries no status", 14),
            ["its session", "record carries", "no status"]
        );
    }

    #[test]
    fn a_word_too_long_for_a_line_of_its_own_is_cut() {
        assert_eq!(wrapped("/data/harness-launcher/worktrees", 8), ["/data/h…"]);
    }

    #[test]
    fn prose_with_no_room_at_all_takes_no_rows_rather_than_blank_ones() {
        assert!(wrapped("nothing fits here", 0).is_empty());
        assert!(broken("nothing fits here", 0).is_empty());
    }

    #[test]
    fn a_record_breaks_through_a_path_rather_than_losing_the_end_of_it() {
        let record = broken("creating /data/harness-launcher/worktrees/a7f3", 20);

        assert_eq!(
            record,
            ["creating", "/data/harness-launch", "er/worktrees/a7f3"]
        );
        assert!(
            record.concat().contains("worktrees/a7f3"),
            "the path did not survive being written down: {record:?}"
        );
    }

    #[test]
    fn a_record_that_fits_reads_as_the_sentence_it_is() {
        assert_eq!(
            broken("creating the worktree /w/a7f3 on spawn/a7f3", 30),
            ["creating the worktree /w/a7f3", "on spawn/a7f3"]
        );
    }

    #[test]
    fn the_gutter_is_the_same_width_whether_the_keyboard_is_there_or_not() {
        assert_eq!(
            gutter(true, SELECTION).content.chars().count(),
            gutter(false, SELECTION).content.chars().count()
        );
        assert_eq!(gutter(true, SELECTION).content, SELECTED);
    }

    #[test]
    fn a_band_is_a_background_and_it_is_amber_when_the_app_is_admitting_something() {
        assert_eq!(band(false).bg, Some(SELECTION_COLOUR));
        assert_eq!(band(true).bg, Some(AMBER));
        assert_eq!(band(true).fg, band(false).fg);
    }
}
