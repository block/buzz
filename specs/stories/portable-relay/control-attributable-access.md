# Story: Control attributable access to portable relay memory

As a local-first builder, I want people, agents, and peer relays to prove who
they are independently of the events they carry so that shared memory remains
attributable without granting authority merely because an event is valid.

## Acceptance criteria

- A valid event signature proves event authorship and integrity, not permission.
- Authentication binds a caller to a principal, audience, and fresh proof
  before protected append or read operations.
- NIP-42 WebSocket and NIP-98 HTTP evidence can identify a Nostr principal
  without storing their authentication events in the durable event journal.
- A direct submission is admitted only when the authenticated principal is the
  event author, presents an explicitly supported valid scoped delegation, or
  satisfies a narrowly declared privacy-envelope rule such as NIP-59 gift-wrap
  author indirection.
- A replication peer may transport events written by other authors, but its
  authenticated node identity must be bound to the configured source stream.
- A self-asserted source ID, DID, public key, or event tag is never sufficient
  authentication evidence.
- The destination independently verifies and authorizes every replicated event.
- Read policy is applied both to the requested filter and to every candidate
  event, including direct lookups by a known event ID.
- Query counts, historical subscriptions, and live delivery reveal only events
  the reader is authorized to observe.
- Authentication or authorization denial changes neither durable history nor
  live state.
- Sensitive content uses recipient encryption; relay authorization alone is
  not treated as confidentiality from the relay operator.
- Relay node identity may remain stable while its authorized verification keys
  rotate.

## Out of scope

- Mandating one DID method, DID resolver, wallet, or key-custody system.
- Defining identity proofing for legal names, biometrics, or governments.
- Treating possession of a DID or public key as an authorization grant.
- Defining a complete membership, role, or capability vocabulary.
- Defining replication discovery, topology, transport, or scheduling.
- Persisting authentication challenges, bearer material, or private keys in the
  event journal.
