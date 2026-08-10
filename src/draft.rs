//! A draft: a spawn somebody is still writing.
//!
//! **A draft is state in the app.** Not a pane, not a process, not a second mode
//! of this binary and nothing on disk — a record in a list, edited by the
//! keyboard and drawn into the slot like anything else. That is what makes
//! several of them free: a `Vec` of them costs what a `Vec` costs.
//!
//! **Composing is not a modal.** A draft takes the slot when it is selected and
//! sits in the list when it is not, so you can walk away from a half-written
//! paragraph, deal with a spawn that stopped, and come back to it exactly as you
//! left it — the text, the caret, and which field the keyboard was in. Nothing
//! about it hides the list, which is the rule the whole product rests on.
//!
//! **The form never learns what any of the choices mean.** It is handed titled
//! lists of labels by the harness and asks *"which of these?"*; it does not know
//! that one of them is about models, what an id says, or how many lists there
//! are. A list that comes back with nothing in it is not a control drawn empty —
//! it is a control that does not exist, which is what lets a harness offering no
//! such choice be a harness rather than a special case.
//!
//! **What is not here is creation.** A draft makes no worktree, no branch and no
//! process; turning one into a spawn is its own piece of work. This is the form,
//! and the text in it.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::harness::{Choice, Choices};
use crate::scaffolding::{self, Footer, scroll_offset};
use crate::screen::Size;

/// What the form calls itself, above everything it asks.
const TITLE: &str = "NEW SPAWN";

/// What the list calls a draft nothing has been typed into yet.
const UNTITLED: &str = "a new spawn";

/// What the heading over the repository field says.
const REPOSITORY: &str = "Repository";

/// What the heading over the description says.
const WORK: &str = "Work";

/// How far a control's body sits from the left: one cell for the gutter the
/// keyboard's mark goes in, and one more so the text sits inside its heading.
const INDENT: usize = 2;

/// The mark against the option a list has settled on.
///
/// A shape rather than a colour alone, for the reason the list's statuses are
/// one: a form read without colour still has to say which option it will use.
const PICKED: &str = "› ";

/// What the foot of the form says the keyboard does.
///
/// Both halves are worth saying. `Tab` is the only way around the form and
/// nothing on screen implies it; and that the list's own keys still work is the
/// promise the product rests on, where a full-height form is exactly the place
/// somebody would assume they had been taken somewhere modal.
///
/// The shortest form that can still spare it two rows is eight: below that
/// there is not room for a heading, a field and a list of choices, which is the
/// least of the form worth showing.
const HINT: Footer = Footer::new(
    &[
        "Tab moves between fields",
        "F6 / F7 leave it — nothing is lost",
    ],
    8,
);

/// Which draft.
///
/// A count rather than a name: a draft has nothing on disk to identify itself
/// by, and a name taken from what has been typed would change under the
/// selection as it was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Id(u64);

/// What the keyboard asked of a draft.
///
/// Which key is which is settled where every other key is ([`crate::app`]);
/// what a key *means* depends on the control the keyboard is in, and that is
/// settled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// A character, at the caret.
    Typed(char),
    /// Rub out the character before the caret.
    Erased,
    /// Rub out the character after it.
    Deleted,
    /// The caret one character back.
    Left,
    /// The caret one character on.
    Right,
    /// The caret to the front of the text.
    Start,
    /// The caret to the end of it.
    End,
    /// Up a list of choices.
    Up,
    /// Down it.
    Down,
    /// On to the next control.
    Next,
    /// Back to the one before it.
    Previous,
    /// Whatever the control the keyboard is in makes of `Enter`.
    Entered,
}

/// Every draft in flight, and where the next one's identity comes from.
pub struct Drafts {
    /// What the harness offers. Asked once: the lists do not change between
    /// drafts, and asking per draft would invite two forms in one session
    /// offering different things.
    choices: Vec<Choices>,
    /// Every draft there is, in the order they were started.
    all: Vec<Draft>,
    /// How many have ever been started. It only counts up, so an identity is
    /// never handed out twice — including to a draft that replaces one thrown
    /// away.
    started: u64,
}

impl Drafts {
    /// No drafts yet, and the choices the ones to come will offer.
    pub fn new(choices: Vec<Choices>) -> Self {
        Self {
            choices,
            all: Vec::new(),
            started: 0,
        }
    }

    /// Every draft, in the order they were started.
    pub fn all(&self) -> &[Draft] {
        &self.all
    }

    /// Start one, and say which it is, so the selection can land on it.
    pub fn start(&mut self) -> Id {
        let id = Id(self.started);
        self.started += 1;
        self.all.push(Draft::new(id, &self.choices));

        id
    }

    /// The draft with this identity, if it is still here.
    pub fn of(&self, id: Id) -> Option<&Draft> {
        self.all.iter().find(|draft| draft.id == id)
    }

    /// Do to a draft what the keyboard asked.
    ///
    /// A draft that is not here is not an error: what the selection is on and
    /// what drafts exist are settled separately, so a keystroke aimed at one
    /// that has gone should do nothing rather than something.
    pub fn edit(&mut self, id: Id, edit: Edit) {
        if let Some(draft) = self.all.iter_mut().find(|draft| draft.id == id) {
            draft.edited(edit);
        }
    }
}

/// A half-written spawn: a repository, what it should do, and one answer per
/// list of choices the harness offers.
pub struct Draft {
    /// Which draft this is.
    id: Id,
    /// The repository it would be started against.
    repository: Text,
    /// What it would be asked to do.
    work: Text,
    /// One picked option per list the harness had something to offer in.
    choices: Vec<Picked>,
    /// Which control the keyboard is in, counted the way [`control`] counts.
    on: usize,
}

impl Draft {
    /// A blank draft offering these choices.
    ///
    /// **A list with nothing in it is dropped here**, so an empty control cannot
    /// be drawn, cannot be reached by the keyboard, and does not exist to be got
    /// wrong later. Leaving it out only at the point of drawing would keep a
    /// control the form never shows.
    fn new(id: Id, choices: &[Choices]) -> Self {
        Self {
            id,
            repository: Text::line(),
            work: Text::paragraph(),
            choices: choices.iter().filter_map(Picked::of).collect(),
            on: 0,
        }
    }

    /// Which draft this is.
    pub fn id(&self) -> Id {
        self.id
    }

    /// What the list calls it: the first thing said about the work, or a
    /// standing name until something is.
    ///
    /// The first line rather than the whole of it, because the work is a
    /// paragraph and the list is one line per row. A draft with nothing in it is
    /// still a row somebody has to be able to find their way back to.
    pub fn title(&self) -> String {
        self.work
            .text()
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or(UNTITLED)
            .to_string()
    }

    /// Do what the keyboard asked.
    fn edited(&mut self, edit: Edit) {
        match edit {
            Edit::Next => self.on = (self.on + 1) % self.controls(),
            Edit::Previous => self.on = (self.on + self.controls() - 1) % self.controls(),
            edit => {
                let finished = match control(self.on) {
                    Control::Repository => typed(&mut self.repository, edit),
                    Control::Work => typed(&mut self.work, edit),
                    Control::Choice(which) => self
                        .choices
                        .get_mut(which)
                        .is_some_and(|picked| picked.edited(edit)),
                };

                if finished {
                    self.edited(Edit::Next);
                }
            }
        }
    }

    /// How many controls there are: the two fields, and one per list of choices
    /// the harness had something to offer in.
    fn controls(&self) -> usize {
        2 + self.choices.len()
    }

    /// The form, laid out for a region of this shape.
    ///
    /// Laid out once and rendered from, rather than drawn straight into a
    /// buffer, because where the caret goes falls out of the same arithmetic
    /// that wraps the text — and a caret worked out a second way would sit one
    /// cell from the character it belongs to on exactly the lines that wrapped.
    pub fn form(&self, region: Size) -> Form {
        let hint = HINT.rows(region.rows);
        let height = usize::from(region.rows.saturating_sub(hint));
        let width = usize::from(region.columns).saturating_sub(INDENT);

        let mut lines = vec![Line::styled(TITLE, scaffolding::HEADING), Line::raw("")];
        let mut caret = None;
        let mut showing = None;

        for at in 0..self.controls() {
            let on_it = at == self.on;
            let from = lines.len();

            match control(at) {
                Control::Repository => {
                    if let Some(at) = field(&mut lines, on_it, REPOSITORY, &self.repository, width)
                    {
                        caret = Some(at);
                    }
                }
                Control::Work => {
                    if let Some(at) = field(&mut lines, on_it, WORK, &self.work, width) {
                        caret = Some(at);
                    }
                }
                Control::Choice(which) => {
                    if let Some(picked) = self.choices.get(which) {
                        lines.push(heading(on_it, picked.title));
                        lines.extend(picked.options());
                    }
                }
            }

            if on_it {
                showing = Some((from, lines.len() - 1));
            }
            lines.push(Line::raw(""));
        }

        let scroll = scroll_offset(showing, height);

        Form {
            lines,
            scroll,
            hint,
            caret: caret.and_then(|(column, row)| on_region(column, row, scroll, region, height)),
        }
    }
}

/// One text field: its heading, however many lines its text wrapped onto, and —
/// when this is the field the keyboard is in — where the caret goes in the form.
fn field(
    lines: &mut Vec<Line<'static>>,
    on_it: bool,
    title: &str,
    text: &Text,
    width: usize,
) -> Option<(u16, u16)> {
    lines.push(heading(on_it, title));

    let wrapped = text.wrapped(width);
    let row = lines.len() + wrapped.caret.0;
    lines.extend(
        wrapped
            .lines
            .into_iter()
            .map(|line| indented(&line, Style::new())),
    );

    if !on_it {
        return None;
    }

    Some((
        u16::try_from(INDENT + wrapped.caret.1).unwrap_or(u16::MAX),
        u16::try_from(row).unwrap_or(u16::MAX),
    ))
}

/// Where the caret lands on the region, once the form has been scrolled under
/// it.
///
/// Nothing at all when it falls outside — a form with no room for the field the
/// keyboard is in should have no caret rather than one parked at the edge,
/// saying what you type will land somewhere it will not.
fn on_region(
    column: u16,
    row: u16,
    scroll: u16,
    region: Size,
    height: usize,
) -> Option<(u16, u16)> {
    let row = row.checked_sub(scroll)?;

    (column < region.columns && usize::from(row) < height).then_some((column, row))
}

/// Do to a text field what the keyboard asked, and say whether it was being
/// finished with.
///
/// `Enter` is the one key that means two things, and the field settles which by
/// what it holds: a description is a paragraph and takes the line break, while a
/// path has no lines in it, so there `Enter` is *done with this field*.
fn typed(field: &mut Text, edit: Edit) -> bool {
    match edit {
        Edit::Typed(character) => field.typed_in(character),
        Edit::Erased => field.erased(),
        Edit::Deleted => field.deleted(),
        Edit::Left => field.left(),
        Edit::Right => field.right(),
        Edit::Start => field.start(),
        Edit::End => field.end(),
        Edit::Entered if field.takes_lines => field.typed_in('\n'),
        Edit::Entered => return true,
        Edit::Up | Edit::Down | Edit::Next | Edit::Previous => {}
    }

    false
}

/// Which control is the one at this position: the two fields first, and then
/// the harness's lists in the order it offered them.
fn control(at: usize) -> Control {
    match at {
        0 => Control::Repository,
        1 => Control::Work,
        other => Control::Choice(other - 2),
    }
}

/// Which control the keyboard is in.
enum Control {
    /// The repository the spawn would be started against.
    Repository,
    /// What it would be asked to do.
    Work,
    /// One of the harness's lists of choices.
    Choice(usize),
}

/// A control's heading, with the mark in the gutter when the keyboard is in it.
fn heading(on_it: bool, title: &str) -> Line<'static> {
    Line::from(vec![
        scaffolding::gutter(on_it),
        Span::styled(title.to_string(), scaffolding::HEADING),
    ])
}

/// A line of a control's body, set in under its heading.
fn indented(text: &str, how_it_reads: Style) -> Line<'static> {
    Line::styled(format!("{}{text}", " ".repeat(INDENT)), how_it_reads)
}

/// One of the harness's lists of choices, and which of them is picked.
struct Picked {
    /// What the list is called. The form draws it and knows nothing else about
    /// it.
    title: &'static str,
    /// What can be picked. Never empty — see [`Picked::of`].
    options: Vec<Choice>,
    /// Which one is picked.
    at: usize,
}

impl Picked {
    /// The list, on the harness's own default — or nothing at all, when the
    /// harness offered nothing.
    fn of(choices: &Choices) -> Option<Self> {
        if choices.options.is_empty() {
            return None;
        }

        let at = choices
            .options
            .iter()
            .position(|option| *option == choices.default)
            .unwrap_or_default();

        Some(Self {
            title: choices.title,
            options: choices.options.clone(),
            at,
        })
    }

    /// Do what the keyboard asked, and say whether the list was being finished
    /// with.
    ///
    /// Both ends stop rather than wrap, for the reason the list of spawns does:
    /// a control you can hold a place in is one whose ends you can feel.
    fn edited(&mut self, edit: Edit) -> bool {
        match edit {
            Edit::Up => self.at = self.at.saturating_sub(1),
            Edit::Down => self.at = (self.at + 1).min(self.options.len() - 1),
            Edit::Entered => return true,
            _ => {}
        }

        false
    }

    /// Every option, the picked one marked.
    fn options(&self) -> Vec<Line<'static>> {
        let unmarked = " ".repeat(PICKED.chars().count());

        self.options
            .iter()
            .enumerate()
            .map(|(at, option)| {
                let picked = at == self.at;
                let mark = if picked { PICKED } else { unmarked.as_str() };
                let how_it_reads = if picked {
                    scaffolding::HEADING
                } else {
                    Style::new()
                };

                indented(&format!("{mark}{}", option.label), how_it_reads)
            })
            .collect()
    }
}

/// The form, laid out for the region it is about to be drawn into.
pub struct Form {
    /// Everything above the hint.
    lines: Vec<Line<'static>>,
    /// How far down those lines sit, so the control the keyboard is in is on
    /// screen.
    scroll: u16,
    /// How many rows the hint takes, which is none on a region too short to
    /// spare them.
    hint: u16,
    /// Where the caret goes, when the keyboard is in something you can type
    /// into.
    caret: Option<(u16, u16)>,
}

impl Form {
    /// Where the caret goes, relative to the region's own top left.
    ///
    /// Nothing when the keyboard is in a list of choices: there is nothing to
    /// type there, and a caret parked beside a list would say there was.
    pub fn caret(&self) -> Option<(u16, u16)> {
        self.caret
    }
}

impl Widget for Form {
    /// Draw the form into its region.
    ///
    /// The hint is anchored to the bottom rather than set down after the last
    /// control, so it stays where the eye last found it however long the form
    /// is — the same as the list's own footer, and for the same reason.
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let [body, hint] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(self.hint)]).areas(area);

        Paragraph::new(self.lines)
            .scroll((self.scroll, 0))
            .render(body, buffer);
        Paragraph::new(HINT.lines(usize::from(hint.width))).render(hint, buffer);
    }
}

/// Something typed into, and where the caret is in it.
///
/// Characters rather than bytes: the caret counts characters, and an index into
/// a `String` would have to be kept on a character boundary by hand — a class of
/// panic taken on for the sake of an allocation nobody would measure.
struct Text {
    /// What has been typed.
    typed: Vec<char>,
    /// How many characters are before the caret.
    caret: usize,
    /// Whether it can hold more than one line.
    takes_lines: bool,
}

impl Text {
    /// A field holding one line, like a path.
    fn line() -> Self {
        Self {
            typed: Vec::new(),
            caret: 0,
            takes_lines: false,
        }
    }

    /// A field holding a paragraph, like an instruction.
    fn paragraph() -> Self {
        Self {
            takes_lines: true,
            ..Self::line()
        }
    }

    /// What it holds.
    fn text(&self) -> String {
        self.typed.iter().collect()
    }

    /// Take a character at the caret.
    ///
    /// Control characters are dropped: a terminal sends plenty of them that are
    /// not keys anybody meant to type, and a cell holding one draws as a hole in
    /// the middle of a sentence.
    fn typed_in(&mut self, character: char) {
        if character != '\n' && character.is_control() {
            return;
        }

        self.typed.insert(self.caret, character);
        self.caret += 1;
    }

    /// Rub out the character before the caret.
    fn erased(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.typed.remove(self.caret);
        }
    }

    /// Rub out the one after it.
    fn deleted(&mut self) {
        if self.caret < self.typed.len() {
            self.typed.remove(self.caret);
        }
    }

    /// The caret one character back, stopping at the front.
    fn left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    /// One character on, stopping at the end.
    fn right(&mut self) {
        self.caret = (self.caret + 1).min(self.typed.len());
    }

    /// The caret to the front of the text.
    fn start(&mut self) {
        self.caret = 0;
    }

    /// The caret to the end of it.
    fn end(&mut self) {
        self.caret = self.typed.len();
    }

    /// The text as it lands on a field this wide, and where the caret is in it.
    ///
    /// **A line too long is broken where it runs out of room**, rather than at
    /// the space before it. Prose reads better wrapped on its spaces — the list
    /// does exactly that with the one sentence it shows — but this text is being
    /// typed into, and the caret has to sit on the cell holding the character it
    /// is beside. A wrap that moved a word down to the next line would move the
    /// caret with it, in the middle of typing the word. *Accepted cost:* a long
    /// word is split across two lines.
    ///
    /// **A line that is exactly full rolls over before anything else happens**,
    /// including before a line break. It has no cell left for the caret to sit
    /// on, and the next character typed there would be drawn underneath it
    /// anyway — so that is where the caret goes. *Accepted cost:* a line the
    /// user ended exactly at the width is followed by a blank row, which a
    /// terminal that defers its wrap would not show.
    fn wrapped(&self, width: usize) -> Wrapped {
        let width = width.max(1);
        let mut lines = vec![String::new()];
        let mut caret = (0, 0);

        for (at, character) in self.typed.iter().enumerate() {
            if full(&lines, width) {
                lines.push(String::new());
            }
            if at == self.caret {
                caret = end_of(&lines);
            }
            if *character == '\n' {
                lines.push(String::new());
                continue;
            }

            if let Some(line) = lines.last_mut() {
                line.push(*character);
            }
        }

        if self.caret >= self.typed.len() {
            if full(&lines, width) {
                lines.push(String::new());
            }
            caret = end_of(&lines);
        }

        Wrapped { lines, caret }
    }
}

/// Whether the line being written has run out of room.
fn full(lines: &[String], width: usize) -> bool {
    lines
        .last()
        .is_some_and(|line| line.chars().count() >= width)
}

/// Where the next character would go: which line is being written, and how far
/// along it.
fn end_of(lines: &[String]) -> (usize, usize) {
    (
        lines.len() - 1,
        lines.last().map_or(0, |line| line.chars().count()),
    )
}

/// Text as it lands on a field of a given width.
struct Wrapped {
    /// One string per line it takes up. Never empty: a field with nothing in it
    /// is still a line, because the caret has to be somewhere.
    lines: Vec<String>,
    /// Which of those lines the caret is on, and how far along it.
    caret: (usize, usize),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Two lists of choices, neither of them the harness's own: the form is
    /// handed titles and labels and knows nothing else, so a test using the real
    /// lists would be testing the harness instead.
    fn offered() -> Vec<Choices> {
        vec![
            Choices {
                title: "Colour",
                options: vec![
                    Choice {
                        id: "red",
                        label: "Red",
                    },
                    Choice {
                        id: "blue",
                        label: "Blue",
                    },
                ],
                default: Choice {
                    id: "blue",
                    label: "Blue",
                },
            },
            Choices {
                title: "Size",
                options: vec![
                    Choice {
                        id: "small",
                        label: "Small",
                    },
                    Choice {
                        id: "large",
                        label: "Large",
                    },
                ],
                default: Choice {
                    id: "small",
                    label: "Small",
                },
            },
        ]
    }

    /// One draft, offering those choices.
    fn draft() -> Draft {
        Draft::new(Id(0), &offered())
    }

    /// A draft with these keystrokes already in it.
    fn edited(edits: &[Edit]) -> Draft {
        let mut draft = draft();
        for edit in edits {
            draft.edited(*edit);
        }

        draft
    }

    /// The keystrokes that type something.
    fn typing(what: &str) -> Vec<Edit> {
        what.chars().map(Edit::Typed).collect()
    }

    /// Drafts with these descriptions typed into them.
    ///
    /// Shared with the list's tests and the screen's, which both have to draw
    /// drafts and have no business making them a second way.
    pub fn drafting(work: &[&str]) -> Drafts {
        let mut drafts = Drafts::new(offered());

        for typed in work {
            let id = drafts.start();
            drafts.edit(id, Edit::Next);
            for character in typed.chars() {
                drafts.edit(id, Edit::Typed(character));
            }
        }

        drafts
    }

    /// The region the form is drawn into in these tests, which is exactly the
    /// room a blank one needs.
    const REGION: Size = Size {
        columns: 40,
        rows: 18,
    };

    /// The form as text, one string per row of the region.
    fn drawn(draft: &Draft, region: Size) -> String {
        let mut terminal = Terminal::new(TestBackend::new(region.columns, region.rows)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(draft.form(region), frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();

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

    #[test]
    fn the_form_asks_for_a_repository_the_work_and_one_answer_per_list() {
        let screen = drawn(&draft(), REGION);

        assert_eq!(
            screen,
            "\
NEW SPAWN

▍Repository


 Work


 Colour
    Red
  › Blue

 Size
  › Small
    Large

Tab moves between fields
F6 / F7 leave it — nothing is lost"
        );
    }

    #[test]
    fn what_is_typed_lands_in_the_field_the_keyboard_is_in() {
        let screen = drawn(&edited(&typing("/code/project")), REGION);

        assert!(screen.contains("  /code/project"), "{screen}");
    }

    #[test]
    fn tab_moves_on_and_leaves_what_was_typed_where_it_was() {
        let mut edits = typing("/code/project");
        edits.push(Edit::Next);
        edits.extend(typing("add retry logic"));

        let screen = drawn(&edited(&edits), REGION);

        assert!(screen.contains(" Repository\n  /code/project"), "{screen}");
        assert!(screen.contains("▍Work\n  add retry logic"), "{screen}");
    }

    #[test]
    fn the_keyboard_walks_every_control_and_comes_back_round() {
        let mut draft = draft();
        let mut visited = Vec::new();

        for _ in 0..5 {
            visited.push(draft.on);
            draft.edited(Edit::Next);
        }

        assert_eq!(visited, [0, 1, 2, 3, 0]);
    }

    #[test]
    fn going_back_from_the_first_control_reaches_the_last() {
        assert_eq!(edited(&[Edit::Previous]).on, 3);
    }

    /// The rule, stated as a test: a harness with nothing to offer under a
    /// heading gets no control there at all, rather than an empty one.
    #[test]
    fn a_list_with_nothing_in_it_is_not_a_control() {
        let mut offered = offered();
        offered[0].options.clear();

        let draft = Draft::new(Id(0), &offered);
        let screen = drawn(&draft, REGION);

        assert!(!screen.contains("Colour"), "{screen}");
        assert!(screen.contains("Size"), "{screen}");
        assert_eq!(
            draft.controls(),
            3,
            "the keyboard can still be moved into the control that is not there"
        );
    }

    #[test]
    fn a_harness_that_offers_no_choices_at_all_still_gets_a_form() {
        let screen = drawn(&Draft::new(Id(0), &[]), REGION);

        assert!(screen.contains("Repository"), "{screen}");
        assert!(screen.contains("Work"), "{screen}");
    }

    #[test]
    fn a_list_starts_on_what_the_harness_would_have_picked() {
        let screen = drawn(&draft(), REGION);

        assert!(screen.contains("› Blue"), "{screen}");
        assert!(screen.contains("› Small"), "{screen}");
    }

    #[test]
    fn a_choice_is_made_by_moving_up_and_down_the_list_it_came_in() {
        let screen = drawn(&edited(&[Edit::Next, Edit::Next, Edit::Up]), REGION);

        assert!(screen.contains("› Red"), "{screen}");
        assert!(!screen.contains("› Blue"), "{screen}");
    }

    #[test]
    fn both_ends_of_a_list_of_choices_stop_rather_than_wrap() {
        let top = edited(&[Edit::Next, Edit::Next, Edit::Up, Edit::Up, Edit::Up]);
        let bottom = edited(&[Edit::Next, Edit::Next, Edit::Down, Edit::Down]);

        assert!(drawn(&top, REGION).contains("› Red"));
        assert!(drawn(&bottom, REGION).contains("› Blue"));
    }

    #[test]
    fn the_form_is_told_what_a_choice_is_called_and_nothing_else_about_it() {
        // An id no harness has, for the same reason the lists above are about
        // colours: a test carrying a real harness's vocabulary would put that
        // vocabulary outside the harness module, which is the invariant this
        // very test is about.
        let named = vec![Choices {
            title: "Whatever this harness calls it",
            options: vec![Choice {
                id: "as-much-as-it-takes",
                label: "As much as it takes",
            }],
            default: Choice {
                id: "as-much-as-it-takes",
                label: "As much as it takes",
            },
        }];

        let screen = drawn(&Draft::new(Id(0), &named), REGION);

        assert!(
            screen.contains("Whatever this harness calls it"),
            "{screen}"
        );
        assert!(screen.contains("As much as it takes"), "{screen}");
        assert!(
            !screen.contains("as-much-as-it-takes"),
            "the form showed an id nobody reads:\n{screen}"
        );
    }

    #[test]
    fn a_draft_is_called_after_the_work_as_soon_as_there_is_any() {
        assert_eq!(draft().title(), UNTITLED);

        let mut edits = vec![Edit::Next];
        edits.extend(typing("fix the worktree cleanup\nand the branch"));

        assert_eq!(edited(&edits).title(), "fix the worktree cleanup");
    }

    #[test]
    fn the_work_takes_the_line_breaks_a_paragraph_needs() {
        let mut edits = vec![Edit::Next];
        edits.extend(typing("first"));
        edits.push(Edit::Entered);
        edits.extend(typing("second"));

        let screen = drawn(&edited(&edits), REGION);

        assert!(screen.contains("  first\n  second"), "{screen}");
    }

    #[test]
    fn a_path_has_no_lines_in_it_so_enter_is_being_finished_with_the_field() {
        let mut edits = typing("/code/project");
        edits.push(Edit::Entered);
        edits.extend(typing("add retry logic"));

        let draft = edited(&edits);

        assert_eq!(draft.repository.text(), "/code/project");
        assert_eq!(draft.work.text(), "add retry logic");
    }

    #[test]
    fn the_caret_takes_characters_where_it_is_rather_than_at_the_end() {
        let mut edits = typing("ac");
        edits.extend([Edit::Left, Edit::Typed('b')]);

        assert_eq!(edited(&edits).repository.text(), "abc");
    }

    #[test]
    fn a_typo_in_the_middle_can_be_rubbed_out_from_either_side() {
        let mut erased = typing("abXc");
        erased.extend([Edit::Left, Edit::Erased]);
        let mut deleted = typing("abXc");
        deleted.extend([Edit::Left, Edit::Left, Edit::Deleted]);

        assert_eq!(edited(&erased).repository.text(), "abc");
        assert_eq!(edited(&deleted).repository.text(), "abc");
    }

    #[test]
    fn the_caret_stops_at_both_ends_of_the_text() {
        let mut front = typing("ab");
        front.extend([Edit::Left, Edit::Left, Edit::Left, Edit::Typed('x')]);
        let mut back = typing("ab");
        back.extend([Edit::Right, Edit::Right, Edit::Typed('x')]);

        assert_eq!(edited(&front).repository.text(), "xab");
        assert_eq!(edited(&back).repository.text(), "abx");
    }

    #[test]
    fn the_caret_reaches_either_end_of_the_text_in_one_key() {
        let mut edits = typing("bc");
        edits.extend([Edit::Start, Edit::Typed('a'), Edit::End, Edit::Typed('d')]);

        assert_eq!(edited(&edits).repository.text(), "abcd");
    }

    #[test]
    fn rubbing_out_an_empty_field_is_not_an_error() {
        assert_eq!(edited(&[Edit::Erased, Edit::Deleted]).repository.text(), "");
    }

    #[test]
    fn a_character_no_terminal_should_have_sent_is_not_written_into_the_text() {
        let draft = edited(&[Edit::Typed('a'), Edit::Typed('\u{7}'), Edit::Typed('b')]);

        assert_eq!(draft.repository.text(), "ab");
    }

    #[test]
    fn a_line_too_long_for_the_field_carries_on_underneath_it() {
        let mut edits = vec![Edit::Next];
        edits.extend(typing(&format!("{}bbbb", "a".repeat(40))));

        let screen = drawn(&edited(&edits), REGION);

        // Forty columns, two of them the indent: thirty-eight to a line.
        assert!(
            screen.contains(&format!("  {}\n  aabbbb", "a".repeat(38))),
            "{screen}"
        );
    }

    /// Where the terminal's own cursor goes, which is the only thing on screen
    /// saying where what you type will land.
    fn caret(draft: &Draft) -> Option<(u16, u16)> {
        draft.form(REGION).caret()
    }

    #[test]
    fn the_caret_sits_after_what_has_been_typed() {
        let blank = caret(&draft()).expect("a caret in a field");
        let typed = caret(&edited(&typing("abc"))).expect("a caret in a field");

        assert_eq!(typed, (blank.0 + 3, blank.1));
    }

    #[test]
    fn the_caret_follows_the_text_onto_the_line_it_wrapped_onto() {
        let mut wrapping = vec![Edit::Next];
        wrapping.extend(typing(&"a".repeat(39)));

        let wrapped = caret(&edited(&wrapping)).expect("a caret in a field");
        let short = caret(&edited(&[Edit::Next, Edit::Typed('a')])).expect("a caret in a field");

        assert_eq!(wrapped.1, short.1 + 1, "the caret stayed on the first line");
        assert_eq!(wrapped.0, u16::try_from(INDENT).unwrap() + 1);
    }

    /// The caret has to be on a cell of the field, and a line that is exactly
    /// full has no cell left on it — so the caret belongs at the start of the
    /// line under it, which is also where the next character will be drawn.
    #[test]
    fn a_caret_at_the_end_of_a_line_it_filled_is_at_the_start_of_the_next_one() {
        // Forty columns, two of them the indent: thirty-eight fills a line.
        let mut edits = vec![Edit::Next];
        edits.extend(typing(&"a".repeat(38)));
        edits.push(Edit::Entered);
        edits.extend(typing("b"));
        // Back over the `b` and onto the line break, which is where the caret
        // used to be reported one cell off the right-hand edge and vanish.
        edits.extend([Edit::Left, Edit::Left]);

        let at = caret(&edited(&edits)).expect("a caret in the field being typed into");

        assert_eq!(at.0, u16::try_from(INDENT).unwrap());
    }

    #[test]
    fn a_list_of_choices_has_no_caret_because_there_is_nothing_to_type_into_it() {
        assert!(caret(&edited(&[Edit::Next, Edit::Next])).is_none());
    }

    #[test]
    fn a_form_too_short_for_everything_shows_the_control_the_keyboard_is_in() {
        let short = Size {
            columns: 40,
            rows: 7,
        };

        let screen = drawn(&edited(&[Edit::Previous]), short);

        assert!(screen.contains("Size"), "{screen}");
        assert!(
            !screen.contains("Tab moves"),
            "a form with no room for its fields spent rows on the hint:\n{screen}"
        );
    }

    #[test]
    fn a_wider_form_is_a_wider_layout_rather_than_the_same_one_with_space_beside_it() {
        let mut edits = vec![Edit::Next];
        edits.extend(typing(&"a".repeat(50)));
        let draft = edited(&edits);

        let narrow = drawn(&draft, REGION);
        let wide = drawn(
            &draft,
            Size {
                columns: 80,
                rows: 18,
            },
        );

        assert!(!narrow.contains(&"a".repeat(50)), "{narrow}");
        assert!(wide.contains(&"a".repeat(50)), "{wide}");
    }

    #[test]
    fn several_drafts_are_several_records_and_nothing_else() {
        let mut drafts = Drafts::new(offered());

        let first = drafts.start();
        let second = drafts.start();
        drafts.edit(first, Edit::Next);
        drafts.edit(first, Edit::Typed('a'));
        drafts.edit(second, Edit::Next);
        drafts.edit(second, Edit::Typed('b'));

        assert_eq!(drafts.all().len(), 2);
        assert_eq!(drafts.of(first).unwrap().title(), "a");
        assert_eq!(drafts.of(second).unwrap().title(), "b");
    }

    #[test]
    fn no_two_drafts_are_the_same_draft() {
        let mut drafts = Drafts::new(offered());

        let started: Vec<Id> = (0..3).map(|_| drafts.start()).collect();

        assert_ne!(started[0], started[1]);
        assert_ne!(started[1], started[2]);
        assert_ne!(started[0], started[2]);
    }

    #[test]
    fn a_keystroke_aimed_at_a_draft_that_is_not_there_does_nothing() {
        let mut drafts = Drafts::new(offered());
        let id = drafts.start();

        drafts.edit(Id(404), Edit::Typed('a'));

        assert_eq!(drafts.of(id).unwrap().repository.text(), "");
        assert!(drafts.of(Id(404)).is_none());
    }
}
