# Buzz Codebase Context

> Working architecture note for quickly restoring codebase context. This is a
> descriptive snapshot, not contributor policy. `AGENTS.md`, `CONTRIBUTING.md`,
> and the implementation take precedence when they disagree with this file.
>
> Last reviewed: 2026-08-04 at commit `8342dfcc`.

## Product in one paragraph

Buzz is a self-hostable team workspace where people, agents, workflows, Git
activity, and collaboration features use signed Nostr events. A relay is the
authoritative server for a community: it authenticates principals, verifies and
stores events, applies authorization and feature-specific state transitions,
fans events out live, indexes searchable content in PostgreSQL, and invokes
automation. Desktop is the broadest client; mobile focuses on communication and
activity; the browser client currently focuses on invitations and Git browsing;
the CLI and ACP crates provide the agent-facing surface.

## System map

```text
Desktop / Mobile / Web / CLI / ACP agent harness
                        |
                  WS Nostr + HTTP
                        |
                buzz-relay (Axum)
       auth -> conformance -> ingest -> PostgreSQL
                        |
       +----------------+------------------+
       |                |                  |
     Redis          object storage     post-commit work
  pub/sub, cache      media + Git       local fan-out,
  invalidation                          workflows, audit
```

The relay is an application server, not just a generic Nostr relay. Its Axum
router combines:

- NIP-01 WebSocket and NIP-11/NIP-05 metadata;
- generic Nostr HTTP bridges (`/events`, `/query`, `/count`);
- invitations, join policy, moderation, and operator community management;
- Blossom-compatible media storage;
- Git smart HTTP and Git policy hooks;
- workflow webhooks;
- huddle audio WebSockets;
- health, readiness, metrics, admin UI, and public web bundle serving.

Primary code: `crates/buzz-relay/src/router.rs`.

## Repository surfaces

### Relay and services

- `buzz-core`: I/O-free event types, filters, verification, tenant types, and
  the canonical event-kind registry.
- `buzz-relay`: orchestration boundary and public server process.
- `buzz-db`: PostgreSQL access, projections, event storage, migrations, and
  tenant-scoped durable state.
- `buzz-auth`: NIP-42, NIP-98, token scopes, and authorization primitives.
- `buzz-pubsub`: Redis fan-out, presence, invalidation, connection control, and
  rate limiting.
- `buzz-search`: PostgreSQL full-text search behavior.
- `buzz-audit`: per-community tamper-evident audit chains.
- `buzz-workflow`: YAML workflow evaluation and execution.
- `buzz-media`: media validation and S3-compatible object storage.
- `buzz-relay-mesh`, `buzz-voice`: mesh and voice/huddle support.
- `buzz-push-gateway`: push matching and APNs delivery service.

Service crates generally depend on `buzz-core`, not on each other. The relay
coordinates cross-service behavior.

### Agents and developer tools

- `buzz-acp`: bridges Buzz events and managed AI agents through ACP.
- `buzz-agent`: minimal ACP-compatible agent runtime.
- `buzz-cli`: agent-first command surface; new agent operations should begin
  here.
- `buzz-sdk`: typed signed-event builders.
- `buzz-dev-mcp`: shell and file tools exposed to managed agents.
- `buzz-persona`, `buzz-backend-kubernetes`, and `sprig`: persona packaging,
  Kubernetes compute, and bundled agent harnesses.

### Clients

- `desktop/`: Tauri 2, React 19, TanStack Query/Router, Tailwind, and a Rust
  native backend. This is the complete product surface.
- `mobile/`: Flutter with Riverpod and hooks. It connects directly to the relay
  and covers pairing, channels, activity, forum, pulse, search, profile, and
  settings.
- `web/`: React browser client for invite flows and Git repository browsing.
- `admin-web/`: small operator interface served only when configured.

## Central execution paths

### Connection and tenant boundary

1. Resolve a `TenantContext` from the HTTP/WebSocket host before request data
   reaches feature handlers.
2. Reject unknown hosts in multi-community mode rather than falling back to a
   default tenant.
3. Authenticate WebSockets with NIP-42 and HTTP requests with NIP-98 or the
   explicitly supported endpoint-specific mechanism.
4. Register subscriptions and connections with their resolved community ID.
5. Recheck receiver tenant and access at the final live-delivery chokepoint.

Important files:

- `crates/buzz-relay/src/tenant.rs`
- `crates/buzz-relay/src/connection.rs`
- `crates/buzz-relay/src/handlers/auth.rs`
- `crates/buzz-relay/src/handlers/req.rs`
- `crates/buzz-relay/src/handlers/event.rs`

### Persistent event ingestion

WebSocket `EVENT` and HTTP `POST /events` converge on
`handlers::ingest::ingest_event`:

1. Resolve the authenticated principal and required scope.
2. Reject unsupported kinds and invalid transport/kind combinations.
3. Verify signature, ID, author rules, timestamps, tags, and tenant claims.
4. Apply kind-specific authorization and pre-storage validation.
5. Insert the event or execute the appropriate command/projection behavior.
6. Record conformance actions through the guarded trace seam.
7. Enqueue audit work with bounded backpressure.
8. Return durable acceptance; schedule Redis publication, live fan-out, and
   workflow triggering as post-commit asynchronous work.

Primary files:

- `crates/buzz-relay/src/handlers/ingest.rs`
- `crates/buzz-relay/src/handlers/event.rs`
- `crates/buzz-relay/src/handlers/side_effects.rs`
- `crates/buzz-relay/src/conformance/`

Acceptance means the write is durable. It does not guarantee that detached
live delivery or workflow triggering has completed. Search is not a detached
indexing job: searchable state is part of the PostgreSQL event row through its
generated FTS column.

### Live delivery

Local and Redis-delivered events both pass through access filtering before a
WebSocket send. The delivery fence verifies:

- the receiver connection belongs to the event's community;
- private-channel membership remains current;
- author-only and shared-gated kinds obey their visibility rules;
- viewer-private kinds reach only their intended owner;
- a stale subscription is never sufficient by itself to authorize delivery.

Redis provides cross-node fan-out and local-echo deduplication. Slow clients use
bounded send buffers and are eventually disconnected instead of accumulating
unbounded memory.

### Desktop startup and community switching

`desktop/src/main.tsx` installs the optional E2E bridge before React mounts,
migrates legacy local community storage, and builds the global provider tree.
`desktop/src/app/App.tsx` gates onboarding and community initialization, then
keys the community-scoped application subtree so a switch causes a React
remount.

React remounting does not clear module-level state. The explicit singleton
reset list lives in
`desktop/src/features/communities/useCommunityInit.ts::resetCommunityState`.
Any new community-scoped module cache must be added there unless its lifetime is
owned by a hook with reliable cleanup.

### Mobile startup

`mobile/lib/app.dart` chooses pairing or the authenticated home shell. Once
authenticated, it eagerly watches the relay session, observer subscription,
lifecycle integration, user-status cache, unread state, and deep-link handling.
State belongs in Riverpod; local widget state uses Flutter hooks.

## Invariants worth protecting

- The request host, not event tags, chooses the community.
- `h` tags scope events inside channels; channel descriptor/membership events
  use channel IDs in `d` tags where specified by NIP-29.
- Every relay query includes explicit `kinds`; broad kindless queries hit the
  p-gate.
- Live delivery performs authorization again at send time.
- New Nostr kind numbers are defined first in
  `crates/buzz-core/src/kind.rs`.
- Thread reply insertion updates materialized root counters.
- Relay-signed derived events retain the initiating human/agent as the audit
  actor.
- Workflow execution events and relay workflow output cannot recursively
  retrigger workflows.
- New desktop community-scoped singletons have an explicit reset lifecycle.
- Desktop readable text uses named rem-based Tailwind tokens, not arbitrary
  pixel or rem literals.
- Mobile feature modules import only shared code, not other feature modules;
  widgets use Riverpod/hooks rather than `StatefulWidget`.

## Architectural strengths

- One protocol and identity model covers humans, agents, and automation.
- Transport-neutral ingestion avoids separate WebSocket and HTTP semantics.
- Tenant isolation is layered across routing, storage, caches, subscriptions,
  Redis topics, and final delivery.
- Conformance tracing treats an unclassified ingest exit as an implementation
  defect and connects implementation behavior to the multi-tenant formal spec.
- Resource controls include body limits, connection and handler semaphores,
  bounded queues, slow-client eviction, upload limits, and Git size limits.
- Observability includes structured logs, Prometheus metrics, OpenTelemetry,
  audit chains, liveness/readiness endpoints, and read-replica freshness fences.
- Testing spans Rust units, real PostgreSQL/Redis integration tests,
  conformance/property tests, desktop helper tests, Playwright, Flutter widget
  tests, and Nostr interoperability suites.

## Complexity and risk areas

### Oversized modules

Several files concentrate a large amount of behavior and are expensive to
review safely. Examples at the review snapshot:

- `crates/buzz-db/src/lib.rs`: roughly 8.5k lines;
- `crates/buzz-acp/src/pool.rs`: roughly 7k lines;
- `crates/buzz-relay/src/handlers/ingest.rs`: roughly 4.8k lines;
- `desktop/src/testing/e2eBridge.ts`: roughly 12.9k lines;
- several desktop UI/API modules: roughly 1k-2.2k lines.

Prefer extracting behavior around stable domain boundaries rather than adding
new branches to these files indefinitely.

### Detached post-commit work

Redis publication, local fan-out, and workflow triggering occur after durable
acceptance in spawned tasks. A process exit in that window can leave the event
stored without immediate live delivery or trigger execution. Audit enqueue is
awaited and backpressured, but audit worker database failures are logged rather
than retried. Changes requiring exactly-once or eventually-guaranteed behavior
should use a durable outbox/reconciliation design rather than relying on these
tasks.

### Manually synchronized event kinds

Rust is canonical, but desktop TypeScript and mobile Dart maintain their own
kind constants:

- `crates/buzz-core/src/kind.rs`
- `desktop/src/shared/constants/kinds.ts`
- `mobile/lib/shared/relay/nostr_models.dart`

There is no obvious generated cross-language registry. When adding a client-
visible kind, inspect and update all applicable surfaces and tests.

### Community reset registry

The desktop reset list is a deliberate safety boundary but depends on developer
discipline. A new module-level cache can leak data or presentation state across
communities if it is omitted. Prefer community-owned providers/stores for new
state where practical.

### Deployment topology

Huddle audio has topology-sensitive behavior. The non-mesh path uses in-process
rooms and cannot safely serve peers placed on different relay pods. Mesh is
opt-in; deployments without a working cross-pod audio path must disable huddle
audio when horizontally scaled.

## Known documentation drift at this snapshot

Verify these against code before relying on older architecture prose:

- `ARCHITECTURE.md` still describes an asynchronous `search_index_tx`; current
  PostgreSQL FTS is generated from the persisted event row.
- `ARCHITECTURE.md` describes presence fan-out as local-only; current relay code
  includes Redis-based global presence fan-out and local-echo suppression.
- `AGENTS.md` says migrations are auto-applied at relay startup; current
  `buzz-relay/src/main.rs` runs migrations only when `BUZZ_AUTO_MIGRATE` is
  explicitly truthy. Development setup applies migrations separately.
- Counts of registered event kinds in prose are snapshots and may lag
  `buzz-core/src/kind.rs::ALL_KINDS`.

## Testing and verification

Activate Hermit before Git hooks or project commands:

```bash
. ./bin/activate-hermit
```

Useful gates:

```bash
just ci            # full no-infrastructure local gate
just test-unit     # fast unit coverage
just test          # integration coverage; requires PostgreSQL and Redis
just desktop-e2e-smoke
```

Special cases:

- The desktop Tauri crate is excluded from the root Cargo workspace; test it
  with its own manifest or the corresponding `just` task.
- Build desktop E2E assets with `pnpm build:e2e`, never the ordinary production
  build.
- Agents must not run Flutter build/run/clean/upgrade commands; use analysis,
  formatting checks, and tests only.
- Run `just ci` before a PR and sign commits with `git commit -s`.

## Fast orientation checklist

For a new task, answer these before editing:

1. Is the operation best represented as a Nostr event rather than a new HTTP
   endpoint?
2. Which event kind and tag convention apply?
3. Where is tenant context established, and is it preserved through every DB,
   cache, Redis, and fan-out call?
4. Is the operation a command, a durable event, an ephemeral event, or a
   derived relay-signed event?
5. Does it need a `buzz-cli` command for agents?
6. Which desktop/mobile/web surfaces implement or intentionally omit it?
7. Does it introduce a module-level desktop cache requiring community reset?
8. Does it insert replies, change authorization, or add post-commit behavior?
9. Which unit, integration, conformance, and client tests cover the boundary?

## First files to read by task type

| Task | Start here |
|---|---|
| Event kind or validation | `buzz-core/src/kind.rs`, `buzz-relay/src/handlers/ingest.rs` |
| Live subscriptions/fan-out | `buzz-relay/src/handlers/req.rs`, `event.rs`, `subscription.rs` |
| Database behavior | `buzz-db/src/event.rs`, `channel.rs`, then `lib.rs` |
| Authentication/authorization | `buzz-auth/`, relay `handlers/auth.rs`, `api/auth.rs` |
| Desktop application shell | `desktop/src/main.tsx`, `desktop/src/app/App.tsx`, `AppShell.tsx` |
| Desktop community behavior | `desktop/src/features/communities/useCommunityInit.ts` |
| Mobile lifecycle | `mobile/lib/app.dart`, `mobile/lib/shared/relay/relay_session.dart` |
| Agent operation | `buzz-cli/src/commands/`, `buzz-cli/src/client.rs`, `buzz-sdk` |
| Workflows | `buzz-workflow/`, relay `workflow_sink.rs` |
| Media | `buzz-media/`, relay `api/media.rs` |
| Git | relay `api/git/`, `docs/git-on-object-storage.md` |
| Multi-tenant correctness | `docs/multi-tenant-relay.md`, `docs/spec/MultiTenantRelay.tla`, relay `conformance/` |

