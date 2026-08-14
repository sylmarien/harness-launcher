# harness-launcher

**Run a fleet of Claude Code sessions from one terminal and keep every one in
view.**

Start several spawns, each on a local git repository you choose. The app
shows which ones need you, and you open any of them without losing the rest.
It creates worktrees for you; there are no terminals to juggle.

## Why

One coding agent is a clean loop: you start it, it works, it stops and asks
you. Several agents break the loop:

- Each workstream needs its own terminal, switched by hand.
- You must find which agent is blocked on you.
- Concurrent agents cannot share a checkout, so each workstream needs a
  hand-made worktree.

Past a handful of tasks, the overhead cancels the gain. harness-launcher
removes it. Its key property: **opening a spawn never hides the others.** The
spawn list stays on screen in every state of the app. There are no modal
sessions and no hand-off to a tmux window.

## Quick start

You need git, tmux, [rustup](https://rustup.rs), and
[Claude Code](https://docs.anthropic.com/en/docs/claude-code) logged in. Then:

```
cargo run
```

The app opens on a blank form. Write where the work should happen and what
you want done, then press `F5`. The app creates a worktree and a branch,
starts the agent, and shows the live session. You can also pass everything on
the command line:

```
cargo run -- ~/code/api "add rate-limit headers" \
       --and ~/code/web "fix the flaky login test" --model sonnet
```

[`docs/users/getting-started.md`](docs/users/getting-started.md) covers this
step by step. [`CHEATSHEET.md`](CHEATSHEET.md) lists every key and mark on
one page.

## Features

- **Fifteen or twenty spawns at once, across repositories.** Spawns group
  under their repository. Each group header shows a status bar for its
  spawns, and spawns that need attention sort to the top. Each row is one
  line, so the whole fleet fits on screen.
- **Instant switching.** The app keeps every spawn's screen current in
  memory, shown or not. Selecting a spawn is a re-render, not a pane shuffle.
  Nothing resizes, and the spawn you left keeps running.
- **Compose without stopping.** A draft is a row in the same screen, not a
  dialog. Leave it to answer a stopped spawn and return; the text, caret and
  field are unchanged. Several drafts can be open at once.
- **Quitting kills nothing.** Sessions run in a headless tmux server the app
  owns. Close the app and every agent keeps working;
  `tmux -L harness-launcher attach` shows them. Your own tmux is never
  touched.
- **Honest statuses.** A spawn is working, stopped, or `unknown` when the
  app's own instrumentation fails. For `unknown`, the app writes the full
  account over that spawn's screen. It never guesses why an agent stopped.
- **Refusal over guessing.** Retiring a spawn with uncommitted work is
  refused. The check spells out its git flags, so
  `status.showUntrackedFiles = no` cannot blind it. A failed creation returns
  your draft intact, with the reason underneath. The branch and its committed
  work always survive.

## Where next

- [`docs/users/`](docs/users/) — getting started, the command line, the spawn
  form, and what the app does and does not do.
- [`docs/developers/`](docs/developers/README.md) — the components, the
  end-to-end scenarios, the glossary, and
  [working on the project](docs/developers/working-on-the-project.md).

## License

[MIT](LICENSE). Do what you like with it; keep the copyright notice.
