# A creation that fails halfway

The spine ([from keypress to running harness](from-keypress-to-running-harness.md))
covers the full sequence; this page covers every point where it can stop.
Every stop is a refusal, never a guess, and every refusal returns to the
draft: the row is a draft again, all text intact, with the reason under the
form. Vocabulary is defined in [the glossary](../glossary.md).

## Where "halfway" can be

The refusal points, in order:

1. **Before anything runs.** A draft that names no repository, or no work,
   refuses in place (`Draft::submitted` in `src/draft.rs`); nothing was
   attempted, so the record is the refusal alone. A harness not on the `PATH`
   refuses here too (`creation::harness_installed` in `src/creation.rs`); the
   check runs before the thread starts, so nothing is created.
2. **Resolving the repository** (`git::open` in `src/git.rs`). The path does
   not exist, is not a git repository, or is a bare repository with no
   working tree to branch from.
3. **Resolving the default branch** (`git::default_branch`). No commits yet, a
   detached `HEAD`, no `origin` remote, or `origin/HEAD` unrecorded or
   resolving to something that is not a remote branch. Every unknown refuses
   instead of guessing: a wrong base means an agent working from the wrong
   code for an hour.
4. **Preparing the worktree root** (`worktrees::prepare` in
   `src/worktrees.rs`). The app-owned root cannot be created.
5. **The worktree itself** (`Plan::create` → `git::add_worktree`).
   `git worktree add` refuses: the start point's ref is gone, or the path
   already exists. The refusal carries git's own error text.
6. **The window and the launch** (`creation::start`, on the main thread).
   Opening the window or the `respawn-pane` fails. This is past the worktree,
   so it is the one refusal that leaves something real behind.

A harness that starts and dies immediately is none of these: the creation
succeeded, and the result is a spawn that stopped. There is no fourth status.

```mermaid
sequenceDiagram
    participant M as main thread (src/app.rs)
    participant W as worker (creation::making)

    M->>W: Wanted
    W-->>M: Doing: "reading &lt;repo&gt; and resolving the branch…"
    Note over W: git::open, default_branch can refuse — nothing made yet
    W-->>M: Doing: "creating the worktree &lt;path&gt; on &lt;branch&gt;"
    Note over W: git worktree add can refuse — the record already names the path
    W-->>M: Refused(why)
    M->>M: Drafts::failed — a draft again, text intact, "!" on the row
```

## What the row says afterwards

Each line is written before its step runs (`made` in `src/creation.rs`), so a
failed creation leaves a record of what was already made. The worktree line
names a path and a branch before they exist, and survives even if the thread
is never heard from again.

On refusal (`Drafts::failed` in `src/draft.rs`):

- the row shows `!` in the gutter (`src/list.rs`), marking the draft as
  needing the user's attention;
- the heading over the record changes to `NOT STARTED`, with the reason under
  it in amber, quoting git's error text verbatim;
- the draft takes keys again, with every character, the caret, and the
  choices exactly as they were. A refusal never loses the typed text.

## What may now exist on disk

It depends on the refusal point. Steps 1–3 made nothing except, at most, the
app-owned worktree root, which is a shared directory rather than litter. A
step 5 refusal that git got partway into leaves at worst what git leaves.
Step 6 leaves a complete worktree and branch for a session that never ran.

Nothing cleans any of it up. Retiring removes a worktree, but retiring acts
on a spawn, and there is no spawn here: what would have become one is what
failed to start (`reported` in `src/app.rs`). The litter is announced, not
removed: the draft's record names the worktree and branch and stays until
somebody deals with them. The discard confirmation prompt also lists them
([a draft discarded mid-creation](a-draft-discarded-mid-creation.md)), and
the next run reports what it finds under the root
([starting and leaving](../components/starting-and-leaving.md)).

## Trying again

`F5` runs the same sequence with fresh names: the suffix is random and paths
are never reused (`src/names.rs`), so a worktree stranded by the last attempt
never blocks the next one. Fix the cause — add the remote, set origin's
`HEAD`, install the harness — then start again; the draft still holds
everything that was typed.

The full creation path, and the three marks a draft can show, are in
[drafts and creation](../components/drafts-and-creation.md).
