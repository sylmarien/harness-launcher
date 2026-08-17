//! The list: every spawn, every draft being written, and which one is
//! selected.
//!
//! Drafts are pinned above every repository. Spawns group under the
//! repository they were started against, ordered attention-first (stopped,
//! unknown, working), and each repository header carries a bar of its spawns'
//! status marks. Status is a shape and a colour decided together, once, in
//! [`shown_as`], so the list survives a colour-blind reader.
//!
//! Every entry is one line, selected or not; text that does not fit is cut,
//! never wrapped. A spawn's row ends with the age of its status, right
//! aligned in a column the whole list shares. A row shows its age only when
//! its own name fits in full beside it, so no name is cut to make room for an
//! age. The selected row is painted as a full-width band, not just marked.
//! The selection is held by name, never by position, because rows re-order as
//! statuses change. Scrolling to rows the selection is not on is still an
//! open design question.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::draft::{self, Draft};
use crate::harness::GLYPH;
use crate::retirement::{Retirement, Retirements};
use crate::scaffolding::{AMBER, DIM, ELLIPSIS, HEADING, band, gutter};
use crate::scaffolding::{Footer, elided, scroll_offset};
use crate::snapshot::{Row, Snapshot, Status};

/// The mark against a draft being written — deliberately unlike the status
/// marks, since a draft has no agent.
const DRAFT: &str = "+";

/// The mark against a draft that is being made into a spawn.
const STARTING: &str = ">";

/// The mark against a draft that failed to start, and against a spawn that
/// would not retire: both mean the app needs a person.
const STOPPED: &str = "!";

/// The mark against a spawn being retired.
const RETIRING: &str = "-";

/// How far a row's name sits from the left: the gutter, the status mark, the
/// harness column, and a space.
const INDENT: usize = 4;

/// What a draft puts in the harness column: a blank of the same width, so
/// names line up and a draft becoming a spawn does not shift its row.
const NOTHING_YET: &str = " ";

/// What the foot of the list says the keyboard does.
///
/// Eight is the shortest list that can spare four rows for it: four rows of
/// spawns is the least that reads as a list at all.
const FOOTER: Footer = Footer::new(
    &[
        "F2 starts a draft",
        "F6 / F7 move the selection",
        "F9 retires the spawn",
        "F10 quits — nothing is killed",
    ],
    8,
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
/// A draft is held by its assigned id: a name taken from what has been typed
/// would slip out from under the selection as it was typed.
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
    /// Both ends stop rather than wrap. A cursor on nothing — or on a row no
    /// longer in the list — lands on the first row, whichever way it was
    /// asked to go.
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
    /// Which of them are being retired, and what came of it.
    retirements: &'a Retirements,
    /// Which row the list is on.
    cursor: &'a Cursor,
}

impl<'a> Listing<'a> {
    /// The snapshot the list drew from, so the rest of the screen draws one
    /// frame from the same moment.
    pub fn snapshot(&self) -> &'a Snapshot {
        self.snapshot
    }

    /// Where a spawn's retirement has got to, for the same reason as
    /// [`Listing::snapshot`].
    pub fn retirement(&self, spawn: &str) -> Option<&'a Retirement> {
        self.retirements.of(spawn)
    }

    /// The list of these drafts and spawns, as this snapshot found them.
    pub fn new(
        drafts: &'a [Draft],
        entries: &'a [Entry],
        snapshot: &'a Snapshot,
        retirements: &'a Retirements,
        cursor: &'a Cursor,
    ) -> Self {
        Self {
            drafts,
            entries,
            snapshot,
            retirements,
            cursor,
        }
    }

    /// Everything above the footer, and where the selected row is in it.
    fn rows(&self, width: usize) -> Rows {
        let mut lines = vec![Line::styled("SPAWNS", HEADING), Line::raw("")];
        let mut selected = None;
        let groups = grouped(self.entries, self.snapshot, self.retirements);
        let age_cells = age_column(&groups);

        for draft in self.drafts {
            let on_it = self.cursor.draft() == Some(draft.id());
            if on_it {
                selected = Some((lines.len(), lines.len()));
            }
            lines.push(drafted(draft, on_it, width));
        }

        for (at, group) in groups.iter().enumerate() {
            if at > 0 || !self.drafts.is_empty() {
                lines.push(Line::raw(""));
            }
            lines.push(group.header(width));

            for placed in &group.spawns {
                let on_it = self.cursor.spawn() == Some(placed.name());
                if on_it {
                    selected = Some((lines.len(), lines.len()));
                }
                lines.push(placed.row(on_it, width, age_cells));
            }
        }

        Rows { lines, selected }
    }
}

/// The list's lines, and which of them the selected row takes up.
struct Rows {
    /// Every line there is, whether or not the region can hold them all.
    lines: Vec<Line<'static>>,
    /// The first and last line of the selected row (the same line twice); the
    /// pair is what [`scroll_offset`] takes.
    selected: Option<(usize, usize)>,
}

/// A draft's row: the gutter, its mark, and its title so far.
///
/// The mark tracks the three states a draft can be in — being written,
/// starting, stopped — because the list is the only place a draft appears.
fn drafted(draft: &Draft, on_it: bool, width: usize) -> Line<'static> {
    let shown = if draft.stopped() {
        Shown {
            mark: STOPPED,
            how_it_reads: HEADING.fg(AMBER),
        }
    } else if draft.starting() {
        Shown {
            mark: STARTING,
            how_it_reads: HEADING,
        }
    } else {
        Shown {
            mark: DRAFT,
            how_it_reads: HEADING,
        }
    };

    lined(&shown, NOTHING_YET, &draft.title(), "", on_it, width, None)
}

/// How wide the age column is, or nothing at all when no row has an age.
///
/// The widest age the list is about to draw decides it, once, for every row.
/// So the ages line up. Which rows show an age is then [`lined`]'s call, one
/// row at a time. Every age sizes the column, including the ages of rows that
/// go on to drop them. That can leave the column a cell or two wider than the
/// rows that use it need.
fn age_column(groups: &[Group<'_>]) -> Option<usize> {
    groups
        .iter()
        .flat_map(|group| &group.spawns)
        .map(|placed| placed.age().chars().count())
        .filter(|age| *age > 0)
        .max()
}

/// One row, whatever it stands for: gutter, mark, harness column, name, and
/// the age against the right edge.
///
/// Padded out to the full width of the list, so a selected row's band spans
/// the row rather than highlighting the name. `age_cells` is the column
/// [`age_column`] sized for the whole list, so every row that shows an age
/// cuts its name at the same place and the ages share a right edge. A row
/// shows its age only when its own name fits in full beside it. A row whose
/// name does not fit drops its own age and keeps the whole width, and so does
/// a row with no age: no name is ever cut to make room for an age.
fn lined(
    shown: &Shown,
    runs_under: &'static str,
    name: &str,
    age: &str,
    on_it: bool,
    width: usize,
    age_cells: Option<usize>,
) -> Line<'static> {
    let how_it_reads = shown.reading(on_it);
    let room = width.saturating_sub(INDENT);
    let beside = age_cells
        .filter(|_| !age.is_empty())
        .map(|cells| (room.saturating_sub(cells + 1), cells))
        .filter(|(named, _)| name.chars().count() <= *named);
    let (named, tail) = match beside {
        Some((named, cells)) => (named, format!(" {age:>cells$}")),
        None => (room, String::new()),
    };

    Line::from(vec![
        gutter(on_it, how_it_reads),
        Span::styled(shown.mark, how_it_reads),
        Span::styled(runs_under, how_it_reads),
        Span::styled(
            format!(" {:<named$}{tail}", elided(name, named)),
            how_it_reads,
        ),
    ])
}

/// An age as the list writes it: `4m`, `31m`, `1h4m`. A minute is the finest
/// unit, and the hours are not capped at a day.
fn written_as(age: Duration) -> String {
    let minutes = age.as_secs() / 60;

    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, minutes) => format!("{hours}h{minutes}m"),
    }
}

impl Widget for Listing<'_> {
    /// Draw the list into its region, with the footer anchored to the bottom
    /// and dropped entirely on a list too short to spare its rows.
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

/// Every row, in the order the list draws them — which has to be the order
/// the cursor moves through.
///
/// Retirements are deliberately ignored: a row that re-sorted as its
/// retirement started would move out from under the selection.
pub fn order(drafts: &[Draft], entries: &[Entry], snapshot: &Snapshot) -> Vec<On> {
    let pinned = drafts.iter().map(|draft| On::Draft(draft.id()));

    pinned
        .chain(
            grouped(entries, snapshot, &Retirements::default())
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
    /// The repository's line: its name (at most half the line) and a mark per
    /// spawn.
    ///
    /// A bar with too little room drops its tail — where the attention-first
    /// order put the least urgent marks — and ends in an ellipsis rather than
    /// silently under-reporting.
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
    /// Where its retirement has got to, if it is being retired at all.
    retirement: Option<&'a Retirement>,
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

    /// How it shows. A retirement outranks a status: a spawn somebody is done
    /// with is not one to go and look at, whatever its agent was doing.
    fn shown(&self) -> Shown {
        match self.retirement {
            Some(retirement) if retirement.refused() => Shown {
                mark: STOPPED,
                how_it_reads: HEADING.fg(AMBER),
            },
            Some(_) => Shown {
                mark: RETIRING,
                how_it_reads: Style::new().fg(Color::DarkGray),
            },
            None => shown_as(self.status()),
        }
    }

    /// Its status, as the one mark a header's bar is made of.
    fn mark(&self) -> Span<'static> {
        self.shown().span()
    }

    /// How long its status has held, as the list writes it. Nothing at all
    /// before the first snapshot that holds the spawn, and nothing for an
    /// adopted spawn whose status the app never saw begin.
    fn age(&self) -> String {
        self.row
            .and_then(|row| row.age)
            .map_or_else(String::new, written_as)
    }

    /// Its own line: gutter, status, harness glyph, name, age.
    ///
    /// The branch and the worktree are deliberately absent: both are
    /// derivable from the name, which the row keeps.
    fn row(&self, selected: bool, width: usize, age_cells: Option<usize>) -> Line<'static> {
        lined(
            &self.shown(),
            GLYPH,
            self.name(),
            &self.age(),
            selected,
            width,
            age_cells,
        )
    }
}

/// The spawns, under their repositories, attention-first.
///
/// The sort is stable and that is load-bearing: only a status actually
/// changing ever moves a row.
fn grouped<'a>(
    entries: &'a [Entry],
    snapshot: &'a Snapshot,
    retirements: &'a Retirements,
) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();

    for entry in entries {
        let placed = Placed {
            entry,
            row: snapshot.of(&entry.spawn),
            retirement: retirements.of(&entry.spawn),
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

/// How near the top of its group a status puts a spawn: stopped first
/// because it might need you, then unknown, working, and never-heard-of.
fn attention(status: Option<Status>) -> u8 {
    match status {
        Some(Status::Stopped) => 0,
        Some(Status::Unknown) => 1,
        Some(Status::Working) => 2,
        None => 3,
    }
}

/// How a status shows: a shape and a colour, decided together.
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

    /// How the whole row reads: its own style, or black on the band when
    /// selected. The status colour is not carried onto the band — the marks
    /// tell every state apart without it — except that an alarmed row's band
    /// goes amber.
    fn reading(&self, selected: bool) -> Style {
        if selected {
            band(self.alarmed())
        } else {
            self.how_it_reads
        }
    }

    /// Whether the app is admitting something about this row — read off the
    /// [`AMBER`] rather than listing the states again, so the two can never
    /// disagree.
    fn alarmed(&self) -> bool {
        self.how_it_reads.fg == Some(AMBER)
    }
}

/// How a status is shown: mark and colour together.
///
/// Working is grey rather than dim because some terminals ignore dim, where
/// it would read like no status at all. Stopped is the theme's own foreground
/// at full weight — naming a colour would mean white, invisible on light
/// terminals. The rule: the app names a foreground only where it painted the
/// background itself (the selection band); on the user's background it leaves
/// colours alone. No status yet is a blank of the same width, so a row does
/// not shift when the first snapshot lands.
fn shown_as(status: Option<Status>) -> Shown {
    let (mark, how_it_reads) = match status {
        Some(Status::Working) => ("·", Style::new().fg(Color::DarkGray)),
        Some(Status::Stopped) => ("●", HEADING),
        Some(Status::Unknown) => ("?", HEADING.fg(AMBER)),
        None => (" ", Style::new()),
    };

    Shown { mark, how_it_reads }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::tests::drafting;
    use crate::scaffolding::SELECTED;
    use crate::snapshot;
    use std::time::Instant;

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

    /// What the supervisor would have said about those five, with an age
    /// apiece: one of every shape the list writes.
    fn saying(reason: Option<&str>) -> Snapshot {
        Snapshot {
            rows: vec![
                for_minutes(said("add-retry-logic", Status::Working, None), 4),
                for_minutes(said("fix-worktree-cleanup", Status::Stopped, None), 31),
                for_minutes(said("spawn-form-choices", Status::Unknown, reason), 64),
                for_minutes(said("rate-limit-headers", Status::Working, None), 121),
                for_minutes(said("drop-legacy-auth", Status::Stopped, None), 7),
            ],
        }
    }

    /// A row whose status has held this many minutes.
    fn for_minutes(row: Row, minutes: u64) -> Row {
        Row {
            age: Some(Duration::from_mins(minutes)),
            ..row
        }
    }

    /// A row of a status the supervisor has just this moment read.
    fn said(name: &str, status: Status, reason: Option<&str>) -> Row {
        Row {
            name: name.to_string(),
            status,
            unaccounted: reason.map(|why| snapshot::cannot_account(why, None)),
            last_known: snapshot::last_read(status),
            changed: Some(Instant::now()),
            age: Some(Duration::ZERO),
        }
    }

    /// The list on a terminal of exactly this size, with nothing drafted.
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
    fn with_drafts(
        width: u16,
        height: u16,
        drafts: &[Draft],
        entries: &[Entry],
        snapshot: &Snapshot,
        cursor: &Cursor,
    ) -> String {
        read(&painted(
            width,
            height,
            drafts,
            entries,
            snapshot,
            &Retirements::default(),
            cursor,
        ))
    }

    /// What a buffer says, as text with trailing blanks trimmed, so expected
    /// screens can be written the way they read.
    fn read(buffer: &Buffer) -> String {
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
        retirements: &Retirements,
        cursor: &Cursor,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Listing::new(drafts, entries, snapshot, retirements, cursor),
                    frame.area(),
                );
            })
            .unwrap();

        terminal.backend().buffer().clone()
    }

    /// The cells with some spawns being retired and nothing drafted.
    fn painted_on(
        width: u16,
        height: u16,
        entries: &[Entry],
        retirements: &Retirements,
        cursor: &Cursor,
    ) -> Buffer {
        painted(
            width,
            height,
            &[],
            entries,
            &saying(None),
            retirements,
            cursor,
        )
    }

    /// What that says, for the tests written against the text of a screen.
    fn while_retiring(
        width: u16,
        height: u16,
        entries: &[Entry],
        retirements: &Retirements,
        cursor: &Cursor,
    ) -> String {
        read(&painted_on(width, height, entries, retirements, cursor))
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
        let screen = drawn(30, 14, &five(), &saying(None), &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ●✻ fix-worktree-cleanup   31m
 ?✻ spawn-form-choices    1h4m
 ·✻ add-retry-logic         4m

acme-api ●·
 ●✻ drop-legacy-auth        7m
 ·✻ rate-limit-headers    2h1m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    #[test]
    fn the_selected_spawn_is_one_line_like_every_other_row() {
        let screen = drawn(
            34,
            14,
            &five(),
            &saying(None),
            &Cursor::on_spawn("add-retry-logic"),
        );

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ●✻ fix-worktree-cleanup       31m
 ?✻ spawn-form-choices        1h4m
▍·✻ add-retry-logic             4m

acme-api ●·
 ●✻ drop-legacy-auth            7m
 ·✻ rate-limit-headers        2h1m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    #[test]
    fn the_reason_an_unknown_spawn_is_unknown_is_not_in_the_list() {
        let screen = drawn(
            30,
            18,
            &five(),
            &saying(Some("its session record carries no status")),
            &Cursor::on_spawn("spawn-form-choices"),
        );

        assert!(screen.contains("▍?✻ spawn-form-choices"), "{screen}");
        assert!(
            !screen.contains("session record"),
            "the list drew the sentence the slot is already drawing:\n{screen}"
        );
    }

    #[test]
    fn a_row_says_neither_the_branch_nor_the_worktree() {
        let screen = drawn(
            60,
            14,
            &five(),
            &saying(None),
            &Cursor::on_spawn("add-retry-logic"),
        );

        assert!(!screen.contains("spawn/add-retry-logic"), "{screen}");
        assert!(!screen.contains("/w/add-retry-logic"), "{screen}");
    }

    #[test]
    fn a_draft_is_a_row_of_its_own_pinned_above_the_repositories() {
        let drafts = drafting(&["fix the worktree cleanup"]);

        let screen = with_drafts(
            30,
            15,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        assert_eq!(
            screen,
            "\
SPAWNS

 +  fix the worktree cleanup

harness-launcher ●?·
 ●✻ fix-worktree-cleanup   31m
 ?✻ spawn-form-choices    1h4m
 ·✻ add-retry-logic         4m

acme-api ●·
 ●✻ drop-legacy-auth        7m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
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

        assert!(screen.contains(" +  a new spawn"), "{screen}");
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

        assert!(screen.contains("▍+  the second"), "{screen}");
        assert!(screen.contains(" +  the first"), "{screen}");
    }

    #[test]
    fn a_drafts_row_says_whether_it_is_being_written_started_or_stopped() {
        let mut drafts = drafting(&["being written", "being started", "stopped"]);
        let started = drafts.all()[1].id();
        let stopped = drafts.all()[2].id();
        // The started one has a repository; the stopped one never named one,
        // so its submit is refused in place.
        drafts.edit(started, draft::Edit::Previous);
        for character in "/code/project".chars() {
            drafts.edit(started, draft::Edit::Typed(character));
        }
        drafts.submit(started);
        drafts.submit(stopped);

        let screen = with_drafts(
            30,
            16,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        assert!(screen.contains(" +  being written"), "{screen}");
        assert!(screen.contains(" >  being started"), "{screen}");
        assert!(screen.contains(" !  stopped"), "{screen}");
    }

    #[test]
    fn a_draft_that_stopped_reads_as_something_needing_a_person() {
        let mut drafts = drafting(&["fix the worktree cleanup"]);
        let stopped = drafts.all()[0].id();
        drafts.failed(stopped, "there is no such repository".to_string());
        let painted = painted(
            30,
            14,
            drafts.all(),
            &five(),
            &saying(None),
            &Retirements::default(),
            &Cursor::default(),
        );

        // Row 2 is the draft's row: heading, blank, then the draft.
        assert_eq!(painted[(MARK, 2)].symbol(), "!");
        assert_eq!(painted[(MARK, 2)].style().fg, Some(AMBER));
    }

    /// A retirement of this spawn, at whatever step it has reached.
    fn being_retired(spawn: &str, step: &str) -> Retirements {
        let mut retirements = Retirements::default();
        retirements.asked_for(spawn);
        retirements.doing(spawn, step.to_string());

        retirements
    }

    #[test]
    fn a_spawn_being_retired_says_so_on_its_row() {
        let retirements = being_retired("add-retry-logic", "stopping the session");

        let screen = while_retiring(
            30,
            16,
            &five(),
            &retirements,
            &Cursor::on_spawn("add-retry-logic"),
        );

        assert!(
            screen.contains("▍-✻ add-retry-logic"),
            "a spawn being retired reads like one that is working:\n{screen}"
        );
        assert!(
            !screen.contains("stopping the session"),
            "the list drew the sentence the slot is already drawing:\n{screen}"
        );
    }

    #[test]
    fn a_retirement_that_was_refused_reads_as_something_needing_a_person() {
        let mut retirements = being_retired("add-retry-logic", "removing the worktree");
        retirements.refused(
            "add-retry-logic",
            "/w/add-retry-logic has work in it that is not committed".to_string(),
        );

        let screen = while_retiring(
            30,
            16,
            &five(),
            &retirements,
            &Cursor::on_spawn("add-retry-logic"),
        );
        // Painted with the keyboard elsewhere: the amber must be the row's
        // own, not the selection's.
        let painted = painted(
            30,
            16,
            &[],
            &five(),
            &saying(None),
            &retirements,
            &Cursor::default(),
        );

        assert!(screen.contains("▍!✻ add-retry-logic"), "{screen}");
        let bar = screen
            .lines()
            .find(|line| line.starts_with("harness-launcher"))
            .unwrap_or_else(|| panic!("the repository has no header:\n{screen}"));
        assert!(
            bar.contains('!'),
            "the repository's bar does not carry the refusal: {bar}"
        );
        // Row 5: heading, blank, group header, then two spawns sort first.
        assert_eq!(painted[(MARK, 5)].symbol(), "!");
        assert_eq!(painted[(MARK, 5)].style().fg, Some(AMBER));
    }

    #[test]
    fn a_spawn_being_retired_stays_where_it_was_in_the_list() {
        let entries = five();
        let cursor = Cursor::on_spawn("add-retry-logic");
        let retirements = being_retired("add-retry-logic", "stopping the session");

        let untouched = while_retiring(30, 18, &entries, &Retirements::default(), &cursor);
        let retired = while_retiring(30, 18, &entries, &retirements, &cursor);

        assert_eq!(
            rows_of(&untouched),
            rows_of(&retired),
            "the list re-sorted itself around a spawn being retired"
        );
    }

    /// The spawns a screen has a row for, in the order it drew them.
    fn rows_of(screen: &str) -> Vec<String> {
        let named: Vec<String> = five().iter().map(|entry| entry.spawn.clone()).collect();

        screen
            .lines()
            .map(|line| line.chars().skip(INDENT).collect::<String>())
            .filter(|name| named.contains(name))
            .collect()
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
            rows: vec![for_minutes(
                said("an-extremely-long-spawn-name-indeed", Status::Working, None),
                31,
            )],
        };

        let screen = drawn(24, 10, &entries, &snapshot, &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

a-repository-with-a-v… ·
 ·✻ an-extremely-long-s…


F2 starts a draft
F6 / F7 move the select…
F9 retires the spawn
F10 quits — nothing is …"
        );
    }

    #[test]
    fn a_wider_list_is_a_wider_layout_and_not_the_same_one_with_space_beside_it() {
        let entries = vec![entry("acme-api", "an-extremely-long-spawn-name-indeed")];
        let snapshot = Snapshot {
            rows: vec![for_minutes(
                said("an-extremely-long-spawn-name-indeed", Status::Working, None),
                31,
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

        a_list_of(&named, |spawn| match spawn {
            "task-03" | "chore-02" => Status::Stopped,
            "task-07" => Status::Unknown,
            _ => Status::Working,
        })
    }

    /// The entries and the snapshot for these spawns, with the statuses this
    /// rule gives them.
    fn a_list_of(
        named: &[(&str, String)],
        status: impl Fn(&str) -> Status,
    ) -> (Vec<Entry>, Snapshot) {
        let entries = named
            .iter()
            .map(|(repository, spawn)| entry(repository, spawn))
            .collect();
        // A spread of ages, so a pinned screen shows the column doing its job.
        let rows = named
            .iter()
            .enumerate()
            .map(|(at, (_, spawn))| {
                let minutes = u64::try_from(at).unwrap_or_default();

                for_minutes(said(spawn, status(spawn), None), 3 + minutes * 11)
            })
            .collect();

        (entries, Snapshot { rows })
    }

    #[test]
    fn twenty_spawns_still_read_as_two_projects() {
        let (entries, snapshot) = twenty();

        let screen = drawn(30, 29, &entries, &snapshot, &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

acme-api ●?··········
 ●✻ task-03                25m
 ?✻ task-07               1h9m
 ·✻ task-01                 3m
 ·✻ task-02                14m
 ·✻ task-04                36m
 ·✻ task-05                47m
 ·✻ task-06                58m
 ·✻ task-08              1h20m
 ·✻ task-09              1h31m
 ·✻ task-10              1h42m
 ·✻ task-11              1h53m
 ·✻ task-12               2h4m

dotfiles ●·······
 ●✻ chore-02             2h26m
 ·✻ chore-01             2h15m
 ·✻ chore-03             2h37m
 ·✻ chore-04             2h48m
 ·✻ chore-05             2h59m
 ·✻ chore-06             3h10m
 ·✻ chore-07             3h21m
 ·✻ chore-08             3h32m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    /// Where a row's status mark sits: after the gutter the selection uses.
    const MARK: u16 = 1;

    /// Where the harness glyph sits: the column after the mark.
    const HARNESS: u16 = MARK + 1;

    /// The row the keyboard is on, found by the mark in the gutter.
    fn selected_row(painted: &Buffer) -> u16 {
        (0..painted.area.height)
            .find(|row| painted[(0, *row)].symbol() == SELECTED)
            .unwrap_or_else(|| panic!("nothing in the list is selected"))
    }

    /// The five, on a terminal big enough for them, with the keyboard on one.
    fn with_the_keyboard_on(spawn: &str, retirements: &Retirements) -> Buffer {
        painted_on(30, 14, &five(), retirements, &Cursor::on_spawn(spawn))
    }

    #[test]
    fn every_row_wears_the_glyph_of_what_it_runs_under() {
        let painted = with_the_keyboard_on("add-retry-logic", &Retirements::default());

        for row in [3, 4, 5] {
            assert_eq!(
                painted[(HARNESS, row)].symbol(),
                crate::harness::GLYPH,
                "row {row} does not say what it runs under"
            );
        }
        // On an unselected row the glyph reads the same as the mark.
        assert_eq!(
            painted[(HARNESS, 3)].style().fg,
            painted[(MARK, 3)].style().fg
        );
    }

    #[test]
    fn the_selected_row_is_painted_black_on_a_band_the_width_of_the_list() {
        let painted = with_the_keyboard_on("add-retry-logic", &Retirements::default());
        let row = selected_row(&painted);

        for column in 0..painted.area.width {
            let cell = &painted[(column, row)];
            assert_eq!(
                cell.style().bg,
                Some(Color::Cyan),
                "column {column} of the row is not on the band: {cell:?}"
            );
            assert_eq!(
                cell.style().fg,
                Some(Color::Black),
                "column {column} of the row is not black on it: {cell:?}"
            );
        }
    }

    #[test]
    fn the_band_under_an_alarmed_spawn_is_amber_rather_than_cyan() {
        let mut refused = being_retired("add-retry-logic", "removing the worktree");
        refused.refused(
            "add-retry-logic",
            "there is work that is not committed".to_string(),
        );

        for (spawn, retirements) in [
            ("spawn-form-choices", Retirements::default()),
            ("add-retry-logic", refused),
        ] {
            let painted = with_the_keyboard_on(spawn, &retirements);
            let row = selected_row(&painted);

            assert_eq!(
                painted[(0, row)].style().bg,
                Some(AMBER),
                "the band under {spawn} is not the colour the app admits things in"
            );
            assert_eq!(painted[(MARK, row)].style().fg, Some(Color::Black));
        }
    }

    #[test]
    fn a_status_is_a_shape_and_a_colour_at_once() {
        let (entries, snapshot) = (five(), saying(None));
        let painted = painted(
            30,
            12,
            &[],
            &entries,
            &snapshot,
            &Retirements::default(),
            &Cursor::default(),
        );
        // The first group's rows, which are stopped, unknown and working.
        let stopped = &painted[(MARK, 3)];
        let unknown = &painted[(MARK, 4)];
        let working = &painted[(MARK, 5)];

        assert_eq!(
            [stopped.symbol(), unknown.symbol(), working.symbol()],
            ["●", "?", "·"]
        );
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

    /// Twenty spawns over four repositories, five apiece — the shape the
    /// tranche is aimed at.
    fn twenty_over_four() -> (Vec<Entry>, Snapshot) {
        let named: Vec<(&str, &str)> = vec![
            ("harness-launcher", "fix-worktree-cleanup"),
            ("harness-launcher", "control-mode-backpressure"),
            ("harness-launcher", "prune-stale-symlinks"),
            ("harness-launcher", "status-ladder-grace-period"),
            ("harness-launcher", "rotate-the-deploy-keys"),
            ("acme-api", "add-retry-logic"),
            ("acme-api", "rate-limit-headers"),
            ("acme-api", "idempotency-keys"),
            ("acme-api", "terraform-state-locking"),
            ("acme-api", "alert-on-disk-pressure"),
            ("dotfiles", "drop-legacy-auth"),
            ("dotfiles", "tidy-the-shell-prompt"),
            ("dotfiles", "font-fallback-for-emoji"),
            ("dotfiles", "cheaper-log-retention"),
            ("dotfiles", "pagination-cursors"),
            ("infra", "spawn-form-choices"),
            ("infra", "openapi-drift-check"),
            ("infra", "neovim-lsp-config"),
            ("infra", "ssh-agent-forwarding"),
            ("infra", "blue-green-cutover"),
        ];
        let named: Vec<(&str, String)> = named
            .into_iter()
            .map(|(repository, spawn)| (repository, spawn.to_string()))
            .collect();

        a_list_of(&named, |spawn| match spawn {
            "prune-stale-symlinks" | "idempotency-keys" | "neovim-lsp-config" => Status::Stopped,
            "font-fallback-for-emoji" => Status::Unknown,
            _ => Status::Working,
        })
    }

    /// Pins the whole screen as text: a list that stopped reading well would
    /// have to change this text to pass.
    #[test]
    fn twenty_spawns_over_four_repositories_read_as_four_projects() {
        let (entries, snapshot) = twenty_over_four();

        let screen = drawn(66, 33, &entries, &snapshot, &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●····
 ●✻ prune-stale-symlinks                                       25m
 ·✻ fix-worktree-cleanup                                        3m
 ·✻ control-mode-backpressure                                  14m
 ·✻ status-ladder-grace-period                                 36m
 ·✻ rotate-the-deploy-keys                                     47m

acme-api ●····
 ●✻ idempotency-keys                                         1h20m
 ·✻ add-retry-logic                                            58m
 ·✻ rate-limit-headers                                        1h9m
 ·✻ terraform-state-locking                                  1h31m
 ·✻ alert-on-disk-pressure                                   1h42m

dotfiles ?····
 ?✻ font-fallback-for-emoji                                  2h15m
 ·✻ drop-legacy-auth                                         1h53m
 ·✻ tidy-the-shell-prompt                                     2h4m
 ·✻ cheaper-log-retention                                    2h26m
 ·✻ pagination-cursors                                       2h37m

infra ●····
 ●✻ neovim-lsp-config                                        3h10m
 ·✻ spawn-form-choices                                       2h48m
 ·✻ openapi-drift-check                                      2h59m
 ·✻ ssh-agent-forwarding                                     3h21m
 ·✻ blue-green-cutover                                       3h32m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    #[test]
    fn an_age_is_written_in_minutes_until_it_is_written_in_hours_too() {
        assert_eq!(written_as(Duration::from_secs(0)), "0m");
        assert_eq!(written_as(Duration::from_secs(59)), "0m");
        assert_eq!(written_as(Duration::from_mins(4)), "4m");
        assert_eq!(written_as(Duration::from_mins(64)), "1h4m");
        assert_eq!(written_as(Duration::from_mins(121)), "2h1m");
        assert_eq!(written_as(Duration::from_hours(48)), "48h0m");
    }

    /// Pins acceptance criterion 3: the narrowest list on which every row
    /// shows its age is the one that still writes every name in full.
    #[test]
    fn every_row_gets_the_same_age_column_however_wide_its_own_age_is() {
        let screen = drawn(29, 14, &five(), &saying(None), &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ●✻ fix-worktree-cleanup  31m
 ?✻ spawn-form-choices   1h4m
 ·✻ add-retry-logic        4m

acme-api ●·
 ●✻ drop-legacy-auth       7m
 ·✻ rate-limit-headers   2h1m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    /// Pins acceptance criterion 3: the list gives an age up before it gives a
    /// name up, and it gives up only the age of the row that is short of room.
    /// One cell narrower than the list above, and the twenty-character name
    /// drops its own age. The four rows beside it keep theirs.
    #[test]
    fn a_row_too_long_for_its_age_drops_it_and_the_rows_beside_it_keep_theirs() {
        let screen = drawn(28, 14, &five(), &saying(None), &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●?·
 ●✻ fix-worktree-cleanup
 ?✻ spawn-form-choices  1h4m
 ·✻ add-retry-logic       4m

acme-api ●·
 ●✻ drop-legacy-auth      7m
 ·✻ rate-limit-headers  2h1m
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is kill…"
        );
    }

    /// Whether a rendered line ends in an age rather than in a name. No spawn
    /// or repository in these tests is named for a number.
    fn ends_in_an_age(line: &str) -> bool {
        line.rsplit(' ').next().is_some_and(|last| {
            last.ends_with('m') && last.starts_with(|first: char| first.is_ascii_digit())
        })
    }

    /// Every row that shows an age cuts its name at the same place, so the
    /// ages share a right edge. That is what the column is for.
    #[test]
    fn the_rows_that_show_an_age_line_them_up_against_one_right_edge() {
        for width in 20..40u16 {
            let screen = drawn(width, 14, &five(), &saying(None), &Cursor::default());

            for line in screen.lines().filter(|line| ends_in_an_age(line)) {
                assert_eq!(
                    line.chars().count(),
                    usize::from(width),
                    "at {width} columns an age sits off the right edge:\n{screen}"
                );
            }
        }
    }

    /// Names come first at every width. A row that shows an age shows its
    /// whole name beside it, so no name is ever cut to make room for an age.
    #[test]
    fn no_name_is_cut_to_make_room_for_an_age() {
        for width in 10..60u16 {
            let screen = drawn(width, 14, &five(), &saying(None), &Cursor::default());

            for line in screen.lines().filter(|line| ends_in_an_age(line)) {
                assert!(
                    !line.contains(ELLIPSIS),
                    "at {width} columns a name was cut for an age:\n{screen}"
                );
            }
        }
    }

    /// A row with no age is not measured for the age column. It keeps the
    /// whole room for its name, long enough to be cut, and the row beside it
    /// keeps its own age.
    #[test]
    fn a_row_with_no_age_does_not_take_the_column_from_the_rows_that_have_one() {
        let entries = vec![
            entry("acme-api", "api-retry"),
            entry("acme-api", "fix-the-worktree-cleanup-a7f3"),
        ];
        let snapshot = Snapshot {
            rows: vec![for_minutes(said("api-retry", Status::Working, None), 31)],
        };

        let screen = drawn(28, 10, &entries, &snapshot, &Cursor::default());

        let aged = screen
            .lines()
            .find(|line| line.contains("api-retry"))
            .unwrap_or_else(|| panic!("the aged spawn has no row:\n{screen}"));
        assert_eq!(aged, " ·✻ api-retry            31m");
        let ageless = screen
            .lines()
            .find(|line| line.contains("fix-the-worktree"))
            .unwrap_or_else(|| panic!("the spawn with no age has no row:\n{screen}"));
        assert_eq!(ageless, "  ✻ fix-the-worktree-cleanu…");
    }

    #[test]
    fn a_long_draft_title_keeps_the_room_the_age_column_takes_from_the_names() {
        let drafts = drafting(&["fix the worktree cleanup so retiring refuses"]);

        let screen = with_drafts(
            29,
            16,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        assert!(
            screen.contains(" +  fix the worktree cleanup…"),
            "a draft's title was cut for an age the draft does not have:\n{screen}"
        );
        assert!(
            screen.contains("31m"),
            "a draft's title dragged the age column off the spawns:\n{screen}"
        );
    }

    #[test]
    fn a_draft_has_no_age_because_it_has_no_status() {
        let drafts = drafting(&["fix the worktree cleanup"]);

        let screen = with_drafts(
            30,
            15,
            drafts.all(),
            &five(),
            &saying(None),
            &Cursor::default(),
        );

        let drafted = screen
            .lines()
            .find(|line| line.contains("fix the worktree cleanup"))
            .unwrap_or_else(|| panic!("the draft has no row:\n{screen}"));
        assert_eq!(drafted, " +  fix the worktree cleanup");
    }

    #[test]
    fn a_spawn_the_app_has_heard_nothing_about_shows_no_age() {
        let entries = five();
        let snapshot = Snapshot {
            rows: vec![for_minutes(
                said("add-retry-logic", Status::Working, None),
                4,
            )],
        };

        let screen = drawn(30, 16, &entries, &snapshot, &Cursor::default());

        let heard_nothing = screen
            .lines()
            .find(|line| line.contains("fix-worktree-cleanup"))
            .unwrap_or_else(|| panic!("the spawn has no row:\n{screen}"));
        assert_eq!(heard_nothing, "  ✻ fix-worktree-cleanup");
    }

    /// A spawn an earlier run left running has no age: the app was not
    /// watching when its status began, and `0m` would be a confident lie. It
    /// keeps the whole width for its name, like a spawn with no row at all.
    #[test]
    fn a_spawn_adopted_from_an_earlier_run_shows_no_age_at_all() {
        let entries = vec![entry("harness-launcher", "add-retry-logic-a7f3")];
        let snapshot = Snapshot {
            rows: vec![Row {
                age: None,
                ..said("add-retry-logic-a7f3", Status::Unknown, None)
            }],
        };

        let screen = drawn(30, 10, &entries, &snapshot, &Cursor::default());

        let row = screen
            .lines()
            .find(|line| line.contains("add-retry-logic-a7f3"))
            .unwrap_or_else(|| panic!("the spawn has no row:\n{screen}"));
        assert_eq!(row, " ?✻ add-retry-logic-a7f3");
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
            screen.contains("▍·✻ chore-08"),
            "the selection moved somewhere the screen does not reach:\n{screen}"
        );
    }

    #[test]
    fn a_list_that_fits_is_not_scrolled_at_all() {
        let screen = drawn(
            30,
            16,
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
