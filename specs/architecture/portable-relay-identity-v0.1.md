# Portable Relay Identity v0.1

Status: experimental; implemented by the laptop reference adapter

## Decision

`portable-relay-identity-v0.1` is an optional security profile surrounding the
portable relay core. It defines how a runtime turns fresh cryptographic evidence
into an authenticated principal and how that principal is authorized to append,
replicate, query, count, subscribe, and receive events.

The profile deliberately separates four claims:

1. **Event authorship:** the Nostr public key whose signature covers the event.
2. **Caller identity:** the principal that proved control of a key for this
   request, connection, or mutually authenticated channel.
3. **Transport provenance:** the relay node that carried a replicated event.
4. **Authorization:** destination-local policy permitting a particular
   principal, operation, event, and scope.

None of these claims implies another. In particular, a valid event is not an
authorization token, a DID is not a credential, and an authenticated relay peer
does not acquire the authority of the event authors it transports.

## Architectural shape

```text
authentication evidence
          |
   +------v-------+
   | authenticator|-----> authenticated principal
   +--------------+                 |
                                    v
signed event + append context -> append authorizer -> portable ingest
filter + read context          -> read authorizer   -> query/subscription

replication evidence + declared source
          |
   +------v----------------+
   | replication peer bind |-----> authenticated peer binding
   +-----------------------+                 |
                                             v
                                 replication sink + destination policy
```

Authentication adapters understand proof formats. Authorization adapters
understand deployment policy. The event kernel understands neither.

## Ubiquitous language

### Event author

The event author is the Nostr public key in `event.pubkey`. The event ID and
Schnorr signature prove that this key signed the exact kind, timestamp, tags,
and content. Adapters never rewrite the author when storing, serving, or
replicating an event.

### Principal

A principal is a security subject recognized by local policy:

- a **Nostr principal**, identified by a Nostr public key;
- a **relay node principal**, identified by a stable URI such as a DID;
- a **system principal**, identified by a deployment-controlled verification
  key and used only for explicitly declared system operations.

An anonymous caller is not an authenticated principal. A deployment may allow
anonymous reads only through an explicit public-read policy.

Principal identifiers are names, not proof. Authentication requires evidence
of current control over an authorized verification method.

### Authentication evidence

Authentication evidence is a fresh, audience-bound proof presented at a
transport boundary. A conforming authenticator verifies:

- proof syntax and cryptographic signature;
- the intended relay or request audience;
- a challenge, nonce, request target, or mutually authenticated channel
  binding;
- freshness and replay constraints required by the proof method;
- that the verification method is currently authorized for the principal.

Supported bindings may include:

- NIP-42 challenge-response for a WebSocket connection;
- NIP-98 request authentication for an HTTP operation;
- a challenge signature or mutually authenticated channel for a relay node.

Authentication events, challenges, bearer material, session secrets, and
private keys are never appended to the relay event journal. An implementation
may keep a separate security audit record that contains no reusable secret.

### Replay-state durability

Every adapter declares the lifetime of its consumed-proof state:

- the laptop adapter uses a process-scoped security epoch; restarting the
  process ends that epoch, and persistent replay state is a later promotion
  boundary;
- a durable adapter persists consumed proof identifiers outside event history
  for at least the authentication evidence freshness window; and
- the Cloudflare identity phase denies replay across Durable Object eviction
  and compatible deployment.

Capability evidence names the replay-state scope it proves. Durable replay
protection is therefore an explicit adapter capability, not an accidental
property of an in-memory implementation.

### Authenticated principal

An authenticated principal is an ephemeral result containing:

- the principal ID and type;
- the proof method;
- the audience to which the proof was bound;
- the proof time or expiry;
- locally resolved grants or restrictions;
- an optional verified delegation chain.

It is scoped to the request, connection, or authenticated peer channel that
created it. It must not be reconstructed from unverified event fields.

### Append context

Every protected append declares exactly one origin:

- `direct`: a person or agent is submitting an event as a client;
- `replication`: an authenticated relay node is transporting an existing signed
  event from a bound source stream;
- `system`: a deployment component is creating a new, explicitly declared
  system event.

For `direct`, the Nostr principal must equal `event.pubkey` unless an adapter
declares and verifies either:

- a scoped delegation mechanism, such as a valid owner attestation; or
- a narrowly scoped privacy-envelope rule, such as NIP-59 gift-wrap author
  indirection.

Delegation conditions must cover the attempted event and must not be inferred
from an unverified tag. A privacy-envelope exception is kind-specific and does
not authorize ordinary mismatched-author events.

For `replication`, the peer principal normally differs from `event.pubkey`.
Peer authentication proves transport provenance only. The destination still
verifies the event and applies its own source, community, kind, membership, and
disclosure policy.

For `system`, every newly created durable event must still be signed by an
authorized event key. A system component must not re-sign or rewrite an
existing authored event.

### Peer binding

A peer binding is destination-controlled trust configuration associating:

- one replication source ID;
- one stable relay node principal;
- the currently authorized verification methods for that principal;
- the destination scopes that source may attempt to populate.

The peer authenticator derives the relay node principal from verified evidence
and compares it with this configuration. A source ID or DID supplied inside the
record is only a claim until this binding succeeds.

A DID Beacon may provide the stable relay node principal, service metadata, and
key-rotation history. DID resolution and method-specific verification remain
adapters. The portable boundary consumes only the verified principal and active
verification method. Rotating a node key must not change the node principal or
invalidate historical event-author signatures.

## Ports

### Authenticator

```text
authenticate(evidence, audience, time) -> authenticated principal | denial
```

The authenticator is transport-aware and policy-neutral. It verifies proof but
does not decide whether the principal may perform the requested domain
operation.

### Append authorizer

```text
authorize_append(principal, append_context, event, destination_scope)
  -> allowed | denied(code)
```

The authorizer receives only authenticated identity context or an explicit
anonymous caller. Denial happens before journal mutation and live publication.

### Read authorizer

```text
authorize_query(principal, operation, filters, destination_scope)
  -> allowed filters | denied(code)

authorize_event_delivery(principal, operation, event, destination_scope)
  -> allowed | denied(code)
```

Read authorization is intentionally two-stage. Request-level authorization
prevents prohibited or dangerously broad queries. Result-level authorization
prevents a known event ID, permissive filter, stale index, or live fan-out path
from bypassing disclosure policy.

`query`, `count`, historical subscription delivery, and live delivery apply the
same result-level rule. Counts include only readable events. A denied candidate
does not reveal its content or existence through a different result shape.

Dynamic policies must be re-evaluated before live delivery or bounded by a
short-lived authorization lease with explicit expiry.

### Replication peer authenticator

```text
authenticate_peer(evidence, audience, source_id, trust_configuration)
  -> peer binding | denial
```

This port binds transport identity to the configured source before any record
reaches the replication sink. It does not replace destination event
verification or authorization.

## Secured append ordering

```text
verify authentication evidence
  -> bind principal and append context
  -> verify event ID and author signature
  -> authorize principal + origin + event + destination scope
  -> classify duplicate / replacement / ephemeral behavior
  -> append durable event and cross durability barrier
  -> update effective state
  -> publish only to authorized live readers
  -> optional non-secret security audit
```

An implementation may reject malformed input before expensive cryptography, but
no rejected operation may mutate durable or live state. Authentication success
alone must never make an invalid event acceptable.

## Secured read ordering

```text
verify authentication evidence when required
  -> authorize operation and requested filters
  -> query candidate effective events
  -> authorize each candidate for this reader
  -> return readable events / readable count
  -> continue result authorization for live delivery
```

Read authorization controls relay disclosure, not payload confidentiality.
Sensitive content must be encrypted to its intended recipients. A conforming
relay preserves ciphertext and its signed envelope exactly.

## Denial semantics

Conformance uses stable denial codes; human-readable transport messages are
informative:

- `authentication_required`
- `invalid_evidence`
- `evidence_expired`
- `audience_mismatch`
- `replay_detected`
- `author_mismatch`
- `delegation_invalid`
- `peer_unbound`
- `source_mismatch`
- `scope_denied`
- `event_disclosure_denied`

Adapters may map these codes to Nostr `OK`/`CLOSED` messages, HTTP status codes,
or local errors. They must not weaken a denial while translating it.

## Security invariants

1. Event authorship, caller identity, transport provenance, and authorization
   remain distinct claims.
2. Authentication evidence is fresh, audience-bound, replay-resistant, and
   never durable event history.
3. Direct append requires author equality, an explicitly verified scoped
   delegation, or a declared kind-specific privacy-envelope rule.
4. Replication source IDs are bound to authenticated relay principals from
   destination-controlled configuration.
5. Destination policy and event verification apply after peer authentication.
6. Authorization denial mutates neither journal nor live state.
7. Every event returned or counted passes result-level read authorization.
8. System-originated durable events have an accountable signing key.
9. A stable node principal may rotate verification keys without rewriting
   historical signed events.
10. Authorization is not confidentiality; sensitive content is encrypted for
    recipients.

## Conformance profile

An adapter claiming `portable-relay-identity-v0.1` must test:

- valid NIP-42 or equivalent challenge evidence binds the signing principal;
- evidence for another audience, expired evidence, and replay fail closed;
- authentication events and secrets never enter the journal;
- matching direct author submission is eligible for policy admission;
- mismatched direct author submission fails without mutation;
- supported delegation is verified, scoped, and fail-closed;
- supported privacy-envelope author indirection is kind-specific and does not
  admit ordinary mismatched-author events;
- a peer proof binds the configured source ID to the expected node principal;
- source spoofing fails before replication ingest;
- peer authentication never bypasses destination event verification or policy;
- query, count, historical subscription, and live delivery apply equivalent
  per-event disclosure rules;
- direct lookup of a protected event does not bypass read authorization;
- active node keys authenticate after rotation and revoked keys do not.

This profile defines an application security boundary, not new public HTTP or
WebSocket routes. Implementations may bind it to existing NIP-01, NIP-42,
NIP-98, local invocation, or mutually authenticated peer transports.

## Explicitly outside v0.1

- a required DID method or global DID resolution network;
- a portable role, membership, or capability policy;
- key generation, custody, recovery, or user-interface flows;
- anonymous credentials and selective-disclosure proofs;
- encryption key distribution;
- relay discovery, replication scheduling, and checkpoint storage;
- an authorization event vocabulary.

## Traceability

- Telos: [`../TELOS.md`](../TELOS.md)
- Story:
  [`../stories/portable-relay/control-attributable-access.md`](../stories/portable-relay/control-attributable-access.md)
- Model:
  [`../models/portable-relay/portable-relay-identity.model.yaml`](../models/portable-relay/portable-relay-identity.model.yaml)
- Behavior:
  [`../features/portable-relay/identity-conformance.feature`](../features/portable-relay/identity-conformance.feature)
- Fixture:
  [`../fixtures/portable-relay/identity-v0.1.json`](../fixtures/portable-relay/identity-v0.1.json)
- Parent boundary:
  [`portable-relay-boundary.md`](portable-relay-boundary.md)
