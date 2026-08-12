---
name: orchestrate
description: "Drive a queue of issues to one reviewed pull request, one sub-agent at a time."
disable-model-invocation: true
---

`/orchestrate #36,37,39,41` — implement those issues, **in that order**, on one branch, one commit each, opened as a single pull request that grows as the queue drains.

**Every line of code in the run is written by a sub-agent, and every review of it by a different sub-agent.** Yours is the queue, the briefs, the findings and the ship.

That division rests on one structural fact: **a sub-agent cannot spawn a sub-agent.** So an implementer cannot run `/code-review`, whose entire value is that the author is not the reviewer. Review is yours, and every brief says so — an implementer left to assume will review its own work.

An issue is **finished** — reviewed, fixed, pushed, and green on CI — before the next one is briefed. CI runs against the branch head rather than the commit alone, so what this buys is not proof of each commit in isolation: it is attribution. A red build implicates one issue's work instead of five, and it arrives before anything is built on top of it.

## 1. Gate the queue

Accept `#36,37`, `36 37`, or a mix. The order given is the order run; duplicates collapse.

Resolve every number **before any work starts**, through `docs/agents/issue-tracker.md` — `gh issue view <n>`. A number is good when it resolves to an *issue* that is *open*. Fall back to `gh pr view <n>` on failure: the two share one number space, so a pull-request number would otherwise pass an "exists and is open" check.

When any number is bad — missing, already closed, or a pull request — stop the whole run and ask, naming each bad number and what it turned out to be. Hold the good ones too: a closed issue in the list usually means the queue was copied from a stale note, so the rest of it has earned the same doubt.

**Done when** every number resolves to an open issue, or the user has said what to do about the ones that do not.

## 2. Prepare the branch

One branch carries the whole queue. Use the session's designated branch when it has one; otherwise create `claude/issues-<first>-<last>-<short-random>` from the default branch.

## 3. Run the queue

One issue at a time, in order — implementers share a working tree, so two at once collide.

**Brief an implementer** with everything under [The brief](#the-brief).

**Audit the claims.** An agent's report is a claim until you have run it yourself: confirm exactly one new commit, a clean tree, and the checks the brief demanded.

**Review.** Invoke `/code-review` with the fixed point `<commit>~1`, passing the issue as the spec so it never stops to ask which spec applies. `/code-review` diffs against `HEAD`, and here `HEAD` *is* the commit under review — which is what this ordering buys.

**Fold the findings into that commit.** It is `HEAD`, so this is `git commit --amend`. A finding that lands on an *earlier*, already-green commit takes `git commit --fixup=<sha>` and an autosquash rebase instead, with `/resolving-merge-conflicts` for what that stirs up — putting the fix in the wrong commit would cost the attribution the loop exists for. Real bugs first and test-first, each test confirmed red before its fix; then documented-standard violations; then judgement calls. A finding may be declined **with its reasoning stated** — silence is the one unacceptable outcome. Record every verdict in a backlog file **outside the repository**, in the session scratchpad, keyed by issue: a long run compacts and the pull request has to name what was left undone, while a file inside the tree would dirty the very thing the audit checks.

**Push, and open the pull request on the first issue** — titled for the queue, described from the issue just landed. Later issues push onto the same one, so it grows as the queue drains and is watchable from the first commit. Amending after a push means a force-push with lease: your own commits, your own branch.

**Wait for CI.** `gh pr checks <n> --watch` blocks until the run settles. Confirm the checks belong to the SHA just pushed — a force-push mid-run leaves the previous run still reporting. A repository with no checks configured reports none: say so once and carry on, rather than waiting on a gate that cannot close.

Reproduce a failure locally before fixing it: a tool's version in CI is not always the version here, so a green local check can sit under a red build, and a fix aimed at the wrong cause costs a round trip. Fix, amend, force-push, wait again — and when the fix changes behaviour rather than applying review feedback, re-run `/code-review` over it, which CLAUDE.md requires and which keeps "reviewed" true of the commit that actually ships. Two rounds without green is a thing to put to the user rather than attempt a third time.

**Pass the run forward.** Each implementer's report becomes context for the next brief: what moved, what the last agent measured, and any gotcha a later agent would otherwise rediscover the hard way.

**Done when** the issue's commit is reviewed, its findings carry verdicts, and CI is green on the pushed head. Only then brief the next issue.

## 4. Finish

The pull request is already open, so finishing is bringing its description up to the whole run: what each commit delivers, the bugs review caught, and — named explicitly — every finding left unaddressed and every acceptance criterion only partially met.

**Never merge**; the merge is the user's.

**Done when** every issue in the queue has a green commit and the description accounts for all of them.

## The brief

Every implementer brief carries:

- **The issue in full** — body and acceptance criteria pasted in, plus the spec or design record that governs it.
- **The workflow** — read `.claude/skills/implement/SKILL.md` directly rather than invoking it. `/tdd` is model-invocable and is the route for behaviour changes.
- **Review is yours.** Say so: you run `/code-review` on their commit once they finish.
- **The branch, and that it stays the branch.**
- **Exactly one commit**, matching the repo's commit style and naming the issue, amending their own work in progress so only one lands. The issue reference is what lets `/code-review` find the spec unattended.
- **Pushing and the pull request are yours**, not theirs.
- **The checks that must pass**, reported as real numbers.
- **What the previous agents learned** — measurements, gotchas, and the commits already reviewed and green, which are not theirs to revisit.

## Escalation

A genuine judgement call goes to the user, not into a commit. An acceptance criterion that cannot be met as written, a fix that turns on a product decision, a spec that contradicts the design record — surface it, keep building everything it does not block, and carry it into the pull request if it is still open when you ship.
