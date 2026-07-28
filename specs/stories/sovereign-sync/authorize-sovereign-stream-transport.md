---
id: authorize-sovereign-stream-transport
type: story
refs:
  journey: specs/journeys/offer-and-accept-sovereign-event-stream.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [6, 7]
---

# Story: Authorize sovereign stream transport

## Narrative

As a sovereign node operator,
I want transport authorization evaluated separately from agreement,
So that cryptographic connectivity never implies policy consent.

## Acceptance Criteria

- [ ] Pull delivery requires an active read grant for the presenting transport key.
- [ ] Destination ingest requires an active admit naming the presenting transport key.
- [ ] Read and admit grants pin the export head when they participate in a matched relationship.
- [ ] Authentication proves current control of an authorized transport key.
- [ ] Transported events retain their original authors, IDs, content, and signatures.
- [ ] The system reports agreement state and transport readiness separately.
- [ ] Revoking a transport key does not rewrite historical event authorship or declaration history.

## Notes

Push-only, pull, and rendezvous-mediated topologies may require different
combinations of read and admit grants, but never different identity semantics.
