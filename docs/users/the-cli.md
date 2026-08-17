# The command line

Everything the spawn form accepts can be given on the command line.
`harness-launcher --help` prints the current version of this reference. The help
text and the app read the same definitions, so they cannot drift apart.

```
harness-launcher
harness-launcher <repository> <work> [options]
                 [--and <repository> <work> [options]]...
```

- `<repository>` — a local git repository, or any directory inside one.
- `<work>` — what the session should do, in your own words. Quote it; it is
  one argument.
- With no arguments, the app opens on a blank form. See
  [the spawn form](the-spawn-form.md).

## `--and`

`--and` separates spawns. Each spawn names its own repository and carries its
own options:

```
harness-launcher ~/code/api "add rate-limit headers" --model sonnet \
                 --and ~/code/web "fix the flaky login test" --level max
```

A separator is required because bare pairs are ambiguous: an unquoted
description would parse as a second spawn, and the app refuses to guess. A
dangling `--and`, or one before the first spawn, is an error.

## Options, per spawn

| Option | Choices | Default |
| --- | --- | --- |
| `--model <id>` | `opus` (the most capable) · `sonnet` (balanced) · `haiku` (the fastest) | `opus` |
| `--level <id>` | `low` · `medium` · `high` · `xhigh` · `max` | `high` |
| `-h`, `--help` | prints usage and the current choice lists | |

An unknown id is refused, and the error lists the valid ids. An incomplete
invocation — a repository with no work, an option with no value — is an
error, not a prompt.

## What happens next

For each spawn, the app:

1. Resolves the repository's default branch from `origin/HEAD`, without
   network access.
2. Creates a worktree on a fresh branch under an app-owned root.
3. Starts the session in the worktree.

Fifteen or twenty spawns at once is normal use. If any repository cannot be
resolved, the app refuses on the shell before taking the screen.
