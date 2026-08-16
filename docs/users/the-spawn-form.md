# The spawn form

You compose work inside the app, in the same screen as everything else. A
draft is not a dialog. It is a row of its own, pinned above the repositories,
with a form in the slot while it is selected.

## Starting a draft

- With no arguments, the app opens on a blank form: nothing running, a draft
  in the slot. Everything the command line accepts can be entered here.
- `F2` starts a draft at any time. Several can be open at once.

## Filling it in

The form has three parts:

1. **Repository** — a local git repository, or any directory inside one. A
   draft that names a repository but no work is refused, not guessed at.
   Typing part of a [saved project](saved-projects.md)'s name suggests the
   projects it matches. `F5` starts the spawn on the one you moved onto with
   `↑` or `↓`, or on a name you typed out in full. Anything else is a path.
2. **The work** — as long as it needs to be. `Enter` inserts a new line here.
3. **Harness options** — a model and an effort level, picked from lists. The
   choices come from the harness itself, so the form never goes stale.

Navigation: `Tab` / `Shift-Tab` move between fields, `↑` `↓` pick an option,
`Enter` moves to the next field everywhere except the work field. A draft
survives leaving it: come back and the text, caret and field are unchanged.

## Starting the spawn

`F5` turns the draft into a spawn. The app announces each step before it
runs: repository read, worktree made, harness started. The slow work runs on
its own thread, so the screen keeps drawing and other spawns keep arriving.
The draft's row shows its state: `+` being written, `>` being made, `!` it
failed to start.

A failed start never loses your text. The row becomes a draft again, with
every character intact and the reason under the form. Fix the named problem
and press `F5` again.

## Discarding a draft

`F3` discards the selected draft. Draft text exists nowhere else, so this is
the only action in the app that asks for confirmation. The first `F3` asks; a
second `F3` confirms; **any other key cancels**.

Two special cases:

- A draft that is mid-creation cannot be discarded until the creation stops.
  Its row is the only record of what is being made.
- If a failed creation left a worktree and a branch behind, the confirmation
  says so before you lose the only note of them.
