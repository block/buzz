---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#59
decided_in: launchpad-26/buzz#59
supersedes: none
---

# ADR-0010 — Should upstream-intelligence reports flag knowledge pages for refresh

## Decision

**Yes.** Upstream-intelligence reports (#3) may flag handbook pages for refresh, and they
do it by reading **the published page index** — the machine-readable enumeration #11
emits alongside the handbook site, listing every page's sources, refs, and pinned
commits.

A flag names the handbook page that may need review — its own path or title within the
handbook site. It does **not** reproduce that page's index entry (the source repo, the
cited file path within it, or the pinned commit) in the flag text itself. See Security
implications for why this boundary matters even though the seam itself is sound.

## Context

**No dependency runs from the handbook to #3.** The index is emitted whether or not #3 is
ever built — the producer shipped first and does not wait on the consumer arriving.
That's deliberate: the handbook doesn't wait on upstream-intelligence, and
upstream-intelligence doesn't need to parse the corpus, understand MkDocs, or know
anything about how pages are written. It reads one file and answers "does this upstream
change affect our documentation?"

**#11 stays authoritative for staleness.** The weekly job that compares each pin against
its tracked ref already exists, already files a report, and remains the mechanism of
record. #3 flagging a page is an *additional* signal, not a replacement — two systems
independently claiming to know which pages are stale is how they end up disagreeing.

**Deduplicated, deliberately.** If both #3 and the staleness job notice the same moved
file, the index is what lets them agree it's one page to look at, not two unrelated
reports.

**What this decision does not do.** It does not make a flagged page a *wrong* page.
Detection is mechanical; triage is judgement. A flag means "the files this page cites
have moved" — a prompt to look, not a verdict — and clearing one means updating the pin
and the verification date in the same change. Nothing here can prove the person read the
diff.

## Consequences

**Good.** One shared identity source (the index) lets two independent systems agree on
what a "stale page" is without either reimplementing the other's fail-closed ancestor
check — the exact duplication risk the issue as filed warned against. The handbook's own
build and publish pipeline is entirely unaffected either way; this is additive to #3, not
a new obligation on #11.

**Bad, stated honestly.** The index maps handbook pages to source repos, refs, and pinned
commits — two of #11's five source repos (`launchpad-26/launchpad`, `launchpad-26/skills`)
are private. That's fine while the mapping stays inside the org-restricted handbook site.
It stops being fine the moment a flag built from it reproduces those fields somewhere
with different membership — which is exactly what #3's own design already allows: its
reports can additionally reach Discord as an out-of-band fallback (#3's Ruling 6).

## Security implications

**The boundary is what to repeat, not whether to read.** Reading the index is safe — it's
already published for exactly this purpose. The risk is downstream: a flag that says
"page X may need review" is safe in any channel. A flag that reproduces the index entry
behind it — "page X cites `launchpad-26/skills` at commit `abc123`" — moves private-repo
structure into a channel that was never restricted to see it, even though naming the page
itself was never the problem. The rule in this ADR's Decision section exists specifically
to keep that from happening as an unnoticed side effect of an otherwise sound design.

Second, restated from #3's own security section: synthesis sends content to an LLM
provider on the reasoning that all of it is public. Feeding raw index entries (rather than
just a flagged page name) into that synthesis step would weaken that reasoning for the
same underlying reason the Discord boundary matters — the handbook's source set is not
all public, even though the index itself is org-restricted.

## Provenance

The seam decision — reading the index, staying one-way, #11 remaining authoritative, and
deduplicating by identity — was taken by the repository owner (@serina-mcfall) on
2026-08-10 while working the open questions on #4, and recorded as a comment on #59 on
2026-08-11, predating this ADR issue and this document. This ADR was raised retrospectively
to hold a decision that already existed, per the same pattern already used for
[ADR-0005](./ADR-0005-launchpad-deployment-boundary.md).

The audience-boundary rule (what a flag may repeat from the index) was decided directly in
conversation with the repository owner on 2026-08-15, following an initial recommendation
that mistakenly proposed a different, lower-coupling seam ("link to #11's issue only")
without first checking whether #59 already carried a decided answer — it did, and the
seam recommendation was withdrawn once found. The boundary rule itself was new discussion,
not a restatement of the 2026-08-11 comment, and was confirmed as a follow-up on #59
before this document was written.

Not verified independently in this document: the exact shape #3's report will use to
render a flag (no upstream-intelligence code exists yet) — this ADR constrains what that
rendering may contain, not how it is built.
