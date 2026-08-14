# Glossary

The project's vocabulary. Use these words as defined here — in issue titles,
commit messages, test names and documents — and do not substitute synonyms:
a spawn is not a "task" or a "job", retiring is not "deleting", and `unknown`
is not "errored".

Each entry names the component document that owns the concept.

- **band** — a painted stripe the app uses to show its own text. Two places:
  the selected row, painted across the full width of the list (gutter
  included); and the top of the slot, where the app writes about the selected
  spawn — an unaccounted spawn's explanation, or a retirement's message —
  drawn over the spawn's screen. → [the screen](components/the-screen.md)
- **control client** — the single tmux control-mode client. tmux streams
  every pane's output to the app through it; the app sends keystrokes and the
  slot's size back through it. Attached before any spawn starts. Runs in a
  pty owned by the app. → [the tmux session](components/the-tmux-session.md)
- **draft** — a spawn that is still being written: a row pinned above the
  repositories, and a form in the slot when selected. Pure app state — no
  process, no pane, nothing on disk. Several can exist at once.
  → [drafts and creation](components/drafts-and-creation.md)
- **grace period** — the first 8 seconds after a spawn starts. During it, a
  status record that does not resolve is ignored, so a spawn that is still
  starting up is not reported as stopped or unknown.
  → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **grid** — one spawn's screen, held in the app's memory. The control client
  keeps every grid current, whether or not that spawn is displayed. A grid is
  a screen, not a history: there is no scrollback. Switching spawns only
  re-renders a grid. → [the screen](components/the-screen.md)
- **gutter** — the narrow column at the left edge of the list and the form
  where marks are drawn. → [the screen](components/the-screen.md)
- **harness** — the coding agent the app launches and watches. Tranche 1
  supports exactly one. Only the harness seam knows which.
  → [the harness seam](components/the-harness-seam.md)
- **harness seam** — `src/harness/`, the one module that holds every
  harness-specific fact. It performs no I/O: it translates specs into plain
  data, and the app acts on that data. Two CI greps enforce the boundary.
  → [the harness seam](components/the-harness-seam.md)
- **holder** — the placeholder process a spawn's pane is created with. It
  gives the app time to attach a grid to the pane before the harness starts
  (`respawn-pane`), so the first output is never missed.
  → [the tmux session](components/the-tmux-session.md)
- **litter** — what the app leaves in the world when it exits or dies: the
  tmux session, worktrees, branches. Litter is accepted but always reported —
  at exit and at the next start-up. It is never adopted or cleaned up
  automatically. → [starting and leaving](components/starting-and-leaving.md)
- **mark** — the one-character glyph in the gutter showing a row's state:
  `·` working, `●` stopped, `?` unknown, `-` being retired, `+` a draft being
  written, `>` a draft being made into a spawn, `!` stopped or refused,
  `▍` the selected row. Shape and colour are decided together, in one place.
  → [the screen](components/the-screen.md)
- **recipe** — the plain data the harness seam produces for a launch: a
  program, arguments, environment and working directory. The app runs it; the
  seam never does. → [the harness seam](components/the-harness-seam.md)
- **retirement** — the explicit user action that ends a spawn and releases
  what the app made. Fixed order: stop the process, confirm it is gone, check
  the worktree is clean, remove the worktree. Refuses if the worktree is
  dirty. The branch is kept. → [retirement](components/retirement.md)
- **scaffolding** — the drawing utilities for the app's own surfaces: marks,
  the gutter, emphasis, and the column behaviour the list and the form share.
  Spawn output never passes through it. → [the screen](components/the-screen.md)
- **slot** — the region beside the list. It holds exactly one thing: the
  selected spawn's screen, or the selected draft's form.
  → [the screen](components/the-screen.md)
- **snapshot** — the immutable record of every spawn's status that the
  supervisor builds each tick and sends to the main thread. The list renders
  the latest one received.
  → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **spawn** — one unit of work: a worktree on its own branch, a tmux window
  running the harness, and a row in the list.
  → [drafts and creation](components/drafts-and-creation.md)
- **stand-in** — the fake harness the scale test runs instead of real agents.
  It redraws a full alternate screen five times a second and keeps a status
  record where the harness keeps one. Tests only.
  → [`docs/evidence/scale-at-twenty.md`](../evidence/scale-at-twenty.md)
- **status ladder** — the four checks, run in order, that decide a spawn's
  status: is the pane alive → does the status record resolve → the
  tie-breaker → `unknown`.
  → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **statuses** — exactly three: **working**, **stopped**, **unknown**. The
  first two describe the agent; `unknown` means the app's own instrumentation
  failed. → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **supervisor** — the thread that determines every spawn's status about five
  times a second and sends the snapshot.
  → [concurrency](components/concurrency.md)
- **tick** — one supervisor pass: a single `tmux list-panes -a` covering
  every spawn, plus a stat of each live spawn's status record (read only if
  its mtime changed). → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **tie-breaker** — the `ps` check run when a live pane's status record does
  not resolve: which process holds the pane's terminal? It can only change
  the answer to *stopped*, never to *working*, and a failed check never
  counts as the agent being gone.
  → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **unaccounted** — how prose and the UI describe a spawn whose status is
  `unknown`: the app cannot account for it. This is about the app's
  instrumentation, not the agent. Selecting the spawn shows the full
  explanation in the slot's band.
  → [knowing what a spawn is doing](components/knowing-what-a-spawn-is-doing.md)
- **worktree** — the per-spawn checkout the app creates, on its own branch,
  under one app-owned directory outside every repository. Removed by
  retirement; the branch survives.
  → [worktrees and branches](components/worktrees-and-branches.md)
