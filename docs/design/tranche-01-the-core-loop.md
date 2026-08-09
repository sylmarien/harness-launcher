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

**The app never types into a session.** After launch, your own keyboard is already on
that pane. See §4.3 for why that is scope rather than architecture.

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

Selecting a spawn puts it in the **slot** — the pane beside the list — where you see
the real Claude Code interface, byte for byte. You type into it, interrupt it, watch
its sub-agents. It is the actual program, not a reconstruction.

**The list stays visible the entire time.**

### Composing new work

Creating a spawn is not a modal dialog. Instructions can be long, and you must be able
to leave a half-written draft, go deal with a spawn that stopped, and come back.

So a **draft is a pane, exactly like a spawn**. It takes the slot when selected, parks
when not, gets its own pinned row in the list, and survives being walked away from.
**Several drafts can be in flight at once**, at no extra cost, because a parked draft is
just a parked pane.

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

A single tmux window: the app's list pane on the left, the slot on the right.

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

**The separator is always tmux's.** Both a spawn and a draft occupy the slot as a tmux
pane, so there is never a mode where the app draws its own divider and the two fail to
match.

Three layouts were built and compared side by side against a mock slot. The one chosen
groups by repository. Rejected: a flat list sorted by attention (denser, but reads as an
inventory rather than a workspace) and sections grouped by attention (strong when
something needs you, but degenerating to one undifferentiated fourteen-item list when
everything is working — which is the common case).

Prototype: branch `prototype/list-pane-layouts`.

---

## 4. How it works

### 4.1 The app drives tmux; it does not embed terminals

This is the load-bearing decision. Everything else assumes it.

Three shapes were considered:

| | what it means | verdict |
| --- | --- | --- |
| **Embed** | the app owns a pty per spawn and renders it in its own pane | rejected |
| **Hand off** | the app manages spawns; viewing one means leaving the app for tmux | rejected |
| **Drive** | the app composes a tmux layout — list pane, slot pane | **chosen** |

**Hand-off dies on simultaneity.** It is what ccmanager and `claude attach` do, and it
takes the list away.

**Embedding was rejected on cost.** Hosting a child terminal means owning a pty *and*
a terminal emulator, and the research found that road well-trodden but sharp-edged. The
specific stack examined was Rust's (`portable-pty` + `vt100` + `tui-term`), but the
hazards are properties of the technique rather than of any one library, and any
language's equivalent stack should be assumed to share them:

- **The emulator must be able to answer the child.** Terminals reply to queries —
  cursor-position reports among them, which Claude Code issues. An emulator with no
  write-back path drops them silently.
- **Resize is lossy**, and a common source of crashes at the boundary between wide
  glyphs and narrow panes.
- **Mouse forwarding is a hand-written protocol bridge**, with several independent
  encodings to get wrong.
- **Scrollback is the binding cost** — roughly **1.3 GB across twenty panes** at
  generous history, because every pane holds a full grid. ccmanager's source shows the tax
concretely — it resets kitty keyboard protocol, `modifyOtherKeys` and focus tracking so
they do not leak between sessions.

**Driving tmux gets the technical wins without the exit.** The multiplexer owns the
pty, so no terminal emulation is written: resize, mouse, scrollback, alternate screen
and colour are its problem and are solved properly. What it does *not* inherit is
hand-off's simplicity — driving is a live control relationship, not a one-way exec.

**tmux only.** zellij is a maybe-someday and its pane model is **knowingly unverified**;
if it differs in kind, that is a rewrite rather than a swap. Accepted deliberately.

**No dependency on Claude Code's own background-session daemon.** It exists — `claude
agents`, `claude attach`, its own worktree management — and building on it was
considered. Rejected: it is Claude-Code-specific and would land on the wrong side of the
harness seam, it is a research preview, and it creates and cleans up worktrees itself,
which collides head-on with the app owning worktrees.

### 4.2 The slot, and the parking mechanism

One visible window. The list on the left; the slot on the right holds **one** spawn or
draft. Everything else is **parked in a dedicated detached holding session** and moved
in and out with `break-pane` / `join-pane`.

Why a holding session rather than off-screen windows in your own session: it keeps the
user's workspace unlittered when the app takes over an existing window, and it serves
both start-up modes with one mechanism.

**Mode detection is `$TMUX`**: not already in tmux → start a session; already in tmux →
take over the current window.

**Verified:** `break-pane` into a fully detached holding session works, preserving pane
id, pid and scrollback. Tested on tmux 3.4.

**The cost, verified and accepted:** every park and unpark **resizes the pane and
SIGWINCHes the child**, structurally — the parked window is created at the source
window's size, and `window-size manual`, a same-sized holding session, and parking via
`join-pane` instead were all tried and none avoids it. So every switch forces Claude
Code to repaint.

**This was tested rather than argued.** Under the **fullscreen renderer, switching
renders perfectly** — no flicker, no tearing, no lost content. Under the **classic
renderer there is visible artifacting**. Prototype: `prototype/redraw-switch`.

That is why §4.6 makes fullscreen a requirement rather than a preference. Note also that
**no comparable project has ever paid this cost**, because none keeps the list visible —
there was no outside evidence, and testing was the only way to know.

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
status_of(record)        -> Working | Stopped             // the app read the file

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

**The input hole is deliberate.** Tranche 1's app never sends input, but the vision's
managing agent will. So *start a session* and *send input to a session* are separate
concepts even though only the first is implemented, and delivering work is **not**
defined as "the prompt is a launch argument" — that is one implementation of *give this
session some work*, already false for a second turn.

**Two invariants, checkable:**

- Nothing outside the module mentions Claude Code — not the binary name, a flag, an
  environment variable, a status vocabulary, or a screen shape.
- The module mentions neither tmux, nor the filesystem, nor processes.

### 4.4 One binary, and drafts as panes

The slot shows a spawn *or* a draft, but a process draws into one pane. So who draws the
draft?

**Rejected:** the app's own pane expanding to full width and drawing both columns itself
when no spawn is joined. It gives the app two rendering modes, and the separator between
the columns would be *ours* in one mode and *tmux's* in the other — a permanent source of
visual mismatch. It also makes multiple drafts a pile of app-side state.

**Chosen:** a draft is a tmux pane running **the same binary** in a draft mode. Not a
second program — it shares the rendering code, the theme, and the choice lists, so a
draft pane and the list pane cannot drift visually.

**The draft does its own creation.** It resolves the default branch, creates the
worktree, and starts the harness — because the draft pane is where progress has to be
*visible*, and only the process doing the work can show it. This also puts the refusals
where the user's typing already is.

**The draft pane becomes the spawn pane.** Once the worktree exists, the draft `exec`s
`claude` in place. The pane id never changes; the window is renamed. No pane to create,
no dead pane, no flicker — and the draft sets the fullscreen environment itself.

**Handover is a file the app already polls**, read on the same tick that reads
`list-panes` and status files. No socket, no protocol, no second event loop.

**Owning the process does not license being opaque.** The draft writes what it is doing
as it does it — intent before action — so a draft that dies mid-creation leaves a record
of the worktree it made rather than a mystery.

**No lock.** A per-repository or global creation lock was considered and dropped: the
collision it would prevent is already impossible, because names carry a random suffix
and paths are never reused, and concurrent `worktree add` operations were verified to
work. Worth noting the cost it would have carried — creation happens in draft processes,
so any lock would have to be an **inter-process** lockfile, not an in-memory mutex.

### 4.5 Knowing what a spawn is doing

**A ladder, not a single signal:**

1. **Is the pane alive?** From tmux — `pane_dead`, or the pane missing entirely. Dead →
   **stopped**, immediately. The one signal that cannot go stale.
2. **If alive, read the session status file**, keyed by `pane_pid`. `busy` and `waiting`
   → **working**; `idle` and `shell` → **stopped**.
3. **If alive but the file will not resolve** → **unknown**, after a grace period.

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
glance — you open it, see it working, move on, and you were going to that pane anyway. A
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
`vim`. **Classic** writes into the terminal's native scrollback — which here means
tmux's.

**The app forces fullscreen.** Not inherited from the user's setting, which varies by
when they first used Claude Code.

Chosen originally for **memory**: tmux server RSS works out at roughly 216 MB across
twenty panes on default scrollback, governed by the user's own `history-limit`, which
people commonly set to 10k–50k. Under the classic renderer every spawn pours its
transcript into that. Under fullscreen, panes hold a screen and not a history — which
dissolves the question of whether the app should manage `history-limit` at all.

Then testing found a **second, independent reason**: switching spawns artifacts under
classic and is flawless under fullscreen (§4.2). So classic is **not a supported
configuration**, and the constraint is load-bearing rather than tidy. It reads like an
imposition on user preference until you know that.

**Accepted cost:** scrollback inside a spawn becomes Claude Code's business —
`capture-pane` will not show history, and tmux-native scrollback search over a spawn's
past output goes away.

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

**There is no per-spawn task.** tmux owns the children and their output is never read.
Twenty spawns are twenty *rows*, not twenty tasks. That collapses the requirement to
three things: the UI stays responsive, a tick runs about five times a second, and
occasionally something slow happens.

- **Main thread** — terminal, input, render.
- **One supervisor thread** — ticks, builds an **immutable snapshot**, sends it down a
  channel. The UI renders the latest snapshot received.
- **Short-lived workers** for slow one-shots, reporting over the same channel.

**A blocking concurrency model, not an asynchronous one.** Asynchrony earns its keep
multiplexing hundreds of simultaneous waits; this is one subprocess per tick and twenty
file stats. Blocking calls on a couple of threads are exactly as fast here and far
easier to read, and the arrangement — one producer of snapshots, one consumer that
renders — maps onto the problem directly. Whatever the language, prefer its plainest
concurrency primitive over its most powerful one.

**A tick** is one `tmux list-panes -a -F` covering every spawn at once, plus a `stat` per
live spawn read only when mtime moved, at roughly 200 ms. The `ps` foreground-process-
group probe is a **tie-breaker**, not a per-tick cost.

**Plain per-command `tmux`. No control mode.** Everything control mode offers had already
been declined — we do not scrape panes, and the tick is polling by design — and it cannot
report process death at all. Rejecting it also **dissolved a known unknown**: Claude
Code's fullscreen renderer is documented as incompatible with iTerm2's tmux integration
mode (`tmux -CC`), and whether our own control client would trip that was undocumented
and untested. Never opening one means the question cannot arise.

**Slow work shows on the draft row**, which is also the error surface. `git worktree add`
takes seconds; pressing enter and getting silence would feel broken. On success the row
becomes a spawn row; **on failure it stays a draft with your text intact**, so "refuse
rather than guess" never costs you the paragraph you just wrote. Notably this needs **no
fourth status** — "creating" is a state of a draft becoming a spawn, not a state of a
spawn.

### 4.9 State, and what survives

**The world is authoritative; the app's memory is a cache.** tmux and the filesystem hold
the truth about panes, pids, worktrees and branches. Where they disagree, the world wins.

The app holds:

- **The spec** — repository, description, model, effort, name. The only thing that
  exists nowhere else once the process is running, and therefore the only thing the app
  truly owns.
- **Handles** — pane id, pane pid, worktree path, branch, holding-session name. Facts
  tmux and git already know, cached to avoid asking every tick.
- **The last observed status**, purely so the list renders between ticks.

**Identity is the random suffix, reused** — the same string that names the branch and the
worktree. The on-disk artifacts therefore identify themselves, which is what lets the
start-up report say something more useful than "there is stuff here".

**Nothing is written to disk as persistence.** The draft handover file is a channel across
a process boundary, not a record; stale ones are cleared at start-up.

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
  the draft, which keeps your text. How an `unknown` spawn's reason reaches the slot, and
  where the exit message and start-up orphan report appear, are not.
- **Configuration** — whether the app remembers anything between runs, and where. The
  worktree root is deliberately not configurable.
- **Keybindings, and scrolling past a screenful** — including how the app and tmux share
  the keyboard.
- **Distribution** — run from source, or something installable.
- **Logging and diagnostics** — how the author debugs the app while twenty children run.

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

---

## 7. Evidence

Design decisions here rest on work that was done rather than reasoning that was asserted.
All of it lives on unmerged branches, as reference material rather than repository
content.

| branch | what it established |
| --- | --- |
| `research/claude-code-control` | Ten documented control surfaces for Claude Code; input is keystrokes with no stable contract; driving the TUI under a pty is undocumented |
| `research/rust-tui-frameworks` | The terminal-UI library landscape **in Rust** — the one finding here that does not transfer; redo it for another language |
| `research/embedded-child-tui` | The four sharp edges of pty embedding, and the ~1.3 GB scrollback arithmetic that killed it |
| `research/git-worktrees-from-rust` | ~30 experiments: the exact dirty check, the silent branch adoption, libgit2's missing worktree removal |
| `research/driving-tmux-from-rust` | 92 verified claims on tmux 3.4: parking works detached; the structural resize; control mode's blind spots |
| `research/prior-art` | firstmate, ccmanager and omnigent read from source — and the correction about the status file |
| `prototype/list-pane-layouts` | Three layouts compared at honest density; layout C chosen; the slot model |
| `prototype/redraw-switch` | The redraw verdict: perfect under fullscreen, artifacting under classic |

Most of this transfers. The tmux, git and Claude Code findings are properties of those
tools, not of the language that drives them; the prototypes' verdicts are about what a
terminal does. **Only the terminal-UI library survey is ecosystem-bound**, and would need
redoing to build this elsewhere.

**Two claims in the research were later corrected**, and both corrections are reflected
above: that `claude agents --json` ruled out the whole status-file mechanism (the command
cannot see interactive sessions; the file can), and that splitting the code into separate
compilation units would enforce the harness module's no-I/O rule (it does not — a standard
library is available unconditionally, so only *external* dependencies are gated).

---

## 8. Building it

The design is sliced into implementation tickets **walking-skeleton first**: the first
ticket goes end to end thinly — pick a repo, create a worktree, start `claude` in a pane
joined to the slot, beside a list of one — and everything after widens a stripe that
already works.

Seams decide **where code lives**; they are a poor guide to what a ticket *delivers*,
because a ticket per seam means nothing runs until the last one lands.

**Project shape.** One program, with the harness module in its own directory. Not a
second program for drafts: the same binary in a different mode (§4.4).

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

**Dependencies** collapsed to almost nothing as decisions landed — no terminal emulation,
no git library, no tmux library, no asynchronous runtime. What remains is **a terminal UI
library and a JSON parser**. Most of the program is subprocess calls and pure functions,
which is why so much of it is cheap to test.

**Test seams, agreed before any test is written:**

1. The harness module's interface — pure unit tests
2. Snapshot building — pure, against **captured** tmux and status-file output
3. Worktree operations — integration, real `git`, throwaway repositories
4. tmux operations — integration, real tmux on a private `-L` socket
5. UI rendering — pure: render into an off-screen buffer and assert on it, no terminal

**Not tested, by agreement:** the visual feel of a redraw or a colour scheme — the
prototypes covered that and it is a judgement, not an assertion; and the draft end-to-end
launching a real `claude`, which costs tokens and needs auth.

**No abstraction and no fake for tmux or git.** A second implementation existing only for tests
would make the harness seam "real" on false pretences and would be the abstraction
deliberately declined in §4.3. It is avoidable because the real things are cheap and
hermetic — demonstrated, not assumed, by the git research and the redraw prototype.

---

## Appendix A — The Rust implementation

Everything above is language-agnostic. This is what was chosen for the build actually
being done, and it is the only part to discard if this design is reused elsewhere.

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

**Terminal UI: ratatui.** Re-chosen after the delegation decision removed its original
deciding requirement (an embedded terminal). It still leads on documentation depth for a
beginner, it does not own the event loop — which suits a blocking design — and
recomputing layout from the real terminal every frame is its normal mode, which §3
requires. Its `TestBackend` renders into a buffer, which is how test seam 5 is realised.

**Tooling:** `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, plus the
two greps. The same set is what runs before a push.
