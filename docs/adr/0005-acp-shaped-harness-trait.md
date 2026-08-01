# Shape the harness trait around ACP, but implement it natively

Supporting many harnesses one at a time is where most of the code in a system
like this ends up. The Agent Client Protocol already models what we need —
`session/new`, `session/prompt`, `session/request_permission`, `session/cancel`
— so our harness trait borrows its vocabulary, while the first implementation
drives Claude Code through its own stream-json interface and hooks.

Claude Code does not speak ACP. An ACP backend would mean running a third-party
TypeScript adapter inside the boundary, holding our gateway connection, which
defeats the point of the boundary on day one. Shaping the trait this way means
adding that backend later is an implementation rather than a redesign.

## Consequences

- The trait carries capability predicates. A role requesting something its
  harness cannot do — an effort level, say — is invalid when the role is
  defined, not silently ignored at runtime.
