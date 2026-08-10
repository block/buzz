---
status: Accepted
date: 2026-08-11
issue: launchpad-26/buzz#57
decided_in: launchpad-26/buzz#11
supersedes: none
---

# ADR-0004 — How stale synthesised pages are detected

## Decision

Staleness is detected with **git alone — no model, no embeddings**.

Each page pins `repo + ref + commit + paths` (ADR-0003). A scheduled job asks one
mechanical question per source:

```bash
git fetch origin "$ref"                                        # 1. get current state
git merge-base --is-ancestor "$commit" FETCH_HEAD || exit 1    # 2. fail closed
git log --oneline "$commit"..FETCH_HEAD -- $paths              # 3. what moved
```

Non-empty output from step 3 means a file the page was built from has changed.

Output is **one GitHub issue per run** listing flagged pages, plus a published
machine-readable index of `page path → sources → pinned commits`.

**Detection is mechanical; triage is judgement.** The job reports only that a page cites
files that moved. Whether the page is now *wrong* is a separate call made by a human or an
agent.

## Context

prd-02 (#4) asked how stale content should be detected and left it open.

Steps 1 and 2 are not optional, and the reason is a genuine trap: `git log A..B` returns
**empty and exits 0** when `A` is not an ancestor of `B`. It does not error. So a pin taken
on a branch that later diverged reports "nothing changed" while the cited file may have
been rewritten or deleted.

This was reproduced on `launchpad-26/buzz` before the check was designed:
`git log c060e936c..32f0988c8 -- <a file>` prints nothing, yet the file exists at the first
commit and not at the second.

Fetching a named `ref` fixes the undefined comparison target; the ancestor check turns a
silent false negative into a loud failure. **A staleness check that can quietly pass is
worse than none, because it is believed.**

## Consequences

**Good.** No inference cost, no model to drift, and a result that is reproducible by anyone
with a shell. Failing closed means the failure mode is a noisy job rather than a corpus
that silently rots. Separating detection from triage keeps the check cheap and trustworthy;
conflating them would make it both expensive and easy to disbelieve.

**Bad.** It detects *movement*, not *meaning* — a whitespace commit to a cited file flags
the page just as a rewrite does, so some flags will be noise. Clearing a flag is a human
procedure, not a mechanical guarantee: it means updating `sources[].commit` and
`last_verified` in the same PR, and nothing can prove the person read the diff. A blind
bump clears the check too.

**A green staleness check is therefore not evidence that anyone reviewed the upstream
change.** It only means no pin is behind. Treating it as review is the way this control
fails.

## Provenance

Decided in #11 ("handbook F — staleness detection and the published page index"). ADR #57
was raised afterwards to give the decision a home on the board. This record ratifies #11
rather than re-opening it.
