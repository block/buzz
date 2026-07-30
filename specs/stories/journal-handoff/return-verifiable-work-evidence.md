---
id: return-verifiable-work-evidence
type: story
refs:
  journey: specs/journeys/return-and-close-delegated-work.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [1, 2, 3]
---

# Story: Return verifiable work evidence

## Narrative

As the exact claimant of a handoff,
I want my return to bind outcome, claim, and artifact custody together,
So that any independent node can verify what I delivered without trusting
my account of it.

## Acceptance Criteria

- [ ] The return links the open with `e/root`, the accepted claim with `e/claim`, and restates `claim_id` in content.
- [ ] The return is signed by the exact cryptographic claimant — not another key belonging to the same owner.
- [ ] The return posts strictly after the claim and carries status `done` or `failed`.
- [ ] Before a return advertises an `x` tag: the manifest referencing the hash exists at the rendezvous, the bytes are readable through the authenticated artifact path, and the fetched bytes hash to the advertised SHA-256.
- [ ] `buzz-ctx handoff verify-artifacts <return-event-id>` succeeds from a node other than the returner's: the return's content artifact list equals its `x` tags, and every blob fetched through the verifier's own reader identity byte-matches its digest.
- [ ] Evidence names exact commits, not branch names.

## Notes

Custody verification is symmetric and third-party: it requires no cooperation
from the claimant beyond the durable artifacts themselves.
