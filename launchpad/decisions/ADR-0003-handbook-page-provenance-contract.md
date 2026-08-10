---
status: Accepted
date: 2026-08-11
issue: launchpad-26/buzz#56
decided_in: launchpad-26/buzz#7
supersedes: none
---

# ADR-0003 — The provenance contract every synthesised page carries

## Decision

Every handbook page carries required frontmatter, obeys the claim rule, and marks origin
per claim.

**Frontmatter:** `title`, `summary`, `category` (one of the eleven navigation slots),
`author`, `sources[]` — each with `repo`, `ref`, a full 40-character `commit`, and `paths` —
plus `reviewed.by`, `reviewed.date`, `runnable`, and `last_verified`.

**The claim rule:**

> A claim about how the system **behaves** needs a source reference.
> A claim about what the cohort **should do** is marked opinion and is attributed to the
> page's `author`, not to a source.
> Nothing is both.

**Origin is a prefix on the claim, not a field on the page** — `[upstream]`, `[launchpad]`,
`[cohort]`, `[supporting]` — because one page may synthesise several repositories, and a
single page-level label would be wrong about half its own content.

**Reference format:** a markdown link to the cited file at the pinned commit, using the full
SHA. Never `blob/main`.

## Context

prd-02 (#4) asked what provenance metadata a synthesised page should carry, and left it
open. Three other pieces of work depend on the answer: the CI provenance gate (#8) checks
the contract, The Professor (#9) produces it, and staleness detection (#11) reads its pins.
Getting it wrong means rewriting all three, which is why #7 built it once, early, and
demonstrated it on a real page rather than describing it in the abstract.

The claim rule exists because the two kinds of statement fail differently. A wrong
behaviour claim is a factual error traceable to a source. A wrong recommendation is
someone's opinion, and attributing it to a repository launders it into apparent fact.

Pinning a full SHA rather than `blob/main` is what makes staleness detection possible at
all — a link to `main` silently changes meaning as `main` moves.

## Consequences

**Good.** Ruling 4 of prd-02 requires that upstream and Launchpad knowledge stay
distinguishable, and per-claim prefixes are the only way to satisfy that on a page drawing
from several repositories. The pinned commits give #11 something mechanical to check. And
separating behaviour from opinion is what makes the corpus safe to retrieve from later:
fact and recommendation stay distinguishable at query time rather than blurring into each
other.

**Bad.** It is a real cost per page. Every behaviour claim needs a source located and
pinned, and authors will feel it — which is part of why #10 caps the first content set
rather than sizing it optimistically. A contract this specific also has to be enforced
mechanically or it decays; that enforcement is #8, and without it the contract is
aspirational.

## Provenance

Decided in #7 ("handbook B — the page contract: frontmatter, claim rule, origin prefixes").
ADR #56 was raised afterwards to give the decision a home on the board. This record
ratifies #7 rather than re-opening it.
