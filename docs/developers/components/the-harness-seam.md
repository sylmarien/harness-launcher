# The harness seam

`src/harness/` is the one module that knows what the app launches. It owns
every harness-specific fact: the binary name, its flags, its environment, its
status vocabulary, the choices it offers, its glyph. Nothing outside the
module may name any of them. The whole module is `src/harness/mod.rs`.

The seam is a place, not an abstraction. There is deliberately no trait,
protocol, or base type with one implementation behind it: such a trait would
be speculative, and its shape would mirror this one harness. The invariant is
locality (every harness fact in one module), and the two greps below enforce
it mechanically.

## No I/O

The module performs no I/O: it translates, and the app acts. A module that
cannot touch a process, the filesystem, or tmux cannot leak a harness fact
into the tmux code. It is also testable without a process, a terminal, or a
multiplexer: every operation is a pure function from a spec to data.

## The surface

As it exists in `src/harness/mod.rs`:

- `launch_recipe(spec) -> LaunchRecipe` — turns the spec into a program,
  arguments (one per element, never a shell string), environment, and working
  directory. The app hands the recipe to `src/tmux.rs` to run
  ([the-tmux-session.md](the-tmux-session.md)). The work travels as the last
  positional argument, exactly as typed.
- `requirement() -> Requirement` — what must be installed, and the one line
  shown to the user when it is not. The app runs the check
  (`process::runnable_on`) before anything is created.
- `choices() -> Vec<Choices>` — the titled lists the spawn form offers, each
  with labelled options and a default. Built from `models()` and
  `effort_levels()`; `default_model()` and `default_effort_level()` supply
  unpicked answers.
- `spec_from(name, work, worktree, answers) -> SpawnSpec` — translates
  anonymous answers back into the harness's vocabulary. Each list recognises
  its own ids, in any order; anything unanswered falls to the default.
- `config_directory_variable()`, `status_files(configured, home)` and
  `StatusFiles::of(pid)` — where the harness records what each live session
  is doing. The module describes the locations; the app reads the
  environment and the file.
- `read_status(record, pid) -> Reading` — the status translation, below.
- `names_the_harness(command, argv0)` — whether a process the tie-breaker
  found in a pane's foreground group is the harness.
- `GLYPH` — the one-character mark a row shows to say which harness its
  spawn runs.

There is no stop recipe on this surface. Stopping a spawn is generic — a
`SIGTERM`, a bounded wait, `kill-pane` as the backstop — with no harness fact
in it, so it lives in `src/retirement.rs` ([retirement.md](retirement.md)).

## Two invariants, checked by greps

CI checks both (`.github/workflows/ci.yml`). The same `cargo fmt --check` /
`cargo lint` / `cargo test` trio a developer runs is in `CHEATSHEET.md` under
**Check**.

1. **The module performs no I/O.** The step named *"The harness module
   performs no I/O"* fails the build if this matches:

   ```
   grep -rEn 'std::process|std::fs|Command::' src/harness/
   ```

   Barring processes also bars tmux, because tmux is reached through a
   process. That is why the pattern names no multiplexer.

2. **Nothing outside the module names the harness.** The step named *"Nothing
   outside the harness module names the harness"* fails the build if this
   matches:

   ```
   grep -rEin 'claude|--effort|CLAUDE_CODE' src/ --exclude-dir=harness
   ```

   No binary name, flag, environment variable, status vocabulary, or screen
   shape may appear outside `src/harness/`.

Known limit: text matching is fooled by aliasing. A fact renamed on its way
out — a string rebuilt at runtime, a flag held in a variable — passes both
greps. The checks catch accidental violations, not deliberate ones.

## The form's choices

The seam supplies everything the spawn form offers: titled lists of labels,
each with a default, in the order the harness wants them read. The form
learns nothing else: not that a list is about models, not what any id means,
not how many lists there are. Its only question is "which of these?". An
empty list is omitted, not padded.

The answers come back as a bag of ids with no headings; `spec_from` decodes
them. Ids are therefore unique across every list `choices()` offers, and a
test holds that. How the form asks is in
[drafts-and-creation.md](drafts-and-creation.md).

## Status translation

The app hands `read_status` the text of a session record and the pid it was
looked up by, and gets a `Reading` back:

- `Working` — the record says `busy` or `waiting`.
- `Stopped` — the record says `idle` or `shell`. Finished, waiting on an
  answer, or dead: the harness distinguishes those; the app deliberately
  does not.
- `Unresolved(sentence)` — the record is not JSON, names another process,
  carries no status, or carries one this app does not know. This is not an
  error: one rung of the status ladder hands over to the next, with a
  sentence for a person rather than a value to branch on.

The harness's own words — `busy`, `idle`, `shell` — never escape the module.
The app's statuses stay exactly three. The ladder that consumes a `Reading`
is [knowing-what-a-spawn-is-doing.md](knowing-what-a-spawn-is-doing.md).

The record format is an undocumented internal of another program and is
treated as fallible throughout. The tests read it from
`captured/session-record.json`, a real capture that itself carries no status;
the status field is added through the `recorded` helper, as
`captured/README.md` describes.

## The input hole

The app relays keystrokes to the selected spawn through the control client
(see [the-tmux-session.md](the-tmux-session.md)) and originates no input of
its own. *Start a session* and *send input to a session* are separate
concepts, even though only the first is implemented. Delivering work is
therefore not defined as "the prompt is a launch argument": that is one
implementation of the first concept, and already false for a second turn.
The glossary terms are in [../glossary.md](../glossary.md).
