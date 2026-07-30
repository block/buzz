---
id: open-attributable-work-offer
type: story
refs:
  journey: specs/journeys/delegate-work-through-journal-handoff.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [1, 2, 3]
---

# Story: Open an attributable work offer

## Narrative

As a delegating owner,
I want to publish a signed offer that anchors work to one repository state
and names exactly who may claim it,
So that the delegation is durable, bounded, and re-derivable by any party.

## Acceptance Criteria

- [ ] The open is a signed kind-1 event with exactly one lifecycle `t` tag and exactly one `h` context.
- [ ] The open names a canonical lowercase 40-character `base_commit`; abbreviated or mixed-case anchors invalidate the open.
- [ ] Open `p` tags name the only keys allowed to claim; an open with no `p` tags is invalid.
- [ ] A `target.pubkey`, when present, also appears in a `p` tag.
- [ ] The open carries scope, acceptance, and embodiment contracts; contract text that restates repository specifications references them at the anchored commit instead of forking them.
- [ ] The opener's effective owner is the author, or the owner recovered from exactly one cryptographically valid, event-covering NIP-OA `auth` tag — never a JSON label.
- [ ] The opener's effective owner is the only owner identity permitted to later verify and close.

## Notes

An invalid open orphans every claim, return, and close that references it.
v0.1 provides no withdraw or acknowledgment transition, so a mistaken open is
re-reported by stewards until a v0.2 retirement primitive exists.
