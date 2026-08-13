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
//! line per spawn the density depends on.
//!
//! **One line per entry, whether or not it is selected**, and no exceptions: at
//! fifteen or twenty spawns a row that grew when you stood on it would push the
//! others off the screen to say what the slot beside it is already saying. Prose
//! is never the list's — the sentence explaining a spawn the app cannot account
//! for, and the one a retirement carries, are both drawn in the slot, over the
//! top of a spawn that is either unreachable or being stopped.
//!
//! **The row the keyboard is on is painted** rather than only marked, because a
//! one-character mark is a thing to hunt for in a list this long. What the band
//! says is *where the keyboard is*, so it spans the whole width of the list —
//! including the gutter, which gives up its own colour to it.
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
use crate::harness::GLYPH;
use crate::retirement::{Retirement, Retirements};
use crate::scaffolding::{AMBER, DIM, ELLIPSIS, HEADING, band, gutter};
use crate::scaffolding::{Footer, elided, scroll_offset};
use crate::snapshot::{Row, Snapshot, Status};

/// The mark against a draft being written.
///
/// Not one of the status marks, and deliberately unlike them: a draft has no
/// agent, so nothing about it is working, stopped or unaccounted for. It reads
/// as the one thing it is — something being made.
const DRAFT: &str = "+";

/// The mark against a draft that is being made into a spawn.
const STARTING: &str = ">";

/// The mark against a draft that was started and could not be, and against a
/// spawn that would not retire.
///
/// One mark for both because they are one thing from where the user sits: the
/// app saying *this one is on you*, in the same amber it admits everything else
/// in.
const STOPPED: &str = "!";

/// The mark against a spawn being retired.
///
/// Not one of the status marks either. A spawn being retired is not doing
/// anything — what its agent was up to stopped being the question the moment
/// somebody said they were done with it.
const RETIRING: &str = "-";

/// How far a row's name sits from the left: the selection's gutter, the status
/// mark, the column saying what the row runs under, and the space between that
/// and the name.
const INDENT: usize = 4;

/// What a row that runs under no harness puts in the harness's column.
///
/// A draft is not running anything yet — that is what makes it a draft — so it
/// says nothing there rather than claiming a session it has not started. It
/// still spends the column, for the same reason a spawn the app has heard
/// nothing about spends its status column: the names in one list line up, and a
/// draft becoming a spawn must not shift its own row sideways.
const NOTHING_YET: &str = " ";

/// What the foot of the list says the keyboard does.
///
/// Four keys and a promise. The promise is the one the whole design rests on
/// and the one nobody would guess: leaving does not stop anything. Starting a
/// draft is on the list rather than anywhere else because a draft that does not
/// exist yet has nowhere else to be announced, and retiring is here because the
/// list is where a spawn is chosen — it acts on the row rather than on what is
/// in the slot.
/// The shortest list that can still spare four of its rows for the footer is
/// eight: four rows of spawns is the least that reads as a list at all.
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
    /// Which of them are being retired, and what came of it.
    retirements: &'a Retirements,
    /// Which row the list is on.
    cursor: &'a Cursor,
}

impl<'a> Listing<'a> {
    /// What the supervisor last said, for whatever else on the screen needs it.
    ///
    /// **One frame, one snapshot.** The list draws every row from it and the
    /// slot draws the sentence about a spawn it cannot account for from the same
    /// one — so the drawing is handed the listing and asks it, rather than being
    /// handed the listing *and* the snapshot it was already made from. Two
    /// parameters carrying one fact are two chances to draw one frame out of two
    /// different moments.
    pub fn snapshot(&self) -> &'a Snapshot {
        self.snapshot
    }

    /// Where a spawn's retirement has got to, for whatever else on the screen
    /// needs it.
    ///
    /// Here for the reason [`Listing::snapshot`] is, and it is the same reason:
    /// the row says a retirement is happening and the slot says what it is
    /// doing, and one frame drawn from two moments would have them disagree.
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

        for draft in self.drafts {
            let on_it = self.cursor.draft() == Some(draft.id());
            if on_it {
                selected = Some((lines.len(), lines.len()));
            }
            lines.push(drafted(draft, on_it, width));
        }

        for (at, group) in grouped(self.entries, self.snapshot, self.retirements)
            .iter()
            .enumerate()
        {
            if at > 0 || !self.drafts.is_empty() {
                lines.push(Line::raw(""));
            }
            lines.push(group.header(width));

            for placed in &group.spawns {
                let on_it = self.cursor.spawn() == Some(placed.name());
                if on_it {
                    selected = Some((lines.len(), lines.len()));
                }
                lines.push(placed.row(on_it, width));
            }
        }

        Rows { lines, selected }
    }
}

/// The list's lines, and which of them the selected row takes up.
struct Rows {
    /// Every line there is, whether or not the region can hold them all.
    lines: Vec<Line<'static>>,
    /// The first and last line the selected row occupies, if one is selected —
    /// the same line twice, now that every row is one line. The pair is what
    /// [`scroll_offset`] takes, because the form beside the list has controls
    /// that are taller than a line.
    selected: Option<(usize, usize)>,
}

/// A draft's row: the gutter, its mark, and what it is called so far.
///
/// One line and nothing under it, selected or not. What it does have is in the
/// slot beside it the moment it is selected.
///
/// **The mark is the one thing about it that moves**, and it moves through the
/// three things a draft can be: being written, being made into a spawn, and
/// stopped without becoming one. That is progress at the granularity a row has —
/// *which* step a creation has reached is a sentence and lives in the form —
/// and it is here because the list is the only place a draft appears at all.
/// Somebody who started one and went off to answer a spawn would otherwise have
/// nothing telling them it had finished, or stopped.
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

    lined(&shown, NOTHING_YET, &draft.title(), on_it, width)
}

/// One row of the list, whatever it stands for: the gutter, the mark, the column
/// saying what it runs under, and what it is called.
///
/// **Padded out to the full width of the list**, which is what makes the band
/// under a selected row a band: a line that stopped at the end of its name would
/// paint the name rather than the row, and read as a highlight on a word.
///
/// One builder for a draft's row and a spawn's, because the two are one shape
/// with different things in the columns — written twice, they would be two
/// places for the list's own geometry to be decided, and a name that lined up in
/// one and not the other.
fn lined(
    shown: &Shown,
    runs_under: &'static str,
    name: &str,
    on_it: bool,
    width: usize,
) -> Line<'static> {
    let how_it_reads = shown.reading(on_it);
    let room = width.saturating_sub(INDENT);

    Line::from(vec![
        gutter(on_it, how_it_reads),
        Span::styled(shown.mark, how_it_reads),
        Span::styled(runs_under, how_it_reads),
        Span::styled(format!(" {:<room$}", elided(name, room)), how_it_reads),
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
///
/// **Nothing about a retirement moves a row**, which is why this asks about
/// none: the selection is on the spawn being retired, and a row that re-sorted
/// as its retirement started would take itself out from under the hand that
/// asked for it.
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

    /// How it shows: what is happening to it if anything is, and what its agent
    /// is doing otherwise.
    ///
    /// **A retirement outranks a status**, because it outranks it in what the
    /// user is being told: a spawn somebody has said they are done with is not
    /// a spawn to go and look at, whatever its agent was in the middle of.
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

    /// Its own line: the gutter, its status, the glyph saying what it runs
    /// under, and its name.
    ///
    /// **The branch and the worktree are not on it, and nowhere else on this
    /// screen either.** Both are the spawn's own name under something fixed — a
    /// branch prefix, one worktree root — so a row carrying them would spend
    /// lines restating the one thing it already says. They are wanted once,
    /// when somebody goes to find the work, and **the name is what they go back
    /// with**: both are derivable from it, and the row keeps it.
    fn row(&self, selected: bool, width: usize) -> Line<'static> {
        lined(&self.shown(), GLYPH, self.name(), selected, width)
    }
}

/// The spawns, under their repositories, attention-first.
///
/// Repositories keep the order their first spawn was started in, and equal
/// statuses keep it too — the sort is stable, and that is load-bearing. A list
/// that reshuffled its working spawns every tick would be unreadable, and the
/// only thing that ever moves a row is a status actually changing.
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

    /// How the whole row reads, which is not the same thing when the keyboard is
    /// on it: a selected row is painted, and everything on it goes black on the
    /// band.
    ///
    /// **The status colour is not carried onto the band.** The marks tell every
    /// state apart without any colour at all — that is what they are for — and
    /// the one state where the colour was doing work, a spawn the app is
    /// admitting something about, is the state the band takes its own colour
    /// from.
    fn reading(&self, selected: bool) -> Style {
        if selected {
            band(self.alarmed())
        } else {
            self.how_it_reads
        }
    }

    /// Whether this is a row the app is admitting something about.
    ///
    /// **Read off the amber rather than listed again**, and the rule is exactly
    /// that: a row is alarmed when it is already being drawn in [`AMBER`].
    /// Amber is the colour the app admits things in and nothing else uses it, so
    /// the states drawn in it *are* the states being admitted, however many of
    /// them there come to be. Naming them here would be a second answer to a
    /// settled question, and the two would come apart the first time a state was
    /// added to one of them — as they would already, since a draft that stopped
    /// without becoming a spawn is drawn in amber too and reaches this by the
    /// same road.
    fn alarmed(&self) -> bool {
        self.how_it_reads.fg == Some(AMBER)
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
///
/// *The one exception, and it is the selected row's band.* That paints its own
/// background, so what goes on top of it is arithmetic rather than a guess about
/// a theme — black on cyan or on amber reads the same on every terminal there
/// is. The rule this leaves behind is narrower than "the app names no colours"
/// and is the one actually being kept: **the app names a foreground only where
/// it has painted the background under it.** Everywhere else it puts a colour on
/// a background it cannot see and has to leave it alone.
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
            unaccounted: reason.map(|why| snapshot::cannot_account(why, None)),
            last_known: snapshot::last_read(status),
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

    /// What a buffer says, as the text a test can be written against.
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

    /// The same, with some of the spawns being retired and nothing being
    /// drafted — the cells, for the tests that ask what colour a row came out.
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
 ●✻ fix-worktree-cleanup
 ?✻ spawn-form-choices
 ·✻ add-retry-logic

acme-api ●·
 ●✻ drop-legacy-auth
 ·✻ rate-limit-headers
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    /// **One line per spawn, the selected one included.** The row the keyboard
    /// is on says no more than any other; what it has to say beyond its name is
    /// in the slot beside it, which is where somebody who selected it is looking.
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
 ●✻ fix-worktree-cleanup
 ?✻ spawn-form-choices
▍·✻ add-retry-logic

acme-api ●·
 ●✻ drop-legacy-auth
 ·✻ rate-limit-headers
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    /// The sentence is the slot's, and only the slot's — it is drawn there over
    /// the top of the spawn's own screen, and a second copy in the list would be
    /// the same words in two places at once for a row that has one line.
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

    /// Both are the spawn's name under something fixed — a prefix and a root —
    /// so a row carrying them would spend two lines restating the one it has.
    /// What the row still says is that name, which is what somebody going to
    /// find the work goes back with.
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

    /// The rule stated as a test: a draft is a row of its own, above every
    /// repository — which is where something half-written has to be, because
    /// nothing else in the app will remind you of it.
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
 ●✻ fix-worktree-cleanup
 ?✻ spawn-form-choices
 ·✻ add-retry-logic

acme-api ●·
 ●✻ drop-legacy-auth
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

    /// The row is where a draft says which of the three things it is, because
    /// the list is the only place a draft appears at all — somebody who started
    /// one and went off to answer a spawn is looking at the list, not at the
    /// form beside it.
    #[test]
    fn a_drafts_row_says_whether_it_is_being_written_started_or_stopped() {
        let mut drafts = drafting(&["being written", "being started", "stopped"]);
        let started = drafts.all()[1].id();
        let stopped = drafts.all()[2].id();
        // The one being started says enough to be started; the one that stopped
        // never said which repository, which is a refusal in place.
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

        // The draft's own row is the first under the heading and the blank
        // beneath it, and the mark sits after the selection's gutter.
        assert_eq!(painted[(MARK, 2)].symbol(), "!");
        assert_eq!(painted[(MARK, 2)].style().fg, Some(AMBER));
    }

    // A spawn being retired, which is the one thing about a row that is neither
    // its status nor its name.

    /// A retirement of this spawn, at whatever step it has reached.
    fn being_retired(spawn: &str, step: &str) -> Retirements {
        let mut retirements = Retirements::default();
        retirements.asked_for(spawn);
        retirements.doing(spawn, step.to_string());

        retirements
    }

    /// The mark is the row's whole share of a retirement. What is happening to
    /// the spawn is a sentence, and a sentence is the slot's — the spawn is
    /// being stopped, so drawing over its screen costs nothing.
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

    /// A refusal is the one thing here somebody has to act on, so it reads the
    /// way everything else the app cannot do reads: the amber mark, on the row,
    /// from wherever in the list you are looking.
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
        // Painted with the keyboard elsewhere: the amber is the row's own, and
        // has to be there whether or not somebody is standing on it.
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
        // The row a refusal is on: the heading, the blank under it, the group's
        // header, and then the two spawns the attention-first order puts first.
        assert_eq!(painted[(MARK, 5)].symbol(), "!");
        assert_eq!(painted[(MARK, 5)].style().fg, Some(AMBER));
    }

    /// A retirement does not move a row. The selection is on the spawn being
    /// retired, and a list that re-sorted under it would take the row out from
    /// under the hand that asked for it.
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

    /// The spawns a screen has a row for, in the order it drew them — the rows
    /// themselves, not the detail under the selected one nor the footer.
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
            rows: vec![said(
                "an-extremely-long-spawn-name-indeed",
                Status::Working,
                None,
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

        a_list_of(&named, |spawn| match spawn {
            "task-03" | "chore-02" => Status::Stopped,
            "task-07" => Status::Unknown,
            _ => Status::Working,
        })
    }

    /// The entries and the snapshot for these spawns, with the statuses this
    /// rule gives them.
    ///
    /// **Two lists of spawns are two sets of data, not two shapes.** What a list
    /// is made *of* — an entry apiece, a row apiece, a status picked by name — is
    /// the same however many repositories they are spread over, so it is written
    /// once. A second copy of it is a second place for the list's own vocabulary
    /// to drift, in the tests whose whole job is to notice that it has.
    fn a_list_of(
        named: &[(&str, String)],
        status: impl Fn(&str) -> Status,
    ) -> (Vec<Entry>, Snapshot) {
        let entries = named
            .iter()
            .map(|(repository, spawn)| entry(repository, spawn))
            .collect();
        let rows = named
            .iter()
            .map(|(_, spawn)| said(spawn, status(spawn), None))
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
 ●✻ task-03
 ?✻ task-07
 ·✻ task-01
 ·✻ task-02
 ·✻ task-04
 ·✻ task-05
 ·✻ task-06
 ·✻ task-08
 ·✻ task-09
 ·✻ task-10
 ·✻ task-11
 ·✻ task-12

dotfiles ●·······
 ●✻ chore-02
 ·✻ chore-01
 ·✻ chore-03
 ·✻ chore-04
 ·✻ chore-05
 ·✻ chore-06
 ·✻ chore-07
 ·✻ chore-08
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
    }

    /// Where a row's status mark sits: after the gutter the selection uses.
    const MARK: u16 = 1;

    /// Where the glyph saying which harness a spawn runs sits: the column after
    /// the mark, and the one before the space that already separates the name.
    const HARNESS: u16 = MARK + 1;

    /// The row the keyboard is on, found the way a reader finds it — by the
    /// mark in the gutter.
    fn selected_row(painted: &Buffer) -> u16 {
        (0..painted.area.height)
            .find(|row| painted[(0, *row)].symbol() == SELECTED)
            .unwrap_or_else(|| panic!("nothing in the list is selected"))
    }

    /// The five, on a terminal big enough for them, with the keyboard on one.
    fn with_the_keyboard_on(spawn: &str, retirements: &Retirements) -> Buffer {
        painted_on(30, 14, &five(), retirements, &Cursor::on_spawn(spawn))
    }

    /// A spawn runs under a harness, and which one is a distinction that starts
    /// mattering the moment there is more than one. It reads as part of the row
    /// rather than as a thing of its own: the same colour as the mark before it
    /// and the name after it.
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
        // A working spawn nobody is standing on: mark, glyph and name are all
        // the one way of reading.
        assert_eq!(
            painted[(HARNESS, 3)].style().fg,
            painted[(MARK, 3)].style().fg
        );
    }

    /// **The band is what says where the keyboard is.** A mark one character
    /// wide in the gutter is a thing to hunt for in twenty rows, so the row is
    /// painted instead — the width of the list rather than the width of the
    /// name, or it would read as a highlight on a word.
    ///
    /// The gutter takes the band as well: its own cyan on a cyan band is the one
    /// cell of the row that would disappear.
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

    /// The band follows the amber wherever it is, rather than listing the states
    /// that have it: two of them here, an unknown spawn and a refused
    /// retirement, and the rule is what is being checked rather than the count.
    /// A spawn's status colour is otherwise not carried onto the band — the
    /// marks tell every state apart without it, and an alarmed spawn's slot is
    /// saying so in words at the same time.
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

    /// Twenty spawns over four repositories, five apiece — the shape the
    /// tranche is aimed at, and the one that was actually run.
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

    /// **What twenty spawns over four repositories look like**, at the width the
    /// list has on a wide terminal — which is the thing the tranche's headline
    /// claim is about, and the one thing about it no assertion can settle.
    ///
    /// What an assertion *can* settle is that the density holds: one line per
    /// spawn, four groups that are still four groups, each header carrying its
    /// own bar, and the three statuses telling themselves apart by shape before
    /// any colour is involved. That is what this pins, and it pins it by writing
    /// the screen out — a list that stopped reading well would have to change
    /// this text to pass, which is the point.
    #[test]
    fn twenty_spawns_over_four_repositories_read_as_four_projects() {
        let (entries, snapshot) = twenty_over_four();

        let screen = drawn(66, 33, &entries, &snapshot, &Cursor::default());

        assert_eq!(
            screen,
            "\
SPAWNS

harness-launcher ●····
 ●✻ prune-stale-symlinks
 ·✻ fix-worktree-cleanup
 ·✻ control-mode-backpressure
 ·✻ status-ladder-grace-period
 ·✻ rotate-the-deploy-keys

acme-api ●····
 ●✻ idempotency-keys
 ·✻ add-retry-logic
 ·✻ rate-limit-headers
 ·✻ terraform-state-locking
 ·✻ alert-on-disk-pressure

dotfiles ?····
 ?✻ font-fallback-for-emoji
 ·✻ drop-legacy-auth
 ·✻ tidy-the-shell-prompt
 ·✻ cheaper-log-retention
 ·✻ pagination-cursors

infra ●····
 ●✻ neovim-lsp-config
 ·✻ spawn-form-choices
 ·✻ openapi-drift-check
 ·✻ ssh-agent-forwarding
 ·✻ blue-green-cutover
F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed"
        );
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
