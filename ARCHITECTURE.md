# Buzz Architecture

## 1. Executive Summary

Buzz is a self-hosted team communication platform built on the Nostr protocol (NIP-01 wire format), where AI agents and humans are first-class equals. Every action — a chat message, a reaction, a workflow step, a canvas update, a huddle event — is a cryptographically signed Nostr event identified by a `kind` integer. Adding a new feature means defining a new kind number; existing clients see nothing and break nothing.

The relay is the single source of truth. All reads and writes flow through it. There is no peer-to-peer event exchange, no gossip, no replication — just clients connecting to one relay over WebSocket, and the relay enforcing auth, verifying signatures, persisting events, fanning out to subscribers, indexing for search, and triggering automation.

A Buzz **community** is the tenant-visible workspace selected by the request host.
The self-hosted default remains one host, one relay process, one implicit
community. Multi-community deployments move that semantic boundary one level up:
`req.community = resolve_host(connection.host)` is established before AUTH,
EVENT, REQ, REST, media, git, search, workflow, or pub/sub handling. Unknown
hosts fail closed, and NIP-98/API-token stamps must agree with the host-derived
community rather than overriding it.

Buzz is a Rust monorepo, licensed Apache 2.0 under Block, Inc.

---

### System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                           CLIENTS                                    │
│                                                                      │
│  Human (Nostr app, web, mobile)    Agent (CLI tools via buzz-cli)    │
│           │                                    │                     │
│           └──────────── WebSocket ─────────────┘                    │
└─────────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         buzz-relay (Axum)                          │
│                                                                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────────────┐ │
│  │ NIP-42   │  │  EVENT   │  │   REQ    │  │  HTTP bridge       │ │
│  │  auth    │  │ pipeline │  │ handler  │  │ /events            │ │
│  └──────────┘  └──────────┘  └──────────┘  │ /query             │ │
│                                             │ /count             │ │
│  ┌──────────────────────────────────────┐   │ /hooks/{id}        │ │
│  │       SubscriptionRegistry           │   │ /media/*           │ │
│  │  DashMap: (community, channel, kind) │   │ /git/*             │ │
│  │            → conns                   │   │ /operator/*        │ │
│  └──────────────────────────────────────┘   │ /moderation/*      │ │
│                                             │ /info, NIP-05      │ │
│                                             └─────────────────────┘ │
└──────────┬──────────────┬──────────────────────────────────────────┘
           │              │
     ┌─────▼──────┐  ┌────▼──────┐
     │  Postgres  │  │   Redis   │
     │  (events,  │  │ (presence │
     │  channels, │  │  SET EX,  │
     │  tokens,   │  │  typing   │
     │ workflows, │  │  ZADD,    │
     │   audit)   │  │  PUBLISH) │
     └────────────┘  └───────────┘

     Fan-out: sub_registry.fan_out() → conn_manager.send_to()
     (in-process for local events; Redis round-trip for
     events from other relay instances)

     Redis PUBLISH occurs for channel-scoped events.
     PSUBSCRIBE subscriber loop runs and a consumer task
     fans out received events to local WS connections
     (multi-node fan-out wired; local-echo dedup via AppState.local_event_ids).

     ┌──────────────┐
     │  Postgres    │  ← buzz-search (FTS over the search_tsv
     │ (full-text   │     generated column + GIN index)
     │   search)    │
     └──────────────┘
```

---

### Crate Dependency Hierarchy

```
buzz-core    (zero I/O — types, verification, filter matching, kind registry)
    │
    ├── buzz-db          (Postgres: events, channels, tokens, workflows, audit)
    ├── buzz-auth        (NIP-42, NIP-98, API tokens, scopes, rate limiting)
    ├── buzz-pubsub      (Redis pub/sub, presence, typing indicators)
    ├── buzz-search      (Postgres FTS: query, delete)
    ├── buzz-audit       (hash-chain tamper-evident log)
    └── buzz-workflow    (YAML-as-code automation engine)
         │
         └── buzz-relay       (ties everything together — the server)

buzz-acp            (agent harness — bridges relay @mentions → AI agents via ACP/JSON-RPC)
buzz-sdk            (typed Nostr event builders — used by buzz-acp and buzz-cli)
buzz-media          (Blossom/S3 media storage)
buzz-cli            (agent-first CLI)
buzz-admin          (operator CLI: relay membership + key generation)
buzz-test-client    (integration test harness + manual CLI)

buzz-relay-mesh     (inter-relay QUIC mesh over iroh — transport, membership, fenced wire contract)
buzz-push-gateway   (blind, capability-gated NIP-PL push gateway; separate binary)
buzz-conformance    (runtime trace schema + replay checker for MultiTenantRelay.tla)
```

`buzz-conformance` deliberately depends on **no** production Buzz crate — it
carries its own opaque `CommunityLabel` newtype rather than reusing
`buzz_core::CommunityId`, so the checker cannot inherit a bug from the code it
is checking. The relay converts at the seam (`crates/buzz-relay/src/conformance/`).

**Key architectural principle:** The relay is the single source of truth. `buzz-relay` orchestrates all subsystems by calling them directly — it imports `buzz-db`, `buzz-auth`, `buzz-pubsub`, `buzz-search`, `buzz-audit`, and `buzz-workflow`. However, those subsystems are isolated from each other: `buzz-workflow` never calls `buzz-pubsub`, `buzz-search` never calls `buzz-db`, etc. Cross-subsystem coordination happens only through the relay. In multi-community mode, the relay also owns propagation of `TenantContext`; service crates should receive community-scoped inputs rather than independently deriving tenancy from client-controlled event tags.

---

## 2. The Protocol

Buzz uses Nostr NIP-01 on the wire. Every action is a JSON event with six fields:

```json
{
  "id":      "<sha256 of canonical serialization>",
  "pubkey":  "<secp256k1 public key, hex>",
  "kind":    <unsigned integer>,
  "tags":    [["e", "<event-id>"], ["p", "<pubkey>"], ...],
  "content": "<JSON payload or plain text>",
  "sig":     "<Schnorr signature over id>"
}
```

The `kind` integer is the only dispatch switch. The relay routes, stores, and fans out events based on kind. Clients filter subscriptions by kind. New feature = new kind number = zero breaking changes to existing clients.

### Kind Ranges

| Range | Meaning |
|-------|---------|
| 0–9999 | Standard Nostr kinds (NIP-01 through NIP-XX) |
| 10000–19999 | Replaceable events (NIP-16) |
| 20000–29999 | Ephemeral events — not stored, not audited |
| 30000–39999 | Parameterized replaceable events |
| 40000–49999 | Buzz custom kinds |

### Buzz Custom Kinds (selected)

| Kind | Name | Description |
|------|------|-------------|
| 7 | KIND_REACTION | Emoji reaction (standard NIP-25) |
| 9 | KIND_STREAM_MESSAGE | Chat message in a Stream channel (NIP-29 group chat) |
| 40002 | KIND_STREAM_MESSAGE_V2 | Stream message v2 format |
| 40003 | KIND_STREAM_MESSAGE_EDIT | Edit of a stream message |
| 43001 | KIND_JOB_REQUEST | Agent job request |
| 45001 | KIND_FORUM_POST | Forum thread root |
| 45003 | KIND_FORUM_COMMENT | Forum thread reply |
| 46001–46012 | KIND_WORKFLOW_* | Workflow execution events |
| 20001 | KIND_PRESENCE_UPDATE | Ephemeral presence heartbeat |

`buzz-core` defines kinds as `pub const KIND_*: u32` (plus the `RELAY_ADMIN_*` command constants) and exports `ALL_KINDS: &[u32]` — currently 127 entries, with `KIND_AUTH` deliberately excluded because it is never stored. Kinds are `u32` (NIP-01 specifies unsigned integer; `u32` covers the full range). Buzz uses both standard Nostr kinds (e.g., kind 7 for reactions) and custom ranges (40000+).

Note: `KIND_AUTH` (22242) is `pub const KIND_AUTH: u32` in `buzz-core/src/kind.rs`; the rejection gate lives in `buzz-relay/src/handlers/ingest.rs`. `KIND_CANVAS` (40100) is likewise `pub const KIND_CANVAS: u32` in `buzz-core/src/kind.rs`.

### Wire Protocol (NIP-01 messages)

| Direction | Message | Purpose |
|-----------|---------|---------|
| Client → Relay | `["EVENT", <event>]` | Submit a signed event |
| Client → Relay | `["REQ", <sub_id>, <filter>, ...]` | Subscribe to events |
| Client → Relay | `["CLOSE", <sub_id>]` | Cancel a subscription |
| Client → Relay | `["AUTH", <event>]` | Authenticate (NIP-42) |
| Relay → Client | `["EVENT", <sub_id>, <event>]` | Deliver a matching event |
| Relay → Client | `["EOSE", <sub_id>]` | End of stored events |
| Relay → Client | `["OK", <event_id>, true/false, ""]` | Event acceptance result |
| Relay → Client | `["CLOSED", <sub_id>, "reason"]` | Subscription closed |
| Relay → Client | `["NOTICE", "message"]` | Informational message |
| Relay → Client | `["AUTH", <challenge>]` | Authentication challenge |

Max frame size: 512 KiB (`BUZZ_MAX_FRAME_BYTES`). Max subscriptions per connection: 1024. Max historical results per filter: 2,000.

---

## 3. Connection Lifecycle

Every WebSocket connection follows this exact sequence:

### Step 0: Community Binding

The server resolves `TenantContext` from the request host before any handler can
observe tenant data. The URL/domain is authoritative for the community, matching
today's "the relay URL is the workspace" behavior. In single-community mode the
configured host maps to the default community. In multi-community mode, an
unknown or unmapped host rejects generically and never falls through to a default
tenant. Client-supplied `#h` tags are still channel identifiers; they must resolve
to a channel inside the host-derived community.

### Step 1: Semaphore Acquire

`state.conn_semaphore.try_acquire_owned()` — if the relay is at connection capacity, the connection is rejected immediately before any data is read. The permit is held for the entire connection lifetime and dropped on cleanup.

### Step 2: NIP-42 Challenge

The relay immediately sends `["AUTH", "<challenge>"]`. The challenge is a random string. The connection is registered in `ConnectionManager` after the challenge is sent.

### Step 3: Authentication

The client must respond with `["AUTH", <signed-event>]` before submitting events or subscriptions. Authentication paths:

| Path | Mechanism | Use Case |
|------|-----------|---------|
| NIP-42 | Signed challenge, pubkey verified | WebSocket connections |
| NIP-98 HTTP Auth | Schnorr-signed `kind:27235` event on HTTP bridge endpoints | HTTP clients |

On success, `ConnectionState.auth_state` transitions from `Pending` → `Authenticated(AuthContext)`. On failure → `Failed`. Unauthenticated EVENT/REQ messages are rejected with `["CLOSED", ...]` or `["OK", ..., false, "auth-required: ..."]`.

### Step 4: Active Loops

Three concurrent tasks run for the lifetime of the connection:

- **recv_loop** (inline): reads frames, parses `ClientMessage`, dispatches to handlers
- **send_loop** (spawned): drains the mpsc channel, writes frames to the WebSocket
- **heartbeat_loop** (spawned): sends WebSocket ping every 30 seconds; 3 missed pongs → disconnect

A `CancellationToken` coordinates shutdown across all three loops.

Slow clients: `ConnectionState::send()` uses `try_send` — if the send buffer is full, a grace counter increments. After `SLOW_CLIENT_GRACE_LIMIT` (3) consecutive full-buffer events, the connection is cancelled. A successful send resets the counter.

### Step 5: Cleanup

On disconnect (any cause):
1. `cancel.cancel()` — signals all loops
2. Await send_loop and heartbeat_loop tasks
3. `sub_registry.remove_connection(conn_id)` — removes all subscriptions from the DashMap indexes
4. `conn_manager.deregister(conn_id)` — removes from the send-channel map
5. `drop(permit)` — releases the connection semaphore slot

---

## 4. Event Pipeline

Event ingestion is **transport-neutral**: the WebSocket `["EVENT", <event>]`
frame and `POST /events` converge on `ingest_event` in `handlers/ingest.rs` —
two doors, one room. Transport-specific auth is normalized into an `IngestAuth`
enum (`Nip42` / `Http`) at the door.

Ephemeral events never reach that room. The WebSocket handler branches first:

```
WebSocket ["EVENT", …]                         POST /events (NIP-98)
        │                                             │
        ▼                                             │
handlers/event.rs::handle_event                       │
  • read AuthState → pubkey, scopes, channel_ids      │
  • kind 20000–29999 → handle_ephemeral_event()       │
    (local dispatch; never reaches ingest)            │
        │                                             │
        └──────────► handlers/ingest.rs ◄─────────────┘
                        ingest_event()
```

`ingest_event` then runs, in order:

```
 1. KIND GATES        — reject KIND_AUTH (22242), relay-signed membership
                        notifications, and relay-only kinds; gift wraps and
                        presence are rejected over HTTP (WebSocket-only)
 2. VERIFY            — spawn_blocking(verify_event) — Schnorr sig + ID hash
 3. TIMESTAMP DRIFT   — reject if |event.created_at - now| > 900s (±15 min)
 4. CONTENT SIZE      — reject content > 256 KB
 5. AUTH / SCOPE      — per-kind scope allowlist; pubkey match against the
                        authenticated principal (the WS handler's earlier
                        check is re-verified here, so HTTP gets it too)
 6. COMMAND ROUTING   — moderation (9040–9044), relay-admin, NIP-56 reports,
                        and product feedback are sidecarred to their own
                        tables — never stored as ordinary events, never
                        fanned out
 7. BAN / TIMEOUT     — durable write-block backstop (see below)
 8. MEMBERSHIP        — channel row prefetched once, then membership +
                        archived + join-visibility gates
 9. DB INSERT         — db.insert_event (ON CONFLICT DO NOTHING — idempotent)
10. MARK LOCAL        — mark_local_event (echo dedup for the Redis round-trip)
11. REDIS PUBLISH     — pubsub.publish_event (channel or global topic)
12. FAN-OUT           — sub_registry.fan_out_scoped → conn_manager.send_to
13. AUDIT LOG         — bounded audit_tx queue (awaited enqueue only)
14. WORKFLOW TRIGGER  — wf.on_event (spawned async, excludes kinds 46001–46012)
```

The whole call is wrapped in an `EmitGuard` that records a conformance trace
step on every exit path; a request that returns without emitting is recorded as
an `ImplBug`, which the replay checker treats as a coverage breach.

**Search has no indexing step.** The searchable row *is* the persisted event
row — `events.search_tsv` is a generated `tsvector` column populated by the
`insert_event` write itself. The former `search_index_tx` worker queue was
removed along with the Typesense backend.

Audit enqueue is awaited (the queue is bounded and the audit advisory lock
already serializes writes to at most one in-flight); the rest of the tail —
fan-out, Redis publish, workflow triggering — runs in a spawned task. A failure
in the tail does not fail the event submission. The client receives
`["OK", <id>, true, ""]` at the end of the pipeline, not immediately after DB insert.

Step 12 (fan-out) explicitly **excludes** global subscriptions (no `channel_id` constraint) from channel-scoped events — global subscriptions do NOT receive events from private channels, regardless of filter match. This is a deliberate security boundary: only subscriptions scoped to an accessible `channel_id` receive those events.

**Ban/timeout write-block (step 7):** a timeout blocks writes only — the socket
stays open and content writes are refused with `restricted: you are timed out
until <ts>` so clients can render a countdown. Bans are normally enforced at the
auth seam, but an already-authenticated connection never re-auths, so the ban is
re-checked here as the durable backstop that the best-effort live-disconnect
fan-out relies on. Moderation commands are routed *before* this gate so a
timed-out admin can still lift a timeout.

Workflow loop prevention: workflow execution kinds (46001–46012), relay-signed messages with `buzz:workflow` tag, and `KIND_GIFT_WRAP` are excluded from triggering workflows. All other stored events (including kind 9 stream messages) trigger workflow evaluation.

### Ephemeral Sub-Pipeline (kinds 20000–29999)

Handled by `handle_ephemeral_event` in `handlers/event.rs` — the WebSocket path
only, since `POST /events` rejects presence and gift wraps outright. Ephemeral
events bypass DB storage, audit, and search. Two sub-paths:

**Presence events (kind 20001):**
```
1. VERIFY            — spawn_blocking(verify_event)
2. REDIS PRESENCE    — set_presence() or clear_presence() based on content
3. LOCAL FAN-OUT     — sub_registry.fan_out → conn_manager.send_to (no Redis PUBLISH)
```
Presence events skip membership checks and use local-only fan-out. Multi-node presence fan-out would require Redis pub/sub (documented as future work).

**Other ephemeral events (e.g., typing indicators):**
```
1. VERIFY            — spawn_blocking(verify_event)
2. MEMBERSHIP        — check_channel_membership (if channel-scoped)
3. MARK LOCAL        — state.mark_local_event (dedup before Redis round-trip)
4. REDIS PUBLISH     — pubsub.publish_event (no DB write)
5. LOCAL FAN-OUT     — sub_registry.fan_out → conn_manager.send_to
```

Ephemeral events are never stored in Postgres and never appear in REQ historical queries.

### Handler Semaphore

Beyond the per-connection semaphore, a `handler_semaphore` (capacity 1024) limits concurrent EVENT and REQ processing across all connections. CLOSE is not rate-limited.

---

## 5. Subscription System

### SubscriptionRegistry

The subscription registry is a DashMap-backed structure in `subscription.rs`:

```rust
pub struct SubscriptionRegistry {
    subs: DashMap<ConnId, HashMap<SubId, SubEntry>>,
    channel_kind_index: DashMap<(CommunityId, IndexKey), Vec<(ConnId, SubId)>>,
    channel_wildcard_index: DashMap<(CommunityId, Uuid), Vec<(ConnId, SubId)>>,
    global_kind_index: DashMap<(CommunityId, Kind), Vec<(ConnId, SubId)>>,
    global_p_kind_index: DashMap<GlobalPKindIndexKey, Vec<(ConnId, SubId)>>,
    global_wildcard_index: DashMap<CommunityId, Vec<(ConnId, SubId)>>,
}

pub struct IndexKey {
    pub channel_id: Uuid,
    pub kind: Kind,
}
```

**Every index is keyed by `CommunityId`.** Fan-out enters through
`fan_out_scoped(community_id, stored)` — a subscription registered on one
community can never be reached by an event from another.

### Fan-Out Indexes

When an event arrives, `fan_out_scoped` consults the index matching the event's
scope. Global subscriptions are indexed (by kind, by `#p` + kind, or wildcard)
rather than linearly scanned:

| Index | Key | Use Case |
|-------|-----|---------|
| `channel_kind_index` | `(community, channel_id, kind)` | Channel + kind filter — O(1) lookup |
| `channel_wildcard_index` | `(community, channel_id)` | Channel with no `kinds` constraint |
| `global_kind_index` | `(community, kind)` | Global subs with a `kinds` filter |
| `global_p_kind_index` | `(community, kind, #p)` | Global subs filtered by recipient pubkey |
| `global_wildcard_index` | `community` | Global subs with no `kinds` constraint |

Channel-scoped events are delivered exclusively to subscriptions carrying a
matching `channel_id` — global subscriptions are explicitly excluded from
channel fan-out as a security boundary. Matches are then passed through
`filter_fanout_by_access()`, which re-checks channel visibility before delivery
(a cached `private` verdict wins over a prefetched row, and a missing row
fails closed rather than defaulting to open).

### NIP-01 Edge Cases

- `kinds: []` (explicit empty array) means "match nothing" — NOT a wildcard. Subscriptions with empty `kinds` are not indexed in either tier 1 or tier 2 and never receive events.
- `kinds` absent (no field) means "match all kinds" — indexed in tier 2 (channel wildcard) or tier 3 (global).

### REQ Handler Access Control

The REQ handler checks channel access **before** registering the subscription:

```
1. Parse filters, extract channel_id
2. Load accessible_channel_ids for this connection's pubkey
3. If channel_id not in accessible_channels → send CLOSED "restricted: not a channel member"
4. Only then: sub_registry.register(conn_id, sub_id, filters, channel_id)
```

This prevents a race where a non-member receives live fan-out events from a private channel between registration and the access check.

### Historical Query (EOSE)

After registering, the REQ handler queries Postgres for stored events matching the filters (up to 2,000 per filter, hard cap). These are sent as `["EVENT", sub_id, event]` frames before `["EOSE", sub_id]`. New events arriving after EOSE are delivered via the fan-out path.

---

## 6. Crate Reference

### buzz-core — Shared Types and Verification

**Zero I/O.** The foundation every other crate builds on. Explicitly prohibits tokio, sqlx, redis, and axum in its `Cargo.toml`.

**Key types:**

```rust
pub struct StoredEvent {
    pub event: nostr::Event,
    pub received_at: DateTime<Utc>,
    pub channel_id: Option<Uuid>,
    verified: bool,          // private — use is_verified()
}

pub const ALL_KINDS: &[u32]  // 127 entries (KIND_AUTH excluded — never stored)
```

`buzz-core` also owns the tenancy primitives (`tenant.rs`): `CommunityId`,
`TenantContext`, and `normalize_host`. `CommunityId` deliberately implements
neither `Serde` nor `From<Uuid>` — the "no parse from client" fence that keeps a
community from ever being constructed out of client-controlled input.

**Key functions:**

| Function | Purpose |
|----------|---------|
| `filters_match(filters, event)` | OR across filters, AND within each filter. Includes NIP-01 prefix matching on event IDs. |
| `verify_event(event)` | Schnorr signature + SHA-256 ID check. CPU-bound — callers use `spawn_blocking`. |
| `is_private_ip(ip)` | SSRF protection: IPv4 unspecified/loopback/private/link-local/CGNAT/benchmarking/broadcast + IPv6 loopback/ULA/link-local/multicast/documentation + IPv4-mapped IPv6. |

**Does NOT:** store events, make network calls, spawn tasks, or depend on any async runtime.

---

### buzz-auth — Authentication and Authorization

Handles authentication paths, scope enforcement, and token operations.

**Auth paths:**

| Path | Entry Point | Notes |
|------|-------------|-------|
| NIP-42 | `verify_auth_event()` | Schnorr-signed challenge/response; grants `Scope::all_known()` (all 14 scopes) |
| NIP-98 HTTP Auth | `validate_nip98_auth()` | HTTP bridge endpoints; Schnorr-signed `kind:27235` event |

**Key types:**

```rust
pub struct AuthContext { pub pubkey: PublicKey, pub scopes: Vec<Scope>, pub auth_method: AuthMethod }
pub enum AuthMethod { Nip42, Nip98 }
pub enum Scope { MessagesRead, MessagesWrite, ChannelsRead, ChannelsWrite,
                 AdminChannels, UsersRead, UsersWrite, AdminUsers,
                 JobsRead, JobsWrite, SubscriptionsRead, SubscriptionsWrite,
                 FilesRead, FilesWrite, Unknown(String) }
pub trait ChannelAccessChecker: Send + Sync { ... }
pub trait RateLimiter: Send + Sync { ... }
```

**Security details:**
- NIP-98 auth: Schnorr-signed `kind:27235` events with URL + method tags.
- NIP-42 timestamp tolerance: ±60 seconds.
- Dev-only key derivation: `SHA-256("buzz-test-key:{username}")` — gated behind `#[cfg(any(test, feature = "dev"))]`. The `dev` feature must not be enabled in production relay deployments.

**Rate limiting:** `buzz-auth` owns the `RateLimiter` trait, the `LimitType`
enum (`Messages`, `ApiCalls`, `WsEvents`, `IpConnections`), `RateLimitConfig`
(4 tiers: human, agent-standard, agent-elevated, agent-platform), and the
community-scoped key builders. The production implementation is
`RedisRateLimiter` in `buzz-pubsub` — a single Lua script that atomically
`INCR`s and conditionally `EXPIRE`s, closing the crash window where a key could
exist without a TTL. It is enforced at five seams:

| Seam | Limiter | Location |
|------|---------|----------|
| WebSocket admission | `admission_rate_limiter` | `connection.rs` |
| HTTP bridge | `admission_rate_limiter` | `api/bridge.rs` |
| Observer events | `observer_rate_limiter` | `handlers/event.rs` |
| Media upload | `media_upload_rate_limiter` | `api/media.rs` |
| Invite claim | `invite_claim_rate_limiter` | `api/invites.rs` |

WebSocket admission uses a 5-second burst window (`ws_admission_budget`) so
desktop startup — which opens several live subscriptions at once — preserves the
configured average rate while allowing that bounded burst.

⚠️ These are **fixed windows**, which allow up to 2× burst at a window
boundary. A sliding window or token bucket would be a better long-term fit.

**Does NOT:** implement the limiter itself — `buzz-auth` holds only the trait
and the `AlwaysAllowRateLimiter` test stub (gated behind
`#[cfg(any(test, feature = "test-utils"))]`); the Redis-backed implementation
lives in `buzz-pubsub`.

---

### buzz-db — Postgres Event Store

All database access. Uses `sqlx::query()` (runtime, not compile-time macros) — no `.sqlx/` offline cache required.

**Key operations:**

| Module | Responsibility |
|--------|---------------|
| `event.rs` | `insert_event` (ON CONFLICT DO NOTHING), `query_events` (QueryBuilder), `get_event_by_id` |
| `channel.rs` | Channel CRUD, membership management, role enforcement (transactional) |
| `feed.rs` | `query_mentions` (INNER JOIN event_mentions), `query_needs_action`, `query_activity` |
| `workflow.rs` | Full workflow/run/approval CRUD; SHA-256 hashed approval tokens |
| `partition.rs` | Monthly range partitioning for `events` and `delivery_log` tables |
| `dm.rs` | DM channel management |
| `reaction.rs` | Reaction storage and retrieval |
| `thread.rs` | Thread/reply tracking |
| `user.rs` | User profile storage |
| `moderation.rs`, `admin_moderation.rs` | Report queue, bans/timeouts, moderation audit |
| `relay_members.rs` | NIP-43 relay membership roster |
| `git_repo.rs` | Git repository records |
| `push.rs` | NIP-PL push leases, match queue, endpoint state |
| `archived_identities.rs` | Identity archive / unarchive state |
| `product_feedback.rs` | Product feedback sidecar table |
| `api_token.rs` | API token storage and scopes |
| `usage.rs` | Per-community usage accounting |
| `replica_fence.rs` | Read-replica staleness fence |
| `migration.rs` | Startup migration runner |
| `error.rs` | Database error types |

**Channel types:** `Stream`, `Forum`, `Dm`, `Workflow`  
**Member roles:** `Owner`, `Admin`, `Member`, `Guest`, `Bot`  
**Workflow statuses:** `Active`, `Disabled`, `Archived`  
**Run statuses:** `Pending`, `Running`, `WaitingApproval`, `Completed`, `Failed`, `Cancelled`

**Key behaviors:**
- `ON CONFLICT DO NOTHING` for event dedup — returns `(StoredEvent, was_inserted: bool)`.
- Rejects `KIND_AUTH` (22242) and ephemeral (20000–29999) with distinct error variants.
- Transactional role enforcement in `add_member`/`remove_member`/`create_channel` — TOCTOU-safe.
- Soft-delete for channel members: `remove_member` sets `removed_at`; re-adding reverses it.
- Feed hard cap: `FEED_MAX_LIMIT = 100` rows regardless of caller-requested limit.
- `query_mentions` uses `INNER JOIN event_mentions` — normalized table with composite index on `(pubkey_hex, created_at)`.
- Approval tokens: `create_approval` receives the raw token and hashes it internally with SHA-256.
- DDL injection protection in partition manager: allowlist of table names + strict suffix/date validators.

**Does NOT:** cache queries, implement connection pooling logic (delegated to sqlx), or make network calls outside Postgres.

---

### buzz-pubsub — Redis Pub/Sub, Presence, Typing

Manages Redis pub/sub fan-out, presence tracking, and typing indicators. In multi-community mode all tenant-visible keys are prefixed or otherwise partitioned by community (`buzz:{community}:...`) so channel fan-out, presence, typing, and cache invalidation cannot cross hosts.

**Architecture:**

```
Publisher  → pool connection   → PUBLISH buzz:channel:{uuid}
Subscriber → dedicated PubSub  → PSUBSCRIBE buzz:channel:*
                                  → broadcast::channel(4096)
```

The subscriber uses a **dedicated** `redis::aio::PubSub` connection — not from the pool. This is intentional: pool connections cannot hold `PSUBSCRIBE` state.

**Current state:** The subscriber loop is spawned in `buzz-relay/src/main.rs` and populates the broadcast channel. A consumer task subscribes via `pubsub.subscribe_local()`, calls `sub_registry.fan_out()` on each received event, and delivers matches to local WebSocket connections via `conn_manager.send_to()`. Multi-node fan-out is now wired end-to-end. Local-echo deduplication is implemented via `AppState.local_event_ids` — events published by the local relay instance are tracked and skipped when received via the Redis round-trip.

**Reconnection:** exponential backoff 1s → 30s (`backoff_secs * 2`). Backoff resets to 1s only after a clean stream end, not on each reconnect attempt.

**Presence:** `SET buzz:presence:{pubkey_hex} {status} EX 90` — 90-second TTL (3× the 30-second heartbeat interval). Single missed heartbeat does not cause presence flap.

**Typing indicators:**
```
ZADD buzz:typing:{channel_id} {now_unix} {pubkey_hex}
ZREMRANGEBYSCORE buzz:typing:{channel_id} -inf {now - 5.0}
EXPIRE buzz:typing:{channel_id} 60
```
5-second activity window. 60-second key TTL prevents orphaned empty sets.

**Does NOT:** implement the rate limiter. Does NOT store events. `PubSubManager` is not `Clone` — callers use `Arc<PubSubManager>`.

---

### buzz-search — Postgres FTS Integration

Full-text search via Postgres FTS. Events are searchable through the
`events.search_tsv` generated `tsvector` column (populated on insert, indexed
by a GIN index) — there is no separate search service or out-of-band indexer.
Privacy-sensitive kinds are excluded at the storage level (the `search_tsv`
`CASE WHEN kind IN (...)` yields `NULL`, which never matches `@@`). In
multi-community mode every query filter includes `community_id`, so the shared
`events` table is infrastructure, not a cross-community result space; the relay
re-authorizes every candidate hit before returning it.

**Key behaviors:**
- `SearchService::new(pool)` wraps a `PgPool`; `search(&SearchQuery)` runs a
  parameterized FTS query against the `events.search_tsv` GIN index and returns
  `SearchResult` (candidate `SearchHit`s).
- `ChannelScope` makes the channel constraint explicit (`Any` /
  `ChannelLessOnly` / `Channels` / `ChannelsOrChannelLess`), closing the
  ambiguity the old `Option<Vec<Uuid>> + bool` matrix could not express.
- Every query carries `community_id`; the FTS predicate is BitmapAnd-ed with
  the community-leading btree filters so a query never crosses tenants.
- Permission filtering is **caller's responsibility** — `buzz-search` returns
  candidate hits; the relay re-authorizes each one (channel membership, `#p`,
  owner gates) before delivering it.

**Does NOT:** enforce channel membership or access control. Does NOT write
events (indexing is the `search_tsv` generated column on the `events` insert).

---

### buzz-audit — Hash-Chain Audit Log

Tamper-evident append-only log with SHA-256 hash chaining.

**Hash chain:** each entry stores `prev_hash` (hash of the previous entry). In multi-community mode audit heads/chains are per-community; operator metrics may aggregate, but tenant-readable audit verification walks one community chain. `verify_chain()` walks entries and recomputes hashes to detect tampering. Genesis entry uses `GENESIS_HASH` (64 zeros).

**Hash covers:** seq (big-endian bytes), timestamp (RFC3339), event_id, event_kind (big-endian), actor_pubkey, action string, channel_id (16 bytes or 16 zero bytes if None), canonical metadata JSON (BTreeMap for deterministic key ordering), prev_hash.

**Single-writer guarantee:** `pg_advisory_lock` before each transaction. Lock released in all branches including panic (`catch_unwind`).

**10 audit actions:** `EventCreated`, `EventDeleted`, `ChannelCreated`, `ChannelUpdated`, `ChannelDeleted`, `MemberAdded`, `MemberRemoved`, `AuthSuccess`, `AuthFailure`, `RateLimitExceeded`.

**Does NOT:** log `KIND_AUTH` (22242) events — returns `AuditError::AuthEventForbidden` immediately. Does NOT log ephemeral events (they never reach the audit pipeline).

---

### buzz-workflow — YAML-as-Code Automation Engine

Parses, validates, and executes channel-scoped workflow definitions. In multi-community mode workflow definitions, runs, approvals, webhook routes, and schedules inherit the host-derived community and evaluate triggers only against events in that community.

**Workflow definition structure:**
```yaml
name: "Incident Triage"
trigger:
  on: message_posted
  filter: "str_contains(trigger_text, 'P1')"
steps:
  - id: notify
    action: send_message
    text: "P1 incident detected: {{trigger.text}}"
  - id: page
    if: "str_contains(trigger_text, 'production')"
    action: request_approval
    from: "{{trigger.author}}"
    message: "Page on-call?"
```

Note: Both `TriggerDef` and `ActionDef` use serde internally-tagged enums. Triggers use `on:` as the tag field; actions use `action:` as the tag field. Fields are flattened into the parent struct, not nested.

**4 trigger types:** `message_posted`, `reaction_added`, `schedule`, `webhook`

**7 action types:**

| Action | Description |
|--------|-------------|
| `send_message` | Post to the workflow's channel (or override channel) |
| `send_dm` | Direct message to a user (pubkey hex or `{{trigger.author}}`) |
| `set_channel_topic` | Update channel topic |
| `add_reaction` | React to the trigger message |
| `call_webhook` | HTTP POST to external URL (SSRF-protected, redirects disabled, 1 MiB response cap) |
| `request_approval` | Suspend execution; fields: `from`, `message`, `timeout` (default 24h) |
| `delay` | Pause execution (max 300 seconds) |

**Template variables:** `{{trigger.text}}`, `{{trigger.author}}`, `{{steps.ID.output.FIELD}}`. Single-pass resolution (not recursive). Unknown variables left as literal text.

**Condition evaluation:** `evalexpr` with `HashMapContext`. Dot notation converted to underscores (`trigger.text` → `trigger_text`). Custom functions registered: `str_contains`, `str_starts_with`, `str_ends_with`, `str_len`. 100ms timeout prevents adversarial expressions from blocking.

**Concurrency:** `Arc<Semaphore>` with 100 permits. `try_acquire()` — returns `CapacityExceeded` immediately rather than queuing.

**Approval gates:** `request_approval` action returns `StepResult::Suspended` with a generated UUID token, but the engine does not yet persist the token or resume execution — runs that hit an approval gate are marked as failed (🚧 WF-08). `execute_from_step()` exists for future resumption support.

**Cron scheduler:** loop ticks every 60 seconds, evaluates cron expressions with window-based matching, and creates workflow runs for matched triggers. Fully implemented.

**Does NOT:** recursively resolve templates (single-pass only). Does NOT queue workflow runs when at capacity — returns `CapacityExceeded` immediately.

---

### Huddle Audio — WebSocket Opus Relay

Real-time voice lives inside `buzz-relay` (`src/audio/`), not a separate crate. A WebSocket endpoint (`wss://.../huddle/{channel_id}/audio`) authenticates each participant with a NIP-42 challenge, checks channel membership, admits them to an in-memory room, and forwards opaque Opus frames between peers. No external SFU.

**Frame protocol (v2):** 8-byte big-endian header (sequence `u16`, 48 kHz timestamp `u32`, level dBov `i8`, flags `u8`) followed by an opaque Opus payload. Invalid `level_dbov` values are clamped rather than dropped — losing a metric beats losing audio.

**Room state:** an admission guard synchronizes joins against the room's ended flag; soft cap 25 peers (hard cap 255 via `u8` peer index). Per-peer audio uses a bounded channel (drop-on-full); the control channel is separate and never drops join/leave.

**Lifecycle events:** the relay emits Nostr events for participant joined / left and huddle ended; the desktop client emits huddle started and guidelines. When the last peer leaves, the room ends and the channel archives atomically.

**Not yet built:** recording and per-track publishing (the corresponding kinds are reserved, no producer exists).

---

### buzz-relay-mesh — Inter-Relay QUIC Mesh

Transport, membership, and the fenced wire contract for relay-to-relay
communication over [iroh](https://iroh.computer/) QUIC. Gated behind the
`BUZZ_MESH` env seam: `boot_mesh` (`buzz-relay/src/mesh_boot.rs`) is the only
place the relay constructs mesh machinery, and it returns `None` — touching
nothing — when `BUZZ_MESH=off`, so mesh-off deployments stay byte-identical to a
relay built before the module existed.

When enabled, boot binds the iroh endpoint (a boot-unique keypair yields a
boot-unique `RuntimeId`), publishes a relay-key-attested `ReadyRecord` to a Redis
ready registry, starts the readiness-gated heartbeat and the `MeshRuntime` loops
(accept, reconcile/dial, gossip), and spawns a drain watcher — on shutdown,
membership gossips `draining=true`, locally-owned huddle leases are
generation-fenced drained, and the heartbeat clears the registry record.

Consumers reach the mesh exclusively through `MeshHandle` via `AppState::mesh()`;
`None` means "behave exactly like a single-instance relay." The relay-side
session layer (`buzz-relay/src/tunnel/`) owns Redis-fenced ownership, strict
generation validation, and profile-specific routing.

---

### buzz-push-gateway — NIP-PL Push

A **separate binary** implementing a blind, capability-gated push gateway for
the mobile app. "Blind" is the design constraint: the gateway relays
notifications without learning message content.

The relay side is `push_runtime.rs` — a durable matcher and delivery worker
backed by Postgres (`push_leases`, `push_match_queue`, `push_endpoint_state`).
The matcher claims batches (≤64, 30s claim), evaluates stored subscription
filters via `filters_match` + `reader_authorized_for_event`, and backs off
between an idle floor of 250 ms and a ceiling of 2 s so an idle relay is not
issuing a claim transaction four times a second forever. Delivery retries up to
8 attempts, with a poison-job sweep every 30 s kept off the claim path. Both
tasks are enabled as one unit and stay dark without an exact configured gateway
URL, so an undeliverable configuration cannot advertise or accumulate work.

---

### buzz-relay — The Server

Axum WebSocket server. Ties all other crates together. The only crate that imports and orchestrates all subsystems.

**Module map** — the relay hosts several subsystems that have no separate crate:

| Module | Responsibility |
|--------|---------------|
| `handlers/ingest.rs` | Transport-neutral event ingestion (see §4) |
| `handlers/event.rs` | WebSocket EVENT entry, ephemeral dispatch, fan-out helpers |
| `handlers/req.rs`, `close.rs`, `count.rs` | REQ / CLOSE / COUNT |
| `handlers/moderation_*.rs` | Moderation commands, authz, and notices |
| `handlers/relay_admin.rs` | NIP-43 relay admin commands |
| `handlers/push_lease.rs` | NIP-PL subscription leases |
| `handlers/side_effects.rs` | NIP-29 / NIP-25 post-storage side effects |
| `handlers/community_provisioning.rs` | Community creation and lifecycle |
| `handlers/identity_archive.rs` | Identity archive / unarchive |
| `api/git/` | Git smart HTTP, pack cache, CAS publish, policy hook |
| `api/media.rs` | Blossom upload/download |
| `api/invites.rs`, `operator.rs`, `admin/` | Invites, operator APIs, admin console |
| `audio/` | Huddle Opus relay |
| `tunnel/` | Mesh session layer (Redis-fenced ownership, generation validation) |
| `push_runtime.rs` | NIP-PL matcher + delivery worker |
| `mesh_boot.rs` | `BUZZ_MESH` seam — the only place mesh machinery is constructed |
| `tenant.rs` | Row-zero host → community binding |
| `conformance/` | Runtime trace emission for the TLA+ replay checker |
| `storage_sweep.rs`, `metrics.rs`, `telemetry.rs` | Retention sweep, Prometheus, tracing |

**`AppState`** (Arc-wrapped, shared across all connections — key fields shown, not exhaustive):

```rust
pub struct AppState {
    pub db: Db,
    pub audit: Option<Arc<AuditService>>,
    pub pubsub: Arc<PubSubManager>,
    pub auth: Arc<AuthService>,
    pub search: Arc<SearchService>,
    pub sub_registry: Arc<SubscriptionRegistry>,
    pub conn_manager: Arc<ConnectionManager>,
    pub community_connections: Arc<CommunityConnectionRegistry>,
    pub workflow_engine: Arc<WorkflowEngine>,
    pub conn_semaphore: Arc<Semaphore>,        // connection limit
    pub handler_semaphore: Arc<Semaphore>,     // 1024 concurrent handlers
    pub git_semaphore: Arc<Semaphore>,
    pub media_upload_semaphore: Arc<Semaphore>,
    pub relay_keypair: nostr::Keys,            // relay identity
    pub admission_rate_limiter: Arc<RedisRateLimiter>,
    pub observer_rate_limiter: Arc<ScopedRateLimiter>,
    pub media_upload_rate_limiter: Arc<ScopedRateLimiter>,
    pub audit_tx: Option<mpsc::Sender<buzz_audit::NewAuditEntry>>,
    pub tracer: Arc<dyn buzz_conformance::Tracer>,
    pub mesh: Arc<OnceLock<MeshHandle>>,       // None when BUZZ_MESH=off
    // Community-keyed moka caches (local-echo dedup, membership,
    // accessible channels, channel visibility, observer/author type):
    pub local_event_ids: Arc<moka::sync::Cache<(CommunityId, [u8; 32]), ()>>,
    pub membership_cache: Arc<moka::sync::Cache<(CommunityId, Uuid, Vec<u8>), bool>>,
    // + config, redis_pool, git_store, media_storage, audio_rooms, shutdown state
}
```

Every cache key leads with `CommunityId` — caches are infrastructure shared
across tenants, never a shared *result* space.

**`ConnectionState`** (per-connection):

```rust
pub struct ConnectionState {
    pub auth_state: RwLock<AuthState>,
    pub subscriptions: Mutex<HashMap<String, Vec<Filter>>>,
    // + send_tx, cancel token
}
pub enum AuthState { Pending { challenge: String }, Authenticated(AuthContext), Failed }
```

**HTTP endpoints:**

| Method | Path | Handler |
|--------|------|---------|
| GET | `/` | WebSocket upgrade or NIP-11 relay info |
| GET | `/info` | NIP-11 relay info |
| GET | `/.well-known/nostr.json` | NIP-05 identity |
| GET | `/health` | Health check |
| GET | `/_liveness` | Liveness probe |
| GET | `/_readiness` | Readiness probe |
| POST | `/events` | Submit a signed Nostr event over HTTP (same ingest path as WebSocket `EVENT`) |
| POST | `/query` | Query Nostr events over HTTP with NIP-01 filters |
| POST | `/count` | Count Nostr events over HTTP with NIP-45 filters |
| POST | `/hooks/{id}` | Workflow webhook trigger (secret-authenticated) |
| PUT | `/upload`, `/media/upload` | Upload media blob (Blossom) |
| GET/HEAD | `/media/{sha256_ext}` | Retrieve/probe media blob |
| GET | `/git/{owner}/{repo}/info/refs` | Git smart HTTP advertisement |
| POST | `/git/{owner}/{repo}/git-upload-pack` | Git smart HTTP fetch |
| POST | `/git/{owner}/{repo}/git-receive-pack` | Git smart HTTP push |
| POST | `/internal/git/policy` | Internal git hook policy check |
| GET | `/huddle/{channel_id}/audio` | Huddle audio WebSocket (NIP-42 + membership) |
| GET/POST | `/operator/communities` | List owned / provision communities |
| POST | `/operator/communities/archive`, `/unarchive`, `/transfer` | Community lifecycle |
| GET | `/operator/communities/availability` | Host availability check |
| POST | `/api/invites` | Mint a relay invite (owner/admin) |
| POST | `/api/invites/claim` | Claim an invite (membership-gate exempt) |
| POST | `/api/invites/accept-policy` | Record join-policy acceptance |
| GET | `/api/join-policy`, `/terms`, `/privacy` | Join policy + standalone policy pages |
| GET | `/moderation/reports`, `/audit`, `/restricted` | Moderation queue reads (NIP-98 + mod-authz) |
| GET | `/api/admin/v1/reports`, `/reports/{id}` | Admin console: report queue |
| GET | `/api/admin/v1/feedback`, `/feedback/{id}`, `/feedback/{id}/attachments/{sha256}` | Admin console: product feedback |

The admin surface under `/api/admin/v1` is mounted only when
`config.admin` is set, is host-checked (`is_admin_host`) ahead of the public web
bundle so it can never fall through to it, and carries a 1 KB body limit plus
`security_headers` middleware. A separate health-only router (no CORS, no
metrics, no body limit) serves `/_liveness`, `/_readiness`, `/_status`, and
`/_mesh` on the dedicated health port.

**Constants:**

| Constant | Default | Env override | Purpose |
|----------|---------|--------------|---------|
| `DEFAULT_MAX_FRAME_BYTES` | 512 KiB | `BUZZ_MAX_FRAME_BYTES` | Max WebSocket frame size |
| `MAX_SUBSCRIPTIONS` | 1024 | — | Per-connection subscription limit |
| `MAX_HISTORICAL_LIMIT` | 2,000 | — | Per-filter historical query cap |
| `max_concurrent_handlers` | 1024 | `BUZZ_MAX_CONCURRENT_HANDLERS` | Concurrent EVENT/REQ handlers |
| `max_connections` | 10,000 | `BUZZ_MAX_CONNECTIONS` | Connection semaphore capacity |
| `send_buffer_size` | 1,000 | `BUZZ_SEND_BUFFER` | Per-connection send channel depth |

**Does NOT:** implement business logic — delegates to the appropriate crate for every operation.

---

### buzz-acp — Agent Communication Protocol Harness

Standalone binary that bridges Buzz relay events to AI agents via the [Agent Communication Protocol](https://agentclientprotocol.com/) (ACP).

**Architecture:**

```
Buzz Relay ──WS──→ buzz-acp ──stdio (ACP/JSON-RPC)──→ Agent (goose/codex/claude)
```

`buzz-acp` spawns AI agent subprocesses (1–32, default 1), connects to the relay via WebSocket with NIP-42 auth, discovers channels via REST API, and queues `@mention` events per channel. At most one prompt is in-flight per channel. Queued events are batched into a single prompt sent via `session/prompt` over ACP.

**Key modules:**

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `relay.rs` | 3,143 | WebSocket + REST relay connection, NIP-42 auth |
| `queue.rs` | 2,565 | Per-channel event queue, batching, dedup |
| `main.rs` | 2,457 | Event loop, pool orchestration, heartbeat |
| `pool.rs` | 2,253 | N-agent pool, claim/return lifecycle |
| `config.rs` | 1,903 | CLI/env/TOML configuration |
| `acp.rs` | 1,785 | ACP client, stdio JSON-RPC, timeouts |
| `filter.rs` | 814 | Subscription rules, evalexpr filtering |

**Key behaviors:**
- Pool of 1–32 agent subprocesses with claim/return lifecycle.
- Per-channel queuing: at most one prompt in-flight per channel; subsequent @mentions queue until the agent responds.
- Crash recovery: agent subprocess crashes are detected and the agent is respawned.
- Depends on `buzz-core` (kind constants) and `buzz-sdk` (relay/REST utilities).

**Does NOT:** persist state.

---

### buzz-admin — Operator CLI

Subcommands:

| Subcommand | Purpose |
|------------|---------|
| `add-member` | Add a pubkey to the relay membership list (`--pubkey`, `--role`); accepts npub or hex; publishes kind:13534 roster |
| `remove-member` | Remove a pubkey from the relay membership list (`--pubkey`, optional `--role` guard); publishes kind:13534 roster |
| `list-members` | List all relay members |
| `generate-key` | Generate a new Nostr keypair (for bootstrapping) |
| `reconcile-channels` | Emit kind:39000/39002 discovery events for channels missing them (idempotent) |

The `buzz-admin` binary is shipped in the relay Docker image (`/usr/local/bin/buzz-admin`) and is the recommended way to manage relay membership in production. Use `./run.sh add-member`, `./run.sh remove-member`, and `./run.sh list-members` in Docker Compose deployments.

---

### buzz-test-client — Integration Test Harness

**`BuzzTestClient`** wraps a WebSocket connection with a `VecDeque<RelayMessage>` buffer for message interleaving. Methods: `connect`, `connect_unauthenticated`, `authenticate`, `send_event`, `send_text_message`, `subscribe`, `close_subscription`, `recv_event`, `collect_until_eose`, `disconnect`.

**Test coverage:**

| File | Tests | Scope |
|------|-------|-------|
| `tests/e2e_relay.rs` | 38 | WebSocket protocol (auth, subscriptions, filters, limits, NIP-11) |
| `tests/e2e_event_reminder.rs` | 29 | Event reminders |
| `tests/e2e_nostr_interop.rs` | 25 | Nostr interop: NIP-50 search, NIP-10 threads, NIP-17 gift wraps, DM discovery |
| `tests/e2e_persona.rs` | 24 | Persona packs |
| `tests/e2e_media_extended.rs` | 21 | Extended media scenarios |
| `tests/e2e_human_edit_agent_content.rs` | 19 | Human edits of agent-authored content |
| `tests/conformance_multitenant.rs` | 18 | Multi-community conformance replay |
| `tests/e2e_long_form.rs` | 8 | Long-form content |
| `tests/e2e_media.rs` | 7 | Media upload/download (Blossom) |
| `tests/e2e_media_video.rs` | 7 | Video media |
| `tests/e2e_managed_agent.rs` | 5 | Managed agent lifecycle |
| `tests/e2e_user_status.rs` | 5 | User status |
| `tests/e2e_mesh_llm.rs` | 4 | Mesh LLM routing |
| `tests/nip42_host_binding_live.rs` | 4 | NIP-42 host/community binding |
| `tests/e2e_team.rs` | 3 | Teams |
| `tests/e2e_git.rs` | 2 | Git smart HTTP |

Most e2e tests are `#[ignore]` — they require a running relay. Total: **219 e2e tests**.

`src/main.rs` is a manual testing CLI (`buzz-test-cli`) with `--send`, `--subscribe`, `--channel`, `--url`, `--kind` flags.

Defines `parse_relay_message`, `OkResponse`, `RelayMessage` directly in `src/lib.rs`.

---

## 7. Security Model

Every security-sensitive operation uses an explicit, verified pattern. No implicit trust.

### Authentication

| Concern | Mechanism |
|---------|-----------|
| NIP-42 timestamp | ±60 second tolerance — prevents replay attacks |
| AUTH events | Never stored in Postgres, never logged in audit chain |
| NIP-98 HTTP Auth | Schnorr-signed `kind:27235` events — URL and method verification |

### Input Validation

| Concern | Mechanism |
|---------|-----------|
| Schnorr signatures | `verify_event()` in `buzz-core` — every event verified before storage |
| Event ID | SHA-256 of canonical serialization verified independently of signature |
| Frame size | `max_frame_bytes` (default 512 KiB) — oversized frames rejected, connection closed |
| Event content | 256 KB cap, enforced in `ingest_event` |
| Timestamp drift | ±15 minutes from server time, enforced in `ingest_event` |
| Search event IDs | 64-char hex validation before URL construction — prevents path injection |
| Workflow step IDs | Alphanumeric + underscore only — prevents evalexpr variable injection |
| Partition names | Allowlist of table names + strict suffix/date validators — prevents DDL injection |

### SSRF Protection

`is_private_ip()` in `buzz-core` covers:
- IPv4: unspecified (0.0.0.0/8), loopback (127.0.0.0/8), private (10/8, 172.16/12, 192.168/16), link-local (169.254/16), CGNAT (100.64/10), benchmarking (198.18/15), broadcast (255.255.255.255)
- IPv6: loopback (::1), ULA (fc00::/7), link-local (fe80::/10), multicast (ff00::/8), documentation (2001:db8::/32)
- IPv4-mapped IPv6 (::ffff:0:0/96) — recursively checks the embedded IPv4 address

Applied in: `buzz-workflow` (CallWebhook action), `buzz-core` (shared utility).

### Audit Integrity

- Hash chain: each entry's SHA-256 covers all fields including `prev_hash` — tampering any entry breaks all subsequent hashes
- Canonical JSON: `BTreeMap` for deterministic key ordering — hash is reproducible
- Single-writer lock: `pg_advisory_lock` — prevents concurrent writes from breaking the chain
- Panic-safe: `catch_unwind` ensures lock release even on panic

### Access Control

- Channel membership is the only gate — enforced by the relay at every operation
- REQ handler checks access before subscription registration — no race window for private channel leaks
- TOCTOU-safe membership operations: all check-then-modify sequences run inside Postgres transactions
- Approval tokens: UUID (CSPRNG), stored as SHA-256 hash, single-use enforced with `AND status = 'pending'` in UPDATE

### Webhook Security

- Workflow webhooks: constant-time XOR comparison of stored UUID secret (not HMAC — compares the secret directly, not a body MAC)
- Outbound webhooks (CallWebhook): SSRF protection + redirects disabled + 1 MiB response cap

---

## 8. Infrastructure

Docker Compose provides the full local development stack. All services include health checks and resource limits.

### Services

| Service | Image | Port | Purpose |
|---------|-------|------|---------|
| Postgres | `postgres:17-alpine` | 5432 | Primary event store — events, channels, tokens, workflows, audit; full-text search (`search_tsv` GIN) |
| Redis | `redis:7-alpine` | 6379 | Pub/sub fan-out, presence (SET EX), typing (sorted sets) |
| Adminer | `adminer` | 8082 | DB web UI (dev only) |
| Keycloak | `quay.io/keycloak/keycloak:26.0` | 8180 → 8080 | OIDC provider, `start-dev` with in-memory DB (dev only) |
| MinIO | `minio/minio` | 9000 (API), 9001 (console) | S3-compatible object storage (media) |
| `minio-init` | `minio/mc` | — | One-shot bucket bootstrap for MinIO |
| Prometheus | `prom/prometheus` | 9090 | Metrics collection |

### Postgres Schema (key tables)

| Table | Purpose |
|-------|---------|
| `events` | All stored Nostr events; monthly range-partitioned by `PARTITION BY RANGE` on `created_at`; multi-community mode keys every tenant-visible event by `community_id` |
| `channels` | Channel records (type, visibility, canvas, topic); `community_id` is immutable after creation in multi-community mode |
| `channel_members` | Membership with roles; soft-delete via `removed_at` |
| `workflows` | Workflow definitions (YAML stored as canonical JSON); scoped by community in multi-community mode |
| `workflow_runs` | Execution records with trigger context and trace |
| `workflow_approvals` | Approval gates (token stored as SHA-256 hash) |
| `audit_log` | Hash-chain audit entries; per-community chain/head in multi-community mode |
| `delivery_log` | Delivery tracking (partitioned; Rust module pending) |

### Redis Key Patterns

| Pattern | Type | TTL | Purpose |
|---------|------|-----|---------|
| `buzz:channel:{uuid}` | Pub/Sub channel | — | Event fan-out (single-community form; shared multi-community Redis must use `buzz:{community}:channel:{uuid}` or equivalent) |
| `buzz:presence:{pubkey_hex}` | String | 90s | Online/away status (single-community form; shared multi-community Redis must scope by community) |
| `buzz:typing:{channel_uuid}` | Sorted Set | 60s | Active typers (5s window; shared multi-community Redis must scope by community) |

### Full-Text Search (Postgres FTS)

Search runs over the `events.search_tsv` generated `tsvector` column on the
`events` table (no separate collection or service). The column is populated on
insert — `to_tsvector('simple', content)` — and excludes privacy-sensitive
kinds via `CASE WHEN kind IN (1059, 30300, 30622) THEN NULL`, so those rows are
storage-level unsearchable (a `NULL` tsvector never matches `@@`). A GIN index
(`idx_events_search_tsv`) backs the `@@` probe; in multi-community mode the
community-leading btree filters BitmapAnd with the GIN probe so every query is
fenced to its `community_id`.

---

## 9. Known Limitations

These are verified gaps in the current implementation — not design aspirations.

| # | Limitation | Detail |
|---|-----------|--------|
| 1 | **No sqlx offline query cache** | Uses `sqlx::query()` (runtime) not `sqlx::query!()` (compile-time). No `.sqlx/` directory. Queries are not validated at compile time. |
| 2 | **Rate limiting uses fixed windows** | `RedisRateLimiter` (`buzz-pubsub`) is enforced at the WebSocket, HTTP bridge, observer, media-upload, and invite-claim seams. The windows are fixed, so up to 2× burst is possible at a boundary; a sliding window or token bucket would be stricter. |
| 3 | **No dedicated typing REST endpoint** | Typing indicators (kind 20002) are delivered via both local fan-out and Redis pub/sub (cross-node). There is no REST endpoint to query current typers — `/api/presence` returns online/away status only, not typing state. |
| 4 | **Huddle recording/tracks not built** | Voice, room lifecycle, and join/leave/end events are wired (see Huddle Audio above). Recording and per-track publishing have reserved kinds but no producer yet. |
| 5 | **Approval gates not wired end-to-end** | The executor returns `StepResult::Suspended` and the relay has grant/deny API endpoints with DB CRUD, but the engine intercepts before creating `WaitingApproval` rows — runs that hit an approval gate are marked as Failed (🚧 WF-08). |
| 6 | **Workflow actions partially stubbed** | The `send_dm` and `set_channel_topic` workflow actions are in the schema but return `NotImplemented` — a run that reaches one fails at execution (🚧 WF-07). |
