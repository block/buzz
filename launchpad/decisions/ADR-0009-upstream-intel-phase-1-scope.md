---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#49
decided_in: launchpad-26/buzz#49
supersedes: none
---

# ADR-0009 — Which upstream product streams Phase 1 covers

## Decision

Upstream-intelligence Phase 1 (#3) covers **the relay only, with a stated, testable
condition for expanding to a second stream** — not bare relay-only, and not all three
streams from the start. Desktop and mobile are explicitly deferred.

Relay is the only component the cohort operates and deploys; its release cadence (roughly
monthly, three tags in 44 days) won't be drowned out by a shared channel the way desktop's
cadence would (six releases in nine days), and mobile has no clean upstream release
trigger at all — only release-candidate tags, no GitHub Release object. "All three streams
from day one" and "desktop-first because it's easier to detect" are both rejected, for the
reasons already given in the issue as filed: a third of that scope would have no defined
trigger, and ease-of-detection is not the same thing as relevance to what the cohort
actually runs.

## Context

Verified on 2026-08-10 against `block/buzz`: relay has three tags ever, no GitHub Release
object for any of them; desktop published six Releases in nine days; mobile has only
`-rc` tags, no Release object. One finding materially shapes this decision: the relay's
own changelog is not relay-only — `crates/buzz-relay/CHANGELOG.md` already contains
`feat(desktop)` and mobile entries under `relay-v0.2.1`. So scoping to the relay narrows
which release stream triggers a report, but does not by itself bound what content a relay
report might touch — that's a separate reduction problem, not solved by this decision.

## Consequences

**Good.** A narrow Phase 1 makes the acceptance criteria reachable within one milestone,
and gives the reduction/synthesis step a bounded corpus (relay's changelog is roughly a
ninth the size of desktop's). It matches the PRD's own framing that the relay is
"especially important because it is the component we operate," and it covers the
highest-consequence signal first: `relay-v0.2.1`'s changelog alone contains three
access-control fixes in the component the cohort exposes to the public internet.

**Bad, accepted explicitly rather than left implicit:** a desktop or mobile security fix
will not surface through this capability, even though cohort members use those clients
daily. This is a real, named gap — not an oversight — and the contingency below exists
because of it, not instead of stating it.

**Contingency — two distinct triggers, and what happens when either fires:**

*Trigger 1 — planned expansion, once the mechanism is proven.* After 3 relay reports have
been published and read without complaint, evaluate adding desktop as the second stream.
Desktop, not mobile, is the natural next stream: it already publishes proper GitHub
Release objects, so detection is genuinely easier there than it was for relay, which has
none.

*Trigger 2 — reactive, for the accepted gap specifically.* A security-relevant desktop or
mobile release ships upstream and this capability misses it, discovered some other way (a
cohort member happens to read the changelog directly, or a client-side issue actually
causes a problem). If the accepted cost stops being acceptable, that discovery is the
signal, not a scheduled review.

*The fix, once either trigger fires:*
1. Add desktop as a second stream using its own Release-object trigger — no new detection
   mechanism needs inventing, unlike relay's, which had none to start from.
2. Reuse the report format and security-theming already planned for the relay-only report
   (e.g. the "🔐 Security" tagging in #3's own example report) rather than designing a
   second format.
3. Explicitly decide how to avoid the drowning-out problem this ADR rejected for Phase 1 —
   desktop's cadence needs a lower-frequency digest, not the same per-release cadence
   relay gets, or the original reason for deferring it recurs immediately on expansion.

*The safety net until either trigger fires.* There is no automated fallback here — unlike
[ADR-0008](./ADR-0008-security-audit-privilege.md)'s repo-settings check, nobody can
"just go look" at an upstream changelog on a schedule without that being the manual
monitoring this capability exists to remove. The honest floor is an occasional skim of
`CHANGELOG.md` for security-tagged entries whenever someone is already looking at
upstream for another reason — worth naming as a real (if thin) safety net rather than
pretending the gap is covered.

## Security implications

The relay is the cohort's public attack surface, so scoping to it covers the
highest-value security signal first. The accepted exposure is the converse: a client-side
vulnerability fixed upstream would not be surfaced by this capability until one of the two
triggers above fires. That gap is recorded here as accepted, not hidden, per the issue's
own instruction that this ADR "should not be closed as though relay-only meant nothing
else matters."

## Provenance

Decided directly in conversation with the repository owner (@serina-mcfall) on
2026-08-15, following the recommendation and contingency plan both posted as comments on
#49 — the same pattern [ADR-0008](./ADR-0008-security-audit-privilege.md) recorded for a
decision made outside a separate PR review thread. `issue` and `decided_in` both point to
#49 because the decision and its filing issue are the same place.

Not verified independently in this document: whether cohort members in practice read the
desktop and mobile changelogs today (assumed likely but not measured), and whether
upstream intends to start publishing proper Release objects for relay or mobile tags —
either would change the shape of Trigger 1 and neither is knowable from the repository
alone.
