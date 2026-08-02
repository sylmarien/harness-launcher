# Harness Launcher

A tool for starting and directing teams of autonomous coding agents, where each
agent is confined by an OS-enforced boundary to only what its job requires.

## Language

### Agents

**Harness**:
A vendor CLI that runs an autonomous coding agent, such as Claude Code, Codex,
or Pi.
_Avoid_: backend, provider, driver, tool

**Agent**:
One running harness process under the launcher's control, with its own
identity, workspace and policy.
_Avoid_: session, instance, worker, teammate, sub-agent

**Role**:
A reusable named profile that fixes an agent's harness, model, effort and
policy before it starts. Two agents of the same role differ only in the work
they were given.
_Avoid_: agent type, profile, persona, preset, template

**Capability**:
Something a harness is able to do, such as accepting an effort level or
reporting a permission request. A role that asks for a capability its harness
lacks is invalid.
_Avoid_: feature, support, option

**Lead**:
The agent a human talks to directly. It spawns and directs the other agents.
_Avoid_: orchestrator, main agent, parent, supervisor, coordinator

### Confinement

**Policy**:
One of a fixed set of named, enforceable confinements that a role selects, such
as `read-only` or `read-write`. Naming one that does not exist is an error, so a
role can only ask for confinement that is actually implemented.
_Avoid_: permission mode (already means something else in Claude Code),
permissions, guardrails, rules, config

**Boundary**:
The OS-enforced confinement a policy is realised as. What an agent is *able* to
do, regardless of whether its harness cooperates.
_Avoid_: sandbox, jail, container, isolation

**Grant**:
A single deliberate hole in a boundary — a readable path, a writable path, an
allowed domain.
_Avoid_: allowance, exception, rule, mount

**Broker**:
The component living outside every boundary that agents reach through a narrow
channel. It alone can mint new agents, because a confined process cannot create
a less confined one.
_Avoid_: daemon, server, supervisor, controller

**Gateway**:
The component outside every boundary that all agent traffic passes through. It
holds the credentials no agent is given, and enforces each role's reachable
hosts and permitted models.
_Avoid_: proxy, egress proxy, MITM

### Work

**Workspace**:
The filesystem an agent is given to work in. Has its own lifecycle, independent
of any agent that uses it.
_Avoid_: worktree, checkout, working directory, sandbox dir, project

**Attachment**:
The binding of an agent to a workspace. What the agent may do there follows from
its role's policy, never from the attachment. One workspace may have several.
_Avoid_: mount, assignment, link, binding

**Snapshot**:
A workspace taken from another workspace's committed state, which then evolves
independently. The default way one agent sees another's work.
_Avoid_: copy, fork, clone, branch
