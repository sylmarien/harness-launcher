# What it supports

What the app does, what it deliberately does not do, and the edge cases to
know about.

## What it does

- **As many spawns as you ask for**, each on a local git repository you
  choose. Fifteen or twenty at once, across many repositories, is normal use.
- **A list that never goes away.** Statuses update about five times a second.
  Opening a spawn never hides the others.
- **The real interface.** The slot shows the actual session, byte for byte:
  typing, interrupting, colour, sub-agent output and the spinner.
- **Worktrees and branches handled for you.** Each spawn gets a worktree on a
  fresh branch named after the work (`spawn/add-retry-logic-a7f3`), started
  from the repository's default branch. Worktrees live under
  `$XDG_DATA_HOME/harness-launcher/worktrees` (`~/.local/share/...` when that
  is unset).
- **Quitting kills nothing.** Sessions outlive the app;
  `tmux -L harness-launcher attach` finds them, and so does the next run. At
  exit the app reports what it left.
- **The next run picks them back up.** A session still running from an earlier
  run is in the list again, with no prompt and nothing to confirm. Its screen
  starts empty and says so: the app can only show what a spawn has drawn since
  it started watching. A session whose worktree git cannot name a repository
  for is left running and named in the start-up report instead.
- **Refusal over guessing.** A dirty worktree refuses to retire. An
  unresolvable repository refuses to spawn. No refusal loses what you typed.

## What it deliberately does not do

- **Land the work.** Committing, pushing and pull requests stay with the
  session and your own workflow. Retiring removes the worktree and keeps the
  branch. The branch is the product.
- **Guess why an agent stopped.** Finished, waiting for you, or dead all show
  as stopped. You open the spawn and look.
- **Bring back what a spawn drew before.** A picked-up spawn's screen starts
  empty. The model and effort it was started with are gone too: nothing on
  disk records them.
- **Tidy up your worktrees.** A worktree left by an earlier run is named in
  the start-up report and left where it is. Only `F9` removes one.
- **Touch your tmux.** The app runs its own tmux server on its own socket.
  Nothing of the app's appears among your sessions.

## Edge cases

- **A spawn is one screenful.** The slot has no scrollback and no mouse
  forwarding. You see what the session currently shows.
- **The keyboard split is fixed.** The app keeps its function keys
  (`F2` `F3` `F5` `F6` `F7` `F9` `F10`) whatever is in the slot. Every other
  key goes to the session or the draft.
- **Spawns start from the default branch**, resolved locally from
  `origin/HEAD` with no fetch. A stale local default is used as-is. Stacked
  work is arranged by hand.
- **Ignored files are deleted with the worktree** when a spawn retires. They
  do not count as dirty, so a spawn's `.env` is removed with it. Stashes
  survive in the repository's stash list.
- **`?` means the app cannot tell.** The problem is the app's
  instrumentation, not your agent. Select the spawn to see the full account
  over its screen. The session underneath is often still running.
- **One harness.** Claude Code is the only harness it launches today.
- **One install route.** `cargo install` builds it from source. The project
  ships no prebuilt binaries and no system packages. The build needs a Rust
  toolchain. The installed binary does not need one.
