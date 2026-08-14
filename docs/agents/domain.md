# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`docs/developers/README.md`** — the map: one table saying which document answers which question.
- **The component documents** under `docs/developers/components/` that touch the area you're about to work in — nine of them, one per component.
- **`docs/developers/glossary.md`** — the domain vocabulary.

The scenarios under `docs/developers/scenarios/` walk the system end to end — the happy-path spine and every failure path — and are the fastest way to learn what order things happen in.

## What is history, and what is living

- `docs/developers/` and `docs/users/` are **living** — kept true as the code moves.
- `docs/tranches/` and `docs/evidence/` are **frozen records** — read them as history, never update them to match the code, and never append decision notes to them.
- There are **no decision records** — no `CONTEXT.md`, no `docs/adr/`, and none should be created. Why a thing was done the way it was lives in the issue and pull request that did it, and in the code.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `docs/developers/glossary.md`. Don't drift to synonyms: a spawn is never a "task", retiring is never "deleting", `unknown` is never "errored".

If the concept you need isn't in the glossary yet, that's a signal — either you're inventing language the project doesn't use (reconsider) or there's a real gap: add the term to the glossary in the same piece of work that introduces it.

## Keep the living docs true

If your change alters behaviour a document under `docs/developers/` or `docs/users/` describes, updating that document is part of the change — a living doc that quietly lags the code is worse than none.
