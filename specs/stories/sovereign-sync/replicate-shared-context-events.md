---
id: replicate-shared-context-events
type: story
refs:
  journey: specs/journeys/replicate-shared-context-through-rendezvous.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [1, 2, 3, 4, 5]
---

# Story: Replicate shared-context events

## Narrative

As a sovereign node operator,
I want a rendezvous to custody only the events in a shared context,
So that participants retain independent authority while browsing one durable relationship history.

## Acceptance Criteria

- [ ] The shared context has one stable NIP-29 `h` identifier.
- [ ] The replication stream ID remains distinct from the context ID.
- [ ] The stream selection includes context-tagged events and required metadata.
- [ ] The rendezvous preserves exact signed event envelopes.
- [ ] Only an authorized transport principal can drain the exported stream.
- [ ] The destination cursor advances only after checkpoint-safe ingest receipts.
- [ ] Replicated events are browsable locally as the same shared context.
- [ ] The rendezvous has custody but cannot author for participants or change their policy.

## Notes

The shared context is the human relationship space. The stream is the delivery
selection used to move that space between custody boundaries.
