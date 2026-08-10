//! The scaffolding the app draws its own screen with.
//!
//! Two kinds of thing, and they belong together because the same two callers
//! need both. **How it reads** — the marks, the gutter, the ways of emphasising.
//! **What a column does** — cut text to the width it turned out to have, keep
//! what you are on in view, and carry a footer that is all its rows or none.
//!
//! The list and the draft form are each one of these columns, which is why this
//! is shared rather than a helper inside either. Written twice they would drift,
//! and they had already begun to: a selection mark decided in two places is two
//! things that come to disagree, and the two footers had reached the point where
//! one cut its lines and the other let them run off the edge.
//!
//! Nothing a spawn drew passes through here. This is only ever the app writing
//! about itself.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The mark in the gutter of whatever the keyboard is on.
pub const SELECTED: &str = "▍";

/// The mark that says text was cut, and it costs a cell of what it cuts.
pub const ELLIPSIS: char = '…';

/// How a heading reads: a repository's name, the word above the list, the name
/// of a control in the form.
pub const HEADING: Style = Style::new().add_modifier(Modifier::BOLD);

/// How everything the eye should slide over reads — the detail under the
/// selected row, and the lines saying what the keyboard does.
pub const DIM: Style = Style::new().add_modifier(Modifier::DIM);

/// How the mark in the gutter reads. A colour of its own, because it says
/// something about the app rather than about a spawn.
pub const SELECTION: Style = Style::new().fg(Color::Cyan);

/// The gutter at the front of a row or a heading: the mark when the keyboard is
/// on it, and the space it would have taken when it is not.
///
/// A space rather than nothing, so what is beside it does not shift sideways as
/// the selection arrives and leaves.
pub fn gutter(on_it: bool) -> Span<'static> {
    Span::styled(if on_it { SELECTED } else { " " }, SELECTION)
}

/// `text`, cut to fit, ending in the mark that says it was cut.
///
/// Counted in characters, which is what the app writes about itself: a spawn's
/// name is a slug the app made itself, and a branch and a worktree path are
/// built from it. Text with wide characters in it will lose a cell of its cut,
/// and nothing else.
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

/// How far down a column has to be scrolled for `selected` to be on it.
///
/// **Only far enough, and only when there is no other way.** The column sits at
/// its top until the selection would fall off the bottom, and then follows it
/// one line at a time — so a heading stays where it was for as long as it
/// possibly can, and nothing moves under the eye that did not have to.
///
/// This is not scrolling. Reaching something the selection is not on, on a
/// column longer than the screen, is the open question the design records (§5.3)
/// and the scale pass owns. What is settled here is narrower and was asked for:
/// a selection that moves is a selection you can see.
///
/// `selected` is the first and last line the selected thing occupies, which is
/// more than one line for a spawn showing its detail and for a form control with
/// a paragraph in it. Where even the whole of it will not fit, its first line
/// wins.
pub fn scroll_offset(selected: Option<(usize, usize)>, height: usize) -> u16 {
    let Some((first, last)) = selected else {
        return 0;
    };

    let past = last.saturating_sub(height.saturating_sub(1));
    let offset = past.min(first);

    u16::try_from(offset).unwrap_or(u16::MAX)
}

/// What the foot of a column says the keyboard does.
///
/// There are two — under the list, and under a draft's form — and they behave
/// the same way: dim, anchored to the bottom so they stay where the eye last
/// found them, cut rather than wrapped, and **given back to the content
/// entirely when there is not room for both**.
pub struct Footer {
    /// What it says, one line per row.
    says: &'static [&'static str],
    /// The shortest column that can still spare the rows. It differs between
    /// the two: how much content has to survive before saying what the keyboard
    /// does is worth a row is a judgement about that column, not about footers.
    room: u16,
}

impl Footer {
    /// A footer saying this, on any column at least `room` rows tall.
    pub const fn new(says: &'static [&'static str], room: u16) -> Self {
        Self { says, room }
    }

    /// How many rows it takes on a column this tall: all of them, or none.
    ///
    /// Never some. A footer cut off half way through says less than no footer
    /// at all, and the rows it would have taken are the content's.
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
        // Four lines of selection in a column three high: the first line of it
        // is what has to be on screen.
        assert_eq!(scroll_offset(Some((5, 8)), 3), 5);
    }

    #[test]
    fn the_gutter_is_the_same_width_whether_the_keyboard_is_there_or_not() {
        assert_eq!(
            gutter(true).content.chars().count(),
            gutter(false).content.chars().count()
        );
        assert_eq!(gutter(true).content, SELECTED);
    }
}
