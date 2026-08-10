# harness-launcher

Start several Claude Code spawns — each on whatever local git repository you
choose — from one place, see at a glance which ones need you, and open one to
answer. No worktree created by hand, no terminals to juggle.

What is here today is **as many spawns as you ask for**, end to end, with a list
beside them that says what each one is doing.

**It is an ordinary terminal program.** Run it from a shell, or from inside tmux
if that happens to be where you are; it makes no difference to anything.

```
cargo run -- <repository> "<the work>" [--model <id>] [--level <id>] \
            [--and <repository> "<the work>" [--model <id>] [--level <id>]]...
```

`--and` is what separates one spawn from the next, and each of them chooses its
own model and effort level. It is a separator rather than one pair of arguments
after another because bare pairs would read a description somebody forgot to
quote as a second spawn — which is exactly the guess the app is not allowed to
make.

For each spawn it resolves the repository's default branch from `origin/HEAD`
without touching the network, creates a worktree on a branch of its own under an
app-owned root, and starts the session in it. Then it draws the whole screen —
the list on the left, one session in the slot on the right, and the line between
them. Everything you type goes to the session in the slot, as if you had started
it yourself. **F6 and F7 move the selection up and down the list, F10 quits** —
and those three are the whole of the app's keyboard so far. How the keyboard is
split between the app and the spawn is still an open question in the design;
these three are what the list needs and nothing more is claimed.

**Moving the selection is the whole of switching.** Every spawn's screen lives
in the app's memory as its own grid, kept current by the control-mode stream
whether or not it is the one on display, so selecting another spawn is a
re-render of something already up to date: no pane moves, nothing is resized, no
session is told anything, and the spawn you just left carries on working. **And
nothing hides the list** — there is no state of the app in which it is not
there, which is the whole differentiator.

The list groups spawns under the repository they were started against, each
repository header carrying a compact bar of its spawns' statuses so a project
reads without reading its rows. Within a repository the order is
attention-first: stopped, then unknown, then working. Status is carried by an
icon and a colour together, so the list needs no legend and survives a
colour-blind reader. The selected spawn says what the app made for it — its
branch, its worktree, and the reason if the app cannot tell what it is doing.

tmux is here, but you never see it: a **detached** session on a socket of the
app's own, one invisible window per spawn, drawing nothing. It owns the
processes; the app reads what they draw over a **control-mode client** and
renders every cell itself. That is what buys the one thing an owned terminal
could not — **quitting kills nothing.** The sessions are still running
afterwards, and `tmux -L harness-launcher attach` will show you they are. The
socket is the app's own, so whichever tmux server you use yourself is left
entirely alone: nothing of the app's ever appears among your own sessions.

A supervisor thread ticks about five times a second and hands the list an
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

The status ladder is built on what tmux, `ps` and the harness print, and the
terminal emulation on what a spawn draws — none of those a format this project
controls, so their tests read the recordings in
[`captured/`](captured/README.md) rather than strings written from memory. Each
recording says how it was made.

The tmux and control-mode tests drive a **real** tmux on a socket of their own,
and a real pty. There is no fake and no abstraction over either: what they cover
is exactly the part a fake would have to pretend about.

## License

[MIT](LICENSE) — do what you like with it, keep the copyright notice.
