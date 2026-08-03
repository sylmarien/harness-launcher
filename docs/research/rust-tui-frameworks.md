# Research — the Rust TUI framework landscape

> **Status: fact-finding, complete.** This document answers
> [issue #6](https://github.com/sylmarien/harness-launcher/issues/6). It is **not a decision**:
> it does not pick a framework. That call is made later, with the human, and it feeds the
> layout prototype and the project scaffolding.
>
> **Data captured 2026-08-03.** Every number below is a snapshot of that date. Health signals
> rot fast — re-check before leaning on any of them.

## Contents

- [How to read this](#how-to-read-this)
- [What the repo's own docs demand of the answer](#what-the-repos-own-docs-demand-of-the-answer)
- [The field](#the-field)
- [Health signals](#health-signals)
- [Axis 1 — async runtimes and long-running child processes](#axis-1--async-runtimes-and-long-running-child-processes)
- [Axis 2 — a persistent split layout](#axis-2--a-persistent-split-layout)
- [Axis 3 — input handling](#axis-3--input-handling)
- [Axis 4 — documentation, examples and community](#axis-4--documentation-examples-and-community)
- [The hard part: a live child terminal inside a pane](#the-hard-part-a-live-child-terminal-inside-a-pane)
- [Prior art: this exact product already exists, several times](#prior-art-this-exact-product-already-exists-several-times)
- [Which one is a Rust beginner most likely to succeed with](#which-one-is-a-rust-beginner-most-likely-to-succeed-with)
- [What I could not determine](#what-i-could-not-determine)
- [Questions this research does not answer](#questions-this-research-does-not-answer)
- [Sources](#sources)

## How to read this

**Verified vs inferred.** Claims are tagged where the distinction matters:

- **[verified]** — read directly out of a primary source: crate metadata from the crates.io
  API, repository metadata from the GitHub API, or the text of a file in the project's own
  repository.
- **[inferred]** — my reading of those facts. Reasoning is given so it can be disagreed with.

**A caveat on method.** This session's network egress policy blocked `docs.rs`, `ratatui.rs`
and `github.com` HTML. `crates.io`'s API, `raw.githubusercontent.com` and git-over-HTTPS were
reachable, so the primary sources here are **the repositories themselves** — READMEs, doc
comments, `Cargo.toml`s, example directories and the Markdown source of the documentation
sites — read from shallow clones. This is a *stronger* form of primary sourcing than the
rendered pages would have been, but it means quoted documentation is quoted from its source
file rather than its published URL. Published URLs are given anyway, in
[Sources](#sources), for the reader who can reach them.

**A caveat on download counts.** crates.io totals include CI, mirrors and transitive pulls.
They are a useful *relative* signal between crates in the same niche and a poor absolute one.

## What the repo's own docs demand of the answer

Four constraints out of [the vision](../product-vision.md) and
[tranche 1](../tranches/01-the-core-loop.md) do most of the filtering. They matter because
they rule things out before any beauty contest starts.

1. **A persistent split, not a mode.** *"Opening a spawn does not replace the dashboard. I see
   the agent **and** the list of spawns at the same time."* — tranche 1, "Opening a spawn".
   The detail pane and the list pane are both live, always, together.

2. **The detail pane surfaces the harness; it does not rebuild it.** The vision puts
   *"reimplementing a harness's own interface"* explicitly out of scope, and tranche 1 asks
   for *"the full view of the agent, exactly as if I had started it by hand: watch it work in
   real time, type into its prompt, interrupt it, see its sub-agents."*

   **[inferred]** Taken together these two force a specific technical shape: the detail pane
   has to be a **terminal emulator** — a pseudoterminal (PTY) hosting the real `claude`
   process, its output parsed as ANSI/VT sequences and painted into a sub-rectangle of the
   screen, with keystrokes forwarded back into it. It cannot be a log tail, and it cannot be
   a re-drawn imitation of Claude Code's UI. **This is the single hardest requirement in the
   tranche, and it is the one that most sharply discriminates between the candidates.**

3. **Live, at scale, with no refresh.** 15–20 concurrent spawns, each a running harness in
   its own worktree; *"What the app shows is always live… There is no refreshing"*. So the
   render loop must be driven by events arriving from ~20 child processes, not by user
   keystrokes.

4. **The author is new to Rust and using this project to learn it.** Issue #6 says to weight
   maturity, documentation quality and example-corpus size above flexibility and cleverness.

## The field

There is no crowded field. Once you require an actively maintained, generally-purposed,
non-toy library, the credible set is **three**, and ratatui's own README names exactly the
same three:

> **Alternatives**
> - [Cursive](https://crates.io/crates/cursive) — a ncurses-based TUI library.
> - [iocraft](https://crates.io/crates/iocraft) — a declarative TUI library.
>
> — `README.md`, ratatui/ratatui **[verified]**

Everything else is a layer below, a framework on top, or dead.

### The three general-purpose libraries

| | **ratatui** | **cursive** | **iocraft** |
|---|---|---|---|
| Paradigm | Immediate mode — UI rebuilt every frame | Retained mode — a tree of `View` objects | Declarative/React — `element!` macro, components + hooks |
| Layout | `Layout` + constraints, you compute `Rect`s | View tree, `LinearLayout`, sizing negotiated | Flexbox, via [`taffy`](https://crates.io/crates/taffy) |
| Owns the event loop? | **No** — you write it | **Yes** — `siv.run()`; `runner()`/`step()` to drive manually | **Yes** — `render_loop()` returns a future you drive |
| Focus management | None — you track it | Built in (`take_focus`, focus traversal) | Built in |
| Backend | crossterm (default), termion, termwiz, termina | crossterm (default), others via wiki | crossterm, internal |
| Async story | You bring your own; crossterm `event-stream` feeds a `tokio::select!` | `cb_sink()` — send `Send` closures in from other threads | `use_future` / `use_async_handler` hooks; runtime-agnostic future |
| Terminal-emulator widget | Yes, third-party ([`tui-term`](https://crates.io/crates/tui-term)) | Yes, third-party ([`cursive-multiplex`](https://crates.io/crates/cursive-multiplex)) | **None found** |

Every cell in that table is **[verified]** against the sources listed at the end.

### Frameworks built *on* ratatui

These are not alternatives to ratatui; they are opinionated shells around it, and choosing one
means adopting *both* it and ratatui.

- **[tui-realm](https://crates.io/crates/tuirealm)** — "a ratatui framework inspired by Elm
  and React", with a `View` that "manages mounting/unmounting, focus and event forwarding for
  you" and an optional `async-ports` feature built on tokio. The most established of the
  bunch: v4.1.0, 974 stars. **[verified]**
- **[ratatui-kit](https://crates.io/crates/ratatui-kit)** (985 downloads/90d),
  **[rat-salsa](https://crates.io/crates/rat-salsa)** (1,016/90d),
  **[widgetui](https://crates.io/crates/widgetui)** (223/90d, last release 2024-10-27).
  **[verified]** **[inferred]** These are effectively single-author projects with negligible
  adoption; for a Rust beginner they add a second, thinly-documented abstraction to learn on
  top of the one that has the documentation.

### The layer below — terminal backends, not TUI frameworks

You do not build a multi-pane dashboard directly on these, but every candidate sits on one, so
their health is load-bearing.

- **[crossterm](https://crates.io/crates/crossterm)** — the default backend for both ratatui
  and cursive. 167.7M downloads all-time, 40.4M in 90 days. Ships an `event-stream` feature
  producing a `futures::Stream` of events, with first-party examples for **both tokio and
  smol** (`examples/event-stream-tokio.rs`, `examples/event-stream-smol.rs`). **[verified]**
  Ratatui's own guidance: *"Choose Crossterm for most tasks… If you have no particular reason
  to use Termion, Termwiz, or Termina, you will find it easiest to learn Crossterm simply due
  to its popularity."* **[verified]**
- **[termwiz](https://crates.io/crates/termwiz)** — WezTerm's terminal library. It does have a
  `Widget` trait ("allows composition of UI elements at a higher level") so it is
  *technically* a TUI toolkit, but its own README opens with *"It is currently in active
  development and subject to fairly wild sweeping changes."* **[verified]** **[inferred]**
  Not a credible choice for a beginner as a UI layer; it is credible as a ratatui *backend*,
  and ratatui's flowchart recommends it only if the TUI is for WezTerm users exclusively.
- **[termina](https://crates.io/crates/termina)** — a newer cross-platform VT library from the
  Helix editor project, now a supported ratatui backend (`ratatui-termina`). 1.73M of its
  1.77M all-time downloads are from the last 90 days. **[verified]** **[inferred]** Fast-rising
  but very young; not a reason to deviate from crossterm today.
- **termion** — a supported ratatui backend, excluded from ratatui's own default workspace
  members with the comment *"this is not included as it doesn't compile on windows"*.
  **[verified]**

### Dead or dying — named so they can be ruled out on sight

**[verified]** all:

- **[tui-rs](https://crates.io/crates/tui)** (`fdehau/tui-rs`) — **the archived ancestor.**
  Repository flagged `archived: true`, last push 2023-08-06, last release 0.19.0 on
  2022-08-14. Ratatui is its continuation: *"Ratatui was forked from the tui-rs crate in 2023
  in order to continue its development."* Still 10,868 stars and 331k downloads a quarter,
  which is exactly why it needs naming — **a lot of Google results and blog posts still say
  `tui`, and they are all obsolete.** Ratatui's website carries a
  [migration guide](https://ratatui.rs/recipes/apps/migrate-from-tui-rs/) for this reason.
- **[zi](https://crates.io/crates/zi)** — declarative monospace UI library. Last release
  0.3.2, 2022-04-22. 444 downloads in 90 days. No repository listed on crates.io.
- **[dioxus-tui](https://crates.io/crates/dioxus-tui)** — last stable 0.4.3 (2023-12-07), one
  alpha in 2024-02, 371 downloads in 90 days.
- **[pancurses](https://crates.io/crates/pancurses)** (last release 2021-09-29) and
  **[ncurses](https://crates.io/crates/ncurses)** (a "very thin wrapper", 2024-07-12) — raw
  curses bindings. **[inferred]** Learning C-shaped `unsafe`-adjacent APIs is the opposite of
  what "using this project to learn Rust" wants.

## Health signals

All **[verified]** on 2026-08-03. Sources: crates.io API for release/download data, GitHub API
for repository metadata, `git log` over a full-history clone for commit activity.

| | ratatui | cursive | iocraft | tui-realm |
|---|---|---|---|---|
| Latest release | **0.30.2**, 2026-06-19 | **0.21.1**, 2024-08-03 ⚠️ | **0.8.4**, 2026-07-13 | **4.1.0**, 2026-05-02 |
| First release | 2023-02-08 (fork of 2016 lineage) | 2016-06-25 | 2024-09-23 | 2021-04-20 |
| Downloads, all-time | 41,394,535 | 1,664,631 | 147,203 | 215,679 |
| Downloads, last 90d | **15,373,659** | 340,674 | 46,460 | 31,430 |
| GitHub stars | **22,041** | 4,836 | 1,478 | 974 |
| Forks | 729 | 266 | 56 | 40 |
| Open issues | 218 | 216 | 28 | 10 |
| Last commit | 2026-07-25 | 2026-07-14 | 2026-07-29 | 2026-07-29 |
| Commits, last 12mo | 413 (256 by dependabot) | 47 | 60 | 361 |
| Commits, last 3mo | **96** | 17 | 13 | **4** ⚠️ |
| Distinct authors, 12mo | **64** | 15 | 20 | 4 ⚠️ |
| Named maintainers | **4** current, 5 past, in `MAINTAINERS.md` | 1 (21 of 47 commits) | 1 (+1 regular) | 1 dominant (305 of 361) |
| Licence | MIT | MIT | Apache-2.0 / MIT | MIT |
| MSRV | 1.88.0 | edition 2024 (per changelog) | not declared in workspace | — |

Three things in that table are worth saying out loud.

- **cursive's crates.io release is nearly two years stale**, while its repository is active
  (last commit 2026-07-14) and its `cursive-core` sub-crate *did* ship recently (0.4.7,
  2026-06-12). Its `CHANGELOG.md` has an `## Unreleased` section and a `## Next (cursive
  0.22.0)` section listing "Update crossterm to 0.29" as a breaking change. **[verified]**
  **[inferred]** The work is being done; the *releases* are not happening. A beginner pulling
  `cursive = "0.21"` gets a build that is two years behind the repo they will be reading for
  help, and behind the crossterm everyone else is on.
- **ratatui is the only one of the four with a genuine bus factor above one.** 64 distinct
  authors in twelve months, four named maintainers in a checked-in `MAINTAINERS.md`, and a
  documented handover history (five past maintainers listed). **[verified]**
- **tui-realm's activity is misleading.** 361 commits in twelve months looks healthy until you
  see 305 of them are one contributor and only 4 commits landed in the last three months.
  **[verified]**

## Axis 1 — async runtimes and long-running child processes

### ratatui

Ratatui takes no position, and says so. Its own FAQ:

> `ratatui` isn't a native `async` library. So is it beneficial to use `tokio` or
> `async`/`await`? […] As a user of `ratatui`, there really is only one point of interface
> with the `ratatui` library and that's the `terminal.draw(|f| ui(f))` functionality […]
> Everything else in your code is your own to do as you wish.
>
> — `faq.md`, ratatui/ratatui-website **[verified]**

And the rendering concept page names the cost of that honestly:

> **Render loop management**: In Immediate mode rendering, the onus of triggering rendering
> lies on the programmer. […] **Event loop orchestration**: Along with managing "the render
> loop", developers are also responsible for handling "the event loop".
>
> — `concepts/rendering/index.md` **[verified]**

**[inferred]** For harness-launcher this cuts *for* ratatui rather than against it. A
supervisor of 20 child processes has a genuinely custom event loop — PTY output, process
exits, keystrokes, a frame timer — and a library that owns the loop would have to be talked
out of owning it. Ratatui simply hands you `terminal.draw()` and stays out of the way.

The pattern is not left to the imagination. The first-party
[`async-github` example](https://github.com/ratatui/ratatui/tree/main/examples/apps/async-github)
is a `#[tokio::main]` app that merges a frame timer and a crossterm event stream:

```rust
let period = Duration::from_secs_f32(1.0 / Self::FRAMES_PER_SECOND);
let mut interval = tokio::time::interval(period);
let mut events = EventStream::new();
```

— `examples/apps/async-github/src/main.rs`, depending on
`crossterm = { features = ["event-stream"] }`, `tokio = { features = ["macros",
"rt-multi-thread"] }` and `tokio-stream`. **[verified]**

Beyond that: an eight-page
[async tutorial](https://ratatui.rs/tutorials/counter-async-app/) built on tokio, and two
scaffolding templates (`simple-async`, `event-driven-async`) generated by `cargo generate
ratatui/templates`. **[verified]**

**Long-running child processes: an important gap.** Ratatui's *official* recipe for running
another program is
[Spawn External Editor (Vim)](https://ratatui.rs/recipes/apps/spawn-vim/), and it is the
**suspend-the-whole-TUI** pattern:

> To spawn Vim from our TUI app, we first need to relinquish control of input and output […]
> we leave the alternate screen and disable raw mode to restore terminal to its original
> state. […] At this point, we have given up control of our TUI app to vim.
>
> — `recipes/apps/spawn-vim.md` **[verified]**

**[inferred]** That recipe is the *opposite* of what harness-launcher needs — it is precisely
"opening a spawn costs you sight of the others". First-party ratatui documentation covers
async I/O well and **does not cover hosting a live child process in a pane at all**. That
capability lives in the third-party ecosystem; see
[the hard part](#the-hard-part-a-live-child-terminal-inside-a-pane).

### cursive

Cursive owns the loop (`siv.run()`). The documented way in from a background thread is
`cb_sink()`:

> Returns a sink for asynchronous callbacks. Returns the sender part of a channel, that allows
> to send callbacks to `self` from other threads. Callbacks will be executed in the order of
> arrival on the next event cycle. **Notes:** Callbacks need to be `Send`, which can be
> limiting in some cases.
>
> — doc comment on `Cursive::cb_sink`, `cursive-core/src/cursive_root.rs` **[verified]**

There is an escape hatch: `Cursive::runner()` returns a `CursiveRunner` with `step()` and
`refresh()`, so the loop *can* be driven manually. **[verified]** No async runtime integration
ships with the crate; `cursive-async-view` is third-party and last released 2024-08-12, with
1,311 downloads in 90 days. **[verified]**

**[inferred]** Workable, but it is inversion of control: your 20 PTY readers live in threads
and shove `Send` closures at the UI. For a Rust beginner, `Send + 'static` closure boundaries
are one of the sharper edges of the language, and this design puts them on the critical path
from day one.

### iocraft

Async is a first-class concept: `use_future` binds a task to a component's lifetime, and
`use_async_handler` exists alongside it. **[verified]**

> Spawns a future which is bound to the lifetime of the component. When the component is
> dropped, the future will also be dropped.
>
> — doc comment on `UseFuture::use_future` **[verified]**

`render_loop()` returns a plain future, so it is executor-agnostic and can be driven by tokio
**[verified]** — though the crate depends on `smol` and every doc example uses
`smol::block_on` and `smol::Timer`. **[verified]** **[inferred]** Mixing a smol-flavoured UI
crate with a tokio-flavoured process supervisor is a papercut a beginner does not need.

Nothing in iocraft's examples or its nine built-in components addresses child processes.
**[verified]**

### Verdict on this axis

**[inferred]** ratatui, on the strength of a first-party tokio example, a tokio tutorial, two
async templates, and — decisively — the fact that it does *not* own the loop that
harness-launcher needs to own. All three have a real answer for async; only ratatui has an
answer for a *live child terminal*, and that answer is third-party.

## Axis 2 — a persistent split layout

This axis turns out not to discriminate. All three do it, easily.

**ratatui.** `Layout::horizontal([...])` / `Layout::vertical([...])` with `Constraint`s
(`Length`, `Min`, `Max`, `Ratio`, `Percentage`), resolved by `.split(area)` or the
const-generic `.areas::<N>(area)` returning `[Rect; N]`. **[verified]** — `Layout` API in
`ratatui-core/src/layout/layout.rs`. The concept page walks the exact
list-beside-detail shape, and there is a dedicated
[constraint-explorer example](https://github.com/ratatui/ratatui/tree/main/examples/apps/constraint-explorer)
for learning how constraints resolve. **[verified]**

Both panes stay live for free: immediate mode redraws everything every frame from current
state, so "both live at once" is the default rather than a feature. **[inferred]**

The list pane is directly supported — `List`/`ListState` and `Table`/`TableState` are
`StatefulWidget`s in `ratatui-widgets`, alongside `Block`, `Paragraph`, `Scrollbar`, `Gauge`,
`Tabs`, `Chart`, `Sparkline`, `BarChart`, `Canvas`, `Calendar` and `Clear`. **[verified]**

**cursive.** `LinearLayout`, plus `Panel`, `ScrollView`, `ResizedView`, `SelectView`,
`ListView`, `StackView`, `FixedLayout` — 38 view modules in `cursive-core/src/views/`
altogether. **[verified]** Third-party `cursive-split-panel` adds a movable divider.
**[verified]**

**iocraft.** Full flexbox via `taffy` — the most *expressive* layout engine of the three.
**[verified]** **[inferred]** Also the least relevant advantage here: a fixed two-pane split
is the one layout that needs no expressiveness.

**[inferred]** Conclusion: **do not let layout drive this decision.** The requirement in
tranche 1 that *sounds* like a layout requirement ("list alongside detail, both live") is
satisfied trivially by every candidate. The requirement that actually bites is what goes
*inside* the detail pane.

## Axis 3 — input handling

**ratatui — nothing, deliberately.**

> There are many ways to handle events with the `ratatui` library. Mostly because `ratatui`
> does not directly expose any event catching; the programmer will depend on the chosen
> backend's library.
>
> — `concepts/event-handling.md` **[verified]**

You read from crossterm and dispatch yourself. The website documents three patterns
(centralised, centralised-catch-plus-message-passing, distributed loops) with pros and cons
for each, and three application architectures to hang them on — The Elm Architecture, Flux,
and a Component architecture with a matching `component` template. **[verified]**

There is no focus management: which pane has the keyboard is a field in your own state.
**[inferred]** For a two-pane app this is roughly ten lines. For a twenty-pane app it would be
a burden — but harness-launcher is the former.

Gaps are filled by well-adopted third-party crates **[verified]**:
[`tui-input`](https://crates.io/crates/tui-input) (0.15.3, 2026-04-18, 459,952 downloads/90d),
[`tui-textarea`](https://crates.io/crates/tui-textarea) (0.7.0, 2024-10-22 — stale but
720,194/90d), [`crokey`](https://crates.io/crates/crokey) for parsing keybindings
(1,360,810/90d), and [`terminput`](https://crates.io/crates/terminput) as a backend-agnostic
input abstraction (181,478/90d).

**cursive — the most given to you.** Focus traversal, `on_event` handlers per view, a menubar,
`EditView`/`TextArea` widgets, and `focus.rs` / `key_codes.rs` examples. **[verified]**

**iocraft — hooks.** `use_terminal_events`, plus a built-in `TextInput` component.
**[verified]**

**One cross-cutting trap, documented by ratatui and applying to anything on crossterm:**

> on Windows, when using `Crossterm`, this will send the same `Event::Key(e)` twice; one for
> when you press the key […] and one for when you release […] On MacOS and Linux only
> `KeyEventKind::Press` […] is generated.
>
> — `faq.md` **[verified]** (Not a concern for a Linux/macOS-only tool, but worth knowing it
> is a known, documented gotcha rather than a mystery.)

**[inferred]** On raw ergonomics cursive wins this axis. But note the interaction with
requirement 2: when the detail pane is a PTY, most keystrokes must be **forwarded verbatim to
the child**, not interpreted by the framework. A framework with strong opinions about focus
and key handling is something you then have to fight. Ratatui having no opinion is, for this
specific application, closer to a feature than a gap.

## Axis 4 — documentation, examples and community

This is the axis issue #6 says to weight most heavily, and it is not close.

### ratatui

**A structured documentation site with its own repository** (`ratatui/ratatui-website`),
whose Markdown source contains **169 pages** organised as **[verified]**:

- **Tutorials** — Hello Ratatui; a counter app in three escalating forms (single function,
  multiple functions, multiple files); an **eight-page async counter app** on tokio; a
  JSON editor.
- **Concepts** — rendering (incl. "under the hood"), layout, event handling, widgets, storing
  state, the builder-lite pattern, backends (incl. a comparison page with a decision
  flowchart), and three application architectures (Elm, Flux, Component).
- **Recipes** — 23 how-tos: panic hooks, `color_eyre`, logging with `tracing`, config
  directories, CLI arguments, centring a widget, grid layouts, dynamic layouts, collapsing
  borders, snapshot testing, debugging widget state, releasing your app, migrating from
  tui-rs, and spawning an external editor.
- **Templates** — a walkthrough of the component template, file by file.
- **Highlights** — per-release notes back to v0.21.

**Crucially, the site's code samples are compiled, not pasted.** The prose `{{ #include
@code/... }}`s real files from a Cargo workspace in the same repo. **[verified]** **[inferred]**
This is the mechanism that stops a tutorial rotting silently — the thing that most often
wastes a beginner's afternoon.

**Examples in the library repo** **[verified]**: 32 runnable example *applications*, each its
own crate with its own README (demo, demo2, todo-list, user-input, input-form, popup, table,
scrollbar, weather, tracing, panic, inline, hyperlink, async-github, constraint-explorer,
flex, colors-rgb, custom-widget, advanced-widget-impl, …), plus 17 per-widget examples in
`ratatui-widgets/examples/`.

**Scaffolding** **[verified]**: `cargo generate ratatui/templates` offers Hello World, Simple,
Simple Async, Event Driven, Event Driven Async, and Component.

**Ecosystem** **[verified]**: [`awesome-ratatui`](https://github.com/ratatui/awesome-ratatui)
is an official-org curated list with **467 entries** across frameworks, widgets, utilities,
bindings and applications.

**Community** **[verified]**: Discord, a Matrix bridge (`#ratatui:matrix.org`), a dedicated
Discourse forum (`forum.ratatui.rs`), GitHub Discussions, a EuroRust 2024 conference talk, and
a contributing guide that explicitly addresses AI-generated contributions.

**The honest caveat.** Documentation freshness is *good, not perfect*. Most of the site's
compiled sample crates are on `ratatui = "0.30.2"` (current), but three are behind — the Elm
architecture sample and the async template counter on `0.28.1`, the async component template
on `0.29.0` — and the async-counter tutorial's prose shows a `Cargo.toml` pinning `ratatui =
"0.28.0"` while the code directory it includes from is on `0.30.2`. **[verified]** **[inferred]**
A beginner following the async tutorial should expect one or two version-drift papercuts.

Separately, **0.30 restructured the crate into a workspace** — `ratatui-core`,
`ratatui-widgets`, `ratatui-crossterm`, `ratatui-termion`, `ratatui-termwiz`,
`ratatui-termina`, `ratatui-macros`, with `ratatui` as the façade. **[verified]**
**[inferred]** Applications should keep depending on plain `ratatui`; the split matters to
widget authors. But it does mean tutorials, blog posts and Stack Overflow answers written
before 2025-12-26 describe a different import surface.

### cursive

Three hand-written tutorials in `doc/` (`tutorial_1.md` … `tutorial_3.md`), a GitHub wiki
(including the backends page), rustdoc, and **48 examples** in `cursive/examples/` covering
dialogs, menubars, selects, focus, themes, a minesweeper game, a TCP server and a debug
console. A README listing 14 third-party views and 23 showcase applications (`ncspot`,
`git-branchless`, `ripasso`, `grin-tui`, …). Gitter for chat. **[verified]**

**[inferred]** This is a respectable corpus — it would look excellent for almost any other
crate. Against ratatui it is roughly an order of magnitude smaller, has no structured book, no
tutorial on async, no scaffolding templates, and no compiled-sample guarantee.

### iocraft

README, rustdoc, and **15 examples** (table, form, calculator, weather, fullscreen, scrolling,
progress bar, `use_input`, `use_output`, context, borders, overlap, counter, hello world).
Codecov badge. GitHub Discussions. **[verified]** No book, no tutorial series, no templates.

**[inferred]** Appropriate for a two-year-old crate; thin for someone learning the language at
the same time as the library. Its React-shaped API is a real advantage *if* you already know
React — but the Rust concepts it hides (ownership across a component tree, `Send` futures) are
exactly the ones this project exists to learn.

### Verdict on this axis

**[inferred]** ratatui, by a very wide margin, on every sub-measure the ticket names: page
count, tutorial depth, example count, template availability, community surface, and
maintenance of the documentation itself.

## The hard part: a live child terminal inside a pane

Restating requirement 2, because it is where the real risk sits: the detail pane must host a
running `claude` process — visible, typed-into, interruptible — **without the list going
away**. Everything above is table stakes; this is the load-bearing capability.

### The known-good stack

**[verified]** — the following crates compose into exactly this, and there is a working
example of the whole thing:

| Crate | Role | Latest | Downloads/90d |
|---|---|---|---|
| [`portable-pty`](https://crates.io/crates/portable-pty) | Open a PTY, spawn the child in it, resize it | 0.9.0, 2025-02-11 | 4,189,174 |
| [`vt100`](https://crates.io/crates/vt100) | Parse the child's output into a screen model | 0.16.2, 2025-07-12 | 2,989,738 |
| [`tui-term`](https://crates.io/crates/tui-term) | Render that screen model as a ratatui widget | 0.3.4, 2026-04-07 | 266,558 |
| [`ansi-to-tui`](https://crates.io/crates/ansi-to-tui) | (alternative, simpler) ANSI text → `ratatui::text::Text` | 8.0.1, 2026-01-10 | 1,384,195 |

`tui-term` states its own architecture plainly:

> - Opening a pseudoterminal (pty/tty) and obtaining reading and writing handles to the
>   underlying process.
> - Reading the output from the process and using the vt100 crate to parse the output bytes.
> - Displaying the parsed output on the terminal screen within the tui-term library.
> - Handling user input and writing the input bytes directly to the writer handle of the
>   underlying process.
>
> — `docs/ARCHITECTURE.md`, a-kenji/tui-term **[verified]**

And it is explicit about the division of labour, which matches ratatui's philosophy:

> The `ratatui` crate does not enforce a specific input handling pattern. Consequently,
> handling input from the user is the responsibility of the consumer of the `tui-term`
> library. […] The output of the underlying process […] needs to be read by the consumer of
> tui-term.
>
> — same file **[verified]**

**The most directly relevant artefact found in this entire research:** `tui-term`'s
[`smux` example](https://github.com/a-kenji/tui-term/blob/development/examples/smux.rs) —
described in its examples README as *"a simple terminal multiplexer […] Uses: asynchronous
I/O using Tokio"* — is a working `#[tokio::main]` program combining `ratatui::init()`,
`Layout`/`Constraint`, `portable_pty::native_pty_system`, `tui_term::widget::PseudoTerminal`,
`tokio::sync::mpsc` and `spawn_blocking`. **[verified]** That is harness-launcher's detail
pane, demonstrated end to end, in the library's own example directory. Its sibling examples
`nested_shell_async.rs` (async nested shell) and `long_running.rs` (`top`, i.e. a
full-screen alternate-screen child) cover the other awkward cases. **[verified]**

### The risk in that stack

**[verified]** `tui-term`'s own README: *"This project is currently in active development and
should be considered a work in progress."* Its lifecycle helper (the `Controller`) is gated
behind an `unstable` feature and *"currently the support is limited to oneshot commands"* —
which is not what a long-running `claude` session is. Repository health: 228 stars, 19 forks,
7 open issues, one principal author, last commit 2026-08-01, tracking ratatui 0.30 promptly
(depends on `ratatui-core` 0.1.1 / `ratatui-widgets` 0.3.1).

**[inferred]** So the honest picture is: **the hardest requirement in tranche 1 is met by a
small, single-author, self-described work-in-progress crate.** It is well-designed, current,
and demonstrated — but it is the thinnest link in the chain, and it is worth knowing that
before committing. Two mitigations exist and are cheap to note:

1. `tui-term` is a *rendering* crate. The PTY (`portable-pty`, from the WezTerm project,
   4.2M downloads/90d) and the VT parsing (`vt100`, 3.0M/90d) — the parts that are genuinely
   hard — are both mature and separately usable. If `tui-term` stalled, what would need
   replacing is the smallest piece.
2. There is a second architecture, visible in the prior art below: **delegate the PTY to tmux**
   and let the TUI drive tmux rather than emulate a terminal itself.

### The other two candidates on this requirement

- **cursive** — [`cursive-multiplex`](https://crates.io/crates/cursive-multiplex), "a tmux like
  multiplexer for gyscos/cursive views", 0.7.0, last released 2024-08-12, 11,769 downloads/90d.
  **[verified]** **[inferred]** Note the description: it multiplexes *cursive views*, i.e. it
  is a pane splitter, not a terminal emulator. No cursive-native equivalent of the
  `portable-pty` + `vt100` + widget stack was found.
- **iocraft** — nothing found. No PTY or terminal-emulator component among its nine built-in
  components, none among its 15 examples, and no third-party ecosystem to speak of.
  **[verified]** (absence of evidence, from a full read of the component and example lists)

**[inferred]** This is the axis on which the field actually narrows to one.

## Prior art: this exact product already exists, several times

`awesome-ratatui` lists at least eight applications in harness-launcher's problem space —
multi-agent-session dashboards, most of them naming Claude Code explicitly. **[verified]**,
quoted from `awesome-ratatui/README.md`:

- **[bosun](https://github.com/yetidevworks/bosun)** — "A tmux-native TUI for orchestrating AI
  coding agent sessions (Claude Code, Codex) with live previews and per-session state."
- **[claudectl](https://github.com/mercurialsolo/claudectl)** — "Mission control for multiple
  Claude Code sessions with live dashboard, cost tracking, and budget enforcement."
- **[crmux](https://github.com/maedana/crmux)** — "A TUI viewer for monitoring and managing
  multiple Claude Code sessions in tmux."
- **[thurbox](https://github.com/Thurbeen/thurbox)** — "A TUI orchestrator for running multiple
  AI coding agents (Claude Code, Codex, and others) in persistent tmux sessions."
- **[iris](https://github.com/itzenata/iris-tui)** — "Live supervisor for every active Claude
  Code session — status, tokens, estimated cost, and one-pane approval of tool calls."
- **[agent-console](https://github.com/buhuipao/agent-console)** — "A local dashboard for Codex
  and Claude Code."
- **[Reeve](https://github.com/Dancode-188/reeve)** — "A terminal cockpit for AI agents: watch
  a run live, score it, and step in when it goes sideways."
- **[trex](https://github.com/blackopsrepl/trex)** — "A fast tmux session manager with fuzzy
  finding, per session stats and AI Agent tracking."

**[inferred]** Three things follow, and they are probably the most actionable findings here.

1. **The architecture is proven on ratatui.** Whatever else is uncertain, "multi-pane live
   dashboard over several concurrent Claude Code sessions, in Rust, on ratatui" is not
   speculative.
2. **Four of the eight put "tmux" in their one-line description.** That is a strong hint that
   delegating session hosting and persistence to tmux is the path of least resistance, and
   it interacts directly with the vision's "Surviving the app closing" wish (deferred in
   tranche 1, but explicitly on the pile) — a tmux-backed spawn survives the app closing
   almost for free. It also trades away the vision's sandboxing wish being *contained by the
   app*, and adds a hard external dependency.
3. **The equivalent list does not exist for cursive.** ⚠️ **State the bias first:**
   `awesome-ratatui` is a *ratatui-only* list, so finding eight ratatui apps in it proves
   nothing about the others by itself. The comparison that is actually available is cursive's
   own curated showcase — 23 applications, whose subject matter is Spotify clients, password
   managers, sudoku, Wikipedia browsers, a git workflow tool and a note-taker. **Nothing in
   this problem space at all.** **[verified]** iocraft publishes no showcase list.
   **[verified]** **[inferred]** So the honest form of this finding is not "eight teams
   chose ratatui over cursive" — it is "the recent cohort of tools in this exact space is
   visible on ratatui and invisible on cursive", which is weaker evidence but still points
   the same way, and is partly explained by cursive simply having less momentum overall.

*Not verified:* I did not read these projects' source. Their descriptions are quoted from
`awesome-ratatui`; the claim that they are ratatui-based rests on their inclusion in
ratatui's own curated list, which is submission-based and unaudited.

## Which one is a Rust beginner most likely to succeed with

Issue #6 asks for this plainly, so: **ratatui**, and it is not close. **[inferred]**, from the
verified facts above.

To be clear about what this is: it answers the ticket's *"say plainly which option a Rust
beginner is most likely to succeed with, and why"*. **It is not the decision** — that is made
with the human, weighing things this document does not (taste, appetite for risk, and the
embed-vs-tmux question below, which may well matter more).

**Why:**

1. **It is the only one with an off-the-shelf answer to the hard requirement.** A live,
   typed-into `claude` process in a pane alongside a live list needs `portable-pty` + `vt100`
   + a widget to paint the result, and that widget (`tui-term`) is a ratatui widget with a
   working multiplexer example. Cursive has a pane splitter, not a terminal emulator; iocraft
   has neither. Nothing stops someone writing a `vt100`-backed cursive `View` from scratch —
   but "write the terminal emulator yourself" is the opposite of what a beginner-weighted
   comparison should recommend. **The beginner-weighting barely gets to apply: the
   requirement filters the field before it does.**
2. **The documentation gap is the size of the thing being weighted.** 169 structured pages
   with compiled samples, an eight-part async tutorial, 49 runnable examples, six scaffolding
   templates, 467 curated ecosystem entries — against 48 examples and three tutorial pages
   (cursive) or 15 examples and a README (iocraft). Issue #6 says maturity and documentation
   matter more than flexibility; this is where that instruction cashes out.
3. **When you get stuck, there are people and answers.** 22k stars, 15.4M downloads a quarter,
   64 distinct commit authors in a year, four maintainers, a Discord, a Matrix room, a forum.
   For a beginner the practical form of this is: your error message has been seen before, and
   the LLM you ask about it has read a lot of ratatui.
4. **Not owning the event loop is the right trade *here*.** It is genuinely more work than
   cursive's `siv.run()`, and for a form-and-dialog app cursive would be the kinder library.
   But harness-launcher's loop has to merge ~20 PTY streams, process exits, keystrokes and a
   frame timer — a loop you would end up writing anyway. Better to write it in the open than
   to smuggle it past a framework that wanted to own it.
5. **Immediate mode is the easier mental model for someone learning Rust.** No retained widget
   tree means no long-lived aliased UI objects, which means far less `Rc<RefCell<…>>` and far
   less fighting the borrow checker over your own UI. State lives in one struct; the frame is
   a pure function of it. **[inferred]**, but it follows directly from ratatui's own stated
   advantage — *"Without a persistent widget state, your UI logic becomes a direct reflection
   of your application state"* **[verified]**.

**What a beginner should be warned about, in fairness:**

- You write the event loop, the input dispatch and the focus tracking. Cursive gives you all
  three.
- Pre-0.30 material (before 2025-12-26) describes a single-crate import surface; pre-2023
  material says `tui`, not `ratatui`, and is obsolete.
- Two or three of the website's async samples lag the current release.
- The crate is 0.x. It has shipped **eleven** breaking minor lines since the fork (0.20 in
  2023-03 through 0.30 in 2025-12) and maintains a `BREAKING-CHANGES.md` to manage it.
  **[verified]** Expect migration work over the life of a long-running project — though the
  cadence has slowed markedly: **fourteen months** separated 0.29 (2024-10-21) from 0.30
  (2025-12-26), with only patch releases since. **[verified]** **[inferred]** That reads as an
  API settling down rather than a project stalling, given the commit activity above.

**The runner-up, and when it would win.** Cursive — mature since 2016, batteries included,
focus and forms for free — would be the better answer to a *different* tranche 1: one where
the detail pane showed a summary rather than a live terminal. Its two-year-stale crates.io
release and single-maintainer bus factor would still need weighing. **[inferred]**

**iocraft** is a genuinely nice library and the wrong tool here: too young, too thin a corpus,
no off-the-shelf path to the hard requirement, and its React-shaped abstraction hides
precisely the Rust concepts this project exists to learn. **[inferred]**

## What I could not determine

Stated so the gaps are not mistaken for absences.

- **Issue-response latency and PR-merge times** for any project. `api.github.com` was blocked
  by this session's egress policy; open-issue *counts* came from an authenticated tool but
  issue *timelines* did not.
- **Contributor counts as GitHub computes them.** The author counts above are from
  `git log` over full-history clones — distinct commit-author emails, which under-counts
  reviewers and issue triagers and mis-counts anyone who changed email.
- **Community size in people.** Discord/Matrix/forum member counts were not reachable.
- **iocraft's MSRV** — not declared in either the workspace or the crate `Cargo.toml`.
- **Whether the eight prior-art applications are actually ratatui-based**, and which of them
  embed a PTY versus drive tmux. Their inclusion in `awesome-ratatui` is submission-based, and
  I did not read their source. **This is the highest-value follow-up in this document** — two
  or three of those repositories, read properly, would settle the embed-vs-delegate question
  faster than any amount of further library comparison.
- **Whether `tui-term` handles everything Claude Code's UI does** — bracketed paste, mouse
  reporting, kitty keyboard protocol, resize semantics under a live child. `vt100` is a
  general VT parser and the `long_running` example covers alternate-screen children, but I
  did not test.
- **Performance at 15–20 concurrent PTYs.** No benchmark found for that shape. `tui-term`
  ships criterion/divan/iai benches, but for widget rendering, not multi-PTY throughput.

## Questions this research does not answer

Flagged for the decision conversation, not resolved here — per issue #6, this is fact-finding.

1. **Embed the PTY, or delegate to tmux?** The two architectures visible in the prior art.
   This is a bigger decision than the framework choice and probably precedes it. It touches
   the vision's sandboxing wish, its "surviving the app closing" wish, and whether the tool
   has a hard external dependency.
2. **If embedding: is `tui-term` an acceptable dependency**, or should the app own the
   `vt100`-to-buffer rendering directly and depend only on `portable-pty` + `vt100`?
3. **Does the "status" requirement** — working vs stopped, tranche 1's only two statuses —
   need anything from the framework at all? **[inferred]** It appears to be a process/PTY
   question, not a UI one.
4. **What does `terminal.draw()` cost** when the detail pane is a full-screen VT grid updating
   at Claude Code's output rate, and what frame rate is actually wanted? The `async-github`
   example's 60 FPS interval is a starting point, not a recommendation.

## Sources

All fetched or cloned on 2026-08-03. Marked ⛔ where this session's egress policy blocked the
canonical URL and the underlying repository file was read instead.

**Ratatui**
- Repository: <https://github.com/ratatui/ratatui> — `README.md`, `MAINTAINERS.md`,
  `Cargo.toml`, `ARCHITECTURE.md`, `BREAKING-CHANGES.md`, `examples/`,
  `ratatui-widgets/examples/`, `ratatui-core/src/layout/layout.rs`
- Documentation site: <https://ratatui.rs> ⛔ — read from
  <https://github.com/ratatui/ratatui-website> (`src/content/docs/**`, `code/**`)
- Templates: <https://github.com/ratatui/templates>
- Curated ecosystem: <https://github.com/ratatui/awesome-ratatui>
- API docs: <https://docs.rs/ratatui> ⛔
- Crate metadata: <https://crates.io/api/v1/crates/ratatui> (and `ratatui-core`,
  `ratatui-widgets`, `ratatui-crossterm`, `ratatui-termion`, `ratatui-macros`)

**Cursive**
- Repository: <https://github.com/gyscos/cursive> — `README.md`, `CHANGELOG.md`,
  `cursive/Cargo.toml`, `cursive/examples/`, `cursive-core/src/cursive_root.rs`,
  `cursive-core/src/cursive_run.rs`, `cursive-core/src/views/`
- Crate metadata: <https://crates.io/api/v1/crates/cursive>, `.../cursive-core`,
  `.../cursive-multiplex`, `.../cursive-async-view`, `.../cursive_table_view`

**iocraft**
- Repository: <https://github.com/ccbrown/iocraft> — `README.md`, `Cargo.toml`, `examples/`,
  `packages/iocraft/src/hooks/`, `packages/iocraft/src/components/`,
  `packages/iocraft/src/element.rs`, `packages/iocraft/src/render.rs`
- Crate metadata: <https://crates.io/api/v1/crates/iocraft>

**tui-realm**
- Repository: <https://github.com/veeso/tui-realm> — `README.md`,
  `crates/tuirealm/Cargo.toml`, `crates/tuirealm/examples/`, `crates/tuirealm/docs/en/`
- Crate metadata: <https://crates.io/api/v1/crates/tuirealm>, `.../tui-realm-stdlib`

**Backends and the PTY stack**
- crossterm: <https://github.com/crossterm-rs/crossterm> — `README.md`, `Cargo.toml`,
  `examples/`; <https://crates.io/api/v1/crates/crossterm>
- termwiz: <https://raw.githubusercontent.com/wezterm/wezterm/main/termwiz/README.md>;
  <https://crates.io/api/v1/crates/termwiz>
- termina: <https://crates.io/api/v1/crates/termina>
- portable-pty: <https://crates.io/api/v1/crates/portable-pty>
- vt100: <https://crates.io/api/v1/crates/vt100>
- tui-term: <https://github.com/a-kenji/tui-term> — `README.md`, `Cargo.toml`,
  `docs/ARCHITECTURE.md`, `examples/` (incl. `smux.rs`, `nested_shell_async.rs`,
  `long_running.rs`); <https://crates.io/api/v1/crates/tui-term>
- ansi-to-tui, tui-input, tui-textarea, crokey, terminput, taffy, smol: crates.io API

**Ruled out**
- tui-rs (archived): <https://github.com/fdehau/tui-rs>; <https://crates.io/api/v1/crates/tui>
- zi, dioxus-tui, widgetui, ratatui-kit, rat-salsa, pancurses, ncurses: crates.io API

**Repository metadata** (stars, forks, open issues, archived flag, licence, last push) came
from the GitHub REST API via an authenticated tool. **Commit activity** came from `git log`
over `--filter=blob:none` full-history clones of ratatui, cursive, iocraft and tui-realm.
