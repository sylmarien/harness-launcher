# harness-launcher

## Agent skills

### Issue tracker

Issues and PRDs live as GitHub issues on `sylmarien/harness-launcher`, driven by the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles are used as-is: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context — when domain docs are written, they go to one `CONTEXT.md` and `docs/adr/` at the repo root. Neither exists yet; `/domain-modeling` creates them lazily. See `docs/agents/domain.md`.

## Workflow

### Asking the user a question

**`AskUserQuestion` ends the turn.** Asking is the signal that you are done and it is the user's move.

- Never call another tool after it in the same turn.
- Never ask a second question in the same turn.
- **Never re-issue, reword, or replace a question that is already on screen.** The user may be part-way through typing an answer; replacing the prompt destroys what they have written.
- If the user's reply says a question was answered wrongly or approved without them, treat the reply itself as the answer and move on. Do not re-put the question.

### Pull requests

- **Open a PR whenever a piece of work is finished** — don't wait to be asked. "Finished" is defined below.
- **Never merge a PR.** Merging is the user's call, always. This includes auto-merge.
- **On feedback, amend the existing PR** rather than opening a new one.
- **A PR always carries exactly one commit**, unless explicitly instructed otherwise. Fold follow-up work in with `git commit --amend` or a rebase, then `git push --force-with-lease`.
- History rewriting is scoped: **only amend commits you authored, only on the feature branch you created.** Never rewrite history containing someone else's commits, and never force-push to the default branch or a shared branch.

### Definition of "finished"

Work is not finished — and no PR is opened — until **`/code-review` has been run over it** and its findings dealt with. The skill pins a fixed point, then spawns one sub-agent per axis so neither review is written by the agent that wrote the code:

- **Standards** — repo coding standards, plus a baseline of Fowler code smells.
- **Spec** — does the diff match what the originating issue or PRD asked for.

Then:

- Address the findings.
- **Anything left unaddressed is explicitly highlighted to the user** in the report of work done. Never drop a finding silently.

Re-run it before any force-push that **changes behaviour**. Amendments that only apply review or user feedback don't re-trigger it.

### Skills

All 22 skills vendored in `.claude/skills/` appear below. This table is a **map, not a licence**: 13 of them are `disable-model-invocation: true` — the user reaches for those, an agent never invokes them unprompted. Within a row, ▸ marks that boundary: everything after it is user-invoked only.

| Phase              | Skills                                                                       |
| ------------------ | ---------------------------------------------------------------------------- |
| Design exploration | `/grilling`, `/research`, `/prototype` · ▸ `/grill-me`, `/grill-with-docs`   |
| Design             | `/codebase-design`, `/domain-modeling` · ▸ `/improve-codebase-architecture`  |
| Planning           | ▸ `/wayfinder`, `/to-spec`, `/to-tickets`, `/triage`                         |
| Implementation     | `/tdd`, `/resolving-merge-conflicts` · ▸ `/implement`                        |
| Diagnosis          | `/diagnosing-bugs`                                                           |
| Review             | `/code-review`                                                               |
| Meta               | ▸ `/handoff`, `/teach`, `/ask-matt`, `/writing-great-skills`, `/setup-matt-pocock-skills` |

**Not covered by any skill:** the PR ritual above — open on finish, never merge, one commit per PR — is prose only.
