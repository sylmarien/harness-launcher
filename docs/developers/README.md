# Developer documentation

How harness-launcher is built, component by component and scenario by
scenario. For how to *use* it, see [`docs/users/`](../users/); for the
one-page key reference, [`CHEATSHEET.md`](../../CHEATSHEET.md).

These documents describe the current design. They do not record why
alternatives were rejected — that history lives in the issues and pull
requests. `src/` has detailed doc comments; these pages cover what a doc
comment cannot: how the parts fit together, in what order things happen, and
which file to open.

## Which document answers what

| Question | Document |
| --- | --- |
| How is the screen drawn? The list, the rows, the slot, the marks? | [components/the-screen.md](components/the-screen.md) |
| How does typing a description become a running harness? | [components/drafts-and-creation.md](components/drafts-and-creation.md) |
| What does tmux do here, and what is control mode? | [components/the-tmux-session.md](components/the-tmux-session.md) |
| How does the app decide a spawn's status? What does *unaccounted* mean? | [components/knowing-what-a-spawn-is-doing.md](components/knowing-what-a-spawn-is-doing.md) |
| What happens when a spawn is retired, and when does that refuse? | [components/retirement.md](components/retirement.md) |
| Where do worktrees live, how are they named, what survives? | [components/worktrees-and-branches.md](components/worktrees-and-branches.md) |
| Which code knows what the app launches, and what keeps that contained? | [components/the-harness-seam.md](components/the-harness-seam.md) |
| What happens at start-up and at exit? What is litter? | [components/starting-and-leaving.md](components/starting-and-leaving.md) |
| What runs on which thread, and what crosses a channel? | [components/concurrency.md](components/concurrency.md) |
| What does a word mean here? | [glossary.md](glossary.md) |
| How do I build, lint, and test this? | [working-on-the-project.md](working-on-the-project.md) |

## Scenarios

[`scenarios/`](scenarios/) describes the system end to end — the happy path
first, then every failure path:

- [From keypress to running harness](scenarios/from-keypress-to-running-harness.md)
  — the full sequence from `F2` to a live session on screen, with the
  component that owns each step.
- [A spawn the app cannot account for](scenarios/an-unaccounted-spawn.md)
- [A retirement that refuses](scenarios/a-retirement-that-refuses.md)
- [A dead pane](scenarios/a-dead-pane.md)
- [Litter found at start-up](scenarios/litter-at-startup.md)
- [A creation that fails halfway](scenarios/a-creation-that-fails-halfway.md)
- [A draft discarded mid-creation](scenarios/a-draft-discarded-mid-creation.md)

## Design rules

These rules apply across every component. Changes are expected to keep them
true:

- **Never trade one view for another.** The list is visible in every state of
  the app.
- **Refuse rather than guess.** And a refusal never deletes what the user
  typed.
- **Where the app must guess, it shows the uncertainty** instead of hiding
  it — see [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md).
- **tmux and the filesystem are the source of truth.** The app's memory is a
  cache. The one exception is a spawn's grid: bytes already received, which
  cannot be re-read from anywhere.
- **Exactly three statuses, all about the agent.** "Starting", "creating" and
  "failed" are states of other things (a draft, a retirement) and are shown on
  those things.
- **Litter is accepted; invisible litter is not.** The app may leave things
  behind, but always says so.
- **Nothing has a fixed size.** Every dimension is computed from the real
  terminal, every frame.
- **Missing features are scope, not architecture.** Where a simplification
  could harden into an assumption, an explicit extension point is left. The
  harness seam's input hole is the standing example.

## History

The tranche documents are **frozen** records. They are not updated when the
code changes:

- [`docs/tranches/01-the-core-loop.md`](../tranches/01-the-core-loop.md) — the
  scope of tranche 1, as agreed before it was built.
- [`docs/tranches/01-the-core-loop-design.md`](../tranches/01-the-core-loop-design.md)
  — its design record: what was decided and why, as of the end of the tranche.
- [`docs/evidence/scale-at-twenty.md`](../evidence/scale-at-twenty.md) — the
  measured run behind the "twenty spawns" claims, dated and reproducible.

Where these pages and a frozen record disagree, these pages are the ones kept
correct.
