# Product Vision — harness-launcher

> **Status: working draft, incomplete.** This is a *wishlist* document: what the tool
> should do when it's done. It is deliberately not a design, not a feasibility study,
> and not a delivery plan. No phasing, no tranches, no implementation choices.
>
> Its job is to be the thing we point at later, when we ask: *is this proposed
> solution aligned with what the product is supposed to be?*
>
> **Everything here is a wish.** Appearing in this document does not mean the product
> gets it early, or ever. Scoping happens later, when design and planning start.
>
> Wishes are unbounded in **depth** — how far any one capability could be taken has no
> natural limit. They are *not* unbounded in **breadth**: the app's responsibilities
> have an edge, and it is described under "Where the app stops" below.

## The problem

### Working with one agent

The current loop with a coding agent is:

1. Go to where you want to work.
2. Start the agent (e.g. Claude Code) there.
3. Ask it to do some work.
4. It works.
5. When it needs input, it stops and asks you.

For one thing at a time, this is fine. The loop is legible and there is exactly one
place to look.

### What breaks when you want more than one thing at a time

The moment you try to raise your throughput by running several pieces of work
concurrently, the process gets messy:

- **Terminal juggling.** One command line per piece of work, and you switch between
  them by hand — including working out which one is currently waiting on you.
- **Repo contention.** Several agents working in the same project must not share one
  copy of the repo. Each needs its own — a separate clone, or a git worktree.
- **It doesn't scale.** Both of the above are manual setup, repeated per workstream.
  Past a small number of concurrent tasks, the overhead of running the process eats
  the productivity the concurrency was supposed to buy.

The bottleneck being attacked is not agent capability. It is the **scattered, manual
process around the agents** — and specifically the human attention it consumes.

## The objective

Replace that scattered manual process with something that makes running multiple
concurrent agent workstreams:

- **efficient** — the setup per workstream is not paid by hand every time,
- **easy to monitor** — you can see what's running and what needs you, without
  hunting through terminals,
- **easy to manage** — you can act on the right workstream when it needs you,
- **correctly placed** — tasks get started in the right project(s), with the right
  isolated copy of the code to work in.

### Multi-project reach

"The right project(s)" is plural in both available senses, and the finished product
is meant to handle both:

1. **Many tasks, many projects** — concurrent workstreams spread across different
   projects, not just several within one.
2. **One task, many projects** — a single piece of work that spans more than one
   project at once.

Both are in scope for the end state. This document takes no position on which
arrives first.

## The experience

> **Framing.** What follows describes *what the user can do* — not what the software
> looks like, and not what kind of software it is. **"The app"** is a placeholder for
> this project's software carrying no implication of shape: command line, TUI, desktop
> app, or anything else remains an open technical decision.
>
> The flows below also assume a **single git repository** per piece of work. The
> multi-project cases are set aside here purely to keep the description simple — they
> have not left scope.
>
> Features are to be *extracted* from this experience later. The experience is the
> source; any feature list is derived from it.

### Starting work — a "spawn"

When I want to start new work, from the app I:

- choose the local git repository to work on,
- describe the work I want done,
- choose which **harness** to use,
- choose which **model**,
- choose which **effort level**.

The app then starts the correct harness, with that model and effort level, hands it
the prompt I typed, and it begins working.

That unit — the act of starting work and the ongoing thing it produces — is a
**spawn**.

**The app does the setup before the harness starts.** For a spawn in a git project,
that means creating a git worktree and starting the harness inside it. Beyond keeping
concurrent spawns out of each other's way, this matters because it gives sandboxing
something to confine: a single folder the agent can be locked into.

### Watching work — the "dashboard"

Once created, a spawn takes its place as an entry alongside every earlier spawn and
every later one. Each entry carries a **status** — working, idle, waiting for my
input, done with its turn, and so on.

Those are illustrative, not a settled set. In particular, "done with its turn" means
only that *the agent has stopped* — which may well turn out to be indistinguishable
from *waiting for my input*. Neither is the same as being finished with the spawn
itself.

I can select any spawn and open it, and get the full view of the agent I started,
exactly as if I had started it by hand:

- watch it work in real time,
- type into its prompt box,
- interrupt it,
- see its sub-agents, if it created any.

The view holding all the spawns is the **dashboard**. That name says nothing about
screen real estate — it might be full screen, it might be a pane.

**Opening a spawn never costs me sight of the others.** I am never made to choose
between following one piece of work and knowing whether another has stopped and needs
me; losing the list while reading a spawn would reinstate exactly the hunting this
product exists to remove.

**What the app shows is always live.** What is running, and what state each spawn is
in, is a real-time view. There is no refreshing: I never have to ask the app to go and
look again, and I never have to wonder whether what I am reading is current. This is a
baseline property of the app, not a feature layered on later.

The baseline interaction model is that **I go to the app** to see what is up and to
act on it.

### Retiring a spawn

**The app does not land the work.** Committing, pushing, opening a pull request —
that belongs to the harness, driven by whatever the project's own workflow says. The
app stays out of it.

**The app does own what it created.** The worktree, and anything else it set up on a
spawn's behalf, is the app's to clean up. But that cleanup is gated on an explicit
action from me that means *"we are done with this spawn"*. It is never inferred from
the agent falling silent.

So there are two distinct notions, and only the second releases anything:

1. **The agent's turn ends.** It stopped. It may be finished, it may be waiting on me,
   it may have died — and the app may not be able to tell these apart.
2. **The spawn is retired.** I have said so, deliberately. Now the worktree and the
   rest of what the app owns can be torn down.

A spawn whose agent has gone quiet may still hold work I care about and have not
committed. Destroying its worktree unbidden would be the most damaging thing this app
could do.

## How many spawns at once

**Today's baseline: 4–5 parallel sessions, handled manually.** That is what the tool
has to beat, and it should beat it on friction reduction alone — before any of the
cleverer monitoring features exist.

**The starting point to aim at is 15–20 spawns, all of them live** — 20 spawns means
20 running harnesses and 20 worktrees. Some will go a long while without my attention;
the app draws no distinction between those and the rest, and has no notion of a spawn
being set aside.

**The ceiling moves.** It is not a fixed property of the product but a function of how
much help the app gives with monitoring. Better monitoring raises it. 40 is not the
starting target — I would not work on 40 tasks at once today — but nothing in this
vision caps it there.

This number decides one thing in particular: **whether the monitoring agent is a
convenience or a necessity.** At a handful of spawns a list is scannable by eye and
the agent is a nicety. As the number climbs, no human scans that list, and the agent
becomes the primary interface with the dashboard as the fallback.

## Harness support

**Any harness in principle; only the ones I need in practice.**

- **Claude Code** is the initial target, and currently the only harness I use.
- **Pi** and **Codex** are the likely next candidates — later, with no immediate plan
  behind them.
- Beyond those, there is no present intent to support anything.

So breadth of harness support is not itself a goal, and the product is not measured by
how many harnesses it speaks to. **Extensibility is the commitment**: adding a new
harness has to be possible, and as simple as it can reasonably be made — there should
be a sensible interface for a new harness to slot into.

This is one of the few places where the vision deliberately reaches toward
implementation, and it is the clearest example of the wishlist's second job: baking a
single harness's assumptions into the app is exactly the kind of choice that is hard
to walk back.

## Beyond the core loop

Also part of the vision. Listed in no particular order, and with no claim about when
any of it arrives.

### Sandboxing a spawn

Starting a harness should not have to mean simply "run the harness". A spawn should be
able to run sandboxed — under bubblewrap, in a container, or another form of
sandboxing. The worktree-per-spawn setup described above is what gives this something
to confine the agent to.

### An agent for creating spawns

Creating a spawn need not mean clicking buttons or assembling a command line. An LLM
agent could take that job instead: I describe what I want in natural language, and it
creates the spawn.

### An agent for monitoring spawns

The monitoring side of the dashboard could work the same way — an LLM acting as the
interface between me and the spawns. When a spawn has a question, the monitoring agent
brings it to me; I answer; it relays the answer back. The gain is that I no longer
have to go find which spawn needs me.

This is explicitly **an additional interface, not a replacement**. Opening a spawn and
interacting with it directly, as described above, remains available.

Whether the spawn-creating agent and the monitoring agent are one agent or two is
open.

### An agent that answers on my behalf

Far-fetched, and noted here only so it is on the pile: an agent primed with a dump of
my own *thinking method*, answering spawns' questions autonomously for me — rather
than only relaying them to me and waiting.

This is the kind of capability that raises the ceiling described under "How many
spawns at once": it attacks the limit that monitoring alone cannot, which is that
every question still has to be answered by me.

### Notifications

Ways of **raising information to me** rather than waiting for me to come and look.
These sit on top of an app that already works: nice-to-have, but high on the list.

How they would actually happen is left open — a great many of those choices depend on
what kind of software the app turns out to be, which is undecided.

### Where spawns run

Spawns running on the local machine only is fine. Running them somewhere else is a
thing I might want one day; there is no intent behind it yet.

### Surviving the app closing

Crude behaviour here is **acceptable, not a failure of the vision**: closing the app
shutting everything down, the next start being a blank slate, and leftovers never
being cleaned up at all. Early on I may simply have to stop the harnesses myself.

From that floor, the wishes get progressively more ambitious — listed in increasing
order of ambition, which is *not* an order of delivery:

- detect dangling worktrees and clean them up;
- better than cleanup: **manage** worktrees — reuse them rather than always creating
  and destroying;
- restart what was in flight when the app went away.

How deep this could go has no natural limit.

## Who this is for

**Me — one software engineer.** This is a pet project to make myself more productive,
now and for the foreseeable future.

If it becomes public one day, other people may like it and want to use it. That is not
a goal, and it is not a constraint on any decision.

The governing criterion is narrower and sharper than general usefulness: **a tool that
helps me work more efficiently, in a way that aligns with how I want to work.** When
weighing whether a proposed solution fits this vision, that is the question to ask —
not whether it would serve a broad audience well.

## Where the app stops

The line:

> **The app helps with starting, monitoring and observing work. It is not involved in
> the *doing* of the work.**

The work is done by agents, in a harness — exactly as it would be if I had started
them by hand, the old manual way. The app never becomes the thing that does it.

Note this is *not* the same as "the app never touches the content of the work". It
permits capabilities that look at the work, as long as the app is not the one
performing it.

**Out:**

- being an editor or an IDE;
- reimplementing a harness's own interface — the app *surfaces* the harness, it does
  not rebuild it;
- teams, multi-user, coordination between people.

**In, but narrowly — or far off:**

- **Spawns sourced from a backlog.** Not the app making prioritisation decisions, but
  something like *"here is my backlog in priority order, take the top ten tickets and
  add spawns to tackle them"*. That is a helper for **using the app**; the work itself
  still gets done by agents in a harness, unchanged.
- **Seeing what a spawn produced** — diffs, a summary of what changed, without having
  to go into the harness to get them. Considerable scope creep, and only worth having
  once the app already does a lot of other things right, but not excluded.
- **Code review.** Further out still, and closer to the line than either of the above,
  but not ruled out.

## Deliberately left open

Some things are missing from this document on purpose. They are **not omissions for
whoever notices them to tidy up**:

- **How a task finds its home** in the multi-project cases — how work gets matched to
  a project and a working copy. This is a design and implementation question, to be
  answered when multi-project work is actually being designed. (The single-repo case
  is settled above: I choose the repository.)
- **Whether model and effort level are fixed for a spawn's life**, or remembered per
  project so they are not re-picked twenty times over.
- **What happens when a spawn goes wrong** — a crashed harness, a sandbox that killed
  it, a worktree git will not clean up.

The last two are on the pile and go no further than that. The intent is explicitly not
to think about them until the problem is felt: being bothered by the first, or having
to deal with the second, is what should trigger the decision — not their presence on a
list.

## How this document is used

This is a long-running project. The intent is to deliver small pieces, use them, and
let the feel of using them inform what comes next. This document is the fixed
reference that outlasts any single one of those pieces: it does not say what gets
built first, only what the thing is ultimately for.

It is expected to be revised as the vision sharpens — but revisions should be
deliberate changes to the vision, not silent drift to match whatever was convenient
to build.

### Wishlist, then tranche, then design

This document does not need to be bounded, because bounding happens downstream of it.
Before any design work, a **tranche** gets scoped. What is in that tranche is what
gets designed; that is where realism, effort and sequencing enter the conversation.

Everything still sitting on the wishlist then does a second, different job: it warns
us off choices that are **hard to walk back**, and keeps us conscious of when a
decision makes some later wish harder — or impossible. The wishlist is not a queue of
work. It is peripheral vision.
