# Tranche 1 — The core loop: design

**Status: settled.** Every design question for tranche 1 is closed. This document is
the complete record of what was decided and why.

It is written to stand alone. The decisions were reached across a long series of
conversations and recorded as GitHub issues; this compiles them so the knowledge is
not trapped there. Nothing here requires reading an issue to understand — the issues
remain the audit trail, not the reference.

**This document is language-agnostic.** The design is a set of decisions about
behaviour, mechanism and boundaries — none of it depends on the implementation
language. Where a choice *is* language-specific it lives in **Appendix A**, so the body
can be reused to build this in something other than the language first chosen.

**Related documents.** [`docs/product-vision.md`](../product-vision.md) is the
unbounded wishlist — what the tool should do when it is *done*, across all tranches.
[`docs/tranches/01-the-core-loop.md`](../tranches/01-the-core-loop.md) is the frozen
scope of this tranche — what is in and what is out. **This document is the design**:
how the scope is met, and why each choice beat its alternatives.

> **Revised 2026-08-10.** The load-bearing decision in §4.1 changed after the rejected
> alternatives were prototyped: tmux no longer draws anything, and the app renders every
> cell itself.
>
> **§§3, 4.1, 4.2, 4.4, 4.6, 4.8, 4.9, 8 and Appendix A reversed a decision**, and each
> carries a note saying what it used to say. **§§2, 4.3, 4.5 and 5.3 were corrected for
> consistency** — no decision changed there, but statements that only made sense under the
> old mechanism did. Everything else stands as written.
>
> The scope in the tranche document is untouched: this is a different mechanism for the
> same product.

---

## 1. What tranche 1 accomplishes

### The problem

Working with one coding agent is a clean loop: go where the work is, start the agent,
ask, it works, it stops and asks you. There is one place to look.

Raising throughput by running several pieces of work at once breaks that loop:

- **Terminal juggling** — one command line per workstream, switched by hand, including
  the work of finding which one is currently blocked on you.
- **Repo contention** — concurrent agents in one project cannot share a checkout; each
  needs its own clone or worktree.
- **Neither scales.** Both are manual setup paid per workstream, so past a handful of
  concurrent tasks the process overhead eats the throughput it was meant to buy.

The bottleneck is not agent capability. It is the scattered manual process around the
agents, and specifically the human attention it consumes.

### What ships

> Start several Claude Code spawns — each on whatever local git repository you choose —
> from one place, see at a glance which ones need you, and open one to answer. No
> worktree created by hand, no terminals to juggle.

**The goal is not to escape the terminal.** This lands as a TUI, in a terminal, and
that is fine. What goes away is having to juggle *several* terminals. The terminal is
not the problem; the plural is.

**The bar** is beating 4–5 concurrent sessions handled manually today, **on friction
reduction alone**, before any of the wishlist's cleverer features exist. The target is
15–20 live spawns.

### The differentiator, and it is the whole point

**Opening a spawn never costs sight of the others.**

A study of three comparable tools —
[firstmate](https://github.com/kunchenguid/firstmate),
[ccmanager](https://github.com/kbwo/ccmanager) and
[omnigent](https://github.com/omnigent-ai/omnigent) — found that **none of them
provides this**. ccmanager is modal: enter a session and the list is gone. firstmate
and omnigent hand you to a tmux window or a separate client. Every one of them trades
sight of the whole for focus on the one.

Two consequences follow, and they govern how the rest of this document should be read:

1. **Their design choices are evidence about a different problem.** Where this design
   diverges from all three, that is expected rather than alarming.
2. **Costs traceable to simultaneity are the price of the product, not mistakes.** They
   should be paid deliberately, not engineered away.

---

## 2. What it does

### Starting work — a spawn

From the app you choose a local git repository, describe the work, and choose a model
and an effort level. The app creates a git worktree, starts `claude` inside it, and the
session appears in your list.

That unit — the act and the ongoing thing it produces — is a **spawn**.

Everything the form collects becomes **one command line**:

```
claude --model <model> --effort <level> -n <name> "<the work>"
```

run with the worktree as its working directory. The prompt is a positional argument, so
it cannot be swallowed by a text box and needs no verification. `--effort` accepts
exactly `low, medium, high, xhigh, max`; the tranche's "effort level" is a first-class
flag rather than something invented here.

**The app never types into a session.** It relays your keystrokes to the selected spawn
and originates none of its own. See §4.3 for why that is scope rather than
architecture.

### Watching work — the list

Every spawn appears in a list, grouped by repository, each repository header carrying a
compact bar summarising its spawns' statuses — so a project's state reads without
reading its rows.

**Three statuses, and they are about the agent:**

| status | meaning | how it reads |
| --- | --- | --- |
| **working** | the agent is busy | recedes — faint mark, dim text |
| **stopped** | the agent has stopped | the only bright thing on screen |
| **unknown** | *the app* cannot tell | the outlier — amber, distinct |

Status is carried by **an icon and a colour together**, so it survives a colour-blind
reader and needs no legend at twenty entries.

The app never infers *why* an agent stopped. Finished, waiting for a question, or dead
are the same status, because the response is identical: go and look.

**`unknown` is not a kind of stopped.** It means the app's own instrumentation failed —
the status file would not resolve, a probe failed, or the pane is gone. The user acts
differently on it: *stopped* means look at the spawn, *unknown* means something is
wrong with the tooling. The specific reason is available in the detail view, never
promoted to another status.

### Opening a spawn

Selecting a spawn puts it in the **slot** — the region beside the list — where you see
the real Claude Code interface, byte for byte. You type into it, interrupt it, watch
its sub-agents. It is the actual program, not a reconstruction: the app draws the
screen, but every cell in it came from `claude`.

**Two honest asterisks on "byte for byte"**, both consequences of §4.1 and both recorded
rather than buried: the app does not implement **scrollback** (§4.6), and it does not
implement **mouse forwarding**. A spawn is the real interface, live and interactive, but
in tranche 1 it is a screenful of it. Nothing else about the promise is qualified —
typing, interrupting, colour, sub-agent output and the spinner are all the real thing.

**The list stays visible the entire time.**

### Composing new work

Creating a spawn is not a modal dialog. Instructions can be long, and you must be able
to leave a half-written draft, go deal with a spawn that stopped, and come back.

So a **draft is a first-class row, exactly like a spawn**. It takes the slot when
selected, sits quietly in the list when not, and survives being walked away from.
**Several drafts can be in flight at once**, at no extra cost.

The form asks *"which of these?"* rather than *"which effort level?"* — the choices are
supplied by the harness, and where a harness offers none, the control is omitted
entirely rather than shown empty.

### Retiring a spawn

An explicit act meaning *we are done with this spawn* — the only thing that releases
what the app created. Never inferred from an agent falling silent.

The order is strict, and the order is the point:

1. stop the process
2. confirm it is gone
3. check the worktree is clean
4. remove the worktree

A dirty check against a live agent is a race, and losing that race deletes work.
Stopping first is what makes the refusal meaningful.

**Retiring removes the worktree and leaves the branch.** The worktree is the app's to
remove because the app created it; the branch holds committed work and deleting it is a
different and riskier act.

**If the worktree is dirty, retiring refuses** — no confirmation flow. Clean it up
yourself, then retire. Consequence accepted: the refusal lands *after* the kill, so a
dirty spawn ends up stopped and needing manual cleanup. An early-out check before
stopping was considered and declined as unlean.

### Quitting

**Quitting kills nothing.** tmux outlives the app and that default is kept: ending
twenty agents mid-turn because you closed a viewer would be the most destructive thing
this app could do — and it would foreclose the recovery a later tranche wants, because
there would be nothing left to recover.

On the way out the app says what it left behind. On the way in it reports any orphans
it finds and adopts none of them. Litter is accepted; *invisible* litter is not.

---

## 3. How it looks

One screen, drawn end to end by the app: the list on the left, the slot on the right.

```
 SPAWNS                          │ ┌─ claude ─ fix-worktree-cleanup ─ harness-launcher
                                 │
 harness-launcher ▪▪▪▪▪          │ > fix the worktree cleanup so retiring a dirty
▍● fix-worktree-cleanup     31m  │   spawn refuses instead of deleting
 ? spawn-form-choices      1h4m  │
 · add-retry-logic            4m │   ⏺ I'll start by reading how retirement is wired.
 · status-ladder              2m │
                                 │     Read(src/worktree.rs)
 acme-api ▪▪▪▪▪▪                 │     └─ 214 lines
 ● drop-legacy-auth          22m │
 · rate-limit-headers         7m │   ⏺ The check runs after the process stops, which is
 · openapi-drift-check        3m │     right, but it inherits the user's config.
 …                               │
                                 │   Shall I pass --untracked-files=all explicitly?
```

**Nothing is a fixed size.** Every dimension is computed from the real terminal. A
maximised window must not be a bigger frame around the same small layout. This is
recorded as a constraint because the first layout prototype hardcoded its widths, which
flattered every candidate equally and hid how narrow the list really is.

**The separator is the app's, and there is only one renderer.** Everything on
screen — the list, the slot, a draft, the contents of a spawn — is drawn by the app in one
pass, so there is no mode in which two halves of the screen are drawn by different things
and fail to line up.

> **Revised 2026-08-10.** This previously read "the separator is always tmux's", and the
> sketch above was a tmux window with two panes: the slot's header was tmux's pane border,
> and the divider was tmux's. Both are now drawn by the app (§4.1). **The picture is
> unchanged** — that is the point of recording it. What changed is who paints it, which
> is invisible from the user's chair and decisive everywhere else in this document.

Three layouts were built and compared side by side against a mock slot. The one chosen
groups by repository. Rejected: a flat list sorted by attention (denser, but reads as an
inventory rather than a workspace) and sections grouped by attention (strong when
something needs you, but degenerating to one undifferentiated fourteen-item list when
everything is working — which is the common case).

Prototype: branch `prototype/list-pane-layouts`.

---

## 4. How it works

### 4.1 tmux supervises; the app renders

This is the load-bearing decision. Everything else assumes it.

> **Revised 2026-08-10.** This section originally chose **drive**: the app composes a
> tmux layout and tmux draws the children. Prototyping the rejected alternatives
> unseated it. The new choice is **supervise + render** — tmux still owns the processes,
> but it draws nothing and the app draws everything. The reasoning that rejected *hand
> off* is untouched; the reasoning that rejected *embed* turned out to rest on an
> arithmetic that a later decision in this same document had already invalidated.
> Recorded as a revision rather than a rewrite, because how the earlier conclusion was
> reached matters.

Four shapes:

| | what it means | verdict |
| --- | --- | --- |
| **Hand off** | the app manages spawns; viewing one means leaving the app for tmux | rejected |
| **Drive** | the app composes a tmux layout — list pane, slot pane — and tmux draws | rejected; originally chosen |
| **Own the pty** | the app spawns each harness on a pty of its own and renders it | rejected |
| **Supervise + render** | tmux owns the processes headlessly; the app reads their output over control mode and draws every cell | **chosen** |

**Hand-off dies on simultaneity.** It is what ccmanager and `claude attach` do, and it
takes the list away. Unchanged, and the reason the product exists.

**What unseated *drive*.** It works — that was verified, not assumed. What it costs is
that every surface the user sees belongs to tmux, so the app has to arrange its product
around a layout engine it does not control. Three consequences, which read as separate
design problems until you notice they are one:

- the slot is a tmux pane, so **a draft has to be a pane too** (§4.4) — which means a
  process, which means a handover across a process boundary;
- showing a spawn means **moving a pane into the slot**, which resizes the child and
  forces a full repaint on every switch (§4.2);
- **the app itself must run inside tmux**, and must detect and cope with whether it was
  started inside a session or outside one.

None of those is fatal on its own. Together they are a tax paid on every feature, in
exchange for not writing a terminal emulator.

**Owning the pty and supervising differ in exactly one thing: who holds the process.**
Both make the app a terminal emulator; both give it the whole screen. Both were built,
behind a single shared renderer, precisely so the comparison could not be confounded by
rendering differences — and **they are indistinguishable to look at**. That is the
finding: for rendering, control mode buys nothing over owning the pty.

So the choice between them turns on one question — what happens to twenty agents
mid-turn when the app exits. A pty the app owns dies with the app. A tmux pane does not.
§2 already promises **quitting kills nothing**, and that promise is worth more than the
control-mode machinery costs. tmux stays.

**Worth naming: that argument leans on something the tranche defers.** "Surviving the app
closing" is explicitly out of scope — the frozen scope says shutting everything down on
exit "is acceptable here". So the deciding argument for tmux is a behaviour the tranche
does not require. It still holds, for two reasons. First, *acceptable* is not *desirable*:
killing twenty agents mid-turn is the most destructive thing this app could do, and
choosing an architecture that makes it the default would be choosing the harm rather than
tolerating it. Second, it is the difference between a later tranche being able to add
recovery and there being nothing left to recover. But it is a scope-adjacent argument
carrying an architectural decision, and that should be visible rather than smuggled.

**tmux is a headless process supervisor.** It is never attached to the user's terminal,
it draws nothing anyone sees, and its layout is not a layout — one session, one window
per spawn, one pane per window, none of them visible. Its durable value is exactly one
thing: **process lifetime independent of the app**.

**tmux is not storage either.** The grids are the app's, held in memory. tmux's copy of a
pane's screen is where the bytes come *from*, not where they live — which is why losing
the app loses the display history (§4.9) even though the processes survive.

**The app does not run inside tmux.** `$TMUX` detection, taking over the current window,
and the two start-up modes are all gone. The app is an ordinary terminal program: run it
anywhere, including inside tmux if that happens to be where you are, and it makes no
difference to anything.

**The cost is a terminal emulator, and it is real.** The four sharp edges the research
found are now the app's to solve rather than tmux's:

- ~~**The emulator must be able to answer the child.**~~ **Withdrawn 2026-08-10** — it
  read *"Terminals reply to queries — cursor position among them, which Claude Code
  issues. An emulator with no write-back path drops them silently. This is the sharpest
  of the four and control mode does not solve it: the reply path exists (`send-keys -H`),
  but something still has to generate the reply."* Control mode does solve it, and by
  removing the problem rather than by carrying the reply: **tmux is the child's terminal
  and answers the queries itself.** The full account is in §5.3, including why the app
  must now be careful *not* to answer. This edge belongs to *own the pty*, which is the
  alternative that was rejected.
- **Resize is lossy**, and a common source of crashes where wide glyphs meet narrow
  panes.
- **Mouse forwarding is a hand-written protocol bridge**, with several independent
  encodings to get wrong.
- **Scrollback is the app's problem**, and in tranche 1 the app does not implement it
  (§4.6).

**The arithmetic that killed embedding had expired before it was applied.** The figure
was roughly **1.3 GB across twenty panes**, from every pane holding a full grid *plus a
generous history*. But §4.6 forces the fullscreen renderer, which draws on the alternate
screen — and the alternate screen accrues no scrollback. With zero history allocated a
grid is on the order of **300 KB**, about **6 MB across twenty**. The number that decided
the question had been invalidated by a decision taken later in the same design, and
nobody went back. Recorded because every step was locally sound, which is what makes
this class of error hard to see.

**And memory was never the deciding factor anyway.** Confirmed with the author: the
machine this runs on has orders of magnitude more than either figure needs. The honest
cost of embedding is the emulator's four edges, not its footprint.

**tmux only.** zellij is a maybe-someday and its pane model is **knowingly unverified**.
The supervise-only role makes this less binding than it was — the app now needs far less
from the multiplexer than when it was also the layout engine — but it is a swap nobody
has tested.

**No dependency on Claude Code's own background-session daemon.** It exists — `claude
agents`, `claude attach`, its own worktree management — and building on it was
considered. Rejected: it is Claude-Code-specific and would land on the wrong side of the
harness seam, it is a research preview, and it creates and cleans up worktrees itself,
which collides head-on with the app owning worktrees.

### 4.2 The slot

The list on the left; the slot on the right holds **one** spawn or draft. Both are
regions of the app's own screen, drawn in a single pass.

> **Revised 2026-08-10.** This section formerly described **parking**: every spawn not in
> the slot lived in a dedicated detached holding session, moved in and out with
> `break-pane` / `join-pane`, with `$TMUX` choosing between two start-up modes. All of
> that is gone — the mechanism, the holding session, and the mode detection. The research
> behind it stands (parking does work detached, and it does force a structural resize);
> it is simply no longer used.

**Switching is free.** Every spawn's screen lives in the app's memory as its own grid,
kept current by the control-mode stream whether or not it is the one on display.
Selecting another spawn is a re-render of a grid the app already holds: no pane moves,
nothing is resized, no `SIGWINCH` reaches the child, nothing repaints.

This is the substantive gain, and it is worth being precise about what was given up to
get it. Parking's cost was structural — every park and unpark resized the pane and forced
Claude Code to redraw, and every alternative tried (`window-size manual`, a same-sized
holding session, `join-pane` rather than `break-pane`) still paid it. It was measured and
judged acceptable, and it *was* acceptable: under the fullscreen renderer the redraw is
clean. But "acceptable" is not "free", and this is what free looks like.

**Panes are still sized — just not by switching.** Each spawn's tmux pane is created at
the slot's dimensions, because that is what the child renders into, and resized when the
*app's* window changes size: one event affecting every spawn at once, never one caused by
the user moving between them.

**Every spawn is live, not only the visible one.** A parked pane was live too, so that is
not new. What is new is that its output reaches the app continuously instead of sitting
in tmux until unparked — which is what makes switching instant, and why the reader thread
in §4.8 carries every pane rather than one.

### 4.3 The harness seam — a place, not an abstraction

The vision names harness extensibility the one decision that is hard to walk back. The
seam is therefore designed with care, and the design is: **do not write the abstraction
yet.**

**No harness abstraction in tranche 1** — no interface, protocol or base type with a
single implementation behind it. One adapter makes a seam hypothetical; two make it
real. Apply the deletion test: remove the abstraction and nothing happens, because no
complexity reappears across callers — there is one call path. That is a pass-through
wearing the costume of architecture. Worse, an abstraction shaped by one implementation
is an abstraction shaped by **Claude Code** — manufacturing the hazard while believing
you are preventing it.

**What is actually hard to walk back is leakage**, not the missing abstraction: `--effort`
reaching the spawn form, the string `claude` in the tmux code, a status reader that knows
the shape of a prompt box. The vision's requirement — *"adding a new harness has to be
possible, and as simple as it can reasonably be made"* — is a **locality** requirement.

So the seam is **one module that owns every Claude-Code-specific fact**, and:

> **The module performs no I/O.** It translates; the app acts.

```
launch_recipe(spec)      -> { program, args, env, cwd }   // the app runs it
stop_plan()              -> { signal, timeout }           // the app sends it
models()                 -> [{ id, label }]               // the app renders them
effort_levels()          -> [{ id, label }]
status_source(pid)       -> path                          // the app reads it
status_of(record)        -> Working | Stopped             // the module translates

// designed-for, unimplemented in tranche 1:
keystrokes_for(text)     -> …
composer_state(screen)   -> Submitted | Pending
```

Three things this buys. **Depth** — every Claude Code fact sits behind a handful of
functions returning plain data. **Testability for free** — each operation is a pure
function from a spec to data, so "this spawn spec produces this argv" needs no process,
terminal or tmux. And it makes the discipline **structural rather than aspirational**: a
module with no I/O *cannot* reach into tmux, and tmux code that only receives a recipe
has nothing Claude-shaped to leak.

**The module also supplies the choices** the spawn form offers. Otherwise the UI
hardcodes `low, medium, high, xhigh, max` and the discipline breaks on the first screen
built. It reframes the seam usefully too: the question the module answers is *"what does
this harness let you choose?"*, not *"what flags does this harness take?"* — the first
survives a harness configured by a file, the second does not.

**The input hole is deliberate.** Tranche 1's app relays your keystrokes to the selected
spawn and **originates none of its own** — it never composes input on your behalf. That
is the hole: the vision's managing agent will. So *start a session* and *send input to a session* are separate
concepts even though only the first is implemented, and delivering work is **not**
defined as "the prompt is a launch argument" — that is one implementation of *give this
session some work*, already false for a second turn.

**Two invariants, checkable:**

- Nothing outside the module mentions Claude Code — not the binary name, a flag, an
  environment variable, a status vocabulary, or a screen shape.
- The module mentions neither tmux, nor the filesystem, nor processes.

### 4.4 Drafts

A draft is a half-written spawn: a repository, a description, a model, an effort level,
and however long it takes you to finish typing it.

> **Revised 2026-08-10.** This section formerly read "One binary, and drafts as panes",
> and made a draft a tmux pane running the same binary in a draft mode, handing over to
> `claude` with an `exec` and a handover file the app polled. Under app-side rendering
> the objection that forced that design does not exist, and the mechanism goes with it.

**A draft is app-side state.** No pane, no process, no `exec` handover, no handover file,
no second mode of the binary.

**The objection that forced drafts into panes has dissolved.** It was that an app-drawn
draft would give the app two rendering modes — the divider between the columns would be
the app's in one and tmux's in the other, a permanent source of visual mismatch. Under
app-side rendering **everything on screen is the app's**, so there is only ever one
renderer and nothing to mismatch.

**Creation still shows its work, and still refuses in place.** The draft's row is where
progress and errors land: intent before action, so a creation that dies half-way leaves a
record of the worktree it made rather than a mystery. On success the row becomes a spawn
row; **on failure it stays a draft with your text intact**, so "refuse rather than guess"
never costs you the paragraph you just wrote. That behaviour is unchanged — only the thing
doing the work moved, from a separate process into a short-lived worker (§4.8).

**Several drafts in flight at once still costs nothing**, and costs less than it did: a
list of records rather than a pile of parked panes.

**No lock, and now no question of one.** A per-repository or global creation lock was
already declined — the collision it would prevent is impossible, because names carry a
random suffix and paths are never reused, and concurrent `git worktree add` was verified
to work. What changed is that such a lock would no longer have had to be an
**inter-process** lockfile, since creation now happens inside the app. The decision
stands; the argument for it got cheaper.

### 4.5 Knowing what a spawn is doing

**A ladder, not a single signal:**

1. **Is the pane alive?** From tmux — `pane_dead`, or the pane missing entirely. Dead →
   **stopped**, immediately. The one signal that cannot go stale.
2. **If alive, read the session status file**, keyed by `pane_pid`. `busy` and `waiting`
   → **working**; `idle` and `shell` → **stopped**.
3. **If alive but the file will not resolve** → **unknown**, after a grace period.

**Output is not a status signal**, and the §4.1 revision does not change that. The app
now sees every byte a spawn draws, and it is tempting to read the stream — but a busy
agent and one waiting on a question both draw, and inferring the difference means
pattern-matching one harness's screen, which is exactly the Claude-Code-shaped corner
the seam exists to avoid. The ladder stands as written.

**Where the file lives, and what its words mean, belong to the harness module.** The app
does the reading — the module performs no I/O — but it is handed a path and handed back
`Working` or `Stopped`, and never sees `idle`, `busy` or `waiting`. Getting this wrong
would put a Claude-Code-specific path *and* a Claude-Code-specific vocabulary in the app,
which is exactly what §4.3's first invariant forbids. The description below is of the
module's knowledge, not the app's.

**About that status file.** Claude Code writes `<config_dir>/sessions/<pid>.json` — its
internal `concurrentSessions` registry, present since v2.1.139, **the same file that
backs `claude agents`** — carrying a status that flips `idle` ⇄ `busy` ⇄ `waiting` for
interactive sessions. The pid equals tmux's `#{pane_pid}` on a launcher-controlled
launch path.

This corrected an earlier conclusion of our own. Research had found that `claude agents
--json` does not list interactive sessions, and we generalised that to "this mechanism
cannot see our sessions". **The command cannot; the file underneath it can.** The
correction came from reading omnigent's source.

**Carried unsoftened:** the file is an **undocumented internal detail**, it is
**version-gated**, and it must always be treated as fallible — omnigent's own watcher
falls back to a PTY watcher rather than erroring, after a bounded number of attempts.

**The grace period matters more than it sounds.** A freshly started spawn is alive
before Claude Code has written its file. Without a grace period every new spawn would
briefly report `unknown` — the one status that is supposed to mean a real problem, fired
routinely.

**Where the app must guess, it surfaces rather than hides.** A false *stopped* costs a
glance — you open it, see it working, move on, and you were going to that spawn anyway. A
false *working* costs the product: a spawn waits for you and never appears in the set
that needs you. **This is the inverse of firstmate's bias, deliberately** — their false
negative launches a duplicate agent onto a live worktree and destroys work; ours only
shows a status. Recorded so nobody later "corrects" it to match the prior art.

**Hooks are not used in tranche 1.** They are the documented mechanism, but they require
writing into the user's Claude Code settings — invasive on a machine the app does not
own — and `--bare` disables discovery. They remain the named fallback if the
undocumented status file ever disappears.

### 4.6 Rendering: fullscreen is required

Claude Code has two renderers. **Fullscreen** draws on the alternate screen buffer, like
`vim`. **Classic** writes into the terminal's native scrollback.

**The app forces fullscreen.** Not inherited from the user's setting, which varies by
when they first used Claude Code.

> **Revised 2026-08-10.** The requirement survives the §4.1 change, but **both of its
> original reasons are gone**. It was chosen for tmux server memory — irrelevant now that
> the app holds the grids and tmux holds no history worth counting — and then reinforced
> by the redraw test, which no longer applies because switching no longer resizes
> anything. The new reasons are below. A requirement whose entire justification has
> turned over is worth re-deriving rather than inheriting.

- **The grids the app holds are screens, not histories.** The same argument as before,
  moved from tmux's memory into the app's: under fullscreen a spawn costs one screenful
  of cells and nothing accumulates. This is what makes twenty embedded emulators cost
  megabytes rather than gigabytes (§4.1).
- **The alternate screen keeps the transcript out of a scrollback the app does not
  implement.** Under classic, output scrolls off the top of the grid and — with no
  app-side scrollback — is simply gone. Fullscreen is what makes "no scrollback" a
  coherent position rather than data loss.
- **Fullscreen under control mode is the configuration that was actually tested.**
  `prototype/render-alternatives` puts Claude Code's fullscreen renderer through a
  control-mode stream and a hand-written emulator, and it renders.

**A known unknown was closed rather than dissolved.** The earlier design avoided control
mode partly because Claude Code's fullscreen renderer is documented as incompatible with
**iTerm2's** tmux integration mode, and whether our own client would trip the same thing
was untested. It does not: the prototype *is* a control-mode client, and the fullscreen
renderer works through it. That incompatibility is about iTerm2's handling, not about
control mode as a transport.

**Accepted cost, and it is larger than it was:** scrollback inside a spawn is Claude
Code's business entirely. `capture-pane` will not show history, tmux-native scrollback
search over a spawn's past output is gone, and the app offers no substitute of its own.
Scrolling past a screenful stays on the deliberately-open list (§5.3).

### 4.7 Worktrees, branches, and the dirty rule

**Worktrees live under one app-owned root**, outside every repository — never inside the
repo and never as a sibling. Inside the repo they show as `??` in the user's own `git
status`, and the only fix would be editing *their* `.gitignore`, which is writing into
their project. A directory the app owns also makes leftovers findable.

*Accepted cost:* the worktrees are not where a git-literate user would look for them.

**Names derive from the work description**, namespaced, with a short random suffix —
the same string for the branch and the directory. Branches **outlive spawns** (retiring
leaves them), so these names get read months later while pruning: `spawn/a7f3c2` is
unreadable then, `spawn/add-retry-logic-a7f3` is not. The suffix is random rather than a
counter because a counter needs state the app does not keep.

**Creation is always `git worktree add -b`, never the bare form.** This is a safety
rule, not a style one: the bare form **silently checks out a pre-existing branch of that
name** instead of creating one, which would drop a fresh agent onto somebody else's
in-progress work.

**Branches start from the default branch**, resolved from `refs/remotes/origin/HEAD` and
taken **locally with no fetch** — spawning stays off the network path, and if the local
default is stale that is the state of the repository, exactly as if you had branched by
hand. *Accepted consequence:* a spawn never continues from the branch you are standing
on, so stacked work is arranged by hand.

**Refuse rather than guess:** unresolvable `origin/HEAD`, no remote, a repository with no
commits, a detached HEAD. Guessing between `main` and `master` picks wrong in a repo that
has both, and a wrong base means an agent works from the wrong code for an hour.

**Dirty is exactly:**

```
git status --porcelain --untracked-files=all --ignore-submodules=none
```

**with the flags passed explicitly.** git's own `worktree remove` runs that status
*without* a `-u` flag, so it honours the user's `status.showUntrackedFiles` — and with
that set to `no`, a real setting people use on large repositories, git's check goes
blind to untracked files and deletes an agent's never-staged work.

**Shell out to `git`; do not use a git library.** This is not a language preference:
**libgit2** — which nearly every language binds — has no worktree-remove at all, and its
prune deletes the directory with **zero** cleanliness checking. Bindings then add their
own hazard on top: the Rust one's status defaults are wrong in *both* directions, one
call counting ignored files and another skipping untracked ones. One tool, git's own
semantics, and no second definition of "clean" to keep in sync.

**Ignored files do not count, and are deleted with the worktree.** This matches git, and
the alternative is unusable: count them and no worktree in a project that builds is ever
retirable. The cost is real — a spawn's `.env` goes with it — and is recorded rather
than hidden. **Stashes do not count either, and that is genuinely safe**: a stash made
inside a worktree lives in the *repository's* stash list and survives removal.

**Known blind spot:** files marked `--assume-unchanged` never appear in status, so they
are invisible to any check built on it. Recorded, not fixed.

**A consequence worth noticing:** a crashed launcher strands worktree metadata that
blocks reuse of that path and is not reaped for three months. Because names carry a
random suffix, **the app never reuses a path**, so stranded metadata never blocks
anything.

### 4.8 Concurrency

**There is still no per-spawn task.** Twenty spawns are twenty *rows* and twenty grids,
served by one reader — not twenty tasks. What changed with §4.1 is that output is now read
at all.

- **Main thread** — terminal, input, render.
- **One reader thread** — a blocking read loop on the control-mode client, decoding
  `%output` notifications and applying the bytes to the emulator that owns that pane.
- **One supervisor thread** — ticks, builds an **immutable snapshot** of statuses, sends it
  down a channel. The UI renders the latest snapshot received.
- **Short-lived workers** for slow one-shots — worktree creation among them (§4.4) —
  reporting over the same channel.

**A blocking concurrency model, not an asynchronous one.** Unchanged, and the reader
strengthens the case rather than weakening it: asynchrony earns its keep multiplexing
hundreds of simultaneous waits, and there is exactly **one** stream to read however many
spawns exist. Whatever the language, prefer its plainest concurrency primitive over its
most powerful one.

**Both transports, each for what it is good at.**

> **Revised 2026-08-10.** This section previously read "Plain per-command `tmux`. No
> control mode." That is reversed for output and retained for everything else.

- **Control mode carries output.** One client, attached once at start-up, running **inside
  a pty of the app's own** — verified: on piped stdio tmux refuses outright with
  `tcgetattr failed: Inappropriate ioctl for device`. **One client carries every pane in
  the session**, including windows that are not visible, which is what makes a single
  reader viable at all — verified on tmux 3.4, with pane ids `%0`…`%(n-1)` in creation
  order on a fresh server.
- **Per-command `tmux` carries facts and commands** — creating windows, sizing panes,
  killing them, and the `list-panes -a -F` tick. Control mode still **cannot report
  process death**, which was one of the original reasons to decline it; that reason was
  correct and survives, so the status ladder (§4.5) still polls.
- **Input goes back over the client**, as `send-keys -H`.

**The single reader is untested at twenty, and that is the sharpest unknown the §4.1
revision introduces.** One thread now decodes every byte that twenty fullscreen agents
draw, and it is a serialisation point with no backpressure story: if decoding cannot keep
up, output backs up in the control client's pipe and every spawn lags at once, not just
the visible one. Nothing in §7 tested control mode past a handful of panes — the prototype
says so itself. The mitigations if it bites are ordinary (decode off the read, one
emulator per thread, or drop frames for spawns that are not on screen) and none of them
changes the design, which is why this is recorded as a risk to measure during the
walking skeleton rather than a decision to take now.

> **Measured 2026-08-10, and only at the cheap end.** Eight panes drawing at once are
> routed correctly and none is dropped — an integration test that will fail if the reader
> ever mixes panes up under load. **That is separation, not throughput**: twenty
> continuously-redrawing fullscreen agents remain untested, and the risk as written stands
> until something drives that many. Worth knowing what the test *would* catch first,
> which is the failure that would otherwise look like one spawn being mysteriously blank.
>
> **Driven at twenty, 2026-08-12.** Twenty spawns over four repositories, each redrawing
> a whole alternate screen five times a second, and **the single reader kept up**: the
> screen was never more than one of a spawn's own repaints behind what that spawn had
> drawn, and switching between spawns mid-turn took **1.2 ms**. A tick stayed **one
> `list-panes` and a stat per live spawn**, with the `ps` probe running once in ten
> seconds rather than per spawn. What was observed — including the tmux server's seven megabytes at twenty
> panes, and everything the measurement does *not* cover — is in
> [`docs/evidence/scale-at-twenty.md`](../evidence/scale-at-twenty.md). **The risk is
> bounded rather than closed**: what drew was a stand-in at a fixed cadence, not twenty
> real agents, which stays the thing to sit down and look at.

**Screen priming is a genuine cost of control mode, not a wrinkle.** It streams only what
is produced **while a client is attached** — attach after a child has already drawn itself
and the grid stays permanently blank, with no catch-up short of priming from
`capture-pane`. Tranche 1 sidesteps this rather than solving it: the app attaches before
it starts anything, so nothing is ever running unobserved. A later tranche that recovers
pre-existing spawns (§4.9) inherits the problem, and priming from `capture-pane` is the
known answer. The prototype hit it head-on, and works around it by starting each pane with
a holder process, attaching, and only then respawning with the real command.

**A tick** is one `tmux list-panes -a -F` covering every spawn at once, plus a `stat` per
live spawn read only when mtime moved, at roughly 200 ms. The `ps` foreground-process-
group probe is a **tie-breaker**, not a per-tick cost.

**Slow work shows on the draft row**, which is also the error surface — the rule and its
reasoning are in §4.4; what belongs here is only that `git worktree add` takes seconds, so
it runs on a worker and not on the tick. Notably it needs **no fourth status**: "creating"
is a state of a draft becoming a spawn, not a state of a spawn.

### 4.9 State, and what survives

**The world is authoritative; the app's memory is a cache.** tmux and the filesystem hold
the truth about panes, pids, worktrees and branches. Where they disagree, the world wins.

The app holds:

- **The spec** — repository, description, model, effort, name. The only thing that
  exists nowhere else once the process is running, and therefore the only thing the app
  truly owns.
- **Handles** — window and pane id, pane pid, worktree path, branch. Facts tmux and git
  already know, cached to avoid asking every tick.
- **The last observed status**, purely so the list renders between ticks.
- **A screen grid per spawn** — new with §4.1, and the one part of the world the app
  cannot re-derive. tmux knows the child's *current* screen; the grid is what the app
  has *observed*, accumulated from the control-mode stream since it attached. It is a
  cache of bytes already seen, held in memory and written nowhere.

**Identity is the random suffix, reused** — the same string that names the branch and the
worktree. The on-disk artifacts therefore identify themselves, which is what lets the
start-up report say something more useful than "there is stuff here".

**Nothing is written to disk as persistence.** With drafts no longer running as separate
processes (§4.4), the handover file that was the one exception is gone too.

> **Revised 2026-08-10.** Two entries above changed with §4.1: the holding-session name
> left the handle list along with parking, and the grids joined it. Recovery got
> strictly harder in one respect — a recovered spawn's grid starts blank, because control
> mode streams only what is produced while attached (§4.8). Priming from `capture-pane`
> is the known answer, and it belongs to the tranche that does recovery.

**Recovery is lossy, and that is accepted.** A future tranche can rediscover panes, pids,
worktrees and branches from the world. It cannot rediscover the spec — though more comes
back than expected, since the branch name carries a slug of the description and `-n` puts
the display name in the terminal title. **Model and effort are genuinely lost.** A marker
file in each worktree would fix it and is **deliberately not written**: that is a decision
to take on its own terms later, not to slip in on suspicion.

---

## 5. What it does not tackle

### 5.1 Deferred — in the vision, not in this tranche

Sandboxing a spawn · the agent that creates spawns from natural language · the agent that
monitors and relays their questions · the agent that eventually answers on your behalf ·
notifications · remote spawns · surviving the app closing · seeing what a spawn produced
(diffs, summaries) · spawns sourced from a backlog · the hard parts of multi-project
(working out which project a task belongs to; one spawn touching several repositories) ·
any harness other than Claude Code.

Having spawns on **different** repositories is *in* — that part was never the hard part.

### 5.2 Excluded — outside the product entirely

Being an editor or IDE · reimplementing a harness's interface rather than surfacing it ·
teams, multi-user, coordination between people · **doing or landing the work** —
committing, pushing and opening pull requests stay with the harness and the project's own
workflow.

### 5.3 Deliberately left open

Not decided, and not oversights:

- **Error presentation beyond the creation path.** Creation is settled — errors land on
  the draft, which keeps your text. Where the exit message and the start-up orphan report
  appear is not.

  > **The `unknown` half was closed 2026-08-12.** This entry also read *"how an `unknown`
  > spawn's reason reaches the slot"* was undecided. It is decided: the reason is drawn
  > **in the slot, over the top of the spawn's own screen rather than instead of it** — an
  > unaccountable spawn is very often still running, since the status is about the app's
  > instrumentation rather than the agent, and taking its screen away would hide a live
  > session in order to complain about the app's own eyesight. The top is what it covers,
  > because the bottom of a harness's screen is where it asks you things. It says more
  > than the sentence the row carries: the pane, the process whose status would not
  > resolve, that the pane is alive, and the last status the app managed to read — which
  > are the facts that separate *the harness moved its records* from *this spawn died and
  > the app has not noticed*. It is the app writing about itself, so it goes the moment
  > the app can account for the spawn again.
- **Configuration** — whether the app remembers anything between runs, and where. The
  worktree root is deliberately not configurable.
- **Keybindings, and scrolling past a screenful** — specifically how the keyboard is
  split between the app's own commands and the spawn in the slot. This previously read
  "how the app and tmux share the keyboard", which §4.1 made meaningless: tmux is never
  attached and holds no keyboard. The split is now entirely the app's to decide, which
  makes it easier but no less undecided — and it is the mechanism behind two things the
  scope explicitly asks for, *typing into a spawn's prompt* and *interrupting it*. Worth
  knowing that it is the largest open item on this list.
- **Distribution** — run from source, or something installable.
- **Logging and diagnostics** — how the author debugs the app while twenty children run.
- ~~**Answering the child's terminal queries**~~ — **closed 2026-08-10 during the
  migration, and it was never a risk under this mechanism.** It read: *"a named risk
  rather than a comfortable omission, new with §4.1. An emulator with no write-back path
  silently drops cursor-position reports and their kin, and Claude Code issues them. The
  transport back exists (`send-keys -H` on the control client); what is undecided is
  which emulator generates the replies, and what breaks if none does. Most likely thing
  to bite during the walking skeleton, so it is the first thing to test against a real
  `claude`."*

  **What it missed is which terminal the child is talking to.** Under *own the pty* the
  app is that terminal, and the gap is real. Under *supervise + render* it is not: tmux
  is the child's terminal, it emulates the pane fully, and it answers `CSI 6 n` and its
  kin itself before ever passing the bytes on. The app's grid renders a copy of a
  conversation that has already been had. Verified against tmux 3.4 by starting a pane
  that asks and reads the answer back on its own input — it arrives, with nothing from
  the app — and kept as an integration test.

  **So the app must not write back**, which is the opposite of the mitigation this entry
  was reaching for: a reply of the app's own would arrive at the child as a *second* one,
  which is to say as keystrokes nobody typed. Recorded at length because the entry was
  right about the mechanism and wrong about whose problem it was, and a later reader
  finding `vt100` has no write-back path would otherwise set out to add one.

  **Still open, and smaller:** anything tmux itself declines to answer is unanswered, and
  a mismatch between tmux's emulation and `vt100`'s shows up as a grid that drifts from
  what the child believes it drew. Neither has been seen; neither is tested against a
  real `claude`, which stays a thing to sit down and look at (§8).

The disposition that governs all of these: **get the app running.** Where simplicity now
creates a problem later, that problem belongs to a later tranche — deliberately, not by
oversight.

---

## 6. Principles

These emerged from the decisions rather than preceding them, and they are the things most
likely to be re-litigated by someone who has not read the reasoning.

**Never trade one view for another.** Simultaneity is the differentiator. It applies past
the spawn list: composing a new spawn is a long-lived, interruptible surface, not a modal.

**Refuse rather than guess** — and **a refusal never destroys the user's typing.** The
second half is what makes the first tolerable.

**Where the app must guess anyway, it surfaces rather than hides.**

**The world is authoritative; the app's memory is a cache.**

**Owning a process does not license being opaque.**

**Three statuses, and they are about the agent.** Anything tempting a fourth — starting,
creating, failed — is a state of something else, and belongs on that thing.

**Nothing is a fixed size.**

**Tranche 1's omissions are scope, not architecture.** Where a simplification would
otherwise harden into an assumption, leave a designed-for hole.

**A decision's reasons can expire before the decision does.** Embedding was rejected on
an arithmetic that a later choice in this same document had already invalidated, and the
fullscreen requirement outlived both of its original justifications. When a decision is
reopened, re-derive its reasons rather than inheriting them — and when a revision lands,
say what the section used to say, because the superseded reasoning is the part that
would otherwise be silently reinvented.

---

## 7. Evidence

Design decisions here rest on work that was done rather than reasoning that was asserted.
All of it lives on unmerged branches, as reference material rather than repository
content.

| branch | what it established |
| --- | --- |
| `research/claude-code-control` | Ten documented control surfaces for Claude Code; input is keystrokes with no stable contract; driving the TUI under a pty is undocumented |
| `research/rust-tui-frameworks` | The terminal-UI library landscape **in Rust** — the one finding here that does not transfer; redo it for another language |
| `research/embedded-child-tui` | The four sharp edges of pty embedding — still load-bearing, and now the app's own problem. Its ~1.3 GB scrollback arithmetic is **superseded**: it assumed a generous history per pane, which fullscreen removes (§4.1) |
| `research/git-worktrees-from-rust` | ~30 experiments: the exact dirty check, the silent branch adoption, libgit2's missing worktree removal |
| `research/driving-tmux-from-rust` | 92 verified claims on tmux 3.4: parking works detached; the structural resize; control mode's blind spots |
| `research/prior-art` | firstmate, ccmanager and omnigent read from source — and the correction about the status file |
| `prototype/list-pane-layouts` | Three layouts compared at honest density; layout C chosen; the slot model |
| `prototype/redraw-switch` | The redraw verdict: perfect under fullscreen, artifacting under classic. Settled parking's cost; parking is now gone, and the fullscreen half of the verdict is what survives |
| `prototype/render-alternatives` | The two rejected alternatives built behind **one shared renderer**: control mode and own-the-pty are indistinguishable to look at, so rendering cannot decide between them. Also: a control client needs a tty; one client carries every pane in a session; control mode streams only what is produced while attached |

Most of this transfers. The tmux, git and Claude Code findings are properties of those
tools, not of the language that drives them; the prototypes' verdicts are about what a
terminal does. **Only the terminal-UI library survey is ecosystem-bound**, and would need
redoing to build this elsewhere.

**Three claims in the research were later corrected**, and all three corrections are
reflected above: that `claude agents --json` ruled out the whole status-file mechanism
(the command cannot see interactive sessions; the file can); that splitting the code into
separate compilation units would enforce the harness module's no-I/O rule (it does not —
a standard library is available unconditionally, so only *external* dependencies are
gated); and the ~1.3 GB scrollback figure, which was measured honestly but applied after
the fullscreen decision had already removed the history it assumed (§4.1).

**The pattern across all three is the same**, and it is worth naming: each was a correct
finding that stopped being true of *our* situation, and none of them announced it. That
is the argument for prototyping decisions that have already been made — which is what
`prototype/render-alternatives` was, and it overturned the biggest one in the document.

---

## 8. Building it

The design is sliced into implementation tickets **walking-skeleton first**: the first
ticket goes end to end thinly — pick a repo, create a worktree, start `claude` in a tmux
pane, read its output over the control-mode client, and draw it in the slot beside a list
of one — and everything after widens a stripe that already works.

> **Revised 2026-08-10.** The skeleton got one segment longer and one segment shorter.
> Longer: it now has to attach a control client and emulate a terminal before anything
> appears on screen, and **the emulator is the riskiest thing in the tranche** (§5.3), so
> the skeleton is where it should be proven. Shorter: no parking, no holding session, no
> draft process, no handover file.
>
> **The skeleton was already built under the old mechanism**, and is on `main`. It walks
> the whole path — worktree, branch, window, a live session in the slot — and everything
> it proved about worktrees, branch naming, the harness seam and refusing before tmux is
> involved is unaffected. What has to be rebuilt is the window: how the app gets a screen
> and how the bytes reach it. That migration is tracked as its own piece of work rather
> than folded silently into the next ticket.

Seams decide **where code lives**; they are a poor guide to what a ticket *delivers*,
because a ticket per seam means nothing runs until the last one lands.

**Project shape.** One program, with the harness module in its own directory. Drafts are
state inside it, not a second program and not a second mode (§4.4).

**The two invariants must be mechanically checked**, whatever the language:

```
nothing in the harness module may reference process, filesystem or tmux APIs
nothing outside it may mention Claude Code — the binary name, a flag,
an environment variable, a status vocabulary, or a screen shape
```

A text search in CI is a legitimate implementation of that check, and its limitation
should be recorded rather than hidden: text matching is fooled by aliasing. Reach for a
stronger mechanism — separate compilation units, a lint with per-directory scope — when
there is a second author, a second harness, or a reason to bar the pure side from an
external dependency. Note that **splitting into separate compilation units does not by
itself enforce the rule**, since a standard library is always available; it gates
*external* dependencies and makes the boundary visible, which is worth something else.

**Dependencies.** Still no git library, no tmux library, no asynchronous runtime; most of
the program remains subprocess calls and pure functions, which is why so much of it is
cheap to test. But **terminal emulation came back** with §4.1, and with it a pty — not for
the children, which tmux owns, but for the control client, which refuses to run on piped
stdio (§4.8). The list is now: **a terminal UI library, a terminal emulator, a pty
library, and a JSON parser.**

That is the price of the §4.1 change stated plainly. The emulator is not a small
dependency dressed as a large one — it is the component that has to be right for the
product to be usable at all, and §5.3 records the specific way it can be wrong.

**Test seams, agreed before any test is written:**

1. The harness module's interface — pure unit tests
2. Snapshot building — pure, against **captured** tmux and status-file output
3. Worktree operations — integration, real `git`, throwaway repositories
4. tmux operations, **including the control-mode client** — integration, real tmux on a
   private `-L` socket: attach, run something that draws, assert the bytes arrive tagged
   with the right pane id
5. UI rendering — pure: render into an off-screen buffer and assert on it, no terminal
6. **Terminal emulation** — pure: feed *captured* child output into the emulator and
   assert on the resulting grid. New with §4.1, and the seam most worth having, because
   this is now the hardest thing the app does

**Not tested, by agreement:** the visual feel of a redraw or a colour scheme — the
prototypes covered that and it is a judgement, not an assertion; and draft creation end
to end launching a real `claude`, which costs tokens and needs auth. Note the tension
with §5.3: emulator fidelity against a *real* `claude` is precisely what automated tests
will not cover, so it stays a thing to sit down and look at.

**No abstraction and no fake for tmux or git.** A second implementation existing only for tests
would make the harness seam "real" on false pretences and would be the abstraction
deliberately declined in §4.3. It is avoidable because the real things are cheap and
hermetic — demonstrated, not assumed, by the git research and the redraw prototype.

---

## Appendix A — The Rust implementation

Everything above is language-agnostic. This is what was chosen for the build actually
being done, and it is the only part to discard if this design is reused elsewhere.

> **Revised 2026-08-10.** §4.1 put two crates back on this list that earlier revisions of
> this document had struck off — a terminal emulator and a pty library — and re-justified
> a third. Nothing here was removed.

**Language: Rust.** Chosen partly for its own sake — this project doubles as the author's
way of learning it. That shapes several calls: **favour idiomatic, well-trodden Rust over
clever**, and prefer the construct that is easiest to reason about and debug over the one
that benchmarks best, at a scale where the difference is unmeasurable anyway.

**A single binary crate**, not a workspace. Cargo ceremony is a poor first Rust lesson,
and — per §8 — a crate split would not have enforced the no-I/O rule it was originally
credited with, because `std` is unconditional.

**The invariant checks** are two greps in CI:

```
grep -rE 'std::process|std::fs|Command::' src/harness/                 # must find nothing
grep -rEi 'claude|--effort|CLAUDE_CODE' src/ --exclude-dir=harness     # must find nothing
```

**Alternatives, for when greps stop being enough.** A **workspace split**, which gates
external crates and makes the boundary crossable only via `use`. And clippy's
**`disallowed-methods`**, which is real linting rather than text matching but is
workspace-scoped, so it cannot express "banned here, allowed there" until a workspace
exists. The two compose: the lint becomes usable *after* the split.

**Concurrency: threads and channels, no async runtime.** Beyond the general argument in
§4.8, this avoids choosing a runtime, coloured functions, and `Send` puzzles — and the
ownership errors threads produce are the ones the book teaches you to read. The cost is
that a good deal of copied-from example material assumes tokio and will need adapting.

**Terminal UI: ratatui.** Chosen for documentation depth at a beginner's level, for not
owning the event loop — which suits a blocking design — and because recomputing layout
from the real terminal every frame is its normal mode, which §3 requires. Its
`TestBackend` renders into a buffer, which is how test seam 5 is realised. Its original
deciding requirement was that it could host an embedded terminal; that requirement went
away when the app delegated rendering to tmux, and **came back with the §4.1 revision** —
so the crate is now chosen on every ground it was ever considered on.

**Terminal emulation: `vt100`, drawn by hand into ratatui cells.** Back on the dependency
list with §4.1. `tui-term` wraps `vt100` for exactly this and is the obvious convenience;
the prototype deliberately drew the grid itself, to keep the experiment about the
emulator's fidelity rather than a wrapper's version compatibility, and the same reasoning
applies to the build until there is a reason to add the layer.

**`vt100` has no write-back path**, and — **as of 2026-08-10 — that is fine, and adding
one would be a bug.** This previously read that the missing path "is precisely the gap
§5.3 names… the first reason to go looking at another crate". It is not a gap: tmux
answers the child's queries as its terminal, and a second answer from the app would reach
the child as keystrokes nobody typed (§5.3). The crate's limitation and this design's
needs happen to line up exactly.

**The pty for the control client: `portable-pty`.** Not for the children — tmux owns
those — but because a control-mode client refuses to run on piped stdio (§4.8). One pty
for the whole app, not one per spawn.

**Tooling:** `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, plus the
two greps. The same set is what runs before a push.
