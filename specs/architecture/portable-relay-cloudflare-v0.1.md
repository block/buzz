# Portable Relay Cloudflare v0.1

Status: specified; implementation not started

## Decision

`portable-relay-cloudflare-v0.1` is the first independent cloud-native adapter
for the [portable relay boundary](portable-relay-boundary.md). It proves that
one stable relay node can move from a laptop process to Cloudflare without
changing signed-event identity, client protocol, durable acceptance semantics,
effective queries, or historical-to-live subscription ordering.

The v0.1 coordination atom is one logical relay node/community. A stateless
Cloudflare Worker normalizes the configured node key and routes every stateful
HTTP or WebSocket operation to exactly one SQLite-backed Durable Object. The
Durable Object owns durable history, effective state, subscriptions, and any
profile-specific security state for that node.

This is a portability and deployment proof, not a production-hosting claim.

## Why this boundary exists

The laptop adapter has now exercised the portable core, replication ports, and
identity profile. A second adapter is needed to reveal assumptions that are
accidentally tied to NDJSON, Axum, Tokio broadcast channels, process lifetime,
or Rust types.

Cloudflare is a useful second runtime because its natural primitives differ
from the laptop while mapping cleanly to the declared ports:

- Worker ingress replaces the Axum listener;
- SQLite-backed Durable Object storage replaces the NDJSON journal;
- Durable Object coordination replaces process-local mutable state;
- hibernatable WebSockets replace always-warm Tokio subscription tasks;
- storage and output gates support the durability-before-acknowledgement
  invariant.

Cloudflare currently recommends designing each Durable Object around an
"atom of coordination," using SQLite-backed objects for new classes, and using
hibernatable WebSockets for long-lived realtime connections. Those platform
recommendations inform this adapter but are not themselves portable relay
semantics.

## Capability claim

The first implementation slice MUST claim:

- `portable-relay-core-v0.1`;
- `portable-relay-cloudflare-v0.1`.

It MUST NOT claim the following until their dedicated scenarios pass:

- `portable-relay-identity-v0.1`;
- `portable-relay-replication-v0.1`;
- `portable-relay-effects-v0.1`;
- production readiness or hosted Buzz parity.

`portable-relay-policy-v0.1` may be declared with an explicit development
policy, but an implicit "Cloudflare is trusted" rule is not a policy claim.

## Logical topology

```text
HTTP / WebSocket client
          |
          v
+--------------------------+
| Cloudflare Worker        |
| edge-terminated traffic  |
| host/node normalization  |
| protocol parsing limits  |
| deterministic DO routing |
+-------------+------------+
              |
              | one stable node key
              v
+--------------------------------------------+
| RelayNode Durable Object                   |
|                                            |
| portable verification / decisions          |
| SQLite journal + effective projection       |
| hibernatable WebSocket subscriptions        |
| optional auth replay + peer checkpoints     |
+--------------------------------------------+
```

The Worker is replaceable ingress. The Durable Object is the authoritative
state and coordination boundary.

## Stable node routing

### Stable node key

A stable node key is an operator-controlled, normalized identifier for one
logical relay/community. It is a routing value, not authentication evidence
and not a Nostr event field.

The adapter MUST:

1. derive it from explicit deployment configuration or the same normalized
   host/community boundary presented to clients;
2. canonicalize case, trailing-dot variants, and scheme-specific default ports
   before routing (`80` only for HTTP/WS and `443` only for HTTPS/WSS), while
   preserving non-default ports;
3. use deterministic named-object resolution so the same key selects the same
   Durable Object across isolates and deployments;
4. reject missing or ambiguous node keys;
5. keep different normalized keys in different Durable Objects.

The adapter MUST NOT accept an arbitrary request body, event tag, replication
record, DID, or source ID as authority to select another node.

### Coordination atom

For v0.1, the stable relay node is the coordination atom. All of its journal
ordering, replacement decisions, query projection, and subscriptions meet in
one Durable Object.

This deliberately prioritizes semantic clarity over horizontal throughput. A
future partitioned profile may split a community or channel only after it
defines ordering, cross-partition query, subscription, and replication
semantics. Such a split is not an invisible optimization of v0.1.

## Port mapping

| Portable concern | Cloudflare v0.1 adapter |
| --- | --- |
| Transport | Worker HTTP ingress and WebSocket upgrade |
| Event journal | Durable Object private SQLite storage |
| Effective query | SQLite projection scoped to the same object |
| Subscription hub | Durable Object hibernatable WebSockets |
| Identity | Deferred; later NIP-42/NIP-98 adapter with durable proof replay state |
| Policy | Explicit adapter policy evaluated inside the node boundary |
| Replication | Deferred; later RPC/HTTP/Queue transport behind existing source/sink ports |
| Effects | Deferred; later Queue or Workflow consumers after committed events |
| Archive | Deferred; later R2 snapshot/export without changing journal semantics |

No D1, R2, KV, Queue, Workflow, or Workers AI dependency is required for the
core v0.1 claim.

## Durable state

SQLite is an adapter representation, not the portable model. The implementation
must nevertheless preserve four logical records:

1. **Journal record** — append sequence plus the exact signed event envelope.
2. **Accepted event identity** — sufficient state to make event-ID replay
   idempotent.
3. **Effective projection** — the current winner for regular, replaceable, and
   parameterized replaceable identities.
4. **Adapter metadata** — schema/capability version and recovery information
   that is never exposed as event history.

When the identity profile is added, consumed proof IDs and current trust
bindings are separate security state. They MUST NOT be inserted into the event
journal. When replication is added, source checkpoints and dead-letter
decisions are also separate operational state.

The adapter may rebuild its effective projection from the journal, but a
projection update and its corresponding durable append must not produce an
observable state that could not be reconstructed after eviction.

## Durable ingest transaction

A durable submission follows the portable ingest order:

```text
decode and bound input
  -> authenticate and apply preliminary policy, if declared
  -> verify event ID and Schnorr signature
  -> authorize verified claims, if declared
  -> classify duplicate / replacement / ephemeral
  -> append durable journal record
  -> update accepted-ID and effective projections
  -> allow the acknowledgement to leave the object
  -> publish matching live observations
  -> enqueue optional committed-event effects
```

For a durable event, journal append, accepted-ID state, and effective projection
must commit atomically or recover to an equivalent state before the caller can
observe success. Cloudflare output gates may enforce the final
storage-before-response barrier, but tests judge only the observable invariant.

Ephemeral events bypass SQLite journal and projection writes. They may be
delivered only to currently matching live subscriptions and are absent after
eviction.

## Queries and counts

The SQLite projection may use indexes to select candidates, but it MUST preserve
the portable filter matcher results:

- filters use NIP-01 OR semantics;
- per-filter limits apply before cross-filter de-duplication;
- output ordering is newest-first with deterministic event-ID tie-breaking;
- counts include only effective events visible to the reader;
- unsupported extensions fail before broad or unfiltered results are returned.

The first slice may deliberately use a simple candidate scan. Index selection
is not a conformance outcome.

## WebSocket lifecycle

The Durable Object owns each WebSocket after upgrade. The v0.1 implementation
uses the hibernation API so an idle object can leave memory without closing
valid connections.

Cloudflare currently limits each serialized attachment to 16,384 bytes. NIP-01
filter sets can exceed that bound, so v0.1 deliberately keeps subscription
filters in object-local SQLite keyed by connection and subscription ID.

The serialized attachment contains only bounded reconstruction metadata:

- an opaque connection ID;
- the stable node key or its non-secret object-local equivalent;
- an authenticated principal identifier and bounded grant summary after the
  identity profile is enabled;
- proof IDs or challenge identifiers that cannot be reused as credentials.

It MUST NOT contain private keys, bearer tokens, reusable authentication
material, decrypted message content, or mutable state that should instead be in
SQLite.

After hibernation or eviction:

- historical state comes from SQLite, not remembered arrays;
- attached subscriptions resume with the same filter meaning;
- a `REQ` still delivers historical `EVENT` frames before `EOSE`;
- later accepted matching events continue until `CLOSE` or disconnect;
- lag, overflow, or unrecoverable attachment errors are observable.

## Identity phase

Identity is phase two, not a prerequisite for the core portability proof. When
enabled, the Cloudflare adapter must consume the existing
`portable-relay-identity-v0.1` vector and preserve its denial codes.

The mapping is:

- NIP-42 challenge response scoped to one WebSocket;
- payload-bound NIP-98 scoped to one HTTP request;
- consumed proof IDs persisted in object-local SQLite with bounded retention;
- direct writes bound to the authenticated author or an explicitly declared
  privacy-envelope/delegation rule;
- query, count, historical subscription, and live delivery subject to the same
  per-event disclosure policy;
- peer keys resolved from destination-controlled trust configuration.

Cloudflare Access headers, Worker routes, or possession of a Durable Object
name do not replace portable cryptographic identity.

## Replication and effects phases

Replication continues to use exact signed envelopes, source-owned opaque
cursors, destination verification, and checkpoint-safe receipts. HTTP, Worker
RPC, Queues, or Workflows may carry the records, but none may change the
application port semantics.

Committed-event effects begin only after durable acceptance. Queue or Workflow
delivery must be idempotent by event ID and expose lag and failure. An effect
failure cannot retract an acknowledged journal event.

Neither phase belongs in the first Cloudflare core implementation.

## Runtime and language boundary

The v0.1 reference adapter should use a small TypeScript Worker/Durable Object
shell because it exercises Cloudflare's native bindings, SQLite API, testing
runtime, and WebSocket hibernation directly.

Workers Web Crypto does not provide Nostr's secp256k1 BIP-340 Schnorr
verification. The TypeScript reference shell therefore uses the repository's
existing `nostr-tools` dependency, backed by `@noble/curves`, pinned through the
workspace lockfile and measured against the same signed vectors as Rust.
Replacing that implementation with Rust/Wasm requires the same conformance
evidence.

Rust and WebAssembly remain valid implementation options. Sharing compiled Rust
code is an optimization, not a conformance requirement. The initial adapter may
reimplement the small deterministic kernel in TypeScript if it consumes the
same fixtures and produces the same normative outcomes. Axum, Tokio, filesystem
APIs, and process-global state must not leak into the Cloudflare adapter.

## Conformance tiers

### Tier 1: deterministic kernel

- shared fixture parsing;
- event ID and signature verification;
- classification and reduction;
- filter matching;
- adapter-independent unit tests.

### Tier 2: local Workers runtime

- real SQLite-backed Durable Object binding;
- HTTP and WebSocket contracts;
- duplicate/replacement/durability behavior;
- forced object eviction with storage preserved through
  `evictDurableObject`;
- hibernatable WebSocket round trips across simulated eviction;
- attachment reconstruction with filters recovered from SQLite.

The implementation pins `@cloudflare/vitest-pool-workers` at `0.16.20` or
newer, where the eviction helpers are available. These tests simulate the
production lifecycle but do not replace Tier 3 evidence from the deployed
runtime.

### Tier 3: deployed preview

- TLS HTTP and WebSocket black-box tests;
- stable node routing across separate requests;
- recovery across a new isolate or deployment;
- exact outcome comparison with the laptop adapter.

The capability is `implemented` only when all three tiers pass. Tier 2 alone
may be reported as `local-runtime-conformant`; it is not deployed conformance.

## Evidence

A conformance run records:

- capability and portable profile versions;
- adapter revision and Cloudflare compatibility date;
- stable node key hash or non-sensitive test identifier;
- environment tier;
- fixture IDs;
- observable decisions and event IDs;
- recovery and hibernation fault injections;
- pass, fail, or explicitly unsupported result.

Evidence must not include private keys, bearer tokens, raw reusable proofs, or
decrypted private content.

## Failure behavior

The adapter fails closed when:

- the stable node key is absent or ambiguous;
- a signature or event ID is invalid;
- a storage transaction cannot complete;
- a schema migration is incomplete;
- security evidence is missing, stale, replayed, or for another audience;
- an unsupported filter or profile is requested;
- subscription state cannot be safely reconstructed.

No failure may be converted into a successful acknowledgement with weaker
semantics.

## Deployment boundaries

The first deployed preview uses:

- a dedicated non-production Worker;
- one SQLite-backed Durable Object class and explicit migration tag;
- generated binding types from the checked-in Wrangler configuration;
- a current pinned compatibility date;
- no production custom domain requirement;
- no real private relay history;
- removable test node keys.

Secrets are configured as Cloudflare secrets and never committed. Deployment
credentials, account IDs, and environment-specific routes remain outside the
portable contracts.

## Non-goals

- Full `buzz-relay` parity.
- A globally shared singleton relay.
- Horizontal partitioning within one stable node.
- Search, media, git, huddles, workflow execution, or agent hosting.
- Production capacity, cost, SLO, backup, or incident-response certification.
- Automatic laptop-log import.
- New Cloudflare-specific Nostr kinds or client operations.

## Implementation sequence

1. Scaffold the Worker, SQLite-backed Durable Object, Wrangler environments,
   and Workers Vitest integration.
2. Implement deterministic node-key normalization and isolation tests.
3. Implement HTTP submit/query/count against the shared core vector.
4. Implement durable recovery and forced-eviction tests.
5. Implement WebSocket `EVENT`, `REQ`, `CLOSE`, `OK`, and `EOSE` with
   hibernation.
6. Run the complete local-runtime conformance suite.
7. Deploy a preview and run black-box parity against laptop outcomes.
8. Only then begin the identity profile.

## Traceability

- Telos: [`../TELOS.md`](../TELOS.md)
- Persona:
  [`../personas/local-first-builder.md`](../personas/local-first-builder.md)
- Journey:
  [`../journeys/promote-portable-relay-to-cloudflare.md`](../journeys/promote-portable-relay-to-cloudflare.md)
- Story:
  [`../stories/portable-relay/prove-cloudflare-portability.md`](../stories/portable-relay/prove-cloudflare-portability.md)
- Model:
  [`../models/portable-relay/portable-relay-cloudflare.model.yaml`](../models/portable-relay/portable-relay-cloudflare.model.yaml)
- Behavior:
  [`../features/portable-relay/cloudflare-conformance.feature`](../features/portable-relay/cloudflare-conformance.feature)
- Fixture:
  [`../fixtures/portable-relay/cloudflare-v0.1.json`](../fixtures/portable-relay/cloudflare-v0.1.json)
- HTTP contract:
  [`../contracts/openapi/local-relay.yaml`](../contracts/openapi/local-relay.yaml)
- WebSocket contract:
  [`../contracts/asyncapi/local-relay.yaml`](../contracts/asyncapi/local-relay.yaml)
- Capability:
  [`../capabilities/portable-relay-cloudflare.capability.yaml`](../capabilities/portable-relay-cloudflare.capability.yaml)

## Platform references

- [Rules of Durable Objects](https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/)
- [SQLite-backed Durable Object storage](https://developers.cloudflare.com/durable-objects/best-practices/access-durable-objects-storage/)
- [WebSocket hibernation](https://developers.cloudflare.com/durable-objects/best-practices/websockets/)
- [Testing Durable Objects](https://developers.cloudflare.com/durable-objects/examples/testing-with-durable-objects/)
- [Workers Vitest Durable Object helpers](https://developers.cloudflare.com/workers/testing/vitest-integration/test-apis/#durable-objects)
- [Workers language support](https://developers.cloudflare.com/workers/languages/)
