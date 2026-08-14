# From keypress to running harness

This page is the spine: one sequence from `F2` to a live session on screen.
Each step names its owning component; detail lives in the linked component
documents. Vocabulary is defined in [the glossary](../glossary.md).

## The sequence

1. **`F2` starts a draft** — [the screen](../components/the-screen.md). The
   key works wherever the selection is (`COMPOSE` in `src/app.rs`); it calls
   `Drafts::start` and moves the selection to the new draft. The draft's row
   appears pinned above the repositories, with `+` in the gutter
   (`src/list.rs`), and the slot shows its form because the selection is on it.
2. **Typing fills the form** —
   [drafts and creation](../components/drafts-and-creation.md). While the
   selection is on a draft, ordinary keys become `Edit`s for it instead of
   bytes for a session (`what_it_means` in `src/app.rs`). `Tab` cycles the
   controls: a repository, the work, and one list per choice the harness seam
   offers. A draft is state in the app and nothing else (`src/draft.rs`), so
   several can be in flight at once and moving to another row loses nothing.
3. **`F5` submits a `Wanted`** — `Held::start` in `src/app.rs`.
   `Drafts::submit` returns the repository path, the work, and the picked ids.
   It returns nothing if either field is missing (the draft refuses in place)
   or a spawn is already being made from it. Then
   `creation::harness_installed` refuses if the harness's program is not on
   the `PATH` — the last refusal that creates nothing.
4. **The worker half runs on its own thread** — `creation::making` in
   `src/creation.rs`. This half is git and takes seconds; the app keeps
   drawing sixty frames of live sessions meanwhile
   ([concurrency](../components/concurrency.md)). The draft's row shows `>`,
   and the form no longer takes keys.
5. **Each step is announced before it runs** (`made` in `src/creation.rs`);
   the lines appear under a `STARTING` heading on the form. First comes *reading
   the repository and resolving the branch to start from*. Then
   `creation::plan` opens the repository (`git::open`), resolves the default
   branch from `origin/HEAD` without fetching (`git::default_branch`), names
   the spawn (`src/names.rs`), and chooses the worktree path under the
   app-owned root
   ([worktrees and branches](../components/worktrees-and-branches.md)) and
   the recipe ([the harness seam](../components/the-harness-seam.md)).
6. **The worktree is created on a fresh branch.** The creation reports
   *creating the worktree \<path\> on \<branch\>*, then `Plan::create` runs
   `git worktree add --no-track -b` (`src/git.rs`). The plan returns to the
   main thread as `Made(Plan)`.
7. **The main-thread half opens the window** — `reported` in `src/app.rs`. It
   reports *starting the harness in \<worktree\>* and calls `creation::start`.
   The window opens running the holder, and the grid is attached to the pane
   before the harness starts, because a control-mode client streams only
   output produced while attached
   ([the tmux session](../components/the-tmux-session.md)).
8. **The harness replaces the holder** — `Server::start` in `src/tmux.rs`, a
   `respawn-pane` carrying the seam's recipe: program, arguments, environment,
   working directory. Each element is one argument, with no shell in between.
9. **The spawn is adopted** — `Held::adopt` in `src/app.rs`. The supervisor is
   told first, so its next tick already knows the pane
   ([knowing what a spawn is doing](../components/knowing-what-a-spawn-is-doing.md)).
   The row joins the list under the repository it was started against; the
   selection follows only if it was still on the draft. The draft's row is
   removed last, because until then it is the only record of where the spawn
   came from.
10. **The first bytes flow.** The harness draws; tmux forwards `%output` down
    the one control client. The reader thread routes it into the spawn's grid,
    and the next frame copies the grid into the slot
    ([concurrency](../components/concurrency.md),
    [the screen](../components/the-screen.md)).
11. **The first snapshot carries its status.** The supervisor's next tick
    lists every pane, including the new one, and the list draws the mark. The
    grace period keeps a spawn still starting up from reading as stopped or
    unaccounted.

## The centrepiece

```mermaid
sequenceDiagram
    actor U as user
    participant M as main thread (src/app.rs)
    participant W as worker (creation::making)
    participant T as tmux server
    participant C as control client
    participant H as harness

    U->>M: F2 — draft row, form in the slot
    U->>M: typing, Tab, choices
    U->>M: F5
    M->>M: Drafts::submit → Wanted, harness on PATH?
    M->>W: making(draft, Wanted, root)
    W-->>M: Doing: "reading &lt;repo&gt; and resolving the branch…"
    W->>W: plan(): git::open, default_branch, name, recipe
    W-->>M: Doing: "creating the worktree &lt;path&gt; on &lt;branch&gt;"
    W->>W: git worktree add --no-track -b
    W-->>M: Made(Plan)
    M-->>M: Doing: "starting the harness in &lt;worktree&gt;"
    M->>T: open_window — running the holder
    M->>C: watch(pane, slot) — grid attached to the pane
    M->>T: start — respawn-pane with the recipe
    T->>H: harness replaces the holder
    M->>M: adopt — supervisor told, row under its repository, draft row removed
    H-->>C: first bytes (%output)
    C-->>M: reader → grid → slot, next frame
```

The first half is git: seconds of work and nearly every refusal, so it runs
on a worker thread. The
second half is two commands to a server already running, through tmux and the
control client, which belong to the one thread that holds them. The refusals
are covered in [a creation that fails halfway](a-creation-that-fails-halfway.md).

## The second entry point: the command line

The same creation, run before the screen exists
([starting and leaving](../components/starting-and-leaving.md)). `src/cli.rs`
parses `<repository> <work> [--model <id>] [--level <id>]`, with several
spawns separated by `--and`, into the same `Wanted` the form produces, so
both entry points build the same spawn with the same code.

The differences are ordering, not code path (`spawn` in `src/main.rs`):

1. Every check that can refuse runs before the screen is taken over, so a
   refusal prints to the shell instead of an alternate screen that closes
   afterwards. The harness check runs first, then every repository is
   resolved before any worktree is made.
2. The worktrees are created in order on the main thread. There is no form to
   report to, and a refusal partway names what was already left on disk.
3. The session is opened, the control client attached, and each spawn started
   as in steps 7 onward: window with the holder, grid attached to the pane,
   then the harness.

Run with no arguments, the command line opens the app instead: no session, a
blank draft in the slot, and the list beside it.
