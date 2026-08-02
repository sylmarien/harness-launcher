# Roles select a policy from a closed set rather than authoring one

A role does not compose its own confinement out of path grants and domain
rules. It names one of a fixed set of policies — `read-only`, `read-write` and
so on — and naming one that does not exist is an error rather than a fallback
to something permissive. The set is closed so that every policy in it is one we
have implemented and can test, which removes the entire class of bug where a
hand-written set of path globs and domain wildcards silently widens a boundary.

## Consequences

- Attachments carry no access mode. What an agent may do in a workspace follows
  from its role's policy.
- Security is decoupled from the parts of a role that are cheap to change.
  Model, effort and harness can be adjusted freely; the policy is a single
  reviewable value, fixed for the agent's life.
- Adding expressiveness means adding a policy to the set and implementing it,
  not letting a user assemble one.
