# Surfacing a live child TUI inside a Rust pane

> **Research note — fact-finding only.** Resolves [issue #7](https://github.com/sylmarien/harness-launcher/issues/7).
> This document does **not** recommend an architecture. That decision is issue #9 and belongs to the human.
>
> Every claim is tagged:
>
> - **[V]** — verified against a primary source (crate source, official docs, issue tracker), cited inline.
> - **[I]** — inference drawn from **[V]** facts. The reasoning is shown so it can be checked.
> - **[R]** — reported by a third party (bug report, blog) and not independently confirmed.
>
> Sources were read between the crate repositories, crates.io API, and the official Claude Code docs.
> Where a source is a bug report, it is cited as a bug report — not as settled behaviour.

## Short answer

**Yes. The technique works, is well-trodden, and at least six shipping Rust projects do it today.** **[V]**

The pieces exist and compose: `portable-pty` owns the pty, a terminal-emulation crate turns the child's
byte stream into a screen model, `tui-term` (or hand-written equivalent) blits that model into a
`ratatui` buffer, and keystrokes are written back into the pty master as raw bytes.

The sharp edges are **not** in "can it be done". They are in the fidelity of the emulation and in what
happens at 15–20 concurrent instances. Specifically:

1. The most popular screen-state crate (`vt100`) **structurally cannot answer terminal queries** —
   it has no path to write bytes back to the child. Claude Code is reported to issue cursor-position
   queries. **[V]/[R]**
2. **Resize does not reflow.** `vt100` truncates and pads on width change; content is destroyed. **[V]**
3. **Mouse forwarding is a re-encoding job the app must do itself**, and it is the single most
   bug-prone part in every prior-art project examined. **[V]**
4. **Memory is the binding constraint at 20 panes**, and it is dominated by scrollback, not by the
   visible grid. Claude Code's *default* renderer appends to scrollback rather than using the
   alternate screen — which is the expensive case. **[V]/[I]**

Details and citations below.

---

## 1. The mechanism, layer by layer

Four layers, each independently swappable:

| Layer | Job | Candidate crates |
| --- | --- | --- |
| **PTY** | Allocate a pty pair, spawn the child with the slave as its controlling terminal, expose master read/write, resize | `portable-pty`, `pty-process`, `rustix-openpty` + `nix` |
| **Emulation** | Consume the child's bytes, maintain a screen model (grid, cursor, modes, scrollback) | `vt100`, `alacritty_terminal`, `wezterm-term`, `avt`, `termwiz`, `par-term-emu-core-rust` |
| **Render** | Copy the screen model into the host TUI's frame buffer | `tui-term`, or a hand-rolled `ratatui` widget |
| **Input** | Capture host key/mouse events, re-encode them as the child expects, write to the pty master | hand-written; `wezterm-term` and `termwiz` provide encoders |

The layers are genuinely decoupled. `wezterm-term`'s README makes the boundary explicit: *"This crate does
not provide any kind of gui, nor does it directly manage a PTY; you provide a `std::io::Write`
implementation that could connect to a PTY, and supply bytes to the model via the `advance_bytes`
method."* **[V]**
([term/README.md](https://raw.githubusercontent.com/wezterm/wezterm/main/term/README.md))

Note that sentence: `wezterm-term` takes a **writer**. That is not decoration — see §7.1.

---

## 2. PTY crates

### 2.1 `portable-pty` — the default choice in prior art

- **Version 0.9.0, published 2025-02-11. ~4.19M downloads in the last 90 days.** **[V]**
  ([crates.io API](https://crates.io/api/v1/crates/portable-pty))
- Lives in the wezterm monorepo (`wezterm/pty/`), MIT, maintained by Wez Furlong. **[V]**

**API shape** **[V]** ([pty/src/lib.rs](https://github.com/wezterm/wezterm/blob/main/pty/src/lib.rs)):

- `PtySize { rows: u16, cols: u16, pixel_width: u16, pixel_height: u16 }`. The doc comments on the
  pixel fields say: *"Note that some systems never fill this value and ignore it."*
- `PtySystem::openpty(size) -> PtyPair { master, slave }`
- `MasterPty::resize(size)`, `get_size()`, `try_clone_reader() -> Box<dyn Read + Send>`,
  `take_writer() -> Box<dyn Write + Send>`, `as_raw_fd()`
- `SlavePty::spawn_command(CommandBuilder) -> Box<dyn Child>`
- `Child::try_wait()`, `wait()`, `process_id()`, `kill()`

**Unix implementation details worth knowing** **[V]**
([pty/src/unix.rs](https://raw.githubusercontent.com/wezterm/wezterm/main/pty/src/unix.rs)):

- `resize()` is `ioctl(master_fd, TIOCSWINSZ, &winsize)`.
- The pty pair comes from `libc::openpty()` (not `posix_openpt`).
- In `pre_exec()` the child calls `setsid()` then `ioctl(0, TIOCSCTTY, 0)`. The source carries the
  comment: *"Failure to do this means that delivery of SIGWINCH won't happen when we resize the
  terminal, among other undesirable effects."*
- On child exit, reads return `EIO`, and the crate deliberately maps it to EOF:
  *"EIO indicates that the slave pty has been closed. Treat this as EOF so that
  `std::io::Read::read_to_string` and similar functions gracefully terminate when they encounter
  this condition."*
- It closes inherited fds ≥ 3 before exec, with comments about *"On Big Sur, Cocoa leaks various
  file descriptors to child processes"* and *"On Linux, gnome/mutter leak shell extension fds to
  wezterm too."*

**`CommandBuilder` does not set `TERM`.** It inherits the parent environment via `std::env::vars_os()`
and offers `env_clear()`. **[V]**
([pty/src/cmdbuilder.rs](https://raw.githubusercontent.com/wezterm/wezterm/main/pty/src/cmdbuilder.rs))
This matters: by default the child inherits the *outer* terminal's `TERM`, `COLORTERM`,
`TERM_PROGRAM`, `TERM_PROGRAM_VERSION` — and will therefore believe it is talking to Ghostty/iTerm2/
whatever the user actually launched from, not to your emulator. **[I]** See §7.3.

**Windows:** `portable-pty` wraps ConPTY (`pty/src/win/conpty.rs`). The `PtySystem` trait exists
specifically because *"an application to work with multiple possible Pty implementations at runtime,
which is important on Windows systems which have a variety of implementations."* **[V]**
The ConPTY source carries no explicit caveat comments beyond `"writer already taken"` on a second
`take_writer()`. **[V]**

**Reads are blocking, and that is by design.** The maintainer's answer in
[wezterm discussion #3739](https://github.com/wezterm/wezterm/discussions/3739) is that pty reads
block and you must use *"spawn_blocking or similar to process reading or writing from/to a pty"*;
the working pattern is a dedicated reader per pty rather than one shared reader. **[V]**
A single shared reader thread across several ptys hangs. **[V]**

### 2.2 Alternatives

- **`pty-process`** (doy) — wraps `std::process::Command` / `tokio::process::Command`, allocating a
  pty and making the child a session leader with the pty as controlling terminal. Has both blocking
  and async APIs; async is behind the `async` feature. Has `resize()`. **[V]**
  ([doy/pty-process](https://github.com/doy/pty-process)). Unix-oriented; no ConPTY story documented.
- **`rustix-openpty`** — a thin wrapper over `rustix::pty` on Linux and `libc::openpty` elsewhere.
  Just opens the pty; no spawning, no resize helper, no Windows. This is what Alacritty uses
  underneath. **[V]** ([sunfishcode/rustix-openpty](https://github.com/sunfishcode/rustix-openpty))
- **`alacritty_terminal::tty`** — Alacritty's own pty layer, including an `EventedPty` abstraction
  for its `EventLoop`. Comes bundled with the emulation crate. **[V]**

**Assessment [I]:** `portable-pty` is the pragmatic default — it is what every prior-art project in
§9 uses, it has the widest platform coverage, and its resize/controlling-terminal handling is the
part that is easy to get subtly wrong by hand. Its cost is a dependency on a monorepo crate whose
release cadence is irregular (0.8.1 in 2023 → 0.9.0 in 2025). **[V]**

---

## 3. Terminal-emulation / screen-state crates

This is the layer where the real choice lives. Three distinct categories:

### 3.1 Parser only (no screen model)

**`vte`** (alacritty) — *"The parser is implemented according to Paul Williams' ANSI parser state
machine. The state machine doesn't assign meaning to the parsed data and is thus not itself
sufficient for writing a terminal emulator."* You implement the `Perform` trait and build the screen
model yourself. **[V]** ([alacritty/vte](https://github.com/alacritty/vte))

This is what **Zellij** does: its `Grid` struct implements `vte::Perform` and holds the viewport,
scrollback, cursor and styles itself. **[R]** (secondary; see §9.1)

Cost: you write the terminal emulator. Benefit: total control over reflow, modes, and query
responses.

### 3.2 Parser + screen model, no writer

**`vt100`** (doy) — the popular choice.

- **Version 0.16.2, published 2025-07-12. ~2.99M downloads in the last 90 days.** **[V]**
  ([crates.io API](https://crates.io/api/v1/crates/vt100))
- API: `Parser::new(rows, cols, scrollback_len)`, `new_with_callbacks(..., callbacks)`,
  `process(&[u8])`, `screen() -> &Screen`. **[V]**
  ([src/parser.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/parser.rs))
- `Screen` exposes `cell(row, col)`, `contents_formatted()`, `contents_diff(prev)`, `set_size()`,
  `alternate_screen()`, `mouse_protocol_mode()`, `mouse_protocol_encoding()`, `application_keypad()`,
  `application_cursor()`, `bracketed_paste()`, `hide_cursor()`. **[V]**
  ([src/screen.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/screen.rs))
- `Callbacks` trait: `audible_bell`, `visual_bell`, `resize`, `set_window_icon_name`,
  `set_window_title`, `copy_to_clipboard`, `paste_from_clipboard`, `unhandled_char`,
  `unhandled_control`, `unhandled_escape`, `unhandled_csi`, `unhandled_osc`. **[V]**
  ([src/callbacks.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/callbacks.rs))

**Critical structural fact: there is no callback and no return value that lets the emulator send bytes
back to the child.** `process()` returns `()`. No `Callbacks` method carries a writer. **[V]**
Consequences in §7.1.

**What `vt100` implements** **[V]**
([src/perform.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/perform.rs)):
CUU/CUD/CUF/CUB (`A`/`B`/`C`/`D`), CUP (`H`), CHA (`G`), VPA (`d`), ICH (`@`), DCH (`P`), IL (`L`),
DL (`M`), ED (`J`), EL (`K`), SU (`S`), SD (`T`), SGR (`m`), DECSET/DECRST (`?h`/`?l`),
DECSED/DECSEL (`?J`/`?K`), ESC `7`/`8`/`=`/`>`/`M`/`c`, OSC 0/1/2 (titles), OSC 52 (clipboard).

**What it does not implement** **[V]** (same file — these fall through to `unhandled_*`):
DSR (Device Status Report), DA (Device Attributes) and cursor-position reports; XTVERSION;
XTGETTCAP; Sixel; the Kitty keyboard protocol; synchronized output (DEC 2026).
Also missing: HVP (`CSI f`), which is treated as unhandled rather than as an alias for CUP —
[open issue #26](https://github.com/doy/vt100-rust/issues/26). **[V]**

**Maintenance:** doy shipped 0.16.0–0.16.2 in July 2025 after a two-and-a-half-year gap (0.15.2 was
February 2023). **[V]** There are published forks — `mprocs-vt100`, `vt100-ctt`, `vt100_yh`,
and [ChrisTitusTech/vt100-rust](https://github.com/ChrisTitusTech/vt100-rust) whose repo description
is literally *"Fix functionality on abandoned vt100 crate"*. **[V]** The existence of several
independent forks is the honest signal here: upstream responsiveness has historically been thin.
**[I]**

**Open bugs in `vt100` that bear directly on this use case** **[V]**
([issue list](https://github.com/doy/vt100-rust/issues)):

| # | Title | Why it matters here |
| --- | --- | --- |
| [#37](https://github.com/doy/vt100-rust/issues/37) | Unexpected panic in `Screen::text` when the screen is narrower than the glyph being drawn | Narrow panes + wide glyphs = panic |
| [#28](https://github.com/doy/vt100-rust/issues/28) | `Row::clear_wide` can panic after resizing truncates a wide character at the last column | Resize + CJK/emoji + `CSI K` = `index out of bounds: the len is 1 but the index is 1` |
| [#26](https://github.com/doy/vt100-rust/issues/26) | Missing HVP (`CSI f`) support | Some apps use `CSI f` instead of `CSI H` |
| [#17](https://github.com/doy/vt100-rust/issues/17) | Missing modes in `MouseProtocolEncoding` | SGR-Pixel and urxvt encodings absent |

Issue #28 is worth reading in full: `Row::resize()` shrinks with `Vec::resize` and does not clear a
wide character whose continuation cell was removed, leaving a cell flagged `is_wide()` with no
`col + 1`; `Row::clear_wide()` then indexes it unchecked. **[V]** That is a resize path, and this
product resizes panes.

### 3.3 Parser + screen model + writer (can answer the child)

**`alacritty_terminal`** — *"Library for writing terminal emulators"*. Version **0.26.0, published
2026-04-06; ~412k downloads in 90 days.** **[V]**
([crates.io API](https://crates.io/api/v1/crates/alacritty_terminal))

The key difference from `vt100` is its `Event` enum, which explicitly carries data destined for the
pty **[V]**
([alacritty_terminal/src/event.rs](https://raw.githubusercontent.com/alacritty/alacritty/master/alacritty_terminal/src/event.rs)):

```rust
pub enum Event {
    MouseCursorDirty,
    Title(String),
    ResetTitle,
    ClipboardStore(ClipboardType, String),
    /// Request to write the contents of the clipboard to the PTY.
    ClipboardLoad(ClipboardType, Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
    /// Request to write the RGB value of a color to the PTY.
    ColorRequest(usize, Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>),
    /// Write some text to the PTY.
    PtyWrite(String),
    /// Request to write the text area size.
    TextAreaSizeRequest(Arc<dyn Fn(WindowSize) -> String + Sync + Send + 'static>),
    CursorBlinkingChange,
    Wakeup,
    Bell,
    Exit,
    ChildExit(ExitStatus),
}
```

`PtyWrite`, `ColorRequest` and `TextAreaSizeRequest` are exactly the query-response machinery `vt100`
lacks. **[V]**

Caveat: it is a library extracted from an application, and its API is not stability-guaranteed —
version numbers moved 0.11 → 0.26 over the period examined. **[V]** A user in
[alacritty#8273](https://github.com/alacritty/alacritty/issues/8273) reports needing arbitrary
`sleep`s to synchronise pty output when driving it as a library, and specifically flags *"testing
terminal applications (like Reedline and Crossterm-based tools) that expect OSC responses from the
host"*. **[R]** — the thread as fetched contained no maintainer answer.

**`wezterm-term`** — full featured: *"terminal escape sequence parsing, keyboard and mouse input
encoding, a model for the screen cells including scrollback, sixel and iTerm2 image support, OSC 8
Hyperlinks and a wide range of terminal cell attributes."* Takes a `std::io::Write` for responses.
**[V]** ([term/README.md](https://raw.githubusercontent.com/wezterm/wezterm/main/term/README.md))

**Blocker: `wezterm-term` is not published on crates.io.**
[wezterm#6663](https://github.com/wezterm/wezterm/issues/6663) is an open request to publish it,
filed by the author of the `yeehaw` TUI library who needed *"querying terminal mouse state"* —
functionality they could not get from `termwiz` or `vt100`. That issue was still open, referencing an
earlier closed discussion (#2799) where the maintainer had reservations. The requester's workaround
was to publish their own fork, `vt100_yh`, noting *"there are already other published forks too."*
**[V]** Third-party republished forks exist (e.g. `tattoy-wezterm-term`). **[V]**

**`termwiz`** (published, same monorepo) — provides an escape-sequence parser that *"decodes
inscrutable escape sequences and gives them semantic meaning"*, a `Surface` that *"models a terminal
display"* with `Cell`s, `Capabilities` probing, a `Terminal` trait over Unix TTYs and the Windows
console, and delta synchronisation between `Surface`s. **[V]**
([termwiz/README.md](https://raw.githubusercontent.com/wezterm/wezterm/main/termwiz/README.md))
It is a screen model but not the same thing as `wezterm-term`'s full emulator — this is the gap
issue #6663 is about. **[I]**

**`avt`** (asciinema virtual terminal) — ANSI parser on the Williams state diagram, primary and
alternate screen buffers, targets the sequence set common to xterm/GNOME Terminal/WezTerm/Alacritty/
iTerm/Ghostty. Explicitly out of scope: *"input handling and rendering"*. Has `cargo bench`.
**[V]** ([asciinema/avt](https://github.com/asciinema/avt)) It does not claim to replicate any
specific terminal exactly.

**`par-term-emu-core-rust`** — the most featureful thing found, MIT, pure Rust core (PyO3 bindings
optional), at v0.45.0. Claims VT100/220/320/420/520, 24-bit colour, strikethrough/blink/dim,
grapheme clusters, alternate screen, *multiple mouse tracking modes and encodings*, bracketed paste,
focus tracking, Sixel, iTerm2 inline images, Kitty graphics, OSC 8, OSC 52, OSC 133,
**Kitty keyboard protocol**, **synchronized updates**, and — uniquely among the crates surveyed —
*"Full Terminal Reflow on Width Resize: Both scrollback AND visible screen content now reflow when
terminal width changes."* **[V]**
([paulrobello/par-term-emu-core-rust](https://github.com/paulrobello/par-term-emu-core-rust))
Unassessed for maturity, adoption, or API stability; it did not appear in any prior-art project
examined. **[I]**

### 3.4 The render layer: `tui-term`

A `ratatui` widget wrapping a screen model. Depends on `ratatui`, `portable-pty` and `vt100`. **[V]**
([a-kenji/tui-term](https://github.com/a-kenji/tui-term))

Its own README says: *"This project is currently in active development and should be considered a
work in progress."* **[V]** Its ARCHITECTURE.md says the vt100 crate is the only supported backend —
*"Because of the initial complexity we limit it to the one crate currently"* — with more general
abstractions planned. **[V]**
([docs/ARCHITECTURE.md](https://raw.githubusercontent.com/a-kenji/tui-term/development/docs/ARCHITECTURE.md))
The source has since grown `Screen` and `Cell` traits, so the backend seam is partly there. **[V]**
([src/widget.rs](https://raw.githubusercontent.com/a-kenji/tui-term/development/src/widget.rs))

It renders by iterating every cell of the pane every frame **[V]**
([src/state.rs](https://raw.githubusercontent.com/a-kenji/tui-term/development/src/state.rs)):

```rust
for row in 0..rows {
    for col in 0..cols {
        if let Some(screen_cell) = screen.cell(row, col) {
            let cell = &mut buf[(buf_col, buf_row)];
            screen_cell.apply(cell);
        }
    }
}
```

`tui-term` explicitly does **not** own input: *"The `ratatui` crate does not enforce a specific input
handling pattern"*; consumers handle input and feed output to vt100 themselves. **[V]**

It also does not own the pty lifecycle in any general way — the `controller` feature is behind the
`unstable` flag and *"currently the support is limited to oneshot commands."* **[V]** A long-lived
interactive agent session is not a oneshot command. **[I]**

---

## 4. Resize

### 4.1 The mechanism works, and it is one call

The pane decides the size; you push it down. `MasterPty::resize(PtySize)` issues
`ioctl(TIOCSWINSZ)` on the master **[V]**, and the kernel delivers `SIGWINCH` to the pty's foreground
process group **[V]** (the `portable-pty` source comment quoted in §2.1 states this is precisely why
`setsid()` + `TIOCSCTTY` are required; general behaviour corroborated by
[TIOCSWINSZ(2const)](https://www.man7.org/linux/man-pages/man2/TIOCSWINSZ.2const.html) **[R]** — the
man page itself returned 403 to automated fetch, so this specific citation is second-hand).

The child re-reads its size and redraws. **This is the same path tmux, screen and ssh use.** **[I]**

**There is no scaling problem.** The pane's dimensions *become* the child's dimensions; you never
render an 80×24 child into a 40×12 hole. **[I]** The correct sequence is: resize the emulator's
screen model **and** the pty, together, to the pane's inner area.

### 4.2 What actually goes wrong

**`vt100` does not reflow.** From `Grid::set_size` **[V]**
([src/grid.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/grid.rs)):

```rust
pub fn set_size(&mut self, size: Size) {
    if size.cols != self.size.cols {
        for row in &mut self.rows {
            row.wrap(false);
        }
    }
    // ... row.resize(size.cols, crate::Cell::new()) per row
}
```

Width change clears every row's `wrapped` flag, then truncates or pads each row to the new column
count. Existing content is **not** rewrapped; narrowing destroys the overflow. Scrollback rows are
not touched; saved cursor positions are clamped. **[V]**

Practical consequence: **narrowing a pane silently eats the right-hand side of everything already on
screen and in history, and it never comes back when you widen again.** **[I]** For a dashboard where
the user toggles between "spawn pane wide" and "spawn pane narrow beside the list", this is visible,
lossy, and permanent.

Mitigations that exist in principle: keep the pane's pty width fixed regardless of the visual pane
width and clip/scroll horizontally instead of resizing (costs fidelity — the child lays out for a
width the user cannot see); or use an emulator that reflows (`par-term-emu-core-rust` claims to,
§3.3); or accept the loss. **[I]** Not evaluated here — that is #9's call.

**Resize is also a crash path.** `vt100` issue #28 (§3.2) is a panic reachable by resizing a pane
containing a wide character at the last column, then issuing `CSI K`. **[V]** Claude Code output
contains box-drawing and emoji routinely. **[I]**

**Resize storms.** A drag-resize in the host TUI generates a resize per frame; each one is a
`TIOCSWINSZ` and a `SIGWINCH` to the child, and Node/Ink-based children redraw fully on each.
Debouncing is the standard remedy. **[I]** — not directly evidenced, flagged as a thing to check.

---

## 5. Mouse input

This is the messiest area in the whole investigation, and every prior-art project has bled on it.

### 5.1 What the app has to do

Mouse events are not forwarded; they are **translated and re-encoded**. The chain is:

1. The host TUI must enable mouse capture with the *outer* terminal (`crossterm::event::EnableMouseCapture`;
   mouse and focus events are off by default in crossterm). **[V]**
   ([crossterm::event](https://docs.rs/crossterm/latest/crossterm/event/index.html) — via search
   summary **[R]**, docs.rs blocked automated fetch)
2. Decide whether the event belongs to the host (pane border, dashboard list) or the child.
   Zellij needed [PR #1584](https://github.com/zellij-org/zellij/pull/1584) *"avoid forwarding mouse
   events on pane border"* to get this right. **[V]**
3. Translate host screen coordinates into pane-local, 1-based cell coordinates.
4. Check what the child asked for: `Screen::mouse_protocol_mode()` and
   `mouse_protocol_encoding()` in `vt100`. **[V]**
5. Encode in that protocol and write to the pty master.

Step 4 and 5 are where it breaks.

### 5.2 Concrete known failures

- **Tracking mode and encoding are independent, and implementations conflate them.**
  [zellij#1633](https://github.com/zellij-org/zellij/issues/1633) enumerates three bugs at once:
  (a) selecting SGR coordinates (`CSI ? 1006 h`) should not by itself emit events without a tracking
  mode active; (b) zellij treated bare `CSI ? 1006 h` as if it were `CSI ? 1002 ; 1006 h`;
  (c) zellij failed to parse multi-parameter DECSET at all — *"is reading only the first param; it
  needs to loop over all of the params."* **[V]** Apps commonly send `CSI ? 1002 ; 1006 h` as one
  sequence.
- **`vt100`'s `MouseProtocolEncoding` is incomplete** — SGR-Pixel and urxvt encodings are missing.
  [vt100#17](https://github.com/doy/vt100-rust/issues/17), open since Dec 2024. **[V]**
- **Zellij's mouse forwarding is SGR-only.** **[R]** (search summary of zellij changelog)
- **The wheel is a three-way routing decision, and getting it wrong breaks Claude Code specifically.**
  [renga#52](https://github.com/suisya-systems/renga/issues/52) is the clearest single artefact found
  for this product's exact use case. Verbatim shape of the problem: the multiplexer treated all wheel
  events from the focused pane as vt100 scrollback scrolling, which works for shell output but fails
  for alternate-screen apps — *"Alternate screen applications don't have vt100 scrollback, so
  scrolling does nothing"* and *"These apps can't receive mouse events, so they can't handle
  scrolling themselves."* It notes **Claude Code 2.1.110+ `/tui fullscreen` uses wheel-based
  conversation history scrolling**, which the multiplexer was swallowing. The fix required
  per-event branching on (alt-screen state × mouse protocol mode), SGR encoding
  (`\x1b[<64;<col>;<row>M` for wheel-up), routing to the *hovered* pane rather than the focused one,
  and an opt-out env var. **[V]**
- **The same class of bug hit Codex.** [openai/codex#2836](https://github.com/openai/codex/issues/2836):
  *"Entering the terminal's alternate screen (alt-buffer) means the application's output is rendered
  into a separate buffer that is not part of the terminal emulator's main scrollback"*, and the app
  enabling alternate scroll (`CSI ? 1007 h`) redirects wheel events to an app that does not handle
  them — so the events are simply lost. **[V]**

**Summary [I]:** mouse is not a "nice to have that comes free". It is a hand-written protocol bridge
with at least four independent failure modes (routing, coordinate translation, tracking mode, encoding),
and Claude Code's fullscreen mode depends on it working.

---

## 6. Alternate screen and colour

### 6.1 Alternate screen

`vt100` tracks it: `Screen::alternate_screen() -> bool` **[V]**, and `Grid` holds two grids so
switching is modelled correctly. **[V]** Nesting works — the host TUI being in its own alternate
screen is orthogonal to the child's. **[I]**

**The Claude Code specifics, from the official docs** **[V]**
([Configure your terminal for Claude Code](https://code.claude.com/docs/en/terminal-config)):

- **Default rendering is *not* the alternate screen.** Fullscreen mode *"draws to a separate screen
  the terminal reserves for full-screen apps **instead of appending to your normal scrollback**"*.
  So by default Claude Code appends to scrollback.
- Fullscreen *"keeps memory usage flat and adds mouse support for scrolling and selection"*.
- In fullscreen *"you scroll with the mouse or PageUp inside Claude Code rather than with your
  terminal's native scrollback"*.
- `/tui fullscreen` is opt-in and persisted; `CLAUDE_CODE_NO_FLICKER=1` sets it via env.

**Implication [I]:** the two Claude Code render modes stress the pane emulator in opposite ways.
Default mode → unbounded scrollback growth in your emulator, wheel should drive *your* scrollback.
Fullscreen mode → flat memory, but wheel must be forwarded to the child, and your scrollback is
empty and useless. The app has to handle both, switching at runtime, and cannot know which the user
picked except by watching `alternate_screen()`.

### 6.2 Colour

`vt100` models colour as **[V]** ([src/attrs.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/attrs.rs)):

```rust
pub enum Color { Default, Idx(u8), Rgb(u8, u8, u8) }
pub struct Attrs { pub fgcolor: Color, pub bgcolor: Color, pub mode: u8 }
```

24-bit truecolor is preserved. `ratatui` has `Color::Indexed(u8)` and `Color::Rgb(u8,u8,u8)`, so the
mapping is lossless at this layer. **[I]** Whether the *outer* terminal renders it is a separate
matter — colour survives your pipeline, then gets whatever the user's real terminal does to it.

**Attribute loss is real but small.** `vt100`'s `mode` byte carries exactly five flags: bold, dim,
italic, underline, inverse. **[V]** **Not tracked: strikethrough, blink, hidden/concealed, double
underline, underline colour, overline.** **[V]** Claude Code's diff rendering leans on background
colours (`diffAdded`, `diffRemoved`, word-level highlights) which *are* preserved **[V]** (per the
theme token reference in the docs), so the practical loss is modest. **[I]**

Claude Code themes accept `#rrggbb`, `#rgb`, `rgb(r,g,b)`, `ansi256(n)` and `ansi:<name>` **[V]** —
i.e. it emits truecolor SGR when the theme uses it.

There is a reported `tui-term` bug where widget/block background styling was ignored, leaving a
transparent pane background — [tui-term#290](https://github.com/a-kenji/tui-term/issues/290),
now closed, resolution not visible in the fetched page. **[R]**

---

## 7. What is known **not** to work

This is the section issue #7 asked for. Ordered by how much it would hurt this product.

### 7.1 `vt100` cannot answer the child's questions — and Claude Code asks

**Verified structural fact:** `vt100::Parser::process()` returns `()`; no `Callbacks` method provides
a way to write bytes back; DSR / DA / cursor-position-report have no match arms in `perform.rs` and
land in `unhandled_csi`. **[V]** (§3.2)

**Reported, and directly on point:**
[anthropics/claude-code#17787](https://github.com/anthropics/claude-code/issues/17787) is a bug report
titled *"TUI input broken on macOS: cursor position responses (`^[[row;colR]`) leak to display instead
of being consumed"*. The reporter states: *"Every keypress causes visible escape codes (`^[[35;1R`)
and the TUI freezes"*, and *"The cursor position query response (`\e[row;colR`) should be silently
consumed from stdin by the TUI, not echoed to the display."* **[R]** — the root-cause analysis in
that issue is the reporter's, not Anthropic's; treat "Claude Code issues DSR 6" as strongly
indicated but not officially confirmed.

The equivalent report exists for another CLI:
[NousResearch/hermes-agent#14692](https://github.com/NousResearch/hermes-agent/issues/14692) —
*"responses to `CSI 6 n` (Device Status Report — Report Cursor Position) queries ... being rendered as
literal text instead of being consumed by the terminal/parser."* **[R]**

And the alacritty library thread flags the same category: *"testing terminal applications (like
Reedline and Crossterm-based tools) that **expect OSC responses from the host**"*
([alacritty#8273](https://github.com/alacritty/alacritty/issues/8273)). **[V]**

**What this means [I]:** if the emulator is `vt100`, a child that queries the terminal gets silence.
Depending on the child, that is a timeout-and-degrade (best case), a hang, or garbage. Handling it
requires either (a) intercepting `unhandled_csi` and synthesising replies onto the pty master
yourself — feasible, since `Callbacks` does report the unhandled sequence with its parameters **[V]** —
or (b) an emulator that has the writer built in (`alacritty_terminal`, `wezterm-term`). **This is
the single most consequential finding in this document.**

### 7.2 Resize is lossy and, with wide characters, a panic risk

Covered in §4.2. `vt100` truncates instead of reflowing **[V]**; `Row::clear_wide` panics on a
resize-truncated wide char **[V]**.

### 7.3 The child inherits the *outer* terminal's identity

`portable-pty::CommandBuilder` inherits the parent environment and sets no `TERM`. **[V]** (§2.1)

So a Claude Code spawned from a harness-launcher running under Ghostty sees `TERM_PROGRAM=ghostty`,
`COLORTERM=truecolor`, and Ghostty's `TERM`. It will then feel entitled to Ghostty's capability set
— which your `vt100`-based pane does not have. **[I]**

Concretely, per the Claude Code docs **[V]**:

- Claude Code sends **desktop notifications** via escape sequences in Ghostty, Kitty and iTerm2, and
  under tmux this requires `set -g allow-passthrough on` — *"The `allow-passthrough` line lets
  notifications and progress updates reach the outer terminal instead of being swallowed."*
  A `vt100`-based pane will swallow them (they land in `unhandled_osc`) unless you explicitly
  re-emit them to the outer terminal. **[V]/[I]**
- Claude Code uses **extended key encodings**. Under tmux you must set
  `set -s extended-keys on` and `set -as terminal-features 'xterm*:extkeys'` — *"The `extended-keys`
  lines let tmux distinguish Shift+Enter from plain Enter so the newline shortcut works."* **[V]**
  `vt100` does not model the Kitty keyboard protocol or `modifyOtherKeys` state **[V]**, so the app
  cannot know which key encoding the child currently wants. **[I]** Expect Shift+Enter and other
  modified keys to be the first user complaint.
- Claude Code uses **synchronized output** (DEC 2026) when it detects support, with
  `CLAUDE_CODE_FORCE_SYNC_OUTPUT=1` to force it *"to stop the flicker"*. **[V]** `vt100` has no
  2026 handling **[V]**; the sequences are ignored, which is harmless in itself but means the pane
  gets no tearing protection unless the app implements it. **[I]**
- Claude Code emits a **progress bar** to the outer terminal, also gated on tmux passthrough. **[V]**

**Under-explored [I]:** setting `TERM` deliberately (and stripping `TERM_PROGRAM`) to match what the
emulator actually implements is the obvious lever, and no prior-art project examined documented what
they set. Worth a spike.

### 7.4 Reads are blocking → one thread per spawn, minimum

The maintainer-confirmed pattern is a dedicated blocking reader per pty; a single shared reader hangs.
**[V]** ([wezterm#3739](https://github.com/wezterm/wezterm/discussions/3739))

At 20 spawns that is ≥20 OS threads doing blocking reads, plus whatever drives the child processes.
**[I]** On Unix you can go non-blocking via `as_raw_fd()` and poll/epoll instead — `portable-pty`
exposes the fd **[V]** — but that abandons the Windows path, since ConPTY is pipe-backed. **[I]**

### 7.5 The producer/consumer imbalance is a known killer

Zellij's own post-mortem identifies its first bottleneck as *"the overflow of the MPSC message
channel, where the data processing rates of PTY threads and Screen threads are out of sync, with the
former sending data much faster than the latter consuming it"*, and its second as per-character heap
allocation when pushing into a row's `Vec`. **[R]** (the primary source,
[poor.dev/blog/performance](https://poor.dev/blog/performance/), returned 403 to automated fetch;
this is quoted from a search-engine summary and a
[secondary translation](https://www.mo4tech.com/performance-optimization-of-zellij.html) — **treat
the wording as approximate and re-read the original before relying on it**.)

Related, verified issue titles from the Zellij tracker **[V]**:

- [#2622](https://github.com/zellij-org/zellij/issues/2622) *"Zellij hangs with 100% CPU when opening
  a new pane with a very long line on screen"*
- [#4682](https://github.com/zellij-org/zellij/issues/4682) *"Line wrapping performance is bad"*
- [#525](https://github.com/zellij-org/zellij/issues/525) *"High memory usage and latency when a
  program produces output too quickly"*
- [#1556](https://github.com/zellij-org/zellij/issues/1556) *"Lag on input (severe with a held key)"*
- [#5044](https://github.com/zellij-org/zellij/issues/5044) *"High idle CPU usage with multiple
  independent Zellij instances"*

An agent that dumps a large file or a long build log into its pane is exactly the "produces output
too quickly" case. **[I]**

### 7.6 Rendering cost is per-cell, per-frame, and was measurably bad

`tui-term` fetches every cell individually every frame. **[V]** (§3.4)

Turborepo profiled this and found two functions running once per terminal cell:
`Cell::contents`, which *"unnecessarily allocated memory on every call"*, and `Grid::visible_row`,
which *"iterated through scrollback and screen rows"* repeatedly. The fix returned a `&str` built
from the content bytes instead of allocating, and indexed directly into the rows. **[V]**
([vercel/turborepo#9123](https://github.com/vercel/turborepo/pull/9123)) The PR reports before/after
profiles showing large reductions in CPU time and allocations, but **no absolute numbers**. **[V]**

That this was worth a dedicated PR from a well-resourced team, for a TUI showing a handful of task
panes, is the signal. **[I]**

### 7.7 Memory: the scrollback multiplier

Verified building blocks:

- `vt100::Cell` is **32 bytes** — `contents: [u8; 22]`, `len: u8`, `attrs: Attrs`; the source comment
  says the 22-byte buffer was *"chosen to make the overall Cell struct 32 bytes"*. **[V]**
  ([src/cell.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/cell.rs))
- `Row::new()` allocates the **full column width up front**: `vec![Cell::new(); cols]`. Rows are not
  sparse. **[V]** ([src/row.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/row.rs))
- `Grid` holds `rows: Vec<Row>` (visible) plus `scrollback: VecDeque<Row>` bounded by
  `scrollback_len`. **[V]** ([src/grid.rs](https://raw.githubusercontent.com/doy/vt100-rust/main/src/grid.rs))

**Arithmetic [I]** — `bytes ≈ (visible_rows + scrollback_rows) × cols × 32`:

| Config (per spawn) | Per spawn | × 20 spawns |
| --- | --- | --- |
| 120 cols, 50 visible, 0 scrollback | ~192 KB | ~3.8 MB |
| 120 cols, 50 visible, 2 000 scrollback | ~7.9 MB | ~157 MB |
| 200 cols, 50 visible, 10 000 scrollback | ~64 MB | **~1.3 GB** |

Scrollback grows lazily as lines are produced, so these are ceilings, not startup costs. **[I]**
The visible grid alone is negligible; **scrollback depth is the entire decision.** **[I]**

Cross-check that the shape is real, not just arithmetic **[V]**:

- [zellij#2104](https://github.com/zellij-org/zellij/issues/2104) — with a 10 000-line scrollback and
  2 000-character lines, RSS reached **1 282 MB** against ~20 MB of actual content; the reporter
  computes ~6 300% overhead, caused by long logical lines being stored unwrapped.
- [zellij#3594](https://github.com/zellij-org/zellij/issues/3594) — an **empty** zellij session at
  ~80 MB vs tmux at ~6 MB. No maintainer explanation in the fetched thread.
- [zellij#3598](https://github.com/zellij-org/zellij/issues/3598) — *"RAM and CPU usage grow
  unbounded"*.

**And the Claude-Code-specific twist [V]/[I]:** default (non-fullscreen) Claude Code *appends to
scrollback*; the docs say fullscreen mode is what *"keeps memory usage flat"*. **[V]** So the default
configuration of the one harness this tranche targets is precisely the memory-expensive one, and a
long agent session fills scrollback steadily. **[I]**

### 7.8 Smaller sharp edges

- **`tui-term` cursor rendering breaks with scrollback offsets** —
  [tui-term#356](https://github.com/a-kenji/tui-term/issues/356), reported against tui-term 0.3.2 /
  vt100 0.16.2, fixed in PR #357. The reporter carried a local workaround meanwhile. **[V]**
- **`tui-term`'s process-lifecycle helper is `unstable` and oneshot-only.** **[V]** (§3.4)
- **Ctrl-C is not a signal you send; it is a byte you write.** The host must be in raw mode (so it
  receives `0x03` as data rather than dying), then write `0x03` to the pty master, whereupon the
  slave's line discipline generates `SIGINT` for the child's foreground process group. **[I]** —
  this follows from POSIX `ISIG`/`VINTR` semantics; the POSIX and Linux man pages both returned 403
  to automated fetch, so it is **not independently verified here** and should be confirmed by
  experiment. The corollary is that the host must reserve *some* key for itself (to leave the pane),
  and any key it reserves is a key Claude Code can no longer see —
  cf. [claude-code#14010](https://github.com/anthropics/claude-code/issues/14010), a request for a
  passthrough escape sequence so screen/tmux detach still works (closed as duplicate). **[V]**
- **Windows is a different animal.** ConPTY is pipe-backed rather than fd-backed; renga carries
  ConPTY-specific flicker ([renga#263](https://github.com/suisya-systems/renga/issues/263)) and
  IME/focus ([renga#255](https://github.com/suisya-systems/renga/issues/255)) bugs. **[V]** bosun
  simply declares *"macOS or Linux (Windows is not supported)"*. **[V]**
- **`vt100` panics on narrow panes with wide glyphs** —
  [vt100#37](https://github.com/doy/vt100-rust/issues/37). **[V]**

---

## 8. Cost at 15–20 concurrent — summary of the evidence

Nothing found gives a measured figure for 15–20 embedded emulators. What is established:

| Dimension | Evidence | Verdict |
| --- | --- | --- |
| Visible grid memory | 32 B/cell, full-width rows **[V]** | ~200 KB/pane. Negligible. **[I]** |
| Scrollback memory | Same, × depth **[V]**; zellij #2104 shows 1.28 GB in the pathological case **[V]** | **The binding constraint.** **[I]** |
| Threads | Blocking reads, one reader per pty **[V]** | ≥20 threads. Fine on any modern machine. **[I]** |
| Parse CPU | Zellij: pty threads outrun screen threads **[R]**; #2622, #4682, #525 **[V]** | Bursty output is the risk, not steady state. **[I]** |
| Render CPU | Per-cell per-frame; turborepo needed a dedicated optimisation PR **[V]** | Real. Scales with *visible* panes, not total spawns. **[I]** |
| Baseline overhead | zellij empty session ~80 MB vs tmux ~6 MB **[V]** | Rust terminal multiplexers are not cheap by default. **[I]** |

**The structural relief [I]:** only the opened spawn needs *rendering*; all 20 need *parsing* and
*storage*. Render cost therefore does not scale with spawn count, but parse and memory cost do.
This does not follow from any single citation — it follows from the fact that `tui-term`'s render
loop is over the pane's area **[V]** while parsing happens on the pty reader thread regardless of
visibility **[V]**.

---

## 9. Prior art — who does this, with what, and what they hit

### 9.1 Rust projects that embed a live child TUI

| Project | PTY | Emulation | Render | Notes |
| --- | --- | --- | --- | --- |
| **[Zellij](https://github.com/zellij-org/zellij)** | own | `vte` + hand-written `Grid` implementing `vte::Perform` **[R]** | own | Full multiplexer. Long, public performance history (§7.5) and a long mouse-encoding history (§5.2). |
| **[mprocs](https://github.com/pvolok/mprocs)** | `mprocs-pty` (fork of `portable-pty`) | `mprocs-vt100` (fork of `vt100`) | `ratatui` | Both dependencies forked. **[V]** (crates.io) |
| **[turborepo](https://github.com/vercel/turborepo)** | — | `vt100` (patched) | `tui-term` | Upstreamed the per-cell render optimisation (#9123). **[V]** |
| **[bosun](https://github.com/yetidevworks/bosun)** | `portable-pty` | `vt100` | `tui-term` + `ratatui` | *tmux-native*: tmux is the source of truth, bosun previews. macOS/Linux only. **[V]** |
| **[renga](https://github.com/suisya-systems/renga) / ccmux** | `portable-pty` (ConPTY on Windows) | `vt100` | `ratatui` + `crossterm` | Purpose-built for multiple Claude Code agents. 10 000 lines of history per pane. **[V]** |
| **[claude-workbench](https://github.com/eqms/claude-workbench)** | — | — | `ratatui` | Multi-pane embedded PTY terminals for Claude Code. **[R]** |
| **[wezterm](https://github.com/wezterm/wezterm)** | own | `wezterm-term` | GPU | The reference implementation of the whole stack. |

**The `vt100` + `portable-pty` + `tui-term` + `ratatui` stack is the de-facto standard.** **[V]**
So is forking parts of it. **[V]**

### 9.2 The dissenting data point

**[cwt](https://github.com/0dragosh/cwt)** solves *the same problem this project solves* — a Rust TUI
managing git worktrees for parallel Claude Code / Codex / Pi sessions — and **deliberately does not
embed**. It hands off to zellij (preferred) or tmux; provider sessions open in named tabs or panes.
Its stated benefit: *"Sessions survive TUI exit — closing cwt does not kill running sessions."* **[V]**

**bosun** takes a middle road: tmux owns the sessions and is *"the source of truth"*, receiving push
notifications via tmux control mode (`tmux -C`); bosun renders a live preview via
`portable-pty` + `vt100` + `tui-term` on a 200 ms tick for *every* managed session, not just the
focused one. **[V]**

Both are worth weighing in #9 — noted here as fact, not as recommendation. Note that the vision's
"Opening a spawn never costs me sight of the others" and the tranche's "I see the agent *and* the
list of spawns at the same time" are constraints a plain tmux hand-off does not obviously satisfy.

---

## 10. What this research did not settle

Honest gaps, so the next reader does not assume coverage:

1. **No measurement.** Every performance number here is either arithmetic from struct sizes or
   someone else's bug report about a different program. Nobody has run 20 `vt100` parsers fed by
   20 Claude Code sessions and measured it. A spike would settle §8 in an afternoon.
2. **Whether Claude Code actually breaks without DSR replies.** #17787 shows what happens when the
   reply is *mishandled*; it does not show what happens when there is *no* reply. Different failure.
   Testable directly.
3. **`par-term-emu-core-rust` is unassessed.** It is the only surveyed crate claiming reflow-on-resize,
   Kitty keyboard, synchronized output *and* full mouse encodings — i.e. it claims to fix most of
   §7. It has no visible adoption. Claims are from its own README only.
4. **`TERM` policy.** No prior-art project documented what it advertises to the child. Since
   `portable-pty` sets nothing and inherits everything **[V]**, the default is probably wrong for
   everyone, and nobody has written down what right looks like.
5. **Ctrl-C / signal forwarding is reasoned, not verified.** The POSIX and Linux man pages blocked
   automated fetch. §7.8's account should be confirmed by experiment before it is designed against.
6. **`alacritty_terminal` and `wezterm-term` were surveyed, not trialled.** Their query-response
   machinery looks like the answer to §7.1, but `wezterm-term` is unpublished **[V]** and
   `alacritty_terminal`'s API is unstable **[V]**. Neither cost was quantified.
7. **The 403 wall.** docs.rs, lib.rs, man7.org, pubs.opengroup.org and web.archive.org all refused
   automated fetch from this environment. Crate APIs were read from GitHub source instead — which is
   arguably better — but the man-page material is second-hand and marked as such.
