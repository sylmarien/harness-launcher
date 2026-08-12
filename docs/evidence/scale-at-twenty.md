# Twenty spawns at once: what was observed

The tranche's headline claim is a number — *comfortably past the four or five
concurrent sessions handled by hand today* — and the design leans on it in
several places without ever having run it. This is that run. It is evidence
rather than an argument: everything below was measured, and where something
could not be measured it says so instead of estimating.

**How to take it again** is in `tests/twenty_spawns.rs`, which is the rig that
produced every number here:

```
cargo test --release --test twenty_spawns -- --ignored --nocapture --test-threads=1
```

It is `#[ignore]`d because it takes a couple of minutes and wants the machine to
itself. Read it before reading the numbers: what a measurement is *of* is decided
there.

## What was asked, and where it is answered

| the claim | where | in one line |
| --- | --- | --- |
| fifteen to twenty spawns at once, across several repositories, all live | §1 | twenty on four repositories, up in 1.1 s |
| a tick stays cheap at that count | §2 | one listing a tick, twenty stats, under two reads |
| the list is still readable at twenty | §3 | twenty-nine lines, four groups, three shapes |
| switching stays clean, mid-turn as well as idle | §4 | 1.2 ms median over thirty-eight moves |
| the interface stays responsive under a slow creation | §5 | 1.3 ms median while a six-second creation ran |
| tmux server memory is sane at twenty panes | §6 | 7 MB, and flat |

§7 is one the ticket did not ask for and the design did: whether the single
reader keeps up with twenty spawns drawing at once.

## The machine, and the run

| | |
| --- | --- |
| taken | 2026-08-12 |
| kernel | Linux 6.18.5, x86-64, in a container |
| cores / memory | 4 / 16 GB |
| tmux | 3.4 |
| git | 2.43.0 |
| build | `--release` |
| terminal | 200 × 50, so the list is 66 columns and the slot 133 |
| spawns | 20, five each on four repositories |

**Nothing here started a real Claude Code.** Twenty real sessions cost tokens and
need credentials, which the design rules out of every test — so what ran in the
twenty panes is the stand-in in the rig. It does the two things this measurement
is about: it draws a **whole screen on the alternate buffer** five times a
second, and it keeps a **session record where the harness keeps one**, keyed by
the pane's process id. Sixteen of the twenty report themselves busy, three report
themselves stopped, and one writes no record at all — so the list carries all
three statuses at once, and the status ladder's tie-breaker has something to
break a tie about.

Where that stand-in is unlike the real thing is the honest limit of the whole
exercise, and it is stated at the bottom rather than buried.

## 1. Twenty spawns, four repositories, all live

Asked for on one command line, twenty groups separated by `--and`:

- **1.1 s** from starting the app to twenty worktrees created, twenty windows
  opened, twenty sessions started and the list drawn. One run, one number: the
  rig starts the app once, so there is no spread behind this and none is claimed.
- tmux held **21 windows in one session**: the holding window and twenty live
  spawns, `pane_dead=0` on every one of them.
- No second session, no parked panes: `list-panes -s -t spawns` is the whole of
  it.

Worth saying plainly because the design leans on it: **twenty spawns are twenty
windows and twenty rows, and nothing else.**

## 2. What a tick costs

The design says a tick is one `list-panes` covering every spawn at once, plus a
stat per live spawn read only when it has moved, and a `ps` probe that is a
tie-breaker rather than a per-tick cost. Every clause of that is a count, so the
app was run under `strace` and the counts taken over ten seconds of twenty
spawns running.

| | measured, per tick |
| --- | --- |
| `tmux list-panes -a` | **1.00** — 42 over 42 ticks |
| `ps` tie-breakers | **0.02** — one in ten seconds, for the one spawn nothing resolves |
| every other process the app started | **0** |
| processes the app started, all told | **1.02** — which is the two rows above, added up |
| stats of the harness's records | **21.0** |
| records actually opened and read | **1.88** |

The first two rows are the ones to read. **1.02** is a total, and it has the
stray `ps` inside it as well as on its own row — so it is not a figure that can
be glossed as "a listing and nothing else". The rig prints each of the app's own
tools at its own rate for exactly that reason.

Over the ten seconds the trace saw forty-two ticks, **forty-two `tmux`** and
**one `ps`**, and nothing else the app started at all. It also saw 64 `mkdir`, which are the
stand-ins keeping their own records rather than anything the app did — every
program the trace saw is reported by the rig rather than only the ones expected,
so nothing is quietly left out of the count.

Three things worth reading twice.

**One subprocess per tick, at twenty spawns — and one caveat about the
counting.** A tick is identified in the trace *by* its `tmux list-panes`, so
"forty-two ticks, forty-two `tmux`" is true by construction and could not have
come out otherwise. That ratio is the definition, not the finding.

What the trace does establish independently is everything *beside* the listing:
over ten seconds of twenty spawns it saw exactly one other process the app
started — a single `ps` — and no second `tmux` of any kind. No per-spawn
listing, no `capture-pane`, nothing. So the claim that survives is the one the
design makes — **the listing is the only per-tick subprocess, and it covers every
pane in one call** — rather than a ratio that was going to be 1.00 whatever
happened. The remaining independent check would be the tick *rate*, and that is
confounded by the tracer's own slowdown (see below).

**Twenty-one stats, and not quite two reads.** Twenty of those stats are the
one-per-live-spawn check that asks *has this moved*; on **1.88** of them a tick
the answer was yes and the record was opened. That is the mtime cache doing
exactly what it was built for — a stand-in rewrites its record only every tenth
turn, and only a rewrite costs a read.

The reads come out of those twenty rather than in addition to them, which leaves
**one stat a tick unaccounted for**. The trace does not say what it is, and it is
left as an unexplained call rather than given a reason it might not have: 21.0
minus twenty is one, and 1.88 reads cannot be what fills it.

**The tie-breaker stays a tie-breaker.** One unaccountable spawn, one `ps` in ten
seconds. Twenty spawns the app could not resolve would cost four a second, and
that is the storm the design's "asked only when the record has already failed"
was written to avoid.

Two notes on how to read this table. The tracer is expensive, so the *rate* under
it is not the app's real rate: ticks came **225 ms** apart (median) rather than
the 200 ms the supervisor sleeps for, and the app managed 4.2 a second rather
than 5. And a raw count of `execve` reads eleven times too high, because a program
named without a path is looked for on each `PATH` directory in turn and every miss
is an `execve` of its own — the C library searching, not the app doing anything.
Only the ones that returned `0` are counted above.

**Untraced, the tick itself** is measured by
`supervisor::tests::a_tick_at_twenty_spawns_stays_comfortably_inside_the_period_between_ticks`,
which builds twenty real panes on a private tmux socket and times
`Supervisor::tick`: ten consecutive ticks ran **3.8 to 5.4 ms**, or about **2% of
the 200 ms between them**. The snapshot the tick then copies to send down the
channel — the obvious thing rather than the clever one, twenty rows cloned five
times a second — costs **2.6 µs**
(`snapshot::tests::a_snapshot_of_twenty_rows_is_cheap_enough_to_copy_every_tick`),
which is a thousandth of a tick and settles the one thing that was flagged as
worth measuring when it was written.

Both tests **print what they measured**, so these two figures can be checked
against the tests they are attributed to rather than taken on trust:

```
cargo test --release --bin harness-launcher -- --nocapture \
    a_tick_at_twenty_spawns a_snapshot_of_twenty_rows
```

They are ordinary unit tests rather than part of the rig, which is why they take
a different command from the one at the top of this document. Repeated runs on
this machine land within a millisecond or two of the range above; the range is
one run's.

## 3. The list at twenty

This is the list as it stood, read back off the app's own terminal — the whole
transcript put through the same emulator the app uses on its children, so it is
what a person would have been looking at rather than a mock-up:

```
SPAWNS

harness-launcher ●····
 ● rotate-the-deploy-keys-2s52
▍· fix-worktree-cleanup-dzec
   spawn/fix-worktree-cleanup-dzec
   /tmp/.tmpKEzYK5/data/harness-launcher/worktrees/fix-worktree-c…
 · control-mode-backpressure-jhie
 · pagination-cursors-0fko
 · ssh-agent-forwarding-hnj5

acme-api ●?···
 ● idempotency-keys-9y94
 ? font-fallback-for-emoji-h5sc
 · add-retry-logic-k81q
 · rate-limit-headers-s2lc
 · alert-on-disk-pressure-ekxu

dotfiles ●····
 ● status-ladder-grace-period-nkg2
 · drop-legacy-auth-0c5f
 · tidy-the-shell-prompt-klvr
 · prune-stale-symlinks-k9w6
 · cheaper-log-retention-wnlz

infra ·····
 · spawn-form-choices-cos1
 · openapi-drift-check-3hl4
 · neovim-lsp-config-och1
 · terraform-state-locking-kg37
 · blue-green-cutover-ekwu

F2 starts a draft
F6 / F7 move the selection
F9 retires the spawn
F10 quits — nothing is killed
```

The three lines under `fix-worktree-cleanup` are the selected row's own — its
branch and its worktree — which is why twenty rows come to twenty-nine lines.

Whether that *reads well* is a judgement rather than a measurement, and it is one
a person has to make. What can be said from the screen itself:

- **Twenty spawns fit in twenty-nine lines** of a fifty-row terminal, one line
  each — so the whole of it is on screen at once with room to spare, and nothing
  scrolls. The density holds at the count.
- **Four groups are still four groups**, each header carrying its own bar — so
  `acme-api ●?···` says one stopped, one unaccountable and three working without
  a single row being read.
- **The three statuses tell themselves apart by shape** — `●`, `?`, `·` — before
  any colour is involved, which is what makes the list survive being read without
  colour. The colours are on top of that, and pinned by
  `list::tests::a_status_is_a_shape_and_a_colour_at_once`.
- **The thing needing attention is at the top of its group.** Every `●` and the
  one `?` are the first rows under their headers, because the order inside a
  group is attention-first.
- **No legend.** The footer says what the keyboard does, not what the marks mean.

A screen of the same *shape* is pinned as a test —
`list::tests::twenty_spawns_over_four_repositories_read_as_four_projects` —
though **not this screen**: that test has its own twenty spawns, its own four
repositories and its own statuses, and its groupings do not match the ones above.
What it pins is the vocabulary and the density — one line per spawn, four groups
that are still four groups, each header carrying its own bar, three statuses
telling themselves apart by shape — so a change that broke any of those would
have to rewrite that screen to pass. **The screen above is this run's, and
nothing asserts it.**

## 4. Switching, at twenty and mid-turn

The selection was walked from the top of the list to the bottom and back, one
spawn at a time, **while every other spawn was drawing** — thirty-eight moves,
each timed from the keystroke leaving the rig to the frame that shows the spawn
moved to:

| | |
| --- | --- |
| samples | 38 |
| median | **1.2 ms** |
| mean | 1.2 ms |
| p95 | 1.3 ms |
| worst | **1.4 ms** |

That is a frame, and it is what the design predicted for a reason worth
restating: **nothing moves when you switch.** No pane is joined or broken, no
child is resized, nothing is told anything. Every spawn's grid was already
current because the one reader thread had been filling all twenty all along, so
selecting another spawn is a re-render of something the app already had.

The walk covers the idle case as well as the mid-turn one: three of the twenty
stand-ins report themselves stopped and draw nothing at all after their first
screen, so three of the thirty-eight moves land on an idle spawn.

**That half is asserted rather than shown**, and it is worth conceding as plainly
as §3 concedes its own. All thirty-eight moves are reported as a single spread;
the three idle ones are not broken out, and nothing here says whether switching
onto a spawn that is drawing nothing is faster, slower or identical. That they
are in the aggregate at all is a property of how the walk was built rather than
something these numbers show. Separating them would need the rig to record which
move landed where.

**These numbers are at the edge of what the rig can resolve.** It looks at the
screen every millisecond, so what they establish is that a switch costs about a
frame — not that it costs 1.2 ms rather than 0.9. The finding is the shape: at
twenty spawns, with nineteen of them drawing, a switch is not something you wait
for.

## 5. While a slow creation runs

`git worktree add` was made to take six seconds on purpose — the rig puts a `git`
in front of the real one that sleeps for the one worktree whose name says so — and
a twenty-first spawn was composed and started while the other twenty ran. The
selection was then walked back and forth for **five of the six seconds** the
creation took — the walk is bounded by its own five-second clock rather than by
the creation finishing:

| | |
| --- | --- |
| samples | 49 |
| median | **1.3 ms** |
| mean | 1.5 ms |
| p95 | 2.5 ms |
| worst | **3.6 ms** |

Which is the number from §4 again — **a slow creation costs the interface nothing
measurable**, because the slow half of it happens on a thread of its own and the
frame loop never waits for it.

The spawn itself was **on the list 6.1 seconds after `F5`**, which is the six
seconds `git` was made to take plus a tenth of a second of everything else. That
the list and the session in the slot both kept going is what the forty-nine
switches above are: each is a keystroke answered by a redrawn frame, taken while
the creation ran. What the draft's own row was showing meanwhile is **not
observed here** — the rig never looks at it — so nothing is claimed about it.

On the way out, with twenty-one spawns running, the app said:

```
harness-launcher: quitting stopped nothing: 21 spawns are still running in the
tmux session `spawns`, with worktrees under <the rig's worktree root>
```

Quitting killed none of them, which is the design's promise, checked at twenty-one
rather than at one.

## 6. Memory, at twenty panes under the fullscreen renderer

| | at twenty panes | thirty seconds later |
| --- | --- | --- |
| tmux server | **7 MB** | **7 MB** |
| the app | **12 MB** | **12 MB** |

Flat, which is the whole point. The design's argument was that the alternate
screen accrues no scrollback, so a spawn costs one screenful of cells and nothing
accumulates — and thirty seconds of steady drawing moved neither figure.
Seventeen of the twenty were repainting a full forty-four-line screen throughout.
**How many bytes that came to was not measured** — the rig counts what the app
*writes*, not what its children produce — so no figure for the volume reaching
the single reader is given here.

The 7 MB is tmux holding twenty-one panes' worth of screen; the 12 MB is the app
holding twenty `vt100` grids plus everything else it is. Both are two orders of
magnitude below the ~1.3 GB figure that once argued against embedding, and the
app's whole resident set sits near enough to the ~6 MB the corrected arithmetic
predicted for twenty grids that the prediction survives with the program itself
thrown in.

**The app's own drawing is nearly free**: it wrote **55 KB in those thirty
seconds** — under 2 KB a second — because it redraws only the cells that changed,
even while the spawn in the slot is repainting itself five times a second.

## 7. How far behind the screen runs

Every screen a stand-in draws carries the time it drew it, so the whole path can
be measured at once: the child's write, tmux, the control-mode stream, the single
reader thread, the emulator, the app's frame, and the rig's own emulator on the
end of it.

| | |
| --- | --- |
| samples | 50, over five seconds |
| median | **68 ms** |
| p95 | 167 ms |
| worst | **171 ms** |

**This is one pane, not twenty.** The lag is read off the first timestamp on the
app's own screen, which is the spawn *in the slot*. The other nineteen are load
on the single reader rather than samples of it — whether any of them was running
further behind is not something this measurement can see. What it answers is
"how stale is the screen you are looking at, while nineteen others draw", which
is the question, but it is worth being exact about which pane answered it.

**Nearly all of that is the stand-in, not the app.** Its draw loop ends by
waiting up to 200 ms for a keystroke, so it repaints **no more often than every
200 ms**, and rather less often once drawing forty-four lines is added. The exact
interval was not measured, and no figure for it is given here. What that floor is
enough for is the number carrying the finding — the **worst**, 171 ms, which is
still **inside one repaint interval**: the app never fell a whole frame of the
child's behind. A reader that could not keep up would show staleness growing past
the repaint interval and going on growing across the five seconds; it did not
move.

That is the answer to the sharpest unknown the design's §4.8 named: one thread
decoding everything twenty fullscreen agents draw, with no backpressure story. At
this load it is not the bottleneck — it is not measurably in the way at all.

## Observed once, and not run down

While the rig was being written, the tmux server was killed out from under a
running app. The app neither refused nor exited: it kept running and **spun at
roughly 80% of a core**. `control::Client` has a `listening` check whose job is
to notice exactly that, and it did not fire.

This is **recorded rather than measured**, and every part of that matters. It
happened once, by hand, outside the committed rig; the figure is off `top` rather
than off an instrument; killing the server is not something the rig does, so
nothing here reproduces it; and a half-torn-down rig is a plausible enough
explanation that this may be an artefact rather than anything a user would meet.

It is written down because the alternative was leaving it out. This document's
own ticket asked for what was actually observed, and the rule it was written
under is that a problem the rig finds belongs to whichever ticket owns the
behaviour. This one belongs to whatever owns `control::Client`, and it has not
been run down here.

## What this does not measure

Stated plainly, because a measurement's limits are part of it.

- **It is not Claude Code.** The stand-in draws a full alternate-screen repaint
  five times a second, which is a fair imitation of an agent mid-turn and is not
  the same thing. A real harness draws differently — bursts, wide characters,
  colour, sub-agent output — and only starting twenty real ones would say what
  that costs. The design already names this as the thing automated tests will not
  cover.
- **Nothing here is typed into a spawn at length.** Keystrokes were sent, but
  nobody held down a key or pasted a page.
- **One machine, one container, four cores.** Everything above is about this
  machine. A slower one will differ, which is why the rig is committed and the
  numbers are dated.
- **Minutes, not hours.** Memory was watched for thirty seconds of steady
  drawing, which catches a leak that grows per redraw and would not catch one
  that grows per hour.
- **No spawn was retired mid-measurement**, and the list was not resized while
  twenty were running.
- **§7 watches one pane.** How far behind the screen runs is read off the spawn
  in the slot. The nineteen drawing off screen are load rather than samples, so
  nothing here says whether the reader was further behind on any of them.
- **Memory is two samples, not a series.** §6 reads `/proc` at twenty panes and
  again thirty seconds later. Two readings that match rule out a leak steep
  enough to show in thirty seconds; they are not a curve, and nothing between
  them was looked at.
- **The instrument is in every timing.** The rig runs its own emulator over the
  app's output and looks at the screen every millisecond, so a millisecond is the
  floor of anything measured here, and the `strace` numbers are counts taken
  while the tracer made everything slower. Each section says which way that cuts.
  That floor is the same order as §4's whole spread — a median of 1.2 ms and a
  worst of 1.4 ms, reported to a tenth of a millisecond, are being measured with
  a one-millisecond ruler. §4 says what survives that; the decimals do not.
