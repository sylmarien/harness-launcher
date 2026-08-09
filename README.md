# harness-launcher

Start several Claude Code spawns — each on whatever local git repository you
choose — from one place, see at a glance which ones need you, and open one to
answer. No worktree created by hand, no terminals to juggle.

What is here today is **one** spawn, end to end, with a list beside it that says
what that spawn is doing.

**Run it from inside tmux.** It composes a window around the session it starts,
and has to be a pane in that window itself; started anywhere else it refuses and
says so.

```
cargo run -- <repository> "<the work>" [--model <id>] [--level <id>]
```

It resolves the repository's default branch from `origin/HEAD` without touching
the network, creates a worktree on a branch of its own under an app-owned root,
composes a tmux window — the app's list on the left, the session on the right —
and starts the session in the worktree. You then type into that pane as if you
had started it yourself; the app never types into it.

It takes over the window you are already in: the slot is split off the app's own
pane, and whatever else you had in that window is left alone.

A supervisor thread then ticks about five times a second and hands the list an
immutable snapshot of what every spawn is doing: **working**, **stopped**, or
**unknown** — the last meaning the app's own instrumentation failed rather than
anything about the agent, with the reason shown beside the row.

`cargo run -- --help` lists the models and effort levels the harness offers.

## Reading the design

- [`docs/product-vision.md`](docs/product-vision.md) — what the tool does when
  it is done.
- [`docs/tranches/01-the-core-loop.md`](docs/tranches/01-the-core-loop.md) — the
  frozen scope of the slice being built.
- [`docs/design/tranche-01-the-core-loop.md`](docs/design/tranche-01-the-core-loop.md)
  — how that scope is met, and why each choice beat its alternatives.

## Working on it

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo fmt` runs with no `rustfmt.toml`, which is the point: the defaults *are*
the Rust Style Guide, and `edition = "2024"` selects its current revision. Note
what it leaves alone — comments and doc comments are never rewrapped, and import
grouping is not enforced, so the prose wrapping and the `std` / external /
`crate` import order in this repo are convention rather than tooling.

Clippy runs its warn-by-default groups plus `pedantic`, which `Cargo.toml`
switches on. `--all-targets` so the tests are linted too. `nursery` and
`restriction` stay off.

Plus the two invariant greps, which CI also runs:

```
grep -rE 'std::process|std::fs|Command::' src/harness/              # must find nothing
grep -rEi 'claude|--effort|CLAUDE_CODE' src/ --exclude-dir=harness  # must find nothing
```

They hold the harness seam in place: everything harness-specific lives in
`src/harness/`, and that module performs no I/O — it translates, and the app
acts.

The status ladder is built on what tmux, `ps` and the harness print, and none of
those three is a format this project controls — so its tests read the recordings
in [`captured/`](captured/README.md) rather than strings written from memory.
Each recording says how it was made.
