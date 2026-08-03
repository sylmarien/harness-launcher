# Tranche 1 — The core loop

> **Status: in progress.** This document defines **the scope of a slice of work** —
> which features from the wishlist are in, and which are not.
>
> It is deliberately **not**: a design, an implementation plan, a breakdown into
> issues, pull requests or commits, or an estimate. None of those belong here. It says
> *what the software does* at the end of this tranche, at a high level, and what that
> feels like to use.
>
> **Once agreed, this document is frozen.** It can still change — but only as a
> deliberate, visible re-scope, never as silent drift to match whatever got built. A
> scope document that quietly tracks reality cannot tell you that you overran, which
> is most of what it is for.

Slices from [the product vision](../product-vision.md).

## When this ships, I can…

> Start several Claude Code spawns — each on whatever local git repository I choose —
> from one place, see at a glance which ones need me, and open one to answer. No
> worktree created by hand, no terminals to juggle.

**The goal is not to get out of the terminal.** This tranche will very likely land as
a TUI, running in a terminal, and that is fine. What goes away is having to juggle
*several* terminals — one per piece of work. The terminal itself is not the problem;
the plural is.

The bar is the one the vision already sets: beat 4–5 concurrent sessions handled
manually, **on friction reduction alone**, before any of the cleverer features exist.

## What's in

Each item names the part of the vision it slices from.

- **Starting a spawn.** I choose a local git repository, describe the work I want done,
  and choose the model and effort level. *(→ The experience — Starting work.)* Harness
  selection is trivial in this tranche: there is only Claude Code.

  **Each spawn names its own repository**, and the dashboard spans all of them. Two
  spawns on different repositories are not a special case — nothing about statuses,
  worktrees or retirement changes. Restricting the whole tranche to a single repository
  would be a constraint the vision never asked for, and would undercut the bar of
  beating 4–5 manual sessions, several of which are in different projects anyway.
- **The app does the setup.** Before the harness starts, the app creates the git
  worktree and starts the harness inside it. *(→ The experience — Starting work.)*
- **The dashboard.** Every spawn sits as an entry alongside the others, each with its
  status. The view is live: no refreshing, and no wondering whether what I am reading
  is current. *(→ The experience — Watching work.)*

  **Two statuses, and only two: the agent is working, or the agent has stopped.** The
  app does not claim to know *why* it stopped — finished, waiting on a question, or
  dead. "Stopped" is exactly the set that might need me; I glance, I see which ones
  stopped, I open them and look. So the headline above means, precisely, *see at a
  glance which ones have stopped* — a promise this tranche can keep, and one that
  still kills the daily round-trip through several terminals hunting for an idle
  agent.

  Inferring *waiting for my input* specifically is deliberately out. The vision already
  warns it may be indistinguishable from a finished turn, and any attempt would key off
  one harness's output — the Claude-Code-shaped corner that is hard to walk back.
- **Opening a spawn.** The full view of the agent, exactly as if I had started it by
  hand: watch it work in real time, type into its prompt, interrupt it, see its
  sub-agents. *(→ The experience — Watching work.)*

  **Opening a spawn does not replace the dashboard.** I see the agent *and* the list of
  spawns at the same time. This matters: I must never have to choose between following
  one piece of work and knowing whether another has stopped and needs me. Losing the
  list while reading a spawn would reintroduce the hunt this tranche exists to remove.
  *(The vision anticipates this where it notes the dashboard "might be a pane".)*
- **Retiring a spawn.** An explicit action from me meaning "we are done with this
  spawn" — and only then does the app clean up the worktree it created. Never inferred
  from the agent falling silent. *(→ The experience — Retiring a spawn.)*

  **Retiring removes the worktree and leaves the branch alone.** The worktree is the
  app's to remove, because the app created it. The branch is not: it holds committed
  work, and deleting it is a different and riskier act than removing a checkout.

  **If the worktree is dirty, retiring refuses.** No confirmation flow, no prompt — I
  clean it up myself, then retire. This is the lean option on purpose: it is the only
  place in this tranche where the app could destroy work, and refusing is both smaller
  to build and safer than asking.
- **Enough scale to be worth it.** Comfortably past the 4–5 concurrent sessions I
  manage by hand today. *(→ How many spawns at once.)*

## What's not in

Two different things live here, and they must not be confused:

### Deferred — in the vision, not in this tranche

- **Sandboxing a spawn.** Agents run loose in their worktrees for now.
- **The agents**: the one that creates spawns from natural language, the one that
  monitors and relays their questions, and the one that eventually answers on my
  behalf.
- **Notifications** — anything that raises information to me rather than waiting for
  me to look.
- **Remote spawns.** Local machine only.
- **Surviving the app closing.** Closing the app shutting everything down, a blank
  slate next start, and leftovers that are never cleaned up is acceptable here — the
  vision says so explicitly. I may have to stop harnesses myself.
- **Seeing what a spawn produced** — diffs, summaries of what changed.
- **Spawns sourced from a backlog.**
- **The hard parts of multi-project.** Having spawns on different repositories is in
  (above). What stays out is anything that *works out* which project a task belongs to,
  and any single spawn touching more than one repository.
- **Any harness other than Claude Code.** Note that the vision's commitment to
  extensibility is a *design* constraint that applies from the start — the interface a
  second harness would slot into is not a tranche 1 feature, but painting ourselves
  into a Claude-Code-shaped corner is exactly the kind of choice that is hard to walk
  back.

### Excluded — outside the product

From "Where the app stops" in the vision. Not happening, in this tranche or any other:

- being an editor or an IDE;
- reimplementing the harness's own interface rather than surfacing it;
- teams, multi-user, coordination between people;
- **doing the work.** Committing, pushing and opening pull requests stay with the
  harness and the project's own workflow. The app never lands the work.

## Back-filling the vision

Anything agreed here that is **not** a slice of the existing wishlist — a feature
invented during this discussion — must also be **added back into
[the product vision](../product-vision.md)**.

Otherwise the vision starts lying by omission the moment tranche 1 is scoped, and it
is the document we point at to judge whether later proposals are aligned. An item here
that cannot name where it came from is either an addition needing back-fill, or scope
creep.
