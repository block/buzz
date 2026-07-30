---
id: close-verified-handoff
type: story
refs:
  journey: specs/journeys/return-and-close-delegated-work.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [4, 5, 6]
---

# Story: Close a verified handoff

## Narrative

As the delegating owner,
I want closure to be possible only over the exact, newest, verified return,
So that a terminal state can never suppress later evidence or be minted by
an unauthorized party.

## Acceptance Criteria

- [ ] The close links the open with `e/root`, the verified return with `e/return`, and restates `return_id` in content.
- [ ] The close posts after the return it verifies.
- [ ] The close is signed directly by the opener's effective owner, or by a signer carrying a cryptographically valid attestation from that owner.
- [ ] Only a close for the newest valid return produces `CLOSED`.
- [ ] A close pinning an earlier return does not suppress a later return.
- [ ] A legacy close identifying only the open is not causally valid and does not suppress steward findings.
- [ ] After closure, a mandated steward's next reduction of the same event union reports the handoff closed.

## Notes

v0.1 verification is single-party: the owner who defined acceptance also
judges it. Acceptance contracts that reference executable checks at the
anchored commit reduce closure to recomputation, which is the v0.2 direction
for multi-node trust.
