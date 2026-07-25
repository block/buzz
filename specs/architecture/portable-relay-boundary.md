# Portable Relay Boundary v0.1

Status: proposed

## Decision

Buzz relay implementations share a behavioral kernel and protocol contract,
not a server framework, async runtime, or storage engine.

The portable boundary is the smallest observable relay behavior that lets a
client move unchanged between a laptop node, a cloud-native node, and the
hosted Buzz relay. An adapter conforms when the same signed events, filters,
and protocol messages produce the same normative outcomes.

This boundary is an event-identity, integrity, and continuity boundary. It is
not, by itself, an authentication, authorization, confidentiality, search,
media, workflow, or distributed-availability boundary. The optional
[`portable-relay-identity-v0.1`](portable-relay-identity-v0.1.md) profile
defines the cryptographic caller and access boundary around the core.

## Architectural shape

```text
NIP-01 / Buzz HTTP clients
              |
       transport adapter
              |
   +----------v-----------+
   | portable relay core  |
   | verify               |
   | classify             |
   | reduce               |
   | match filters        |
   +----------+-----------+
              |
                    declared ports
     +----------+----------+----------+----------+-------------+
     |          |          |          |          |             |
  journal   subscriptions identity  policy   replication   effects
     |          |          |          |       source/sink      |
 runtime-specific adapters and managed services
```

The core decides meaning. Ports express required effects. Adapters decide how
those effects are performed.

## Portable primitives

### Signed event

The immutable NIP-01 event envelope is the unit of event identity and storage,
and the payload carried unchanged across transports. Its event ID and Schnorr
signature must verify before it can alter relay state.

The relay must not rewrite an accepted event when moving it between adapters.
The same event therefore retains the same event ID, author, kind, tags,
content, and signature on every conforming runtime.

### Event classification

Classification is deterministic and depends only on the event:

- regular events are identified by event ID;
- replaceable events are identified by author and kind;
- parameterized replaceable events are identified by author, kind, and `d`
  tag;
- ephemeral kinds `20000..29999` are live-only.

### Effective event reducer

The reducer turns accepted history into the effective queryable set. It owns
duplicate idempotency and NIP-01 replacement ordering. It performs no I/O and
does not decide authorization.

### Filter matcher

The matcher applies NIP-01 filter semantics to effective events. A runtime may
use indexes to find candidates, but indexing must not change match results.

### Relay decision

Every submission produces one normative decision:

- `stored`: a new durable event changed accepted history;
- `duplicate`: the event ID was already accepted;
- `superseded`: a valid replacement candidate lost ordering;
- `ephemeral`: a valid live-only event was accepted;
- `rejected`: the event failed verification or declared policy.

Transport-specific human-readable messages are informative. Conformance relies
on the acceptance boolean, event ID, state transition, and subsequent
observations.

## Ports

### Event journal

The journal is the source of durable relay history.

Required operations:

- append one verified durable event;
- establish a durability barrier before successful acknowledgement;
- replay accepted history after runtime restart;
- preserve append order and every signed event field exactly;
- treat an already accepted event ID idempotently.

The journal may be NDJSON, SQLite, Postgres, or another implementation. Its
storage representation is outside the portable boundary.

### Effective event query

The query port returns and counts effective events matching explicit NIP-01
filters. Results are newest-first with deterministic event-ID tie-breaking.
Unsupported filter extensions must fail explicitly instead of returning
unfiltered results.

### Subscription hub

The subscription port maintains client-selected filter sets, delivers matching
historical events followed by `EOSE`, and then delivers matching accepted live
events until `CLOSE` or disconnect.

Backpressure, connection placement, and hibernation are adapter concerns.
Dropped or lagged delivery must be observable rather than silently presented as
complete history.

### Policy gate

Policy is a separate port around the portable core. A deployment may admit all
loopback callers or enforce NIP-42, NIP-98, membership, scopes, and rate limits.

A valid signature proves authorship and event integrity. It does not grant
permission. A policy denial must happen before journal mutation or live
publication.

The `portable-relay-core-v0.1` profile does not require a particular policy.
Deployments that claim `portable-relay-policy-v0.1` must declare and test their
admission rules.

Cryptographic authentication and read/write principal binding are specified by
[`portable-relay-identity-v0.1`](portable-relay-identity-v0.1.md). Identity
adapters produce an ephemeral authenticated principal; policy adapters decide
what that principal may do. Authentication evidence is never event history.

### Committed-event effects

Observers, audit writers, workflow engines, search indexers, and report
generators consume committed events after the journal durability barrier.
Their failure must not retroactively make an acknowledged journal append
disappear.

Effects must be idempotent by event ID because adapters may provide at-least-once
delivery.

### Replication source and sink

Replication is durable synchronization between independent relay journals. It
is not subscription fan-out, Redis distribution between replicas of one logical
relay, peer discovery, or a grant of destination authority.

The source port returns bounded records in journal order. Each record contains:

- an operator-assigned source stream ID;
- an opaque cursor issued by that source;
- the exact signed event envelope.

Only durable journal events are exportable. Ephemeral events and transient live
signals never enter a replication batch. A cursor is meaningful only for its
issuing source stream; orchestrators persist and return it unchanged rather than
parsing, incrementing, or comparing it.

The source stream ID is a policy label, not a credential. A network transport
must authenticate its peer and bind that identity from trusted configuration;
it must not accept a source ID merely because an incoming record asserts it.

The sink port is disabled by default. For every record it:

1. applies destination policy to the source stream and destination scope;
2. independently verifies the event ID and signature;
3. invokes the destination's normal duplicate, replacement, durability,
   projection, publication, and effect pipeline;
4. returns a receipt bound to the source cursor and event ID.

`stored`, `duplicate`, and `superseded` are terminal checkpoint-safe outcomes.
`rejected` is not checkpoint-safe unless an operator makes and durably records a
separate skip or dead-letter decision. This fail-closed rule prevents a temporary
policy or verification failure from silently creating a permanent gap.

Source acceptance proves neither destination membership nor authorization.
Private-event selection, community mapping, trust relationships, checkpoint
storage, retry scheduling, loop topology, retention, and transport encryption
belong to the replication orchestrator or deployment policy.

The v0.1 port is an application boundary, not a new unauthenticated HTTP route.
An orchestrator may carry records over NIP-01, HTTP, a queue, direct method calls,
or another authenticated transport without changing their semantics.

## Ingest ordering

A conforming durable submission follows this partial order:

```text
decode
  -> authenticate caller and apply preliminary admission, when configured
  -> event ID and signature verification
  -> authorize claims that depend on the verified event, when configured
  -> duplicate / replacement / ephemeral classification
  -> durable journal append and durability barrier
  -> effective-state update
  -> accepted observation and live publication
  -> optional committed-event effects
```

An implementation may combine steps atomically or use an indexed projection,
but it must preserve these invariants:

1. rejected events never mutate durable or live state;
2. durable events are recoverable before acceptance is observable;
3. ephemeral events are never recoverable;
4. live publication never precedes verification and required durable storage;
5. replay produces the same effective set as the original execution.

## Conformance profiles

### `portable-relay-core-v0.1`

Mandatory:

- valid and tampered signed-event decisions;
- regular, replaceable, parameterized replaceable, and ephemeral kinds;
- duplicate idempotency;
- restart recovery for durable events;
- NIP-01 filter query and count semantics;
- WebSocket `EVENT`, `REQ`, `CLOSE`, `OK`, and `EOSE`;
- Buzz HTTP `POST /events`, `POST /query`, and `POST /count`;
- explicit failure for unsupported capabilities.

### `portable-relay-policy-v0.1`

Optional:

- an authenticated actor is bound independently of event claims;
- denied operations do not mutate the journal or publish live events;
- admission behavior is consistent across HTTP and WebSocket transports.

### `portable-relay-identity-v0.1`

Optional:

- fresh, audience-bound evidence produces an ephemeral authenticated principal;
- event authorship, caller identity, transport provenance, and authorization
  remain distinct;
- direct append binds the caller to the event author, a verified scoped
  delegation, or a declared kind-specific privacy-envelope rule;
- relay peers are cryptographically bound to destination-configured source
  streams;
- denied identity and access decisions have no durable or live effects;
- query, count, historical subscriptions, and live delivery enforce equivalent
  per-event disclosure;
- stable relay principals survive authorized verification-key rotation.

### `portable-relay-effects-v0.1`

Optional:

- committed events can trigger idempotent asynchronous observers;
- retry does not duplicate durable domain outcomes;
- effect lag and failure are observable.

### `portable-relay-replication-v0.1`

Optional:

- durable records retain exact signed envelopes and source journal order;
- opaque cursors resume after a source restart;
- ephemeral events are never exported;
- destinations deny replication until a source is explicitly admitted;
- source acceptance never bypasses destination verification or policy;
- replay is idempotent at the destination;
- checkpoint-safe receipts distinguish terminal outcomes from rejections.

### `portable-relay-cloudflare-v0.1`

Adapter-specific:

- the adapter also conforms to `portable-relay-core-v0.1`;
- one normalized stable node key deterministically selects one isolated
  SQLite-backed Durable Object;
- durable acknowledgements survive object eviction and compatible deployment;
- hibernatable WebSockets preserve subscription meaning and
  historical-to-live ordering;
- local Workers-runtime and deployed-preview evidence are reported separately;
- Cloudflare deployment does not imply identity, replication, effects, or
  production-readiness claims.

## Adapter map

| Concern | Laptop reference | Cloud-native target | Hosted Buzz |
| --- | --- | --- | --- |
| Transport | Axum HTTP/WebSocket | Worker ingress/WebSocket | Axum HTTP/WebSocket |
| Journal | append-only NDJSON | per-node Durable Object SQLite | Postgres |
| Effective query | in-memory replay | object-local SQL projection | Postgres queries |
| Live subscriptions | Tokio broadcast | hibernatable Durable Object WebSockets | connection registry + Redis |
| Identity | opt-in NIP-42/NIP-98 + configured node binding | phase-two NIP-42/NIP-98 + durable replay state | NIP-42/NIP-98 + `buzz-auth` |
| Policy | caller-author + result disclosure policy | explicit object-local policy | `buzz-auth` + membership |
| Effects | in-process or absent | deferred Queue/Workflow consumers | audit/search/workflow subsystems |
| Replication | NDJSON cursor + source allowlist | deferred durable-state cursor + edge policy | not yet implemented |
| Portable archive | NDJSON copy | deferred R2 snapshot | event export |

These mappings are informative. Conformance is judged only at the protocol and
behavioral boundary.

## Reference implementation alignment

The Rust reference implementation contains the boundary in two layers:

- `buzz-core` owns I/O-free event verification, event classification,
  effective-state reduction, filter matching, and replication port types;
- `buzz-local-relay` owns NDJSON persistence, Axum transport, and Tokio live
  fan-out, and implements the laptop replication source/sink adapters.

The laptop adapter runs the shared signed-event vector through its public HTTP
and WebSocket surfaces in `crates/buzz-local-relay/tests/portable_conformance.rs`.
A second adapter should consume the same vector and preserve the same observable
outcomes. Runtime dependencies, including Cloudflare APIs, must remain outside
the portable layer.

The reference replication adapter is deliberately transport-neutral. It proves
cursor resume and policy-gated destination ingest without exposing a public peer
endpoint or implying automatic federation.

The laptop adapter implements the identity profile as an opt-in secured mode.
It uses NIP-42 for WebSocket sessions, payload-bound NIP-98 for HTTP requests,
in-memory replay protection, caller-author binding, result-level read policy,
and destination-configured relay peer bindings. Delegation, persistent replay
state, dynamic DID resolution, and NIP-29 membership remain promotion
boundaries. Hosted Buzz provides relevant NIP-42, NIP-98, NIP-OA, scope,
membership, and result-level read mechanisms, but has not yet been measured
against the portable identity conformance vector.

The Cloudflare reference adapter is specified by
[`portable-relay-cloudflare-v0.1`](portable-relay-cloudflare-v0.1.md) but is not
yet implemented. Its first claim is the portable core only: stateless Worker
ingress routes one stable relay node/community to one SQLite-backed Durable
Object, with hibernatable WebSockets and both local-runtime and deployed-preview
conformance evidence. Identity, replication, effects, and production readiness
remain separate later claims.

The OpenAPI paths and NIP-01 frames are normative. Listener addresses, host
names, TLS termination, authentication headers, storage schemas, and operational
health metadata remain adapter-specific.

## Evolution rules

- Additive protocol behavior may extend v0.1 behind a named capability.
- A change to a mandatory decision, ordering invariant, or observable wire
  shape requires a new boundary version.
- Every adapter must run the same signed conformance vectors.
- Platform-specific optimizations must remain behind ports.
- New identity, coherence, and agent capabilities should first be expressed as
  signed event vocabularies, then attached through policy or committed-event
  effects.

## Traceability

- Telos: [`../TELOS.md`](../TELOS.md)
- Story:
  [`../stories/local-relay/run-without-hosted-infrastructure.md`](../stories/local-relay/run-without-hosted-infrastructure.md)
- Model:
  [`../models/portable-relay/portable-relay-boundary.model.yaml`](../models/portable-relay/portable-relay-boundary.model.yaml)
- Behavior:
  [`../features/portable-relay/adapter-conformance.feature`](../features/portable-relay/adapter-conformance.feature)
- Identity profile:
  [`portable-relay-identity-v0.1.md`](portable-relay-identity-v0.1.md)
- Cloudflare adapter:
  [`portable-relay-cloudflare-v0.1.md`](portable-relay-cloudflare-v0.1.md)
- Cloudflare behavior:
  [`../features/portable-relay/cloudflare-conformance.feature`](../features/portable-relay/cloudflare-conformance.feature)
- Identity behavior:
  [`../features/portable-relay/identity-conformance.feature`](../features/portable-relay/identity-conformance.feature)
- HTTP contract:
  [`../contracts/openapi/local-relay.yaml`](../contracts/openapi/local-relay.yaml)
- WebSocket contract:
  [`../contracts/asyncapi/local-relay.yaml`](../contracts/asyncapi/local-relay.yaml)
