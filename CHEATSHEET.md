# Cheat sheet

## Run

```
cargo run                    # opens on a blank form — write the work there
cargo run -- <repository> "<the work>" [--model <id>] [--level <id>]
cargo run -- <repository> "<the work>" --and <other-repository> "<more work>"
cargo run -- --help          # the models and effort levels on offer
```

`<repository>` is a local git repository, or any directory inside one. `--and`
separates one spawn from the next. With no arguments the app opens with nothing
running and a draft in the slot; `F5` starts what you write in it.

## Check

```
cargo fmt --check
cargo lint                   # clippy over everything, warnings are errors
cargo test
```

`cargo lint` is the alias in `.cargo/config.toml`, and CI runs the same three
commands — so a clean run here is a clean run there. Plain `cargo clippy` is not
the same check: it prints the warnings and still exits 0.

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on the
first `cargo` call, so there is nothing to set up. Dependabot opens a pull
request when stable moves past the pin, and CI runs on it — so the bump arrives
knowing whether it is clean.

## Keys

| Key         | Does                                     |
| ----------- | ---------------------------------------- |
| `F2`        | start a draft                            |
| `F3`        | throw the draft away — it asks first     |
| `F5`        | make the draft in the slot into a spawn  |
| `F6` / `F7` | move the selection up / down             |
| `F9`        | retire the selected spawn                |
| `F10`       | quit — nothing is killed                 |

Every other key goes to whatever is in the slot: to the session, or to the draft.

In a draft: `Tab` / `Shift-Tab` move between fields, `↑` `↓` pick an option,
`Enter` is a new line in the work and moves on everywhere else.

## Marks in the list

- Spawns — `·` working · `●` stopped · `?` the app cannot tell · `-` being retired
- Drafts — `+` being written · `>` being made into a spawn
- Either — `!` it stopped, or would not retire · `▍` the selected row
- Every spawn — `✻` beside the mark, the harness it is running under
- Every spawn — its age at the right, how long that status has held (`31m`,
  `1h4m`); a row too narrow to write its own name in full drops its own age
- One line each, selected or not; the selected row is painted across the list,
  amber instead of cyan when it is one the app is admitting something about

## Worth knowing

- **Quitting kills nothing.** The sessions outlive the app —
  `tmux -L harness-launcher attach` finds them.
- **Retiring** stops the session, removes the worktree and **keeps the branch**.
  It refuses if anything in the worktree is uncommitted: clean up, press `F9`
  again.
- Worktrees live under `$XDG_DATA_HOME/harness-launcher/worktrees`
  (`~/.local/share/...` when that is unset).
- **A `?` spawn explains itself when you select it** — its pane, the process
  whose status would not resolve, and what the app could last tell — over the top
  of its screen, which is still live underneath. A retirement writes there too:
  what it is doing, and why it refused. Both at once puts the retirement last.
- **A refusal never costs your typing.** A draft that could not be started keeps
  its text and its choices, says why, and `F5` tries it again.
