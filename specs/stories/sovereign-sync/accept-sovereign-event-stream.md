---
id: accept-sovereign-event-stream
type: story
refs:
  journey: specs/journeys/offer-and-accept-sovereign-event-stream.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [4, 5]
---

# Story: Accept a sovereign event stream

## Narrative

As a sovereign node operator,
I want my acceptance to pin the exact offer I reviewed,
So that later source changes cannot silently expand what my node admits.

## Acceptance Criteria

- [ ] The destination reviews the current export head before accepting.
- [ ] The admit is signed by a destination owner identity named by the export.
- [ ] The admit uses the same stream ID byte-for-byte.
- [ ] The admit `e` tag equals the reviewed export event ID.
- [ ] Admit `p` tags name transport verification keys accepted by the destination.
- [ ] An active admit with no pin is reported as unpinned, not matched.
- [ ] Replacing the export head breaks the match until the destination re-pins.

## Notes

Agreement answers who consented. Transport keys answer who may move records.
Those questions intentionally use different fields and identities.
