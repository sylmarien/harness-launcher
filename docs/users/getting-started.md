# Getting started

From a clone to a running spawn in about a minute.

## Prerequisites

- **git** — the app works on ordinary local git repositories.
- **tmux** — the app drives tmux headlessly; you never see it. Tested against
  tmux 3.4.
- **Rust** — via [rustup](https://rustup.rs). The toolchain is pinned in
  `rust-toolchain.toml` and installs itself on the first `cargo` call.
- **Claude Code** — `claude` on your `PATH`, authenticated. The app starts
  sessions; it does not log you in.

## Run it

```
cargo run
```

The app opens with nothing running: a blank draft in the slot, an empty list
beside it. Then:

1. Enter a local git repository, or any directory inside one.
2. Enter what you want done, in your own words.
3. Optionally pick a model and an effort level (`Tab` moves between fields,
   `↑` `↓` pick an option).
4. Press `F5`.

The app reports each step: repository read, worktree made, harness started.
The session appears in the slot. It is the real Claude Code interface; you
type into it directly.

The form's inputs can also be given on the command line. See
[the command line](the-cli.md).

## Day-to-day use

- `F2` starts another draft. Several can be open at once.
- `F6` / `F7` move the selection. The selected row fills the slot; the list
  stays visible.
- The list marks show status: `·` working, `●` stopped, `?` the app cannot
  tell. A stopped spawn needs you: open it and look.
- `F9` retires the selected spawn. The session stops and the worktree is
  removed. The branch, with the committed work, is kept. If the worktree has
  uncommitted changes, retiring is refused; clean up and press `F9` again.
- `F10` quits. Every session keeps running, and the app reports what it left
  and where.

[`CHEATSHEET.md`](../../CHEATSHEET.md) is the one-page version of this.
[What it supports](what-it-supports.md) lists limits and edge cases.
