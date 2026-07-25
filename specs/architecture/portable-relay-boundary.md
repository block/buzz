# Portable Relay Boundary v0.1

Status: proposed

## Decision

Buzz relay implementations share a behavioral kernel and protocol contract,
not a server framework, async runtime, or storage engine.

The portable boundary is the smallest observable relay behavior that lets a
client move unchanged between a laptop node, a cloud-native node, and the
hosted Buzz relay. An adapter conforms when the same signed events, filters,
and protocol messages produce the same normative outcomes.

This boundary is an identity-integrity and continuity boundary. It is not, by
itself, an authorization, confidentiality, search, media, workflow, or
distributed-availability boundary.

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
     +--------+---------+----------------+
     |                  |                |
 event journal     subscription hub   policy gate
     |                  |                |
 runtime-specific adapters and managed services
```

The core decides meaning. Ports express required effects. Adapters decide how
those effects are performed.

## Portable primitives

### Signed event

The immutable NIP-01 event envelope is the unit of identity, transport, and
storage. Its event ID and Schnorr signature must verify before it can alter
relay state.

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

### Committed-event effects

Observers, audit writers, workflow engines, search indexers, and report
generators consume committed events after the journal durability barrier.
Their failure must not retroactively make an acknowledged journal append
disappear.

Effects must be idempotent by event ID because adapters may provide at-least-once
delivery.

## Ingest ordering

A conforming durable submission follows this partial order:

```text
decode
  -> policy decision, when configured
  -> event ID and signature verification
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

### `portable-relay-effects-v0.1`

Optional:

- committed events can trigger idempotent asynchronous observers;
- retry does not duplicate durable domain outcomes;
- effect lag and failure are observable.

## Adapter map

| Concern | Laptop reference | Cloud-native target | Hosted Buzz |
| --- | --- | --- | --- |
| Transport | Axum HTTP/WebSocket | Worker ingress/WebSocket | Axum HTTP/WebSocket |
| Journal | append-only NDJSON | per-node durable SQLite | Postgres |
| Effective query | in-memory replay | local SQL projection | Postgres queries |
| Live subscriptions | Tokio broadcast | stateful coordination object | connection registry + Redis |
| Policy | trusted loopback | explicit edge policy | `buzz-auth` + membership |
| Effects | in-process or absent | queue/workflow consumers | audit/search/workflow subsystems |
| Portable archive | NDJSON copy | object-storage snapshot | event export |

These mappings are informative. Conformance is judged only at the protocol and
behavioral boundary.

## Reference implementation alignment

The current Rust implementation already contains the boundary in two layers:

- `buzz-core` owns I/O-free event verification and filter matching;
- `buzz-local-relay` owns event classification, effective-state reduction,
  NDJSON persistence, Axum transport, and Tokio live fan-out.

The next implementation refactor should move deterministic classification and
reduction beside the existing `buzz-core` behavior, while keeping the event
journal and subscription hub behind adapter-facing interfaces. It should not
introduce a Cloudflare dependency into the portable layer.

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
- HTTP contract:
  [`../contracts/openapi/local-relay.yaml`](../contracts/openapi/local-relay.yaml)
- WebSocket contract:
  [`../contracts/asyncapi/local-relay.yaml`](../contracts/asyncapi/local-relay.yaml)
