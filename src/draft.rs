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
//! **Starting one is the only thing in the app that creates anything.** The
//! draft hands over what was typed and then narrates: what it is about to do
//! before it does it, so a creation that dies half way leaves a record of the
//! worktree it made rather than a mystery. On success the row goes and a spawn
//! row takes its place; **on failure it is a draft again with the text
//! untouched**, which is what makes *refuse rather than guess* affordable — a
//! refusal never costs somebody the paragraph they just wrote. The making
//! itself is [`crate::creation`]'s; what is here is the asking and the saying.
//!
//! **Throwing one away is the counterpart, and it is deliberately not
//! retirement.** A draft has made nothing — no worktree, no branch, nothing on
//! disk, no process doing work — so there is no order to take it down in, no
//! cleanliness to check and nothing to refuse: it is a record leaving a list,
//! and it costs nothing but the words. What it costs is the words, though, and
//! they exist nowhere else — so **it is the one thing in the app that asks
//! first**. The first press is the question and any other key answers it *no*.
//! The single exception is a draft a spawn is already being made from, which is
//! the one moment a draft owns something: it refuses until the creation stops,
//! because its record is the only thing saying what that creation has made.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::creation::Wanted;
use crate::harness::{Choice, Choices};
use crate::scaffolding::{self, AMBER, DIM, Footer, broken, scroll_offset, wrapped};
use crate::screen::Size;

/// What the form calls itself, above everything it asks.
const TITLE: &str = "NEW SPAWN";

/// What the list calls a draft nothing has been typed into yet.
const UNTITLED: &str = "a new spawn";

/// What the heading over the repository field says.
const REPOSITORY: &str = "Repository";

/// What the heading over the description says.
const WORK: &str = "Work";

/// What the form calls the block a creation writes into.
const STARTING: &str = "STARTING";

/// What it calls that block once the creation has stopped without starting
/// anything. A heading rather than only a colour: the difference between *this
/// is happening* and *this did not happen* is the whole of what the block says.
const NOT_STARTED: &str = "NOT STARTED";

/// What the form calls the block that asks whether to throw the draft away.
const DISCARD: &str = "DISCARD THIS DRAFT?";

/// What the question says under that heading, which is nearly always.
const NOWHERE_ELSE: &str = "what is typed here exists nowhere else, and there is no worktree, \
                            no branch and no session to lose with it — only the words";

/// What it says instead when a creation stopped after it had already made
/// something.
///
/// **The one thing throwing a draft away can leave behind**, and the question is
/// where it is said: the record above names the worktree, and this run is the
/// last thing that will mention it — until the app is started again and reports
/// what it found under the worktree root. Litter is accepted; *invisible* litter
/// is not, and a question claiming there was nothing to lose is exactly how this
/// would have become invisible.
const AND_WHAT_WAS_MADE: &str = "the words go. What the record above says was already made does \
                                 not — it stays where the record says it is, and nothing in this \
                                 run will mention it again";

/// What the form calls that block when the draft will not be thrown away.
const NOT_DISCARDED: &str = "NOT DISCARDED";

/// Why a draft being made into a spawn is not thrown away.
const WHILE_IT_IS_BEING_MADE: &str = "a spawn is being made from it, and this record is the only \
                                      thing saying what that has already made. It can be discarded \
                                      once the creation has stopped";

/// Why a draft with nothing in the repository field cannot be started.
const MISSING_REPOSITORY: &str =
    "it does not say which repository the work is for, and the app will not guess";

/// Why a draft with nothing in the work field cannot be started.
const MISSING_WORK: &str =
    "it does not say what the work is, which is what a spawn is started with";

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
/// All three are worth saying. `Tab` is the only way around the form and
/// nothing on screen implies it; the key that starts it is the only thing in
/// the app that creates anything, and there is nowhere else to learn it; and
/// that the list's own keys still work is the promise the product rests on,
/// where a full-height form is exactly the place somebody would assume they had
/// been taken somewhere modal.
///
/// **The key that throws it away is learnt here**, because here is where a
/// draft is: the list's footer is about spawns, and a draft's own keys belong
/// under its own form. It says that it asks first, because that is what makes it
/// safe to press and the reason it can sit beside the key that starts one.
///
/// The shortest form that can still spare it four rows is ten: below that
/// there is not room for a heading, a field and a list of choices, which is the
/// least of the form worth showing.
const HINT: Footer = Footer::new(
    &[
        "Tab moves between fields",
        "F5 starts it",
        "F3 discards it — it asks first",
        "F6 / F7 leave it — nothing is lost",
    ],
    10,
);

/// What it says while it is waiting to be told whether to throw the draft away.
///
/// Nothing else, because nothing else is what the keyboard is doing: every key
/// on it that does anything at all is now one of two answers. It says which key
/// is *yes* and leaves the rest of the keyboard as *no*, rather than naming a
/// second key — a question with two keys to learn is one somebody answers by
/// pressing whichever they half-remember.
///
/// **It promises what the app honours, and no more.** "Anything else keeps it"
/// would be false on screen: a key the app has no meaning for — `Esc`, `F1`,
/// `F4` — leaves the question standing rather than answering it, so a footer
/// saying otherwise teaches somebody to press `Esc` and walk away from a
/// question that is still up. Typing and moving away are the two that really do
/// take it back, and they are the two anybody reaches for.
///
/// Its column has to be eight rows for the same reason [`HINT`]'s has to be ten:
/// six rows are the least of the form worth showing, and the footer's own lines
/// come on top of them.
const WHILE_ASKING: Footer = Footer::new(
    &["F3 again discards it", "Typing or moving away keeps it"],
    8,
);

/// What it says instead while the spawn is being made.
///
/// Nothing about moving around a form, because nothing typed into it would go
/// anywhere: the text is being used. What is still true is the thing worth
/// saying — walking away does not stop it, which is the same promise the list
/// makes about quitting.
///
/// Seven rows for its one line, by the same arithmetic: six of content, one of
/// footer.
const WHILE_STARTING: Footer = Footer::new(&["F6 / F7 leave it — it carries on"], 7);

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
        self.doing_to(id, |draft| draft.edited(edit));
    }

    /// Ask for a draft to be made into a spawn, and say what to make.
    ///
    /// Nothing at all when there is nothing to make: a draft already being made
    /// is not started twice, and one that has not said enough refuses **in
    /// place**, which is to say on its own row, with every character of what
    /// was typed exactly where it was.
    pub fn submit(&mut self, id: Id) -> Option<Wanted> {
        self.all
            .iter_mut()
            .find(|draft| draft.id == id)
            .and_then(Draft::submitted)
    }

    /// Write down what a creation is about to do, before it does it.
    pub fn doing(&mut self, id: Id, step: String) {
        self.doing_to(id, |draft| draft.doing(step));
    }

    /// Say that a creation stopped, and hand the draft back to the keyboard.
    pub fn failed(&mut self, id: Id, why: String) {
        self.doing_to(id, |draft| draft.failed(why));
    }

    /// Ask for a draft to be thrown away, and say whether it went.
    ///
    /// **It asks first, and the asking is the first press of the key.** Nothing
    /// else in the app destroys something that exists nowhere else: a spawn's
    /// work is on a branch and a refusal is what protects it, where a draft is a
    /// paragraph somebody typed and there is nothing to refuse. So the answer to
    /// the first press is the question, and the second press is the answer.
    pub fn discarded(&mut self, id: Id) -> bool {
        if !self
            .all
            .iter_mut()
            .find(|draft| draft.id == id)
            .is_some_and(Draft::discarded)
        {
            return false;
        }
        self.all.retain(|draft| draft.id != id);

        true
    }

    /// Take back every question that has been asked about throwing a draft
    /// away.
    ///
    /// Every one rather than a named one, because a question is not a thing to
    /// keep track of: it is answered *no* by anything at all that is not the
    /// answer, so the app takes them all back rather than working out which one
    /// the keystroke was near. A draft with no question standing is untouched by
    /// this.
    pub fn take_back_every_question(&mut self) {
        for draft in &mut self.all {
            draft.discarding = None;
        }
    }

    /// Let go of a draft that has become a spawn.
    ///
    /// The row does not become the spawn's row so much as make way for it: the
    /// spawn is listed under the repository it was started against, which is
    /// somewhere a draft — which has not chosen one until it is typed — could
    /// never have been.
    pub fn finished(&mut self, id: Id) {
        self.all.retain(|draft| draft.id != id);
    }

    /// Do something to one draft, if it is still here.
    ///
    /// Everything that happens to a draft after it is submitted arrives from
    /// somewhere else — a thread, a frame later — so *the draft has gone* is an
    /// ordinary thing for any of it to find, not an error.
    fn doing_to(&mut self, id: Id, what: impl FnOnce(&mut Draft)) {
        if let Some(draft) = self.all.iter_mut().find(|draft| draft.id == id) {
            what(draft);
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
    /// What has happened since it was submitted, if it has been.
    progress: Option<Progress>,
    /// Where throwing it away has got to, if anybody has asked to.
    discarding: Option<Discarding>,
}

/// How far the app has got with a request to throw a draft away.
///
/// **A question rather than an act**, which is what makes it a state at all:
/// everywhere else in the app a key does what it says, and here the first press
/// only asks. The second shape is the one case where the answer cannot be *yes*
/// yet, and it is a state for the same reason — the draft has to be able to say
/// so on screen rather than the key appearing to do nothing.
enum Discarding {
    /// Asked, and waiting for the answer.
    Asked,
    /// Refused, because a spawn is being made from it.
    Refused,
}

/// What a creation has said about a draft it was asked to make.
///
/// The three states of a draft are this field's three shapes, which is why it
/// is one field rather than a flag and a list that could disagree: **nothing**
/// is a draft being written, **something with no trouble in it** is one being
/// made, and **something with trouble** is one that stopped and is being
/// written again.
struct Progress {
    /// What has been done, or attempted — each line written before the step it
    /// describes was tried.
    steps: Vec<String>,
    /// Why it stopped, when it did.
    trouble: Option<String>,
}

impl Progress {
    /// A creation that has just been asked for and has done nothing yet.
    fn new() -> Self {
        Self {
            steps: Vec::new(),
            trouble: None,
        }
    }

    /// One that never started at all, and why.
    ///
    /// For what the form can settle itself — a draft that has not said which
    /// repository, or what the work is. Nothing was attempted, so there is
    /// nothing to record but the refusal.
    fn refused(why: &str) -> Self {
        Self {
            steps: Vec::new(),
            trouble: Some(why.to_string()),
        }
    }
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
            progress: None,
            discarding: None,
        }
    }

    /// Which draft this is.
    pub fn id(&self) -> Id {
        self.id
    }

    /// Whether it was started and could not be.
    ///
    /// One of the two things the list draws its mark from — see [`Draft::
    /// starting`] for why the list is told at all.
    pub fn stopped(&self) -> bool {
        self.progress
            .as_ref()
            .is_some_and(|progress| progress.trouble.is_some())
    }

    /// Whether a spawn is being made from it right now.
    ///
    /// Asked by everything that has to behave differently while it is: the form
    /// draws its record, the keyboard is shut out of it, and the list's row
    /// says so with a mark of its own. *What* the creation is doing is a
    /// sentence and stays in the form; that it is happening at all is one
    /// character, and one character is what a row has.
    pub fn starting(&self) -> bool {
        self.progress
            .as_ref()
            .is_some_and(|progress| progress.trouble.is_none())
    }

    /// Hand over what was typed, so a spawn can be made from it.
    ///
    /// The two fields are the two things nothing else can supply; the answers
    /// go as the ids they came in as, because the form was never told what any
    /// of them means and is not about to start pretending it knows.
    fn submitted(&mut self) -> Option<Wanted> {
        if self.starting() {
            return None;
        }

        let repository = self.repository.text().trim().to_string();
        let work = self.work.text().trim().to_string();
        for (missing, why) in [
            (repository.is_empty(), MISSING_REPOSITORY),
            (work.is_empty(), MISSING_WORK),
        ] {
            if missing {
                self.progress = Some(Progress::refused(why));

                return None;
            }
        }

        self.progress = Some(Progress::new());

        Some(Wanted {
            repository: PathBuf::from(repository),
            work,
            answers: self.choices.iter().map(Picked::answer).collect(),
        })
    }

    /// Write down what the creation is about to do.
    ///
    /// Kept even once the next step is written: the whole point of saying it
    /// beforehand is that what a creation *got as far as* survives it stopping.
    fn doing(&mut self, step: String) {
        if let Some(progress) = &mut self.progress {
            progress.steps.push(step);
        }
    }

    /// Say the creation stopped, and give the draft back to the keyboard.
    ///
    /// **Including any refusal it was carrying.** A draft refuses to be thrown
    /// away only for as long as a spawn is being made from it; once that stops,
    /// the sentence saying so is about nothing that is happening. Left standing
    /// it would tell the reader a creation is running, keep the caret away from
    /// a form that is taking keys again, and spend the next character typed on a
    /// question nobody is being asked.
    fn failed(&mut self, why: String) {
        self.discarding = None;
        match &mut self.progress {
            Some(progress) => progress.trouble = Some(why),
            None => self.progress = Some(Progress::refused(&why)),
        }
    }

    /// Ask for it to be thrown away, and say whether it goes.
    ///
    /// The first ask is the question; the second is the answer to it.
    fn discarded(&mut self) -> bool {
        // **The one draft that owns something.** A creation is on a thread of
        // its own, it cannot be called back, and the draft's record is the only
        // thing on screen naming the worktree it is making — so throwing it away
        // would orphan the worktree and the sentence about it at once. Refusing
        // rather than waiting: waiting would mean a keystroke whose effect
        // arrives seconds later, on a row somebody has walked away from.
        if self.starting() {
            self.discarding = Some(Discarding::Refused);

            return false;
        }
        if matches!(self.discarding, Some(Discarding::Asked)) {
            return true;
        }
        self.discarding = Some(Discarding::Asked);

        false
    }

    /// Whether a creation got far enough to have made something that is still
    /// there.
    ///
    /// **Any step at all counts.** Each line was written before the thing it
    /// describes was attempted, so a creation that said one thing may have done
    /// it — and the question this answers is *is there anything to be left
    /// behind*, where a maybe has to read as a yes.
    fn made_something(&self) -> bool {
        self.progress
            .as_ref()
            .is_some_and(|progress| !progress.steps.is_empty())
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
    ///
    /// **Nothing at all while a spawn is being made from it.** The text has
    /// left — it is a command line on its way to a session — so a character
    /// typed into the form now would change what is on screen and nothing else,
    /// which is the one thing a form must never do.
    fn edited(&mut self, edit: Edit) {
        // A question about throwing it away stands until it is answered, and
        // anything but the answer is an answer of *no* — including a key aimed
        // at the form. The keystroke is spent saying so rather than also landing
        // in the text, so *don't* never leaves a character behind.
        //
        // **Only the question is worth a keystroke.** A refusal is a notice
        // rather than something waiting on an answer, so a key clears it and
        // goes on to do what it says: there is nothing there to answer *no* to.
        if matches!(self.discarding.take(), Some(Discarding::Asked)) {
            return;
        }
        if self.starting() {
            return;
        }

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
    ///
    /// **What is kept in view differs by what the form is doing.** While a spawn
    /// is being made, it is what the creation has said, because nothing else on
    /// the form is going to change and that is the only thing worth a short
    /// slot's rows. Once it is being written again — including after a creation
    /// stopped — it is the control the keyboard is in, because a caret you
    /// cannot see is worse than a reason you have to scroll to. And while a
    /// question about throwing it away is standing, it is the question, which
    /// outranks both: it is the only thing on the form waiting on an answer, and
    /// the next key is that answer wherever the form happens to be scrolled to.
    pub fn form(&self, region: Size) -> Form {
        let footer = self.footer();
        let hint = footer.rows(region.rows);
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

        let mut record = None;
        if let Some(progress) = &self.progress {
            let from = lines.len();
            lines.extend(said(progress, width));
            let last = lines.len() - 1;
            record = Some(from);

            showing = Some(match showing {
                // Being written again, so the control the keyboard is in is what
                // has to stay on screen — and what happened is reached for with
                // whatever rows are left over, because a caret you cannot see is
                // worse than a sentence you have to make room for.
                Some((control, _)) if !self.starting() => (control, last),
                _ => (from, last),
            });
        }

        if let Some(discarding) = &self.discarding {
            if record.is_some() {
                lines.push(Line::raw(""));
            }
            let from = lines.len();
            lines.extend(asked(discarding, self.made_something(), width));
            // The newest thing on the form and the only one waiting on a person,
            // so it wins the rows outright — including from the control the
            // keyboard is in, which is not being typed into while a question is
            // up. **The record comes with it** where there is one: both the
            // question and the refusal are about what a creation already made,
            // and pointing at a record that has scrolled off says nothing. A
            // form too short for both keeps the end of the span, which is the
            // question itself.
            showing = Some((record.unwrap_or(from), lines.len() - 1));
        }

        let scroll = scroll_offset(showing, height);

        Form {
            lines,
            scroll,
            footer,
            hint,
            // Nothing to type into while the spawn is being made — or while a
            // question is standing, where the next key is an answer — so nothing
            // saying you can: a caret parked in a field the keyboard cannot
            // reach is the form telling a lie about itself.
            caret: (!self.starting() && self.discarding.is_none())
                .then(|| {
                    caret.and_then(|(column, row)| on_region(column, row, scroll, region, height))
                })
                .flatten(),
        }
    }

    /// What the foot of the form says, which depends on what it is doing.
    ///
    /// A question outranks the rest: while one is standing every key on the
    /// keyboard is one of two answers, so a footer still listing what the form
    /// does would be listing things no key will do next.
    fn footer(&self) -> &'static Footer {
        match &self.discarding {
            Some(Discarding::Asked) => &WHILE_ASKING,
            _ if self.starting() => &WHILE_STARTING,
            _ => &HINT,
        }
    }
}

/// What a creation has said, as the block the form ends in.
///
/// The heading says which of the two this is; under it the steps read as what
/// they were written as — things about to be done — and then whatever stopped
/// it.
///
/// **Both are broken across lines rather than cut**, which is the one place in
/// the app text is not simply elided to the width it has. A step names a
/// worktree and a branch and a refusal carries git's own words, which name
/// paths too; a record you cannot read the end of is not a record of anything.
fn said(progress: &Progress, width: usize) -> Vec<Line<'static>> {
    let stopped = progress.trouble.is_some();
    let mut lines = vec![Line::styled(
        if stopped { NOT_STARTED } else { STARTING },
        if stopped {
            scaffolding::HEADING.fg(AMBER)
        } else {
            scaffolding::HEADING
        },
    )];

    for step in &progress.steps {
        lines.extend(broken(step, width).iter().map(|line| indented(line, DIM)));
    }
    if let Some(trouble) = &progress.trouble {
        lines.extend(
            broken(trouble, width)
                .iter()
                .map(|line| indented(line, DIM.fg(AMBER))),
        );
    }

    lines
}

/// What the form says about throwing the draft away: the question, or why it
/// will not be thrown away yet.
///
/// **In amber, which is the colour the app admits things in.** A question is not
/// an admission, but it is the same thing from where the user sits — the app
/// saying *this one is on you* — and it is the only thing on the form that is
/// waiting for an answer rather than describing something.
fn asked(discarding: &Discarding, made_something: bool, width: usize) -> Vec<Line<'static>> {
    let (heading, why) = match discarding {
        Discarding::Asked if made_something => (DISCARD, AND_WHAT_WAS_MADE),
        Discarding::Asked => (DISCARD, NOWHERE_ELSE),
        Discarding::Refused => (NOT_DISCARDED, WHILE_IT_IS_BEING_MADE),
    };

    let mut lines = vec![Line::styled(heading, scaffolding::HEADING.fg(AMBER))];
    lines.extend(
        wrapped(why, width)
            .iter()
            .map(|line| indented(line, DIM.fg(AMBER))),
    );

    lines
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

    /// What this list answers with, which is an id the form never reads.
    fn answer(&self) -> String {
        self.options[self.at].id.to_string()
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
    /// How far down those lines sit, so what the form is keeping in view is on
    /// screen.
    scroll: u16,
    /// What the hint says, which is not the same while a spawn is being made.
    footer: &'static Footer,
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
        Paragraph::new(self.footer.lines(usize::from(hint.width))).render(hint, buffer);
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
        rows: 20,
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
F5 starts it
F3 discards it — it asks first
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

    // Starting one: what a draft hands over, what it says while it happens, and
    // what it is again if it does not.

    /// A draft with a repository and some work in it, ready to be started.
    fn filled_in() -> Drafts {
        let mut drafts = Drafts::new(offered());
        let id = drafts.start();
        for character in "/code/project".chars() {
            drafts.edit(id, Edit::Typed(character));
        }
        drafts.edit(id, Edit::Next);
        for character in "add retry logic".chars() {
            drafts.edit(id, Edit::Typed(character));
        }

        drafts
    }

    /// The one draft there is.
    fn only(drafts: &Drafts) -> &Draft {
        &drafts.all()[0]
    }

    /// The region a form carrying a creation's record needs, which is longer
    /// than a blank one's by exactly that record.
    const WITH_A_RECORD: Size = Size {
        columns: 40,
        rows: 24,
    };

    #[test]
    fn starting_a_draft_hands_over_what_was_typed_and_what_was_picked() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        let wanted = drafts.submit(id).expect("a draft that says enough");

        assert_eq!(wanted.repository, PathBuf::from("/code/project"));
        assert_eq!(wanted.work, "add retry logic");
        // The ids of what each list settled on, which is all the form ever knew
        // about them.
        assert_eq!(wanted.answers, ["blue", "small"]);
    }

    #[test]
    fn a_choice_made_in_the_form_is_the_answer_that_is_handed_over() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.edit(id, Edit::Next);
        drafts.edit(id, Edit::Up);

        let wanted = drafts.submit(id).expect("a draft that says enough");

        assert_eq!(wanted.answers, ["red", "small"]);
    }

    /// **Refuse rather than guess, and never at the cost of the typing.** The
    /// app has no way to invent a repository, and a draft that stopped for that
    /// reason is one keystroke from being right.
    #[test]
    fn a_draft_that_does_not_say_enough_refuses_in_place_and_keeps_every_character() {
        let mut drafts = Drafts::new(offered());
        let id = drafts.start();
        drafts.edit(id, Edit::Next);
        for character in "add retry logic".chars() {
            drafts.edit(id, Edit::Typed(character));
        }

        assert!(drafts.submit(id).is_none());

        let screen = drawn(only(&drafts), WITH_A_RECORD);
        assert!(screen.contains("NOT STARTED"), "{screen}");
        assert!(screen.contains("which repository"), "{screen}");
        assert!(screen.contains("add retry logic"), "{screen}");
        assert!(only(&drafts).stopped());
    }

    #[test]
    fn a_draft_with_nothing_said_about_the_work_is_not_started_either() {
        let mut drafts = Drafts::new(offered());
        let id = drafts.start();
        for character in "/code/project".chars() {
            drafts.edit(id, Edit::Typed(character));
        }

        assert!(drafts.submit(id).is_none());
        assert!(drafts.all()[0].stopped());
    }

    #[test]
    fn what_it_is_doing_is_shown_in_the_form_as_it_does_it() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);

        drafts.doing(id, "reading /code/project".to_string());
        drafts.doing(
            id,
            "creating the worktree /w/a7f3 on spawn/a7f3".to_string(),
        );

        let screen = drawn(only(&drafts), WITH_A_RECORD);
        assert!(screen.contains("STARTING"), "{screen}");
        assert!(screen.contains("  reading /code/project"), "{screen}");
        assert!(screen.contains("  creating the worktree"), "{screen}");
    }

    /// The whole reason a creation says what it is *about* to do: what it got as
    /// far as survives it stopping, so a worktree left on disk is written down
    /// rather than a mystery.
    #[test]
    fn a_creation_that_stopped_still_says_what_it_had_already_made() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.doing(
            id,
            "creating the worktree /w/a7f3 on spawn/a7f3".to_string(),
        );

        drafts.failed(id, "the worktree root is full".to_string());

        let screen = drawn(only(&drafts), WITH_A_RECORD);
        assert!(screen.contains("NOT STARTED"), "{screen}");
        assert!(screen.contains("  creating the worktree"), "{screen}");
        assert!(screen.contains("the worktree root is full"), "{screen}");
    }

    #[test]
    fn a_draft_being_made_into_a_spawn_takes_no_more_keystrokes() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);

        drafts.edit(id, Edit::Typed('x'));
        drafts.edit(id, Edit::Erased);
        drafts.edit(id, Edit::Next);

        assert_eq!(only(&drafts).work.text(), "add retry logic");
        assert_eq!(
            only(&drafts).on,
            1,
            "the keyboard was moved out of the field"
        );
        assert!(
            only(&drafts).form(REGION).caret().is_none(),
            "a caret said what you type would land somewhere it would not"
        );
    }

    #[test]
    fn a_draft_that_could_not_be_started_is_written_again_from_where_it_was() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.failed(id, "there is no such repository".to_string());

        drafts.edit(id, Edit::Typed('!'));

        assert_eq!(only(&drafts).work.text(), "add retry logic!");
        assert!(only(&drafts).form(REGION).caret().is_some());
    }

    #[test]
    fn one_draft_is_started_once_however_often_it_is_asked_for() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        assert!(drafts.submit(id).is_some());
        assert!(
            drafts.submit(id).is_none(),
            "a second worktree was asked for while the first was being made"
        );
    }

    /// **A refusal costs the choices no more than it costs the paragraph.**
    /// Everything the form collected is still there afterwards — the repository,
    /// the work, and every option that was picked — so starting it again once
    /// the machine is fixed is one keystroke rather than filling a form in twice.
    #[test]
    fn a_draft_that_could_not_be_started_still_holds_everything_that_was_picked() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.edit(id, Edit::Next);
        drafts.edit(id, Edit::Up);
        let asked_for = drafts.submit(id).expect("a draft that says enough");

        drafts.failed(id, "the harness is not installed".to_string());
        let asked_again = drafts
            .submit(id)
            .expect("a draft that stopped can be started again");

        assert_eq!(
            asked_again, asked_for,
            "the refusal cost something that was said"
        );
        assert_eq!(asked_again.answers, ["red", "small"]);
    }

    #[test]
    fn a_draft_that_could_not_be_started_can_be_started_again() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.failed(id, "there is no such repository".to_string());

        assert!(drafts.submit(id).is_some());
    }

    // Throwing one away, which is the one thing in the app that destroys
    // something existing nowhere else — and therefore the one thing that asks.

    /// **A keystroke does not throw away a paragraph.** Everywhere else a
    /// refusal is what protects the typing; here there is nothing to refuse, so
    /// the question is what protects it.
    #[test]
    fn a_draft_asks_before_it_is_thrown_away() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        assert!(!drafts.discarded(id), "the draft went without being asked");

        assert_eq!(drafts.all().len(), 1);
        let screen = drawn(only(&drafts), WITH_A_RECORD);
        assert!(screen.contains("DISCARD"), "{screen}");
        assert!(
            screen.contains("add retry logic"),
            "the question cost the paragraph it was asked about:\n{screen}"
        );
    }

    #[test]
    fn a_draft_asked_about_twice_goes() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        drafts.discarded(id);

        assert!(drafts.discarded(id), "the answer did not throw it away");
        assert!(drafts.all().is_empty());
        assert!(drafts.of(id).is_none());
    }

    /// **Other drafts are unaffected**, which is the whole of what a draft owns:
    /// a record in a list, and throwing one away is one record leaving it.
    #[test]
    fn throwing_one_draft_away_leaves_every_other_exactly_as_it_was() {
        let mut drafts = drafting(&["the first", "the second", "the third"]);
        let second = drafts.all()[1].id();

        drafts.discarded(second);
        assert!(drafts.discarded(second));

        let left: Vec<String> = drafts.all().iter().map(Draft::title).collect();
        assert_eq!(left, ["the first", "the third"]);
    }

    /// A keystroke that is not the answer answers *no* — and is spent saying
    /// so. It has to be spent: a key that both took the question back and typed
    /// itself into the field would leave a character behind from a keystroke
    /// somebody meant as *don't*.
    #[test]
    fn anything_typed_at_the_question_keeps_the_draft_and_is_not_typed_into_it() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.discarded(id);

        drafts.edit(id, Edit::Typed('x'));

        assert_eq!(drafts.all().len(), 1);
        let screen = drawn(only(&drafts), WITH_A_RECORD);
        assert!(
            !screen.contains("DISCARD"),
            "the question is still standing:\n{screen}"
        );
        assert_eq!(only(&drafts).work.text(), "add retry logic");
    }

    /// And a question is never left standing behind you: walking off the row
    /// takes it back, so coming back to a draft never lands on an armed
    /// keystroke.
    #[test]
    fn moving_away_from_a_draft_takes_the_question_back() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.discarded(id);

        drafts.take_back_every_question();

        let screen = drawn(only(&drafts), WITH_A_RECORD);
        assert!(!screen.contains("DISCARD"), "{screen}");
        assert!(
            !drafts.discarded(id),
            "the next press answered a question that had been taken back"
        );
    }

    /// The region a form needs for a creation's record **and** a question about
    /// throwing the draft away, which is the longest a form ever gets.
    const WITH_A_QUESTION: Size = Size {
        columns: 40,
        rows: 32,
    };

    /// **A draft mid-creation is the one draft that owns something.** It has
    /// made, or is about to make, a worktree — and its record is the only thing
    /// on screen saying so, so throwing it away would orphan the worktree and
    /// the sentence naming it in the same keystroke. It refuses, and says why.
    #[test]
    fn a_draft_a_spawn_is_being_made_from_is_not_thrown_away() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.doing(
            id,
            "creating the worktree /w/a7f3 on spawn/a7f3".to_string(),
        );

        assert!(!drafts.discarded(id));
        assert!(
            !drafts.discarded(id),
            "asking twice threw away a draft that is making a worktree"
        );

        assert_eq!(drafts.all().len(), 1);
        let screen = drawn(only(&drafts), WITH_A_QUESTION);
        assert!(screen.contains("NOT DISCARDED"), "{screen}");
        assert!(
            screen.contains("creating the worktree"),
            "the record of what it has already made is not beside the refusal:\n{screen}"
        );
        assert!(
            only(&drafts).starting(),
            "refusing to throw it away stopped the creation being one"
        );
    }

    /// **The one thing throwing a draft away can leave behind.** A creation that
    /// stopped after it had already made a worktree leaves it on disk, and this
    /// record is the only thing on screen naming it — so the question says that
    /// rather than claiming there is nothing to lose but the words. What it
    /// does not do is refuse: the record above the question is what somebody is
    /// answering it with, and the app names what it finds under the worktree
    /// root the next time it starts.
    #[test]
    fn a_draft_whose_creation_already_made_something_says_so_when_it_asks() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.doing(
            id,
            "creating the worktree /w/a7f3 on spawn/a7f3".to_string(),
        );
        drafts.failed(id, "the harness would not start".to_string());

        drafts.discarded(id);

        let screen = drawn(only(&drafts), WITH_A_QUESTION);
        assert!(screen.contains("DISCARD"), "{screen}");
        assert!(
            !screen.contains("no worktree"),
            "the question claimed there was nothing to lose, beside a record of a worktree:\n\
             {screen}"
        );
        assert!(
            screen.contains("/w/a7f3"),
            "the record naming what is being left behind is not beside the question:\n{screen}"
        );
    }

    /// **The footer promises only what the app honours.** A key it has no
    /// meaning for — `Esc`, `F1`, `F4` — leaves the question standing rather
    /// than answering it, so a footer saying that anything else keeps the draft
    /// teaches somebody to press `Esc` and walk away from a question still up.
    #[test]
    fn the_question_names_only_the_answers_the_app_really_takes() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        drafts.discarded(id);

        let screen = drawn(only(&drafts), WITH_A_QUESTION);
        assert!(screen.contains("F3 again discards it"), "{screen}");
        assert!(
            !screen.contains("Anything else"),
            "the footer promises of every other key something the app does not do:\n{screen}"
        );
    }

    /// **A refusal is a notice, and it does not outlive what it was about.**
    /// Press the key mid-creation and the draft says no; when that creation then
    /// stops, the draft is only text again and the refusal is a sentence about a
    /// spawn nobody is making. Left standing it costs three things at once: the
    /// form says a spawn is being made from it, the caret stays away from a form
    /// that is taking keys again, and the next character typed is swallowed by a
    /// question that is not being asked.
    #[test]
    fn a_refusal_does_not_outlive_the_creation_it_was_about() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.discarded(id);
        drafts.failed(id, "the harness would not start".to_string());

        let screen = drawn(only(&drafts), WITH_A_QUESTION);
        assert!(
            !screen.contains(NOT_DISCARDED),
            "a draft nothing is being made from still refuses to be thrown away:\n{screen}"
        );
        assert!(
            only(&drafts).form(WITH_A_QUESTION).caret().is_some(),
            "the form is taking keys again with nowhere to show them going"
        );

        drafts.edit(id, Edit::Typed('!'));

        let typed = drawn(only(&drafts), WITH_A_QUESTION);
        assert!(
            typed.contains("add retry logic!"),
            "the first character typed after the creation stopped was swallowed:\n{typed}"
        );
    }

    /// And the moment the creation stops, the draft is only text again — so it
    /// can be thrown away like any other, record and all.
    #[test]
    fn a_draft_whose_creation_stopped_can_be_thrown_away() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();
        drafts.submit(id);
        drafts.failed(id, "the worktree root is full".to_string());

        assert!(!drafts.discarded(id), "it went without being asked");
        assert!(drafts.discarded(id));
        assert!(drafts.all().is_empty());
    }

    /// Nothing to type into while a question is up: the next key is an answer,
    /// and a caret would say it was going into the field it is sitting in.
    #[test]
    fn a_question_takes_the_caret_away_because_the_next_key_is_an_answer() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        drafts.discarded(id);

        assert!(only(&drafts).form(WITH_A_QUESTION).caret().is_none());
    }

    /// The key that throws a draft away is learnt where the draft is, because
    /// the list's own footer is about spawns — and while the question is
    /// standing the foot of the form says what answers it, since that is the
    /// only thing the keyboard is doing.
    #[test]
    fn a_form_says_what_throws_it_away_and_then_what_answers_the_question() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        let writing = drawn(only(&drafts), WITH_A_QUESTION);
        drafts.discarded(id);
        let asking = drawn(only(&drafts), WITH_A_QUESTION);

        assert!(writing.contains("F3 discards it"), "{writing}");
        assert!(asking.contains("F3 again"), "{asking}");
        assert!(
            !asking.contains("F5 starts it"),
            "the form offers to start a draft it is asking about throwing away:\n{asking}"
        );
    }

    #[test]
    fn a_draft_that_became_a_spawn_is_no_longer_a_draft() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        drafts.finished(id);

        assert!(drafts.all().is_empty());
        assert!(drafts.of(id).is_none());
    }

    #[test]
    fn a_form_says_what_starts_it_and_stops_saying_so_once_it_has() {
        let mut drafts = filled_in();
        let id = only(&drafts).id();

        let writing = drawn(only(&drafts), REGION);
        drafts.submit(id);
        let starting = drawn(only(&drafts), REGION);

        assert!(writing.contains("F5 starts it"), "{writing}");
        assert!(
            !starting.contains("F5 starts it"),
            "the form still offers to start something it is already starting:\n{starting}"
        );
        assert!(starting.contains("F6 / F7"), "{starting}");
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
