# No credential ever enters an agent's boundary

An agent holding an API credential can spend against the account and can
exfiltrate that credential through any domain its policy allows — and
`api.anthropic.com` must be allowed for the agent to work at all. Agents are
therefore given no credential and pointed at the gateway through
`ANTHROPIC_BASE_URL`; the gateway attaches the real one on the way out.

## Consequences

- The gateway sees every request, so a role's permitted models are *enforced*
  rather than merely suggested. An agent cannot promote itself to a more
  expensive model with `/model`.
- Pointing `ANTHROPIC_BASE_URL` at a non-first-party host disables MCP tool
  search unless `ENABLE_TOOL_SEARCH` is set, and disables Remote Control.
