# Vendored skills

These skills are vendored from [mattpocock/skills](https://github.com/mattpocock/skills)
and are licensed MIT — see [`UPSTREAM-LICENSE`](./UPSTREAM-LICENSE).

| | |
| --- | --- |
| Upstream | https://github.com/mattpocock/skills |
| Commit | `2ab958093e83e0ec752e6c1c5932da465bf23e0c` (2026-07-28) |
| Plugin version | 1.2.0 |
| Vendored | 2026-08-01 |

The 22 skills here are exactly the set listed in the upstream `.claude-plugin/plugin.json`
(the maintainer's curated set). The `deprecated/`, `in-progress/`, `misc/` and `personal/`
trees were not vendored.

Directories are flattened: upstream `skills/<category>/<name>/` becomes `<name>/`, which
is the layout Claude Code discovers. Skill contents are otherwise byte-for-byte upstream,
including the `agents/openai.yaml` files that let other harnesses (Codex) pick them up.

## Setup

Run `/setup-matt-pocock-skills` once for this repo. It asks which issue tracker to use,
what triage labels you apply, and where to save generated docs — then writes that config
so the other engineering skills know how this project works.

## Updating

Vendored files are yours to edit; nothing updates automatically. To pull upstream changes,
re-copy the skill directories from a fresh clone and update the commit SHA above, or use
`npx skills@latest add mattpocock/skills`.

Note that upstream also ships as a Claude Code plugin
(`claude plugins install mattpocock-skills`). Don't do both — you would get every skill
twice.

## The skills

`[user]` skills only run when you type them (`/grill-me`). `[auto]` skills can also be
reached for by the agent when a task fits.

### Engineering

- `[user]` **ask-matt** — routes you to the right skill or flow for your situation
- `[user]` **grill-with-docs** — grilling session that also builds `CONTEXT.md` and ADRs
- `[user]` **triage** — move issues through a state machine of triage roles
- `[user]` **improve-codebase-architecture** — scan for deepening opportunities, report, then grill
- `[user]` **setup-matt-pocock-skills** — configure this repo for the engineering skills
- `[user]` **to-spec** — turn the conversation into a spec on the issue tracker
- `[user]` **to-tickets** — break a plan into tracer-bullet tickets with blocking edges
- `[user]` **implement** — build from a spec/tickets, driving `/tdd` and `/code-review`
- `[user]` **wayfinder** — map out work too big for one session as investigation tickets
- `[auto]` **prototype** — throwaway prototype to answer a design question
- `[auto]` **diagnosing-bugs** — reproduce → minimise → hypothesise → instrument → fix
- `[auto]` **research** — investigate against primary sources, capture cited findings
- `[auto]` **tdd** — red-green-refactor, one vertical slice at a time
- `[auto]` **domain-modeling** — sharpen the domain model, update `CONTEXT.md` and ADRs
- `[auto]` **codebase-design** — discipline and vocabulary for designing deep modules
- `[auto]` **code-review** — two-axis review of the diff: standards and spec
- `[auto]` **resolving-merge-conflicts** — resolve conflict hunks by intent, never `--abort`

### Productivity

- `[user]` **grill-me** — get interviewed about a plan until every branch is resolved
- `[user]` **handoff** — compact the conversation into a handoff document
- `[user]` **teach** — teach a concept over multiple sessions in a stateful workspace
- `[user]` **writing-great-skills** — reference for writing and editing skills well
- `[auto]` **grilling** — the reusable interview loop behind the two grill skills
