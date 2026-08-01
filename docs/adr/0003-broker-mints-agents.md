# All agents are minted by a broker, and roles declare what they may spawn

A confined process cannot create a less confined one, so an agent cannot spawn a
sibling under a different policy — something living outside every boundary has
to do it. That component is the broker. Because the ability to spawn is the one
capability that can undo all the others, a role names the specific roles it is
permitted to spawn rather than holding a general spawn power.

## Consequences

- The broker owns every agent's pipes, and is therefore a single point of
  failure: if it dies, the team dies. This is accepted rather than overlooked.
  Agents are re-spawned with `--resume` from the transcripts their harness
  persists, which restores the conversation but not a turn that was in flight.
- Spawn authority is enforced twice: by the broker, and by the harness itself
  through per-tool allowlisting of the broker's MCP tools.
