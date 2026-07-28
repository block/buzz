---
id: offer-sovereign-event-stream
type: story
refs:
  journey: specs/journeys/offer-and-accept-sovereign-event-stream.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [1, 2, 3]
---

# Story: Offer a sovereign event stream

## Narrative

As a sovereign node operator,
I want to publish an attributable offer for an immutable event selection,
So that a named counterparty can assess exactly what I intend to share.

## Acceptance Criteria

- [ ] The offer distinguishes owner identities, node principals, transport keys, and event authors.
- [ ] The stream has one explicit selection: filter, mirror, or upstream source.
- [ ] Changing the selection requires a new stream ID.
- [ ] The export is signed by the source owner identity.
- [ ] Export `p` tags name counterparty owner identities allowed to answer the offer.
- [ ] Publishing an offer grants no transport or read authority by itself.

## Notes

The stream is a selection of signed events. It is not an artifact container and
is not the same identifier as a NIP-29 shared context.
