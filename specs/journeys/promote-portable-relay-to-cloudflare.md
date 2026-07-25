---
id: promote-portable-relay-to-cloudflare
type: journey
refs:
  persona: specs/personas/local-first-builder.md
---

# Journey: Promote a portable relay to Cloudflare

## Actor

The [local-first builder](../personas/local-first-builder.md), after proving a
durable relay experiment on a laptop.

Source Persona: `specs/personas/local-first-builder.md`

## Trigger

The builder wants the same stable relay memory to remain reachable when their
laptop is offline, without adopting the full hosted Buzz infrastructure or
changing client protocol behavior.

## Preconditions

- The laptop adapter passes `portable-relay-core-v0.1`.
- The builder has a stable node key representing one relay/community boundary.
- The shared signed-event fixtures are available to both adapters.
- A Cloudflare development and preview environment can create a SQLite-backed
  Durable Object.

## Flow

### 1. Name the stable node

- **User intent**: Preserve one logical relay identity across deployments.
- **System response**: The adapter derives one deterministic Durable Object
  identity from the normalized stable node key.
- **Next**: Exercise the adapter locally.

### 2. Start the Cloudflare adapter locally

- **User intent**: Test the platform mapping without publishing infrastructure.
- **System response**: The Workers runtime starts a stateless ingress Worker and
  one SQLite-backed Durable Object for the selected node.
- **Next**: Run shared conformance.

### 3. Prove protocol parity

- **User intent**: Know that moving runtimes did not change relay meaning.
- **System response**: The conformance runner submits the same signed events,
  filters, HTTP operations, and WebSocket messages used against the laptop
  adapter.
- **Next**: Exercise recovery.
- → `POST /events`, `POST /query`, `POST /count`, WebSocket `/`

### 4. Exercise eviction recovery

- **User intent**: Know that acknowledged memory is not tied to a warm isolate.
- **System response**: The running Durable Object is evicted while its storage
  remains; the next request reconstructs effective state from durable SQLite
  data.
- **Next**: Exercise the subscription boundary.

### 5. Exercise WebSocket hibernation

- **User intent**: Keep historical-to-live subscription semantics without
  requiring an always-warm process.
- **System response**: The Durable Object hibernates and later resumes the
  connection with its non-secret subscription and principal context intact.
- **Next**: Prove the deployed boundary.

### 6. Deploy a preview node

- **User intent**: Verify behavior on Cloudflare without declaring production
  readiness.
- **System response**: A preview Worker routes the configured node key to the
  same Durable Object boundary and exposes the portable HTTP and WebSocket
  contracts over TLS.
- **Next**: Run black-box conformance.

### 7. Compare observable outcomes

- **User intent**: Decide whether the portable boundary survives promotion.
- **System response**: A report compares laptop, local Workers runtime, and
  deployed preview outcomes by event ID, signed envelope, decision, query,
  count, recovery, and subscription ordering.
- **Next**: Either accept the capability or return the mismatch to the portable
  specification.

## Outcomes

- **Success**: The same client and signed events can use the laptop and
  Cloudflare adapters with equivalent portable outcomes.
- **Failure modes**:
  - two spellings of one node key route to different Durable Objects;
  - separate node keys expose one another's history;
  - an acknowledgement escapes before its journal write is durable;
  - eviction loses effective state or authentication replay state;
  - a hibernated WebSocket silently loses its subscriptions;
  - a platform limitation is hidden behind broader or degraded behavior;
  - local runtime conformance passes but deployed-preview conformance differs.

## Related Stories

- `specs/stories/portable-relay/prove-cloudflare-portability.md`

## E2E Coverage

- Planned:
  `cloudflare/portable-relay/test/portable-relay-cloudflare.conformance.test.ts`
