# Confine agents at the OS boundary, not with harness permissions

Claude Code's sandbox covers only Bash subprocesses — `Read`, `Edit`, `Write`
and `WebFetch` bypass it entirely and are governed by in-process permission
rules instead — so a policy expressed purely in harness settings cannot make a
role genuinely read-only or genuinely offline. Every harness also expresses such
policy differently. We therefore wrap each agent in its own mount namespace with
bubblewrap, a seccomp filter, and an egress gateway, so a role's limits hold
regardless of whether its harness cooperates.

## Consequences

- `bubblewrap` and `socat` become hard runtime dependencies. Their absence must
  fail loudly rather than silently degrading to an unconfined run.
- Linux only, for now.
- `$HOME` is not mounted, so each harness needs a deliberately curated view of
  whatever configuration it requires. For Claude Code this rests on
  `CLAUDE_CONFIG_DIR`, which exists in the binary but is absent from the
  published documentation and could change without notice.
