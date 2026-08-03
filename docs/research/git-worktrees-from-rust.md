# Driving git worktrees from Rust

> **Fact-finding for [issue #8](https://github.com/sylmarien/harness-launcher/issues/8).** This
> document does **not** pick an approach — that is issue #12 ("Worktree placement, naming, branch
> policy and dirty rules"), worked with the human. Everything here is input to that decision.

## How to read this

Claims are tagged:

- **[V]** — **verified by experiment** on this machine, `git version 2.43.0`, Linux. The exact
  commands and their output were run against throwaway repositories.
- **[S]** — **read from primary source** (git's own documentation or C source; libgit2's headers or
  source; git2-rs's source). Not executed.
- **[I]** — **inferred**. Reasoning from [V] or [S], not directly established.

Sources fetched 2026-08-03. Links to `master`/`main` are moving targets; version-pinned links are
used where the version matters.

**One bias runs through this document.** The tranche-1 rule is *"if the worktree is dirty, retiring
refuses"* ([`docs/tranches/01-the-core-loop.md`](../tranches/01-the-core-loop.md)). Being wrong in
the permissive direction — calling a worktree clean when it is not — destroys uncommitted work.
Where a check is silently permissive, this document says so loudly.

---

## 1. The two approaches, at a glance

| Capability | `git` binary (shell out) | `git2` crate / libgit2 |
| --- | --- | --- |
| Create a worktree | `git worktree add` | `Repository::worktree()` **[S]** |
| List worktrees | `git worktree list --porcelain [-z]` | `Repository::worktrees()` + `find_worktree()` **[S]** |
| Remove a worktree **with a cleanliness guard** | `git worktree remove` **[V]** | **Does not exist** **[S]** |
| Delete worktree metadata | `git worktree prune` | `Worktree::prune()` **[S]** |
| Delete the working directory | via `remove` | `WorktreePruneOptions::working_tree(true)` — **no cleanliness check** **[S]** |
| Lock / unlock | `git worktree lock/unlock` | `Worktree::lock()/unlock()/is_locked()` **[S]** |
| Detect staleness | `list --porcelain` reports `prunable <reason>` **[V]** | `Worktree::validate()`, `is_prunable()` **[S]** |
| Move a worktree | `git worktree move` | **Not exposed** **[S]** |
| Repair broken links | `git worktree repair` | **Not exposed** **[S]** |
| Detached-HEAD worktree | `git worktree add --detach` **[V]** | **Rejected** — requires a branch ref **[S]** |
| Worktree from an arbitrary commit-ish | `git worktree add <path> <commit-ish>` **[V]** | Only a `Reference` that `git_reference_is_branch()` accepts **[S]** |
| Orphan / unborn-HEAD repo | inferred automatically **[V]** | not found in the API **[S]** |
| Status / dirty detection | `git status --porcelain` | `Repository::statuses()` **[S]** |

### What libgit2 does *not* support for worktrees

Established by reading
[`include/git2/worktree.h`](https://raw.githubusercontent.com/libgit2/libgit2/main/include/git2/worktree.h)
and
[`src/libgit2/worktree.c`](https://raw.githubusercontent.com/libgit2/libgit2/main/src/libgit2/worktree.c):

1. **There is no `git_worktree_remove`.** The whole header is `list`, `lookup`,
   `open_from_repository`, `free`, `validate`, `add_options_init`, `add`, `lock`, `unlock`,
   `is_locked`, `name`, `path`, `prune_options_init`, `is_prunable`, `prune`. **[S]**
2. **`git_worktree_prune` performs no cleanliness check whatsoever.** With
   `GIT_WORKTREE_PRUNE_WORKING_TREE` set it recursively deletes the working directory; nothing in
   `worktree.c` inspects working-tree content, and `git_worktree_validate()` only checks that the
   required files exist. **[S]** In git2-rs terms, `WorktreePruneOptions::working_tree(true)` +
   `Worktree::prune()` is closer to `rm -rf` than to `git worktree remove`. Any dirty guard on this
   path must be written by hand.
3. **No detached HEAD.** `git_worktree_add` requires `git_reference_is_branch(wtopts.ref)` and
   errors with *"reference is not a branch"* otherwise. **[S]**
4. **No `move`, no `repair`.** **[S]**
5. **The worktree *name* is an independent parameter**, not derived from the path
   (`git_worktree_add(out, repo, name, path, opts)`), and `worktree.c` does not validate it. **[S]**
   The `git` CLI, by contrast, derives the id from the path and sanitises it (see §6.4). This is a
   real behavioural difference between the two backends, not a detail.
6. **`git_worktree_add` fails if the path already exists** — it uses
   `git_futils_mkdir(..., GIT_MKDIR_EXCL)`. **[S]** The CLI tolerates an existing *empty* directory
   (§2.2).

### What each costs

- **Shelling out** costs a process spawn per call and string parsing of the output. Measured on a
  one-file repo, 20 × `git status --porcelain` took **58 ms** (≈2.9 ms each) and 20 ×
  `git rev-parse HEAD` took **52 ms** (≈2.6 ms each) — i.e. on a small tree essentially *all* of it
  is process-spawn overhead, and the status computation itself is noise. **[V]** On a large tree the
  status cost grows with the number of tracked files and is dominated by `lstat` traffic. **[I]**
  It also makes the `git` binary a runtime dependency whose version and behaviour vary per machine,
  and — critically — subjects every call to the *user's* git configuration (§4.4).
- **`git2`/libgit2** costs a C dependency. `git2` 0.21.0 (2026-05-18) binds libgit2 1.9.3; 0.20.x
  binds 1.9.0–1.9.3. **[S]** It buys in-process calls with typed errors and no output parsing, but
  as shown above the one operation this project most needs a guard on — removal — is exactly the one
  libgit2 does not implement.
- **Hybrid** is available: use `git2` for reading (list, status) and the binary for the mutating
  operations, or vice versa. Noted as an option, not recommended here.

---

## 2. `worktree add` — what it requires and what it refuses

### 2.1 Branch conflicts

All **[V]** on git 2.43.0:

| Command | Result |
| --- | --- |
| `git worktree add ../w2 feat` (branch `feat` exists, not checked out) | succeeds, checks out `feat` |
| `git worktree add -b feat ../w3` (branch `feat` exists) | `fatal: a branch named 'feat' already exists` (exit 255) |
| `git worktree add ../w4 feat` (`feat` checked out in another worktree) | `fatal: 'feat' is already used by worktree at '<path>'` (exit 128) |
| `git worktree add -B resetme ../wB` (exists, not checked out) | succeeds — *"resetting branch 'resetme'; was at b5be8d4"* — **moves the branch ref** |
| `git worktree add -B resetme ../wB2` (checked out elsewhere) | `fatal: 'resetme' is already used by worktree at '<path>'` (exit 128) |

The documentation states that `add` *"refuses to create a new worktree when `<commit-ish>` is a
branch name and is already checked out by another worktree"*, and that `--force` *"overrides these
safeguards"*. **[S]**

### 2.2 Path conflicts

**[V]**

- Path exists and is **non-empty** → `fatal: '<path>' already exists` (exit 128).
- Path exists and is **empty** → **succeeds**. Git checks out into it.
- Path is registered but the directory is missing →
  `fatal: '<path>' is a missing but already registered worktree; use 'add -f' to override, or 'prune' or 'remove' to clear`
  (exit 128).
- Same, but the stale entry is also **locked** →
  `fatal: '<path>' is a missing but locked worktree; use 'add -f -f' to override, or 'unlock' and 'prune' or 'remove' to clear`
  (exit 128).

libgit2 refuses *any* pre-existing path (`GIT_MKDIR_EXCL`), including an empty one. **[S]**

### 2.3 Edge repositories

**[V]**

- **Bare repository**: `git worktree add` works. `list --porcelain` reports the bare repo itself as
  a `worktree` entry with a bare `bare` line and no `HEAD`/`branch`.
- **Repository with no commits (unborn HEAD)**: `git worktree add <path>` prints
  *"No possible source branch, inferring '--orphan'"* and succeeds (this inference is present at
  2.43.0; `--orphan` is **not** documented at v2.35.0 **[S]**, so it is version-gated).

### 2.4 Concurrency

Five simultaneous background `git worktree add` invocations against one repository all succeeded and
all five worktrees were registered. **[V]** Each linked worktree gets **its own index** at
`.git/worktrees/<id>/index`, separate from the main checkout's `.git/index` **[V]** — so index
locking does not serialise concurrent spawns. Ref creation still contends on the shared ref store;
that contention was not stress-tested beyond n=5. **[I]**

---

## 3. How the worktree's branch is created

### 3.1 The DWIM default is a footgun

When `<commit-ish>` is omitted, *"the new worktree is associated with a branch (call it `<branch>`)
named after `$(basename <path>)`"*. **[S]** What the documentation does not make obvious is what
happens when that branch **already exists**:

```
$ git branch dwim                      # branch 'dwim' already exists, not checked out
$ git worktree add ../dwim
Preparing worktree (checking out 'dwim')     # <- NOT "new branch"
```

**[V]** — bare `git worktree add <path>` **silently checks out the pre-existing branch** rather than
creating a new one. For a launcher that names worktree directories from a task, a name collision
would drop a fresh agent onto somebody else's branch, with its history and its commits, and the
agent's work would land there.

If the same-named branch is *checked out in another worktree*, git falls back to creating a new
branch named after the path basename **[V]** — so the failure mode is inconsistent and depends on
what other spawns are doing.

The explicit forms are unambiguous and both fail loudly:

- `-b <branch>` → creates; **fails** if the branch exists. **[V]**
- `-B <branch>` → creates or **force-resets an existing branch to the new start point**, discarding
  where it pointed. **[V]** This *moves a ref the user may care about*; it is not a safe default.

`--guess-remote` / `worktree.guessRemote` add a further DWIM layer: with them, a bare `add` whose
basename matches a remote-tracking branch will create a local branch tracking it. **[S]** A launcher
should know whether this config is set on the user's machine, because it changes what a bare `add`
does.

### 3.2 libgit2's equivalent

`git_worktree_add` with `opts->ref == NULL`: if `checkout_existing` is set and a local branch of that
name exists, it uses that branch; otherwise it calls `git_branch_create(&ref, repo, name, commit,
/*force=*/false)`, which fails if the branch exists. It also calls `git_branch_is_checked_out(ref)`
and errors *"reference %s is already checked out"*. **[S]** So `checkout_existing(false)` (the
git2-rs default) gives the loud behaviour the CLI only gives you under `-b`.

### 3.3 Branches survive removal

`git worktree remove` deletes the checkout and the metadata; the branch ref is untouched. **[V]** —
after removing worktrees on branches `t-committed`, `t-ignored`, `t-emptydir`, all three branches
were still listed by `git branch`. This matches the tranche's "removes the worktree and leaves the
branch alone" directly.

**Detached HEAD does not have this property, and the difference is destructive.** **[V]**

```
git worktree add --detach ../t-det
# ... agent commits there ...
git worktree remove ../t-det        # succeeds; tree is clean
git fsck --unreachable --no-reflogs # the commit is now UNREACHABLE
```

Because the commit was only reachable from the worktree's own `HEAD` and its per-worktree reflog at
`.git/worktrees/<id>/logs/HEAD` — **both of which `remove` deletes** — committed work made on a
detached HEAD becomes unreachable and eligible for `gc`. On a branch, the same commit stays reachable
via `refs/heads/<branch>`. **[V]** A branch per spawn is therefore not merely a convention; it is
what makes "leave the branch alone" a real safety property.

---

## 4. Determining whether a worktree is dirty

This is the section where being permissive costs a user their work.

### 4.1 What `git worktree remove` itself checks — exactly

From [`builtin/worktree.c`](https://raw.githubusercontent.com/git/git/master/builtin/worktree.c),
the `remove` path is: validate the worktree → refuse if it is the main worktree → refuse if locked
(unless `-f -f`) → if not forced, `check_clean_worktree()`. `check_clean_worktree()` does two
things **[S]**:

1. `validate_no_submodules()` — reads the index and dies with *"working trees containing submodules
   cannot be moved or removed"* if any `S_ISGITLINK` entry is present.
2. Spawns a **child `git` process**:
   ```c
   strvec_pushl(&cp.args, "status", "--porcelain", "--ignore-submodules=none", NULL);
   ```
   with `cp.dir = wt->path`, `GIT_DIR`/`GIT_WORK_TREE` pointing at the worktree, capturing stdout. If
   **one single byte** comes back, it dies with
   *"'%s' contains modified or untracked files, use --force to delete it"*.

So: **"dirty" is defined as "`git status --porcelain --ignore-submodules=none` produced any
output".** Nothing more, nothing less. Every property of that command is a property of git's dirty
rule.

Empirically **[V]**, `git worktree remove` on git 2.43.0:

| Worktree state | `remove` |
| --- | --- |
| unstaged modification to a tracked file | **refuses** (exit 128) |
| staged-but-uncommitted change | **refuses** (exit 128) |
| untracked file | **refuses** (exit 128) |
| merge conflict in progress (`AA` entries) | **refuses** (exit 128) |
| **`.gitignore`-ignored file present** | **succeeds — file is deleted** |
| **empty untracked directory** | **succeeds — directory is deleted** |
| **stash entries created from this worktree** | **succeeds** |
| **commits on the branch not merged/pushed anywhere** | **succeeds** |
| worktree contains a submodule (otherwise clean) | **refuses** — but `--force` (once) bypasses it |
| worktree is locked | **refuses**; `--force` alone is *not* enough, needs `-f -f` |
| worktree directory already deleted from disk | **succeeds**, cleans up metadata |
| target is the main worktree | `fatal: '<path>' is a main working tree` (exit 128) |
| target is not a registered worktree | `fatal: '<path>' is not a working tree` (exit 128) |

Exit code for every refusal above is **128**, except `git branch -D` on a held branch, which is 1.
Exit code 255 was observed for `add -b <existing>`. **[V]** Exit codes alone are not a reliable
discriminator between "refused because dirty" and "refused for some other reason"; the message text
is, but message text is not a stable interface. **[I]**

### 4.2 The four kinds of "dirty", separated

The ticket asks whether these should count the same. They are genuinely different, and here is what
each *is*:

| Kind | Where it lives | Detected by default `--porcelain`? | Lost if the checkout is deleted? |
| --- | --- | --- | --- |
| **Unstaged changes** to tracked files | working tree only | yes (2nd status column) **[V]** | **yes — irrecoverable** |
| **Staged, uncommitted** changes | worktree's own `.git/worktrees/<id>/index`, blobs in the shared object database | yes (1st status column) **[V]** | blobs survive in the odb but are unreachable; the *arrangement* is lost with the index. Recovery means `git fsck --lost-found` archaeology. **[I]** |
| **Untracked files** | working tree only | yes, as `??` **[V]** | **yes — irrecoverable** |
| **Ignored files** | working tree only | **no** **[V]** | **yes — irrecoverable, and `git worktree remove` deletes them without a word** **[V]** |
| **Stashes** | `refs/stash`, **shared across all worktrees** **[V]** | no **[V]** | **no** — survives removal and stays visible from the main checkout and every other worktree **[V]** |
| **Commits** on the spawn's branch | `refs/heads/<branch>`, shared | no **[V]** | no (on a branch); **yes** on a detached HEAD (§3.3) **[V]** |

Notes that bear on the "should they count the same" question:

- **Ignored files are the sharpest asymmetry.** They are invisible to the check and destroyed by the
  removal. `.env`, local database files, `node_modules`, build output, editor state — all ignored in
  a typical repository, none of it reproducible in general. If harness-launcher adopts git's own
  rule verbatim, it inherits this. Deciding otherwise means counting `--ignored` output, which will
  make nearly every real worktree permanently un-retirable (any `target/`, `node_modules/` or
  `.venv/` sitting in it) — the two ends of this trade-off are both bad and the middle is a policy
  call for #12.
- **Stashes are the opposite case.** They *look* like uncommitted work but are stored in a shared
  ref that outlives the worktree, so nothing is lost. **[V]** — a stash pushed from worktree
  `t-s1` was still in `git stash list` from the main checkout after `t-s1` was removed. What *is*
  lost is the association: the entry is labelled `On t-s1: <message>` and points at a branch that
  may no longer have a checkout. Counting a stash as "dirty" would refuse retirement for work that
  is already safe.
- **Empty untracked directories** are invisible to git in every mode, because git tracks files, not
  directories. **[S]/[V]** They are silently deleted. Usually harmless; occasionally not (a mount
  point, a directory an agent was told to fill).

### 4.3 What `--porcelain` does and does not report

From [git-status(1)](https://git-scm.com/docs/git-status) **[S]**, confirmed **[V]**:

- Default `--untracked-files` is **`normal`**: untracked *files and directories* are shown, but
  files *inside* an untracked directory are collapsed to the directory. Verified: an untracked
  `sub/deeper/f.txt` shows as `?? sub/` by default and `?? sub/deeper/f.txt` under `-uall`. For a
  boolean "is anything untracked" question this collapsing is harmless — either way there is output.
- **Ignored files are not listed unless `--ignored` is passed.** **[S]/[V]**
- **Stash entries are never reported** except via `--show-stash`, which is not a porcelain field.
  **[S]/[V]**
- `--porcelain` (v1) *"will remain stable across Git versions and regardless of user
  configuration"* **[S]** — this promise is about the **format**, not about **which entries appear**.
  §4.4 is the counter-example.

### 4.4 Two verified ways the check silently misses real work

Both destroy data. Both were reproduced on git 2.43.0.

**(a) `status.showUntrackedFiles = no`.**

```
$ git -C wt status --porcelain
?? notes.txt
$ git worktree remove wt
fatal: '<path>' contains modified or untracked files, use --force to delete it

$ git config status.showUntrackedFiles no      # repo-level config, shared by all worktrees
$ git -C wt status --porcelain                 # <- no output at all
$ git worktree remove wt
$ [ -f wt/notes.txt ] && echo present || echo DELETED
DELETED
```

**[V]** Because `git worktree remove` shells out to `git status --porcelain` **without
`--untracked-files=<mode>`**, the user's config decides whether untracked files count. This is a
real setting people enable on large repositories for speed. It is set-able at system, global,
repository and (with `extensions.worktreeConfig`) per-worktree level. **[S]**

A launcher that runs its *own* status check should pass the mode explicitly
(`--untracked-files=normal`) and consider `-c status.showUntrackedFiles=normal`, rather than
inheriting whatever is configured. **[I]**

**(b) `git update-index --assume-unchanged` (and `--skip-worktree`).**

```
$ echo "secret edit" >> wt/README.md
$ git -C wt update-index --assume-unchanged README.md
$ git -C wt status --porcelain      # <- no output
$ git worktree remove wt            # succeeds; the edit is gone
```

**[V]** The bit lives in the worktree's own index. Anything carrying it is invisible to status and is
deleted with the checkout. `--skip-worktree` is expected to behave the same way; only
`--assume-unchanged` was tested. **[I]**

### 4.5 Checking dirtiness with `git2` — and its two default traps

`Repository::statuses(&self, options: Option<&mut StatusOptions>) -> Result<Statuses, Error>` passes
`ptr::null()` to `git_status_list_new` when `options` is `None`. **[S]** libgit2's
`git_status_list_new` does `flags = opts ? opts->flags : GIT_STATUS_OPT_DEFAULTS`, and

```c
#define GIT_STATUS_OPT_DEFAULTS \
	(GIT_STATUS_OPT_INCLUDE_IGNORED | \
	GIT_STATUS_OPT_INCLUDE_UNTRACKED | \
	GIT_STATUS_OPT_RECURSE_UNTRACKED_DIRS)
```

**[S]** — whereas `GIT_STATUS_OPTIONS_INIT` is just `{GIT_STATUS_OPTIONS_VERSION}`, i.e. **flags =
0**. **[S]** And `StatusOptions::new()` in git2-rs calls `git_status_init_options`. **[S]**

The consequence is a pair of opposite traps:

- **`repo.statuses(None)`** → `GIT_STATUS_OPT_DEFAULTS` → **ignored files count as status entries**.
  Too strict: every repo with a `target/` is permanently dirty.
- **`repo.statuses(Some(&mut StatusOptions::new()))`** → flags 0 → **untracked files are NOT
  reported**. **Too permissive — this is the mistake that deletes an agent's brand-new,
  never-`git add`-ed files.**

The correct shape is explicit:

```rust
let mut opts = git2::StatusOptions::new();
opts.include_untracked(true)      // ?? files  -- NOT on by default
    .include_ignored(false)       // policy call for #12
    .include_unmodified(false)
    .recurse_untracked_dirs(false) // presence is what matters, not the list
    .exclude_submodules(false)
    .no_refresh(false);
let dirty = !repo.statuses(Some(&mut opts))?.is_empty();
```

**One genuine advantage of the git2 route:** `src/libgit2/status.c` performs **no git-config
lookups** — it does not read `status.showUntrackedFiles`. **[S]** So the §4.4(a) failure mode does
not apply to `Repository::statuses`, whose behaviour is fixed by the flags the caller passes. The
`.gitignore` / `core.excludesFile` rules *are* still honoured (that is what `INCLUDE_IGNORED`
selects over). Whether libgit2 honours the `assume-unchanged` bit was **not verified**.

Relevant `Status` bits for classifying, rather than merely detecting: `INDEX_NEW/MODIFIED/DELETED/
RENAMED/TYPECHANGE` (staged), `WT_NEW/MODIFIED/DELETED/TYPECHANGE/RENAMED/UNREADABLE` (unstaged +
untracked), `IGNORED`, `CONFLICTED`. **[S]** This is a real advantage over parsing porcelain text if
the app ever wants to *tell the user what is dirty* rather than just refuse.

### 4.6 What no status check catches, either way

- Stashes (§4.2) — by design, and arguably correct.
- Unpushed commits — outside the app's remit; the tranche says the app never lands work.
- A running agent process with the worktree as its cwd, or with open file handles into it. On Linux
  the directory is removed regardless and the process is left with a deleted cwd. **[I]** — not
  tested, but it follows from POSIX unlink semantics. This is a *liveness* concern separate from
  dirtiness, and `git worktree lock` is the mechanism git offers for it (§6.5).

---

## 5. What a removed worktree leaves behind, and when pruning is needed

### 5.1 The on-disk layout

For a linked worktree, **[V]**:

- The worktree's `.git` is a **file**, not a directory:
  `gitdir: /abs/path/to/main/.git/worktrees/<id>`
- The admin directory `main/.git/worktrees/<id>/` contained: `HEAD`, `ORIG_HEAD`, `commondir`,
  `gitdir`, `index`, `logs/`.
  - `gitdir` holds *"the absolute path back to the .git file that points to here"* **[S]** — this is
    the link git follows to decide the worktree still exists.
  - `commondir` held `../..` — the path to the shared repository.
  - `locked`, if present, blocks pruning and may contain a reason. **[S]**
  - `config.worktree` exists only under `extensions.worktreeConfig`. **[S]**
- **Shared** across worktrees: the object database, config, and refs — including `refs/stash` **[V]**.
  **Per-worktree**: `HEAD`, `index`, `logs/HEAD`, `ORIG_HEAD` **[V]**, plus the `refs/bisect`,
  `refs/worktree` and `refs/rewritten` namespaces **[S]**. Verified for `refs/worktree`: a ref
  written as `refs/worktree/mine` inside a worktree was **not** resolvable from the main checkout.
  **[V]**

### 5.2 After a successful `git worktree remove`

**[V]** — nothing is left. The checkout directory is gone, `.git/worktrees/<id>/` is gone (git also
removes the `worktrees` parent directory if it becomes empty **[S]**), and `git worktree list` no
longer mentions it. **No prune is needed.** What survives, deliberately: the branch ref, its commits,
and any stashes.

### 5.3 After a worktree directory is deleted out-of-band

This is the case that needs pruning, and it is the case a crashed launcher creates.

**[V]** After `rm -rf` of a worktree directory:

```
$ git worktree list --porcelain
worktree /.../t-manual
HEAD b5be8d4...
branch refs/heads/t-manual
prunable gitdir file points to non-existent location

$ git worktree prune -n -v
Removing worktrees/t-manual: gitdir file points to non-existent location
```

- The stale `.git/worktrees/<id>/` directory **remains** until pruned. **[V]**
- `git worktree list --porcelain` marks it with a `prunable <reason>` line — a machine-readable way
  to find dangling entries without running `prune`. **[V]**
- `git worktree add` does **not** auto-prune: after deleting one worktree's directory and adding a
  different worktree, the stale entry was still there. **[V]**
- The **branch survives**. **[V]**
- Re-using the path (or the derived id) fails until you prune or remove (§2.2). **[V]**
- `git worktree remove <path>` also works on an already-deleted directory and cleans the metadata —
  a targeted alternative to a repo-wide `prune`. **[V]**
- `git gc` runs `git worktree prune --expire 3.months.ago`; `gc.worktreePruneExpire` overrides the
  grace period. **[S]** So stale entries do eventually disappear on their own, but only after months
  and only if `gc` runs.

`git worktree repair` exists for the inverse problem — the *directory* was moved and the pointers
need fixing. **[S]** Not exposed by libgit2.

### 5.4 Locked worktrees

**[V]**

```
$ git worktree lock --reason "busy" ../t-locked
$ git worktree remove ../t-locked
fatal: cannot remove a locked working tree, lock reason: busy
use 'remove -f -f' to override or unlock first
$ git worktree remove --force ../t-locked     # still refuses
$ git worktree remove --force --force ../t-locked   # succeeds
```

A single `--force` bypasses the *cleanliness* check but **not** the lock; the lock needs `-f -f`.
`git worktree add --lock --reason <s>` creates a worktree locked from birth. **[V]** Locked stale
entries are also protected from `prune` (`GIT_WORKTREE_PRUNE_LOCKED` / `WorktreePruneOptions::locked`
is libgit2's override **[S]**).

---

## 6. Repositories that already have worktrees, and the user working in the main checkout

### 6.1 What the main checkout can no longer do

**[V]** With branch `t-held` checked out in a linked worktree, from the main checkout:

| Command | Result |
| --- | --- |
| `git checkout t-held` | `fatal: 't-held' is already used by worktree at '<path>'` (128) |
| `git switch t-held` | same (128) |
| `git branch -D t-held` | `error: cannot delete branch 't-held' used by worktree at '<path>'` (1) |
| `git branch -f t-held HEAD` | `fatal: cannot force update the branch 't-held' used by worktree at '<path>'` (128) |
| `git update-ref refs/heads/t-held HEAD` | **succeeds (0)** |

The last row matters: **the "held branch" guard lives in the porcelain, not in the ref store.**
Plumbing can move a branch out from under a worktree. **[V]** A launcher should not assume the
branch a spawn sits on is immutable while the spawn runs.

Practically: every branch the app checks out into a worktree is a branch the user can no longer check
out, delete, or force-move in their main checkout until the spawn is retired. At 15–20 concurrent
spawns that is 15–20 branch names taken out of circulation.

### 6.2 Worktrees placed inside the repository

**[V]** Adding a worktree at `<main>/inside-wt` makes the main checkout's `git status --porcelain`
report `?? inside-wt/`. The app would be polluting the user's own status output — and, recursively,
any *other* worktree's dirty check if worktrees were nested. Placement outside the repository, or
inside it plus an ignore entry, avoids this. (Placement is #12's call; this is the constraint it has
to satisfy.)

### 6.3 Reading the worktree list

`git worktree list --porcelain` emits one stanza per worktree, blank-line separated: `worktree
<path>`, then `HEAD <sha>` and `branch <ref>` **or** `detached`, or `bare` for a bare main repo; plus
optional `locked [<reason>]` and `prunable <reason>` lines. **[V]** Parsing caveats **[V]**:

- In `--porcelain` (non-`-z`) mode a lock reason containing a newline is **C-quoted**:
  `locked "line one\nline two"`. Under `-z` the fields are NUL-separated and the reason is raw.
- Paths are emitted **verbatim**, including spaces and non-ASCII, so line-oriented splitting on
  whitespace is wrong; `-z` is the safe form.
- `-z` is **not** documented at v2.35.0 **[S]** and is present at 2.43.0 **[V]** — version-gated.
- `prunable` and `locked` annotations and `list -v` **are** documented at v2.35.0 **[S]**.
- `git worktree remove` accepts a path, a bare worktree name, or `.` from inside the worktree
  itself. **[V]**

### 6.4 The worktree id is derived and sanitised — and can collide

**[V]** The id under `.git/worktrees/` is not the path:

- `add ../a/foo` then `add ../b/foo` → ids `foo` and **`foo1`**. Git disambiguates by suffix.
- `add "../has space & Ünicode"` → id `has-space-&-Ünicode`. Spaces become dashes; `&` and non-ASCII
  survive.

So the id is not a stable key the app can compute itself, and it is not the branch name either. The
**path** is the only identifier the app supplies and controls. libgit2, by contrast, takes the name
as an explicit parameter with no sanitising **[S]** — a launcher using git2 would have to invent and
enforce its own naming discipline.

### 6.5 Locking as a concurrency signal

`git worktree lock --reason "<text>"` is the mechanism git provides for "this worktree is in use,
don't reap it". It survives across processes, appears in `list --porcelain`, blocks `prune` and
requires `-f -f` to remove. **[V]** It is a plausible way for the app to protect a running spawn's
worktree from the user's own `git worktree prune`, and to leave a human-readable trace. Whether to
use it is #12's call; that it exists and how it behaves is established here.

---

## 7. Version floors (bracketed, not pinned)

Verified by fetching the version-pinned documentation **[S]**:

- **v2.16.0** documents only `add`, `list`, `lock`, `prune`, `unlock`. **`remove` and `move` do not
  exist**; the BUGS section lists them as wanted, describing the future `remove` as *"remove a linked
  working tree and its administrative files (and warn if the working tree is dirty)"*.
- **v2.35.0** documents `repair`, `list -v`, and the `locked` / `prunable` annotations, but **not**
  `-z` and **not** `--orphan`.
- **2.43.0** (tested here) has all of the above including `-z` and the unborn-HEAD `--orphan`
  inference. **[V]**

Exact introduction versions between those brackets were not pinned. If the app is to run against
whatever `git` a user has, the floor it requires needs deciding — and if it shells out, it must
tolerate the absence of the newer flags. **[I]**

---

## 8. What is still open (for #12, not answered here)

1. Whether ignored files count as dirty — the sharpest asymmetry in §4.2, with no comfortable answer.
2. Whether to inherit git's dirty rule by calling `git worktree remove` and reading its refusal, or
   to run an independent check first (and thereby be able to *say what* is dirty, and to neutralise
   the §4.4 config traps).
3. Whether `--assume-unchanged`/`--skip-worktree` and `status.showUntrackedFiles` are threats worth
   defending against, or acceptable user-shot-own-foot.
4. Branch naming and collision policy — given that bare `add` silently reuses an existing branch
   (§3.1) and every held branch is locked out of the user's main checkout (§6.1).
5. Worktree placement, given §6.2.
6. Whether to lock worktrees while a spawn runs (§6.5).
7. Minimum supported `git` version, if shelling out (§7).
8. Not investigated here: `gix` (gitoxide) as a third option, and libgit2's behaviour with respect to
   the `assume-unchanged` bit.

---

## Sources

Git documentation and source (fetched 2026-08-03):

- git-worktree(1) — <https://git-scm.com/docs/git-worktree> · source:
  <https://raw.githubusercontent.com/git/git/master/Documentation/git-worktree.adoc>
- git-worktree(1) at v2.16.0 —
  <https://raw.githubusercontent.com/git/git/v2.16.0/Documentation/git-worktree.txt>
- git-worktree(1) at v2.35.0 —
  <https://raw.githubusercontent.com/git/git/v2.35.0/Documentation/git-worktree.txt>
- git-status(1) — <https://git-scm.com/docs/git-status> · source:
  <https://raw.githubusercontent.com/git/git/master/Documentation/git-status.adoc>
- gitrepository-layout(5) —
  <https://raw.githubusercontent.com/git/git/master/Documentation/gitrepository-layout.adoc>
- `gc.worktreePruneExpire` —
  <https://raw.githubusercontent.com/git/git/master/Documentation/config/gc.adoc>
- `builtin/worktree.c` (the `remove` / `check_clean_worktree` implementation) —
  <https://raw.githubusercontent.com/git/git/master/builtin/worktree.c>

libgit2:

- `include/git2/worktree.h` —
  <https://raw.githubusercontent.com/libgit2/libgit2/main/include/git2/worktree.h>
- `src/libgit2/worktree.c` —
  <https://raw.githubusercontent.com/libgit2/libgit2/main/src/libgit2/worktree.c>
- `include/git2/status.h` (`GIT_STATUS_OPT_DEFAULTS`, `GIT_STATUS_OPTIONS_INIT`) —
  <https://raw.githubusercontent.com/libgit2/libgit2/main/include/git2/status.h>
- `src/libgit2/status.c` —
  <https://raw.githubusercontent.com/libgit2/libgit2/main/src/libgit2/status.c>

git2-rs:

- `src/worktree.rs` — <https://raw.githubusercontent.com/rust-lang/git2-rs/master/src/worktree.rs>
- `src/status.rs` — <https://raw.githubusercontent.com/rust-lang/git2-rs/master/src/status.rs>
- `src/repo.rs` — <https://raw.githubusercontent.com/rust-lang/git2-rs/master/src/repo.rs>
- `CHANGELOG.md` (git2 ↔ libgit2 version mapping) —
  <https://raw.githubusercontent.com/rust-lang/git2-rs/master/CHANGELOG.md>

Experiments: all **[V]** claims were run against throwaway repositories on `git version 2.43.0`,
Linux, on 2026-08-03.
