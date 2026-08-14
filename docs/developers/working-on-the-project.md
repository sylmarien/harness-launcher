# Working on the project

## The checks

```
cargo fmt --check
cargo lint
cargo test
```

- `cargo lint` is an alias in `.cargo/config.toml`: clippy over all targets,
  warnings as errors. Plain `cargo clippy` is not the same check — it prints
  warnings and still exits 0. The flags live in the alias so the local
  command and the CI command are identical.
- CI (`.github/workflows/ci.yml`) runs the same three commands. A clean run
  locally is a clean run in CI.
- The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on the
  first `cargo` call. Dependabot opens a PR when stable moves past the pin,
  so the bump arrives with CI already run on it.

## Formatting: no `rustfmt.toml`, on purpose

`cargo fmt` runs with no configuration. The defaults are the Rust Style
Guide, and `edition = "2024"` selects its current revision. Two things stay
convention rather than tooling, because rustfmt does not touch them:

- comment and doc-comment wrapping — done by hand;
- import grouping — the `std` / external / `crate` order is kept by the
  author.

## Clippy: `pedantic` on, `nursery` off

`Cargo.toml` adds the `pedantic` group to clippy's default set. It is
opinionated but stable, and most of what it catches is unstated intent. Its
negative priority lets a single lint be re-allowed later. `nursery`
(unstable, false-positive-prone) and `restriction` (a menu of mutually
contradictory lints) stay off.

## The two invariant greps

CI runs these too. They enforce the [harness seam](components/the-harness-seam.md)
mechanically:

```
grep -rEn 'std::process|std::fs|Command::' src/harness/              # must find nothing
grep -rEin 'claude|--effort|CLAUDE_CODE' src/ --exclude-dir=harness  # must find nothing
```

- The first enforces **the harness module performs no I/O**. It translates;
  the app acts. tmux is reached through a process, so banning processes bans
  tmux too.
- The second enforces **nothing outside the module names the harness**: no
  binary name, flag, environment variable, status vocabulary, or screen
  shape.

Known limit: text matching can be defeated by renaming a string on its way
out of the module. The greps catch honest mistakes, not determined ones.
Stronger mechanisms (a workspace split, a per-directory lint) become worth it
when there is a second author, a second harness, or a reason to bar the seam
from an external dependency.

## Tests use the real programs

- **The tmux and control-mode tests drive a real tmux** on a private `-L`
  socket, and a real pty. There is no fake: what these tests cover is exactly
  the behaviour a fake would have to invent, and a test-only second
  implementation would undermine the harness seam's one-adapter rule.
- **Parsers are tested against recordings.** The status ladder parses tmux,
  `ps` and harness output; the emulator parses spawn output. None of these
  formats is controlled by this project, so the tests read the captures in
  [`captured/`](../../captured/README.md). Each capture documents how it was
  made.
- **The scale test** behind
  [`docs/evidence/scale-at-twenty.md`](../evidence/scale-at-twenty.md) is
  `tests/twenty_spawns.rs`. It is `#[ignore]`d because it takes minutes and
  needs the machine to itself:

  ```
  cargo test --release --test twenty_spawns -- --ignored --nocapture --test-threads=1
  ```

- **Untested, by agreement:** visual judgement calls (redraw feel, colours),
  and creation end to end against a real `claude` — that costs tokens and
  needs auth. Emulator fidelity against the real harness therefore has to be
  checked by running the app.

## Writing docs and code here

- Use the [glossary](glossary.md)'s vocabulary. Do not substitute synonyms.
- Documentation describes what the code does now. The reasons behind past
  decisions live in issues and pull requests. The documents under
  [`docs/tranches/`](../tranches/) are frozen records; do not update or
  append to them.
