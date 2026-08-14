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

/// The colour the app says *the keyboard is on this one* in. A colour of its
/// own, because it says something about the app rather than about a spawn.
///
/// Written once and read two ways — as the mark in a form's gutter, and as the
/// band under a row of the list — so the two can never come to disagree about
/// what being selected looks like.
const SELECTION_COLOUR: Color = Color::Cyan;

/// How the mark in the gutter reads where the row itself is not painted.
pub const SELECTION: Style = Style::new().fg(SELECTION_COLOUR);

/// How a row the keyboard is on reads: black on a band, which the row is then
/// padded out to the full width of to make it a band rather than a highlight.
///
/// **The one place the app names both colours**, and it is the one place it can:
/// everywhere else it puts a colour on the user's own background and has to
/// leave that background alone, so black would be a guess about a theme it
/// cannot see. Here the background is the app's own, so what reads on top of it
/// is arithmetic rather than a guess.
///
/// `alarmed` is the row being one the app is admitting something about, and
/// [`AMBER`] is the whole of the rule: it is the colour reserved for the app
/// admitting things and used for nothing else, so **whatever the caller has
/// already drawn in amber is what is being admitted**. The band takes amber
/// rather than the selection's colour, so the admission survives the row being
/// selected. Listing the states that qualify would be a second answer to a
/// settled question, and the two would come apart the first time a state was
/// added to one of them.
pub fn band(alarmed: bool) -> Style {
    Style::new()
        .bg(if alarmed { AMBER } else { SELECTION_COLOUR })
        .fg(Color::Black)
}

/// The colour reserved for the app admitting something: that it cannot tell
/// what a spawn is doing, or that it could not do what it was asked.
///
/// One colour for both, because they are one thing from where the user sits —
/// the app saying *this one is on you*. Two would be two things to learn.
pub const AMBER: Color = Color::Yellow;

/// The gutter at the front of a row or a heading: the mark when the keyboard is
/// on it, and the space it would have taken when it is not.
///
/// A space rather than nothing, so what is beside it does not shift sideways as
/// the selection arrives and leaves.
///
/// **How it reads is the row's rather than the gutter's**, because a row can be
/// painted: a mark keeping its own cyan on a cyan [`band`] would be the one cell
/// of the row that disappeared. A column that draws no band passes [`SELECTION`]
/// and gets what this always did.
pub fn gutter(on_it: bool, how_it_reads: Style) -> Span<'static> {
    Span::styled(if on_it { SELECTED } else { " " }, how_it_reads)
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
/// column longer than the screen, is the open question
/// `docs/developers/components/the-screen.md` records and the scale pass owns.
/// What is settled here is narrower and was asked for:
/// a selection that moves is a selection you can see.
///
/// `selected` is the first and last line the selected thing occupies, which is
/// more than one line for a form control with a paragraph in it. A row of the
/// list is one line and gives the same number twice; the pair is still what this
/// takes, because the two columns share it. Where even the whole of a selection
/// will not fit, its first line wins.
pub fn scroll_offset(selected: Option<(usize, usize)>, height: usize) -> u16 {
    let Some((first, last)) = selected else {
        return 0;
    };

    let past = last.saturating_sub(height.saturating_sub(1));
    let offset = past.min(first);

    u16::try_from(offset).unwrap_or(u16::MAX)
}

/// `text`, broken across lines of at most `cells`, on the spaces in it.
///
/// For the things the app writes that are prose rather than names: the sentence
/// saying why a spawn is `unknown`, the one a retirement carries, and the one
/// saying why a draft could not be started. None of them is a sentence the app
/// wrote itself — they carry git's words, or the harness's — and a sentence cut
/// at twenty-seven columns says nothing. A word too long for a line of its own
/// is cut, which is the only way it ends.
///
/// A column with no room at all gets no lines rather than a blank one per word:
/// they would show as nothing and still push everything under them down.
pub fn wrapped(text: &str, cells: usize) -> Vec<String> {
    laid_out(text, cells, |word, cells| vec![elided(word, cells)])
}

/// `text`, broken across lines of at most `cells` — on its spaces where it can
/// be, and through a word that does not fit where it cannot.
///
/// For the one thing the app writes that has to survive being written down: the
/// record of what a creation was about to do, which names a worktree and a
/// branch. Everything else the app cuts, it cuts because the rest of it is not
/// worth a second row — but **a path cut with an ellipsis is a path you cannot
/// go and look at**, and going and looking is the whole reason it is written
/// down before it is made.
pub fn broken(text: &str, cells: usize) -> Vec<String> {
    laid_out(text, cells, pieces)
}

/// Words onto lines of at most `cells`, with `too_long` saying what becomes of
/// one that will not fit on a line of its own.
///
/// That is the whole of the difference between the two above, and the reason
/// they are one function underneath: laying words onto lines is the same job
/// either way, and written twice it would be two things that came to disagree
/// about a space.
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

/// One word, as the pieces it goes onto lines as: itself when it fits, and
/// whatever the caller does with it when it does not.
fn whole(word: &str, cells: usize, too_long: &impl Fn(&str, usize) -> Vec<String>) -> Vec<String> {
    if word.chars().count() <= cells {
        return vec![word.to_string()];
    }

    too_long(word, cells)
}

/// `word`, in pieces of at most `cells` characters — which is what a word too
/// long for a line of its own becomes when none of it may be lost.
fn pieces(word: &str, cells: usize) -> Vec<String> {
    let characters: Vec<char> = word.chars().collect();

    characters
        .chunks(cells)
        .map(|piece| piece.iter().collect())
        .collect()
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

    /// The two things a band has to be: a background the row is painted with,
    /// and the admission surviving the row being selected.
    #[test]
    fn a_band_is_a_background_and_it_is_amber_when_the_app_is_admitting_something() {
        assert_eq!(band(false).bg, Some(SELECTION_COLOUR));
        assert_eq!(band(true).bg, Some(AMBER));
        assert_eq!(band(true).fg, band(false).fg);
    }
}
