//! The list: every spawn there is, every draft being written, and which one you
//! are on.
//!
//! **Drafts sit above every repository**, pinned there rather than grouped:
//! a draft has not chosen a repository yet — that is one of the things being
//! typed — and a half-written spawn is something you came back for. The list is
//! the only place it exists, so it is the only thing that can remind you.
//!
//! Spawns sit **under the repository they were started against**, and each
//! repository's header carries a compact bar of its spawns' statuses — so a
//! project's state reads without reading its rows. Within a repository the
//! order is attention-first: stopped, then unknown, then working. The thing you
//! have to do next is always near the top of its group.
//!
//! **Status is carried by an icon and a colour together.** At twenty entries the
//! list has to read without a legend and survive a colour-blind reader, so a
//! shape and a colour decided in two places are two things that can come to
//! disagree. They are decided here, once, in [`shown_as`].
//!
//! **Nothing is a fixed size.** Every width in here is the width the list turned
//! out to have this frame. Text that will not fit is cut with a mark that says
//! it was cut, rather than wrapped into a second row that would break the one
//! line per spawn the density depends on. The one exception is prose — the
//! sentence saying why a spawn is `unknown` — which wraps, because a sentence
//! cut at twenty-seven columns says nothing.
//!
//! **The selection is held by name, never by position.** Rows re-order as
//! statuses change, and a cursor that remembered a row number would find itself
//! on a different spawn because some other spawn stopped. The list follows it
//! far enough to keep it on screen and no further — scrolling proper, to a spawn
//! the selection is not on, is still an open question in the design.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::draft::{self, Draft};
use crate::scaffolding::{DIM, ELLIPSIS, HEADING, gutter};
use crate::scaffolding::{Footer, elided, scroll_offset};
use crate::snapshot::{Row, Snapshot, Status};

/// The colour reserved for the app failing to know something.
const AMBER: Color = Color::Yellow;

/// The mark against a draft.
///
/// Not one of the status marks, and deliberately unlike them: a draft has no
/// agent, so nothing about it is working, stopped or unaccounted for. It reads
/// as the one thing it is — something being made.
const DRAFT: &str = "+";

/// How far a row's text sits from the left: the selection's gutter, the status
/// mark, and the space between that and the name.
const INDENT: usize = 3;

/// What the foot of the list says the keyboard does.
///
/// Three keys and a promise. The promise is the one the whole design rests on
/// and the one nobody would guess: leaving does not stop anything. Starting a
/// draft is on the list rather than anywhere else because a draft that does not
/// exist yet has nowhere else to be announced.
/// The shortest list that can still spare three of its rows for the footer is
/// seven: four rows of spawns is the least that reads as a list at all.
const FOOTER: Footer = Footer::new(
    &[
        "F2 starts a draft",
        "F6 / F7 move the selection",
        "F10 quits — nothing is killed",
    ],
    7,
);

/// What the list has to say about a spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The repository the spawn was started against.
    pub repository: String,
    /// The spawn's name.
    pub spawn: String,
    /// The branch it works on.
    pub branch: String,
    /// The worktree it works in.
    pub worktree: String,
}

/// One row of the list, as the thing it stands for.
///
/// Two kinds of row and two kinds of identity, rather than one namespace with a
/// rule about not colliding: a draft has nothing on disk to name it, and a name
/// taken from what has been typed would slip out from under the selection as it
/// was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum On {
    /// A draft, held by the identity it was given when it was started.
    Draft(draft::Id),
    /// A spawn, held by name.
    Spawn(String),
}

/// Which row the list is on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    /// The row the list is on, if it is on one.
    on: Option<On>,
}

/// Which way the selection was asked to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Towards the top of the list.
    Up,
    /// Towards the bottom.
    Down,
}

impl Cursor {
    /// A cursor on a named spawn.
    pub fn on_spawn(spawn: &str) -> Self {
        Self {
            on: Some(On::Spawn(spawn.to_string())),
        }
    }

    /// A cursor on a draft.
    pub fn on_draft(draft: draft::Id) -> Self {
        Self {
            on: Some(On::Draft(draft)),
        }
    }

    /// The spawn the list is on, if it is on one.
    pub fn spawn(&self) -> Option<&str> {
        match &self.on {
            Some(On::Spawn(spawn)) => Some(spawn),
            _ => None,
        }
    }

    /// The draft the list is on, if it is on one.
    pub fn draft(&self) -> Option<draft::Id> {
        match &self.on {
            Some(On::Draft(draft)) => Some(*draft),
            _ => None,
        }
    }

    /// Move, in the order the list is drawn in.
    ///
    /// Both ends stop rather than wrap: a list with a top and a bottom is one
    /// you can hold a place in, and wrapping past the last row would move the
    /// selection the length of the screen for a keystroke that asked for one
    /// row.
    ///
    /// A cursor on nothing — or on a row no longer in the list — lands on the
    /// first row, whichever way it was asked to go. There is no position to
    /// carry on from, and the first row is where a draft, or the attention-first
    /// order, puts whatever most needs you.
    pub fn moved(&mut self, order: &[On], step: Step) {
        let Some(last) = order.len().checked_sub(1) else {
            self.on = None;
            return;
        };

        let at = self
            .on
            .as_ref()
            .and_then(|on| order.iter().position(|row| row == on));
        let landing = match (at, step) {
            (None, _) => 0,
            (Some(at), Step::Up) => at.saturating_sub(1),
            (Some(at), Step::Down) => (at + 1).min(last),
        };

        self.on = Some(order[landing].clone());
    }
}

/// The list, ready to be drawn into whatever region it has this frame.
pub struct Listing<'a> {
    /// The drafts being written, in the order they were started.
    drafts: &'a [Draft],
    /// The spawns to show, in the order they were started.
    entries: &'a [Entry],
    /// What the supervisor last said they were doing.
    snapshot: &'a Snapshot,
    /// Which row the list is on.
    cursor: &'a Cursor,
}

impl<'a> Listing<'a> {
    /// The list of these drafts and spawns, as this snapshot found them.
    pub fn new(
        drafts: &'a [Draft],
        entries: &'a [Entry],
        snapshot: &'a Snapshot,
        cursor: &'a Cursor,
    ) -> Self {
        Self {
            drafts,
            entries,
            snapshot,
            cursor,
        }
    }

    /// Everything above the footer, and where the selected row is in it.
    fn rows(&self, width: usize) -> Rows {
        let mut lines = vec![Line::styled("SPAWNS", HEADING), Line::raw("")];
        let mut selected = None;

        for draft in self.drafts {
            let on_it = self.cursor.draft() == Some(draft.id());
            if on_it {
                selected = Some((lines.len(), lines.len()));
            }
            lines.push(drafted(draft, on_it, width));
        }

        for (at, group) in grouped(self.entries, self.snapshot).iter().enumerate() {
            if at > 0 || !self.drafts.is_empty() {
                lines.push(Line::raw(""));
            }
            lines.push(group.header(width));

            for placed in &group.spawns {
                let on_it = self.cursor.spawn() == Some(placed.name());
                let from = lines.len();
                lines.push(placed.row(on_it, width));
                if on_it {
                    lines.extend(placed.detail(width));
                    selected = Some((from, lines.len() - 1));
                }
            }
        }

        Rows { lines, selected }
    }
}

/// The list's lines, and which of them the selected row takes up.
struct Rows {
    /// Every line there is, whether or not the region can hold them all.
    lines: Vec<Line<'static>>,
    /// The first and last line the selected row occupies, if one is selected.
    selected: Option<(usize, usize)>,
}

/// A draft's row: the gutter, its mark, and what it is called so far.
///
/// One line and nothing under it, selected or not. There is nothing to say
/// beneath it — a draft has made no branch and no worktree — and what it does
/// have is in the slot beside it the moment it is selected.
fn drafted(draft: &Draft, on_it: bool, width: usize) -> Line<'static> {
    Line::from(vec![
        gutter(on_it),
        Span::styled(DRAFT, HEADING),
        Span::raw(" "),
        Span::styled(
            elided(&draft.title(), width.saturating_sub(INDENT)),
            HEADING,
        ),
    ])
}

impl Widget for Listing<'_> {
    /// Draw the list into its region.
    ///
    /// The footer is anchored to the bottom rather than set down after the last
    /// spawn, so it stays where the eye last found it however many spawns there
    /// are — and gives up its rows entirely on a list too short to spare them.
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let [rows, footer] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(FOOTER.rows(area.height)),
        ])
        .areas(area);

        let listed = self.rows(usize::from(rows.width));
        let scroll = scroll_offset(listed.selected, usize::from(rows.height));

        Paragraph::new(listed.lines)
            .scroll((scroll, 0))
            .render(rows, buffer);
        Paragraph::new(FOOTER.lines(usize::from(footer.width))).render(footer, buffer);
    }
}

/// Every row, in the order the list draws them.
///
/// The order the list moves through has to be the order it shows, or a
/// keystroke asking for the next row lands somewhere else on screen. Both come
/// from here.
pub fn order(drafts: &[Draft], entries: &[Entry], snapshot: &Snapshot) -> Vec<On> {
    let pinned = drafts.iter().map(|draft| On::Draft(draft.id()));

    pinned
        .chain(
            grouped(entries, snapshot)
                .into_iter()
                .flat_map(|group| group.spawns)
                .map(|placed| On::Spawn(placed.entry.spawn.clone())),
        )
        .collect()
}

/// One repository, and the spawns started against it.
struct Group<'a> {
    /// The repository's name, which is what its header says.
    repository: &'a str,
    /// Its spawns, attention-first once [`grouped`] has sorted them.
    spawns: Vec<Placed<'a>>,
}

impl Group<'_> {
    /// The repository's line: its name, and a mark per spawn under it.
    ///
    /// The bar is the whole point of the header — a project's state without
    /// reading its rows — so it is not the first thing dropped when the two will
    /// not fit. The name may take up to half the line, and the bar takes what is
    /// left; each gives back what it does not use.
    ///
    /// A bar with no room for every spawn loses its tail, which is where the
    /// attention-first order has already put the ones with least to say, and
    /// ends in the same mark everything else cut in this list ends in — a header
    /// that quietly reported fewer spawns than the repository has would be worse
    /// than one that says it ran out of room.
    fn header(&self, width: usize) -> Line<'static> {
        let room = width.saturating_sub(1);
        let named = self.repository.chars().count().min(room.div_ceil(2));
        let marks = self.spawns.len().min(room.saturating_sub(named));

        let mut spans = vec![
            Span::styled(elided(self.repository, room.saturating_sub(marks)), HEADING),
            Span::raw(" "),
        ];
        let cut = marks < self.spawns.len();
        let shown = if cut { marks.saturating_sub(1) } else { marks };
        spans.extend(self.spawns.iter().take(shown).map(Placed::mark));
        if cut && marks > 0 {
            spans.push(Span::styled(ELLIPSIS.to_string(), DIM));
        }

        Line::from(spans)
    }
}

/// One spawn, beside whatever the snapshot said about it.
struct Placed<'a> {
    /// What the list says about it.
    entry: &'a Entry,
    /// What the snapshot said about it, if it held it at all.
    row: Option<&'a Row>,
}

impl Placed<'_> {
    /// The spawn's name, which is what the cursor holds it by.
    fn name(&self) -> &str {
        &self.entry.spawn
    }

    /// What it is doing, or nothing at all before the first snapshot lands.
    fn status(&self) -> Option<Status> {
        self.row.map(|row| row.status)
    }

    /// Why the app cannot tell what it is doing, when it cannot.
    fn reason(&self) -> Option<&str> {
        self.row.and_then(|row| row.reason.as_deref())
    }

    /// Its status, as the one mark a header's bar is made of.
    fn mark(&self) -> Span<'static> {
        shown_as(self.status()).span()
    }

    /// Its own line: the gutter, its status, and its name.
    fn row(&self, selected: bool, width: usize) -> Line<'static> {
        let shown_as = shown_as(self.status());

        Line::from(vec![
            gutter(selected),
            shown_as.span(),
            Span::raw(" "),
            Span::styled(
                elided(self.name(), width.saturating_sub(INDENT)),
                shown_as.how_it_reads,
            ),
        ])
    }

    /// What it says beyond its name, when the list is on it.
    ///
    /// Under the selected row and nowhere else. The branch and the worktree are
    /// what the app made and the user did not, and the reason is a sentence: on
    /// every row they would cost twenty spawns sixty lines, which is the density
    /// the list is for. Selecting a row is how you go and read them.
    fn detail(&self, width: usize) -> Vec<Line<'static>> {
        let room = width.saturating_sub(INDENT);

        let mut lines: Vec<Line<'static>> = [&self.entry.branch, &self.entry.worktree]
            .into_iter()
            .map(|text| indented(&elided(text, room), DIM))
            .collect();
        if let Some(why) = self.reason() {
            lines.extend(
                wrapped(why, room)
                    .iter()
                    .map(|line| indented(line, DIM.fg(AMBER))),
            );
        }

        lines
    }
}

/// The spawns, under their repositories, attention-first.
///
/// Repositories keep the order their first spawn was started in, and equal
/// statuses keep it too — the sort is stable, and that is load-bearing. A list
/// that reshuffled its working spawns every tick would be unreadable, and the
/// only thing that ever moves a row is a status actually changing.
fn grouped<'a>(entries: &'a [Entry], snapshot: &'a Snapshot) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();

    for entry in entries {
        let placed = Placed {
            entry,
            row: snapshot.of(&entry.spawn),
        };

        match groups
            .iter_mut()
            .find(|group| group.repository == entry.repository)
        {
            Some(group) => group.spawns.push(placed),
            None => groups.push(Group {
                repository: &entry.repository,
                spawns: vec![placed],
            }),
        }
    }

    for group in &mut groups {
        group
            .spawns
            .sort_by_key(|placed| attention(placed.status()));
    }

    groups
}

/// How near the top of its group a status puts a spawn.
///
/// Stopped first because it is the one that might need you. Unknown next
/// because something is wrong with the tooling and that is worth seeing.
/// Working after both, and a spawn the app has heard nothing about at all last:
/// it is the only rung with nothing to say yet.
fn attention(status: Option<Status>) -> u8 {
    match status {
        Some(Status::Stopped) => 0,
        Some(Status::Unknown) => 1,
        Some(Status::Working) => 2,
        None => 3,
    }
}

/// A line of detail, lined up under the name it belongs to.
fn indented(text: &str, how_it_reads: Style) -> Line<'static> {
    Line::styled(format!("{}{text}", " ".repeat(INDENT)), how_it_reads)
}

/// How a status shows: a shape and a colour, which travel together.
///
/// One type rather than two values, because they are one decision. Where they
/// are read apart — the header's bar takes only the shape — it is the same
/// answer being read from.
struct Shown {
    /// The mark in the status column.
    mark: &'static str,
    /// How that mark, and the name beside it, read.
    how_it_reads: Style,
}

impl Shown {
    /// The mark itself, ready to be drawn.
    fn span(&self) -> Span<'static> {
        Span::styled(self.mark, self.how_it_reads)
    }
}

/// How a status is shown.
///
/// One answer rather than two, because the mark and the colour have to travel
/// together. Working recedes, stopped is the only bright thing, unknown is the
/// outlier. A spawn the app has not heard about yet is a blank of the same
/// width, so a row does not shift sideways when the first snapshot lands.
///
/// **Working is given a colour of its own and stopped is not**, which looks
/// backwards and is not. Grey recedes on a light terminal and on a dark one
/// alike, and it recedes on the terminals that quietly ignore dim — where a
/// working spawn drawn only in dim would read exactly like a spawn the app
/// knows nothing about. Stopped is the user's own foreground at full weight,
/// which is the brightest thing their theme has; naming a colour for it would
/// be picking white, and white is invisible on half the terminals it would be
/// picked for.
fn shown_as(status: Option<Status>) -> Shown {
    let (mark, how_it_reads) = match status {
        Some(Status::Working) => ("·", Style::new().fg(Color::DarkGray)),
        Some(Status::Stopped) => ("●", HEADING),
        Some(Status::Unknown) => ("?", HEADING.fg(AMBER)),
        None => (" ", Style::new()),
    };

    Shown { mark, how_it_reads }
}

/// `text`, broken across lines of at most `cells`, on the spaces in it.
///
/// For the one thing in the list that is prose rather than a name. A word too
/// long for a line of its own is cut, which is the only way it ends.
///
/// A list with no room at all gets no lines rather than a blank one per word:
/// they would show as nothing and still push the spawns under them down.
fn wrapped(text: &str, cells: usize) -> Vec<String> {
    if cells == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();

    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= cells => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(elided(word, cells)),
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::tests::drafting;
    use crate::scaffolding::SELECTED;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    /// A spawn on a repository, named the way the app names them.
    fn entry(repository: &str, spawn: &str) -> Entry {
        Entry {
            repository: repository.to_string(),
            spawn: spawn.to_string(),
            branch: format!("spawn/{spawn}"),
            worktree: format!("/w/{spawn}"),
        }
    }

    /// Two repositories, five spawns, in the order they were started.
    fn five() -> Vec<Entry> {
        vec![
            entry("harness-launcher", "add-retry-logic"),
            entry("harness-launcher", "fix-worktree-cleanup"),
            entry("harness-launcher", "spawn-form-choices"),
            entry("acme-api", "rate-limit-headers"),
            entry("acme-api", "drop-legacy-auth"),
        ]
    }

    /// What the supervisor would have said about those five.
    fn saying(reason: Option<&str>) -> Snapshot {
        Snapshot {
            rows: vec![
                said("add-retry-logic", Status::Working, None),
                said("fix-worktree-cleanup", Status::Stopped, None),
                said("spawn-form-choices", Status::Unknown, reason),
                said("rate-limit-headers", Status::Working, None),
                said("drop-legacy-auth", Status::Stopped, None),
            ],
        }
    }

    fn said(name: &str, status: Status, reason: Option<&str>) -> Row {
        Row {
            name: name.to_string(),
            status,
            reason: reason.map(str::to_string),
        }
    }

    /// The list as it lands on a terminal of exactly this size, with nothing
    /// being drafted.
    fn drawn(
        width: u16,
        height: u16,
        entries: &[Entry],
        snapshot: &Snapshot,
        cursor: &Cursor,
    ) -> String {
        with_drafts(width, height, &[], entries, snapshot, cursor)
    }

    /// The same, with some drafts in flight.
    ///
    /// Trailing blanks are cut so a hand-written snapshot can be written the way
    /// it reads rather than padded out to the width.
    fn with_drafts(
        width: u16,
        height: u16,
        drafts: &[Draft],
        entries: &[Entry],
        snapshot: &Snapshot,
        cursor: &Cursor,
    ) -> String {
        let buffer = painted(width, height, drafts, entries, snapshot, cursor);

        (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|column| buffer[(column, row)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The cells the list put on a terminal of exactly this size.
    fn painted(
        width: u16,
        height: u16,
        drafts: &[Draft],
        entries: &[Entry],
        snapshot: &Snapshot,
        cursor: &Cursor,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Listing::new(drafts, entries, snapshot, cursor),
                    frame.area(),
                );
            })
            .unwrap();

        terminal.backend().buffer().clone()
    }

    /// The names of the spawns an order walks, in the order it walks them.
    fn spawns_in(order: &[On]) -> Vec<String> {
        order
            .iter()
            .filter_map(|row| match row {
                On::Spawn(spawn) => Some(spawn.clone()),
                On::Draft(_) => None,
            })
            .collect()
    }

    #[test]
    fn spawns_group_under_their_repository_attention_first() {
        let screen = drawn(30, 13, &five(), &saying(None), &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ● fix-worktree-cleanup
 ? spawn-form-choices
 · add-retry-logic

acme-api ●·
 ● drop-legacy-auth
 · rate-limit-headers
F2 starts a draft
F6 / F7 move the selection
F10 quits — nothing is killed"
        );
    }

    #[test]
    fn the_selection_is_a_mark_in_the_gutter_and_the_spawn_beneath_it() {
        let screen = drawn(
            34,
            15,
            &five(),
            &saying(None),
            &Cursor::on_spawn("add-retry-logic"),
        );

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ● fix-worktree-cleanup
 ? spawn-form-choices
▍· add-retry-logic
   spawn/add-retry-logic
   /w/add-retry-logic

acme-api ●·
 ● drop-legacy-auth
 · rate-limit-headers
F2 starts a draft
F6 / F7 move the selection
F10 quits — nothing is killed"
        );
    }

    #[test]
    fn the_reason_an_unknown_spawn_is_unknown_reads_under_the_selected_row() {
        let screen = drawn(
            30,
            17,
            &five(),
            &saying(Some("its session record carries no status")),
            &Cursor::on_spawn("spawn-form-choices"),
        );

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ● fix-worktree-cleanup
▍? spawn-form-choices
   spawn/spawn-form-choices
   /w/spawn-form-choices
   its session record carries
   no status
 · add-retry-logic

acme-api ●·
 ● drop-legacy-auth
 · rate-limit-headers
F2 starts a draft
F6 / F7 move the selection
F10 quits — nothing is killed"
        );
    }

    /// The rule stated as a test: a draft is a row of its own, above every
    /// repository — which is where something half-written has to be, because
    /// nothing else in the app will remind you of it.
    #[test]
    fn a_draft_is_a_row_of_its_own_pinned_above_the_repositories() {
        let drafts = drafting(&["fix the worktree cleanup"]);

        let screen = with_drafts(
            30,
            14,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        assert_eq!(
            screen,
            "\
SPAWNS

 + fix the worktree cleanup

harness-launcher ●?·
 ● fix-worktree-cleanup
 ? spawn-form-choices
 · add-retry-logic

acme-api ●·
 ● drop-legacy-auth
F2 starts a draft
F6 / F7 move the selection
F10 quits — nothing is killed"
        );
    }

    #[test]
    fn a_draft_nothing_has_been_typed_into_is_still_a_row_to_come_back_to() {
        let drafts = drafting(&[""]);

        let screen = with_drafts(
            30,
            14,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        assert!(screen.contains(" + a new spawn"), "{screen}");
    }

    #[test]
    fn several_drafts_are_several_rows_and_every_spawn_is_still_there() {
        let drafts = drafting(&["the first", "the second", "the third"]);

        let screen = with_drafts(
            30,
            18,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        for named in ["the first", "the second", "the third", "add-retry-logic"] {
            assert!(
                screen.contains(named),
                "{named} is not on screen:\n{screen}"
            );
        }
    }

    #[test]
    fn the_selection_on_a_draft_reads_the_way_it_does_on_a_spawn() {
        let drafts = drafting(&["the first", "the second"]);
        let on_it = Cursor::on_draft(drafts.all()[1].id());

        let screen = with_drafts(30, 14, drafts.all(), &five(), &saying(None), &on_it);

        assert!(screen.contains("▍+ the second"), "{screen}");
        assert!(screen.contains(" + the first"), "{screen}");
    }

    #[test]
    fn the_order_walks_the_drafts_before_any_repository() {
        let drafts = drafting(&["the first", "the second"]);
        let entries = five();

        let order = order(drafts.all(), &entries, &saying(None));

        assert_eq!(
            order[..2],
            [
                On::Draft(drafts.all()[0].id()),
                On::Draft(drafts.all()[1].id())
            ]
        );
        assert_eq!(spawns_in(&order)[0], "fix-worktree-cleanup");
    }

    #[test]
    fn moving_crosses_from_the_drafts_into_the_first_repository() {
        let drafts = drafting(&["the only draft"]);
        let entries = five();
        let order = order(drafts.all(), &entries, &saying(None));
        let mut cursor = Cursor::on_draft(drafts.all()[0].id());

        cursor.moved(&order, Step::Down);

        assert_eq!(cursor.spawn(), Some("fix-worktree-cleanup"));
    }

    #[test]
    fn a_draft_is_where_the_selection_lands_when_it_has_nowhere_to_carry_on_from() {
        let drafts = drafting(&["the only draft"]);
        let entries = five();
        let order = order(drafts.all(), &entries, &saying(None));
        let mut cursor = Cursor::default();

        cursor.moved(&order, Step::Down);

        assert_eq!(cursor.draft(), Some(drafts.all()[0].id()));
    }

    #[test]
    fn a_name_too_long_for_the_list_is_cut_rather_than_wrapped() {
        let entries = vec![entry(
            "a-repository-with-a-very-long-name",
            "an-extremely-long-spawn-name-indeed",
        )];
        let snapshot = Snapshot {
            rows: vec![said(
                "an-extremely-long-spawn-name-indeed",
                Status::Working,
                None,
            )],
        };

        let screen = drawn(24, 9, &entries, &snapshot, &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

a-repository-with-a-v… ·
 · an-extremely-long-sp…


F2 starts a draft
F6 / F7 move the select…
F10 quits — nothing is …"
        );
    }

    #[test]
    fn a_wider_list_is_a_wider_layout_and_not_the_same_one_with_space_beside_it() {
        let entries = vec![entry("acme-api", "an-extremely-long-spawn-name-indeed")];
        let snapshot = Snapshot {
            rows: vec![said(
                "an-extremely-long-spawn-name-indeed",
                Status::Working,
                None,
            )],
        };

        let narrow = drawn(24, 8, &entries, &snapshot, &Cursor::default());
        let wide = drawn(60, 8, &entries, &snapshot, &Cursor::default());

        assert!(!narrow.contains("spawn-name-indeed"), "{narrow}");
        assert!(
            wide.contains("an-extremely-long-spawn-name-indeed"),
            "{wide}"
        );
    }

    /// Twenty spawns over two repositories, most of them working.
    fn twenty() -> (Vec<Entry>, Snapshot) {
        let named: Vec<(&str, String)> = (1..=12)
            .map(|number| ("acme-api", format!("task-{number:02}")))
            .chain((1..=8).map(|number| ("dotfiles", format!("chore-{number:02}"))))
            .collect();

        let entries = named
            .iter()
            .map(|(repository, spawn)| entry(repository, spawn))
            .collect();
        let rows = named
            .iter()
            .map(|(_, spawn)| {
                let status = match spawn.as_str() {
                    "task-03" | "chore-02" => Status::Stopped,
                    "task-07" => Status::Unknown,
                    _ => Status::Working,
                };

                said(spawn, status, None)
            })
            .collect();

        (entries, Snapshot { rows })
    }

    #[test]
    fn twenty_spawns_still_read_as_two_projects() {
        let (entries, snapshot) = twenty();

        let screen = drawn(30, 28, &entries, &snapshot, &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

acme-api ●?··········
 ● task-03
 ? task-07
 · task-01
 · task-02
 · task-04
 · task-05
 · task-06
 · task-08
 · task-09
 · task-10
 · task-11
 · task-12

dotfiles ●·······
 ● chore-02
 · chore-01
 · chore-03
 · chore-04
 · chore-05
 · chore-06
 · chore-07
 · chore-08
F2 starts a draft
F6 / F7 move the selection
F10 quits — nothing is killed"
        );
    }

    /// Where a row's status mark sits: after the gutter the selection uses.
    const MARK: u16 = 1;

    #[test]
    fn a_status_is_a_shape_and_a_colour_at_once() {
        let (entries, snapshot) = (five(), saying(None));
        let painted = painted(30, 12, &[], &entries, &snapshot, &Cursor::default());
        // The first group's rows, which are stopped, unknown and working.
        let stopped = &painted[(MARK, 3)];
        let unknown = &painted[(MARK, 4)];
        let working = &painted[(MARK, 5)];

        // A shape each, so the list survives being read without colour at all.
        assert_eq!(
            [stopped.symbol(), unknown.symbol(), working.symbol()],
            ["●", "?", "·"]
        );
        // And a way of reading each, so it survives being read at a glance.
        assert_eq!(unknown.style().fg, Some(AMBER));
        assert_eq!(working.style().fg, Some(Color::DarkGray));
        assert!(
            stopped.style().add_modifier.contains(Modifier::BOLD),
            "{:?}",
            stopped.style()
        );
        assert_ne!(stopped.style().fg, unknown.style().fg);
        assert_ne!(stopped.style().fg, working.style().fg);
    }

    #[test]
    fn a_repository_with_more_spawns_than_the_bar_has_room_for_says_so() {
        let (entries, snapshot) = twenty();

        let screen = drawn(16, 27, &entries, &snapshot, &Cursor::default());

        let bar = screen
            .lines()
            .find(|line| line.starts_with("acme-api"))
            .unwrap_or_else(|| panic!("the repository has no header:\n{screen}"));
        assert!(
            bar.ends_with('…'),
            "a bar with twelve spawns in eight cells claimed to be whole: {bar:?}"
        );
        assert!(
            bar.contains('●'),
            "the stopped spawn fell off the bar: {bar}"
        );
    }

    #[test]
    fn a_list_with_no_room_to_spare_gives_the_footers_rows_to_the_spawns() {
        let screen = drawn(30, 5, &five(), &saying(None), &Cursor::default());

        assert!(!screen.contains("F10 quits"), "{screen}");
        assert!(screen.contains("fix-worktree-cleanup"), "{screen}");
    }

    #[test]
    fn a_selection_below_the_fold_is_scrolled_onto_the_screen() {
        let (entries, snapshot) = twenty();

        let screen = drawn(30, 12, &entries, &snapshot, &Cursor::on_spawn("chore-08"));

        assert!(
            screen.contains("▍· chore-08"),
            "the selection moved somewhere the screen does not reach:\n{screen}"
        );
        assert!(
            screen.contains("   spawn/chore-08"),
            "the selected spawn's detail hangs off the bottom:\n{screen}"
        );
    }

    #[test]
    fn a_list_that_fits_is_not_scrolled_at_all() {
        let screen = drawn(
            30,
            15,
            &five(),
            &saying(None),
            &Cursor::on_spawn("rate-limit-headers"),
        );

        assert!(
            screen.starts_with("SPAWNS"),
            "a list that fits scroll_offset anyway:\n{screen}"
        );
    }

    #[test]
    fn a_list_too_narrow_to_say_why_says_the_rest_of_it_anyway() {
        let screen = drawn(
            4,
            10,
            &five(),
            &saying(Some("its session record carries no status")),
            &Cursor::on_spawn("spawn-form-choices"),
        );

        assert!(
            screen.lines().any(|line| line.starts_with("▍?")),
            "the selected spawn is not on a list four cells wide:\n{screen}"
        );
    }

    #[test]
    fn the_order_moved_through_is_the_order_drawn() {
        let entries = five();
        let snapshot = saying(None);

        let order = order(&[], &entries, &snapshot);

        assert_eq!(
            spawns_in(&order),
            [
                "fix-worktree-cleanup",
                "spawn-form-choices",
                "add-retry-logic",
                "drop-legacy-auth",
                "rate-limit-headers",
            ]
        );
    }

    #[test]
    fn a_spawn_the_app_has_heard_nothing_about_sits_under_the_ones_it_has() {
        let entries = five();
        let snapshot = Snapshot {
            rows: vec![said("add-retry-logic", Status::Working, None)],
        };

        let order = order(&[], &entries, &snapshot);

        assert_eq!(spawns_in(&order)[0], "add-retry-logic");
    }

    #[test]
    fn moving_follows_the_order_on_screen_rather_than_the_order_they_started_in() {
        let entries = five();
        let snapshot = saying(None);
        let order = order(&[], &entries, &snapshot);
        let mut cursor = Cursor::on_spawn("fix-worktree-cleanup");

        cursor.moved(&order, Step::Down);

        assert_eq!(cursor.spawn(), Some("spawn-form-choices"));
    }

    #[test]
    fn moving_crosses_from_one_repository_into_the_next() {
        let entries = five();
        let snapshot = saying(None);
        let order = order(&[], &entries, &snapshot);
        let mut cursor = Cursor::on_spawn("add-retry-logic");

        cursor.moved(&order, Step::Down);

        assert_eq!(cursor.spawn(), Some("drop-legacy-auth"));
    }

    #[test]
    fn both_ends_of_the_list_stop_rather_than_wrap() {
        let entries = five();
        let snapshot = saying(None);
        let order = order(&[], &entries, &snapshot);
        let mut top = Cursor::on_spawn("fix-worktree-cleanup");
        let mut bottom = Cursor::on_spawn("rate-limit-headers");

        top.moved(&order, Step::Up);
        bottom.moved(&order, Step::Down);

        assert_eq!(top.spawn(), Some("fix-worktree-cleanup"));
        assert_eq!(bottom.spawn(), Some("rate-limit-headers"));
    }

    #[test]
    fn a_cursor_on_nothing_lands_on_the_first_row() {
        let entries = five();
        let snapshot = saying(None);
        let order = order(&[], &entries, &snapshot);
        let mut cursor = Cursor::default();

        cursor.moved(&order, Step::Up);

        assert_eq!(cursor.spawn(), Some("fix-worktree-cleanup"));
    }

    #[test]
    fn a_cursor_on_a_spawn_that_is_no_longer_listed_lands_on_the_first_row() {
        let entries = five();
        let snapshot = saying(None);
        let order = order(&[], &entries, &snapshot);
        let mut cursor = Cursor::on_spawn("retired-long-ago");

        cursor.moved(&order, Step::Down);

        assert_eq!(cursor.spawn(), Some("fix-worktree-cleanup"));
    }

    #[test]
    fn an_empty_list_leaves_the_cursor_on_nothing() {
        let mut cursor = Cursor::on_spawn("add-retry-logic");

        cursor.moved(&[], Step::Down);

        assert_eq!(cursor.spawn(), None);
    }

    #[test]
    fn a_status_changing_under_the_cursor_does_not_move_it_to_another_spawn() {
        let entries = five();
        let cursor = Cursor::on_spawn("add-retry-logic");
        let everything_else_stopped = Snapshot {
            rows: vec![
                said("add-retry-logic", Status::Stopped, None),
                said("fix-worktree-cleanup", Status::Working, None),
                said("spawn-form-choices", Status::Working, None),
                said("rate-limit-headers", Status::Working, None),
                said("drop-legacy-auth", Status::Working, None),
            ],
        };

        let before = drawn(30, 14, &entries, &saying(None), &cursor);
        let after = drawn(30, 14, &entries, &everything_else_stopped, &cursor);

        for screen in [&before, &after] {
            let selected = screen
                .lines()
                .find(|line| line.starts_with(SELECTED))
                .unwrap_or_else(|| panic!("nothing is selected:\n{screen}"));
            assert!(
                selected.contains("add-retry-logic"),
                "the selection changed spawn when a status did:\n{screen}"
            );
        }
    }
}
