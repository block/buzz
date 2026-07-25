---
id: prove-cloudflare-portability
type: story
refs:
  journey: specs/journeys/promote-portable-relay-to-cloudflare.md
  persona: specs/personas/local-first-builder.md
  steps: [1, 2, 3, 4, 5, 6, 7]
---

# Story: Prove portable relay behavior on Cloudflare

## Narrative

As a local-first builder,
I want a stable relay node to run on Cloudflare with the same portable behavior,
So that I can promote shared memory beyond my laptop without changing its
events, clients, or meaning.

## Acceptance Criteria

- [ ] One normalized stable node key always selects the same durable state
  boundary.
- [ ] Different stable node keys cannot observe or mutate one another's relay
  history.
- [ ] The Cloudflare adapter accepts the same signed-event envelope and exposes
  the same HTTP and WebSocket operations as the laptop adapter.
- [ ] A successful durable acknowledgement is not observable until the event
  can be recovered after Durable Object eviction.
- [ ] Eviction, hibernation, and code deployment do not change the effective
  event set reconstructed from accepted durable history.
- [ ] Historical WebSocket events precede `EOSE`, and matching live events
  continue after hibernation until `CLOSE` or disconnect.
- [ ] Unsupported portable extensions fail explicitly rather than returning
  broader or incomplete results.
- [ ] The shared conformance vectors pass both inside the local Workers runtime
  and against a deployed preview node.
- [ ] Adapter evidence identifies the environment and capability version
  without claiming production availability, scale, or full hosted Buzz parity.

## Notes

The first slice claims only `portable-relay-core-v0.1`. Identity,
replication, and committed-event effects may be added only as separately tested
profiles. The Cloudflare capability reuses the portable relay contracts; it
does not introduce Cloudflare-specific event kinds or client endpoints.

## Out of Scope

- Production SLOs, multi-region disaster recovery, or capacity certification.
- NIP-29 membership, semantic search, media, git hosting, huddles, and
  administrative APIs.
- D1, R2, Queues, Workflows, or Workers AI unless a later profile requires
  their distinct semantics.
- Automatic migration of an existing laptop journal.
- Automatic peer discovery or continuous replication transport.
- Treating local Workers emulation as sufficient evidence of deployed
  conformance.
