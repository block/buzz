# Multi-Tenant Buzz Relay: A Formal Specification

`draft`

## Abstract

This document specifies the data and authorization model that lets one shared
Postgres instance, served by N stateless relay processes, host M independent
**communities** without one community observing or acting on another, and gives
a formal proof of its safety properties. It proves two families of property:
**isolation** — a community is *non-interfering* with every other community
across the relay's logical interface (query results, authorization decisions,
emitted errors, and audit-chain contents) — and **authorization soundness** — no
credential, signature, or forged event lets an actor cross a community boundary.

Today a Buzz relay *process* is the security boundary: one `DATABASE_URL`, one
relay keypair, one relay-global `relay_members` table, with `channel_id` (the
`h` tag) as the only sub-relay locality. The model proven here demotes the relay
process to stateless compute and elevates a new **community** entity to the
tenant/security boundary, carried as a `community_id` on every scoped row. That
move collapses a process-level boundary into a row-level one. The contribution of
this document is the formal characterization that the collapse loses nothing —
proven *relative to* explicitly stated axioms about Postgres row-level security,
Schnorr/NIP-98, a collision-resistant hash, and the relay's own
`channel_id → community_id` resolution.

The architecture is not novel as a *pattern*: row-level multi-tenancy with a
discriminator column and row-level security (RLS) is established practice (see
§Prior Art). The contribution is the **formal treatment** — stating tenant
isolation as non-interference encoded as a label-flow invariant (not a
`WHERE community_id = $1` predicate), mechanizing it (TLA+ for the
concurrency/serving model, Tamarin for the authorization protocol under a
Dolev-Yao adversary), and gating every invariant on a mutation test so the proof
is non-vacuous.

## Scope and Non-Goals

This specification proves **safety** ("nothing bad happens"). It deliberately
does **not** prove:

- **Liveness or performance.** That a query meets a latency budget, or that a hot
  partition does not throttle, is empirical — characterized by the perf rig, not
  by theorem.
- **Postgres's internal correctness.** RLS enforcement, MVCC snapshot isolation,
  and `ON CONFLICT DO NOTHING` semantics are trusted and stated as axioms
  (§Axioms). We prove our *composition* on top of them; we do not reprove them.
- **Cryptographic primitives.** Schnorr signature unforgeability (BIP-340), the
  NIP-98 request binding, and second-preimage resistance of the event-id hash are
  the Tamarin model's equational theory, not reproven.
- **Physical-resource isolation.** Communities share an id space, time
  partitions, a connection pool, and a CPU. The proof covers the *logical*
  interface; bandwidth-limited physical channels are a named, explicit carve-out
  (§Isolation Boundary, class C1).
- **Above-the-interface client leakage.** The proof boundary is the relay's
  observational interface. If a client (a multi-tenant UI, an NIP-19 `nevent`
  share, a screenshot, a leaked log) surfaces a user's own event ids from
  community A while that user is also a member of B, the user then holds an A-id
  out-of-band and can probe the existence oracle from a B connection. The
  composite-index closure (A-RLS-5) means the probe still reveals nothing — B's
  write at that id is a fresh `(community_id, id)` key — but we name this surface
  explicitly: closing it for any *weaker* index shape is above the interface and
  is the client's obligation, not the relay's.

Stating this boundary is part of the claim. "Provably isolated" without naming
the trust boundary does not survive scrutiny; "isolation is machine-checkable
relative to these stated axioms, with every shared logical channel either closed
in-model or closed by a named axiom" does.

## System Model

A **community** `C` is the tenant/security boundary. It owns: a set of channels,
a membership relation, a signing keypair, a token namespace, workflows, an audit
hash chain, and the messages scoped to it. A community is a durable row in a
`communities` table; creating one is an INSERT, never DDL.

The shared store holds three tiers:

- One **canonical message log** `L`: an append-only table keyed by
  `(community_id, created_at, id)`. Every message carries the `community_id` of
  the community it belongs to. Append is idempotent
  (`ON CONFLICT (community_id, created_at, id) DO NOTHING`).
- A **tenant-scoped control plane**: relational, ACID tables — `channels`,
  `channel_members`, `api_tokens`, `workflows`, audit entries — each carrying
  `community_id`, kept relational because authorization needs synchronous current
  state.
- **Disposable projections**: mentions, thread metadata, reactions, full-text
  search — each `community_id`-keyed, rebuildable from `L`, never authoritative.

A **relay process** is stateless compute. It owns no community data; any process
can serve any community, and N processes share the store.

A **connection** is bound to an **actor** (a pubkey, authenticated via NIP-42 on
WebSocket or via a NIP-98-minted bearer token on REST). Every connection
operation is evaluated under a **TenantContext** `⟨community_id, actor⟩`. The
`community_id` is **resolved by the relay**, never read from the client-supplied
`h` tag or claimed community. For a **channel-bearing** operation it is the
community of the channel the operation names (`resolve : channel_id →
community_id`, an indexed lookup the relay owns under the same transaction
snapshot as the operation), and the connection's **host** must agree with it —
an A-host presenting a B-channel event is rejected fail-closed, never acted on as
B. For a **channel-less** operation (profiles, DMs,
long-form, status, read-state, lists — no `h` tag) it is the community bound to
the connection's **host** at establishment (`resolve_host : host → community_id`,
lifting today's per-relay URL identity up to the community); an unmapped host
binds to no community and the connection is rejected fail-closed. The composed
resolver is `ResolveTenant(req, event)` (see P-RESOLVE-HOST).

Two operation classes act on the store:

- **Serve(ctx, q)** — a read (REQ / REST GET, including direct `ids` lookup,
  `#e`/`#a` tag filters, metadata/member discovery, and projection reads). Returns
  rows and derived results matching `q`, confined to `ctx.community_id`.
- **Accept(ctx, e)** — a write (EVENT / REST POST). Appends `e` to `L` (or mutates
  control-plane state) under `ctx.community_id`, after an authorization decision
  over current control-plane state.

A community is either **allowlisted** or **open**. An allowlisted community admits
actors only via a signed NIP-43 member list (§Authorization, S7); an **open**
community (one with no member-pubkey allowlist) auto-registers any authenticated
npub on AUTH — but the registration is stamped to the **host-resolved** community,
never a client-claimed one, so "open" widens *who* may join, never *which*
community they join. Two further control-plane writes are first-class: **channel
creation** stamps a fresh channel atomically from `HostCommunity[host]` (the client
supplies no community id, and the stamp is immutable thereafter), and **no-`#h`
reads** — the kinds-only feed read and the `#e`-only aux read (reactions, edits,
deletes, thread metadata) — resolve their community from the connection's host and
are gated on host-community admission like every other channel-less operation.
These surfaces are modeled, not asserted: see §Isolation (I5) and
§Authorization (S5/S8).

**The resolved `community_id` is the sole tenant authority.** The `h` tag on a
wire event is a *routing hint* a client asserts; it is never the commit point of
tenancy. This is the **confused-deputy** hazard (Hardy 1988): the relay holds
broad authority over a shared DB, and a client supplies an ambient name; if the
relay acts on its broad authority under the client's name, the client escapes its
community. The defense is capability discipline — authority is bound to the
*resolved object* `(community_id, channel_id, capabilities)`, never to a
caller-supplied tag. The model treats the `h` tag as adversary-controlled and
proves it is not load-bearing (Theorem I2 / S1).

## Isolation Boundary

Tenant isolation is stated as **non-interference**: for any two executions equal
on community B's inputs and initial B-visible state, B's observable outputs are
equal regardless of community-A-only actions (Goguen–Meseguer 1982; the
concurrent variant is observational determinism). A `WHERE community_id = $1`
row-return invariant is only *one projection* of this theorem — it implies
nothing about timing, errors, uniqueness collisions, projection rebuild, or the
auth gate. Two execution traces cannot be expressed directly in TLA+; the
standard tractable encoding is a **label-flow invariant**: every state element
(message row, membership, projection cell, in-flight query, emitted error, audit
entry) carries the community label it originated from, and the single-run safety
invariant is *"no high-labeled value ever flows into a low-labeled observation."*
This encoding forces enumeration of every state element's label, which is what
catches the projection-rebuild and error-surface channels that a predicate hides.

Shared channels split into two classes:

**(C1) Bandwidth-limited physical channels — declared, out of scope.** Buffer
cache, autovacuum, planner statistics, partition right-edge throughput, and
connection-pool tail latency are shared. A co-tenant can measure these as timing;
the channel is bandwidth-bounded and orthogonal to the threat model
(cross-tenant data leak, privilege escalation, audit forgery). We declare this
class as git-on-s3 declares physical pack pruning: named, with a deferred future
bandwidth bound. **We do not claim timing non-interference.**

**(C2) Logical channels — in scope, enumerated, each closed.** These are *not*
carve-outs; a B-scoped connection can observe them at the interface, so each must
be closed in-model or by a named axiom:

1. **Event-id existence oracle.** `INSERT … ON CONFLICT DO NOTHING` on the
   content-hash id: a B-writer observing zero rows affected learns *some* tenant
   wrote that id. Closed by **A-RLS-5** (§Axioms): the uniqueness constraint is
   composite over `(community_id, …, id)`, so a B-scoped write at an id A already
   holds gets a *fresh* key, not a conflict — B's rows-affected count is a
   function of B's own state alone, never A's. **A_HASH** is the *supporting*
   axiom: it additionally rules out the adversarial-search variant (B cannot
   *find* a fresh event hashing to a chosen id). Note the residual: A_HASH says
   nothing about ids B already *knows* out-of-band (NIP-19 `nevent` shares,
   multi-tenant client UIs that surface a user's own ids across communities) —
   that exposure is closed by the composite index, not the hash, and any
   above-the-interface client surface that leaks a user's A-ids while they are
   also in B is a named residual in §Scope and Non-Goals, not a relay-closed channel.
2. **Constraint-violation error surface.** Postgres errors can leak constraint
   names, conflicting tuples, and columns. Closed by a fixed **sanitized error
   alphabet** and the structural obligation that the relay emits only errors from
   that alphabet (an implementation code-fence, proven relative to it).
3. **Projection rebuild path.** A rebuild touches every community's events by
   construction. Closed by the invariant that rebuild writes server-side
   projection tables only and **never serves rows** to a tenant-scoped
   connection; a tenant query concurrent with a rebuild sees its own rows or none.
4. **Unauthenticated global surface.** The NIP-11 relay information document at
   `/` is unauthenticated and tenant-unscoped by construction; no B-scoped
   connection, no `c.scope`, no label exists, so the labeling invariant does not
   reach it. Closed by a **typed-input code-fence**: the doc-build function
   consumes only relay-static configuration types — no database handle, no tenant
   context, no audit service. Today `RelayInfo::build`
   (`crates/buzz-relay/src/nip11.rs:122`) takes only static inputs and
   `nip11_facts` (`:176`) reads only `state.config`/`state.relay_keypair`, so the
   surface is clean — but by *current code*, not by the proof; adding a
   `total_events` counter is one `&PgPool` argument away and the labeling
   invariant catches none of it. This is the same enforcement class as the Σ_err
   alphabet (C2.2) — a typed constraint at a seam, lintable over `build`'s
   signature — but disjoint: Σ_err governs *what symbols leave on authenticated
   paths*, C2.4 governs *what state populates unauthenticated paths*. Any future
   unauthenticated relay-level endpoint (NIP-66 monitoring, health probes that
   expose counters) lives under C2.4 by default.

The numeric COUNT (NIP-45) and EOSE cardinality channels are deliberately *not*
on this list: they are closed by the same label propagation as event rows (a
count is `|{B-labeled rows matching the filter}|`), so they belong in the typed
interface, not as distinct C2 mechanisms. The C2 list is the index of *distinct
closure mechanisms* — A_HASH, the Σ_err alphabet, the rebuild behavioral
invariant, and the C2.4 typed-input fence — not the index of channels.

**(C3) Historical writes after revocation — declared, out of scope.** The
admission fence (I5, `Inv_AdmissionFence`) governs **current** capability: it
proves that no membership or channel-less read capability survives for an actor
not currently admitted to that community. Revocation (`RevokeMember`) removes the
current `admittedMembers` row and therefore the capability, but it does *not*
relabel or delete rows the actor wrote while admitted — those historical writes
retain their original community label and remain present. This is sound and
intended: the property we mechanize is "current membership and read capability
track current admission," not "writes are retroactively un-admitted." We declare
this as C1 declares physical timing: named, with retroactive-write redaction left
to an operator data-lifecycle surface outside the isolation model. **We do not
claim historical writes are revoked when a member is revoked.**

### The typed observational interface

The non-interference theorem is stated *over an interface*: the exclusive set of
observations a **B-scoped connection** (one whose *resolved* community is B) can
make. Enumerating this set is load-bearing — a `WHERE community_id = $1` invariant
silently omits cardinality, error, status-code, and global-document channels.
**Any observation not in this set is either C1 (declared) or a model violation.
There is no third category.** Each entry below names its code seam so the TLA+
model, the Tamarin model, and the red-team audit reference the same surface.

**O.WS — WebSocket transport** (`crates/buzz-relay/src/protocol.rs:180-215`). The
relay emits exactly these client-bound messages:

- **`O.WS.EVENT(sub_id, event)`** — a delivered Nostr event. Its `content` is
  high-labeled at the row's community; `e`/`p`/`q` tag references inherit the
  row's label (they may *name* globally-existing ids, but the row reaches B only
  if B-labeled).
- **`O.WS.EOSE(sub_id)`** — end-of-stored-events. The *count* of preceding events
  is the cardinality of B-visible rows matching the filter; it must be a function
  only of B-labeled state.
- **`O.WS.OK(event_id, accepted, message)`** — write ack. `event_id` echoes the
  submission (benign); `accepted` is a function of (validity, signature, resolved
  scope, dedup) over B-labeled state only; `message` is drawn from the sanitized
  alphabet `Σ_err` (the C2.2 seam — the current `String` type admits any value).
- **`O.WS.NOTICE` / `O.WS.CLOSED`** — out-of-band and sub-termination strings;
  same `Σ_err` constraint (`connection.rs:307,326`).
- **`O.WS.AUTH(challenge)`** — NIP-42 challenge; a fresh nonce, function of relay
  randomness only, never of any tenant's writes.
- **`O.WS.COUNT(sub_id, n)`** — NIP-45 count (`protocol.rs:213`). `n` is a numeric
  channel: even under row confinement, a count touching non-B rows leaks A's
  cardinality. The rule: `n` is the count of B-labeled rows matching the filter,
  full stop.

**O.REST — HTTP API surface.**

- **`O.REST.BODY`** — JSON response: row content, projection results, and audit
  entries (`crates/buzz-audit/src/service.rs:get_entries`) must all be B-labeled.
- **`O.REST.META`** — status code, headers, structured error envelope. The status
  code is itself observable: `IngestError::{Rejected,AuthFailed,Internal}` →
  `400/401|403/500` (`handlers/ingest.rs:138-146`) must be a function of
  {request, B-labeled state}, never of A's state.

**O.AUTH — auth verdict.** The Boolean "did this pass the gate," observable via
`O.WS.OK.accepted` and `O.REST.META.status`. It is a function of (submitted
credentials, server-side resolution `channel_id → community`, B-labeled
membership/token/policy state). The *claimed* community never appears in this
function — only the *resolved* one. (Theorem S1.)

**O.AUDIT — audit chain.** `get_entries(scope=B)` returns only B-chain entries;
`verify_chain(scope=B)` is decidable from B-labeled entries alone; compromise of
A's chain key does not affect B's. (Theorem S4.)

**O.NIP11 — relay info document (`/`).** Global and unauthenticated, so by
construction it *cannot* be tenant-labeled — therefore its content must be a
function of relay-static configuration only. `supported_nips` is fine;
`total_events` would be a cross-tenant leak.

Everything outside this set is **C1** (wall-clock latency, buffer-cache hit rate,
planner choice, autovacuum, partition right-edge throughput, pool saturation,
memory/fd/scheduler effects — declared, bandwidth-bounded) or **closed by axiom**
(the `INSERT … ON CONFLICT DO NOTHING` id-existence oracle at `event.rs:151`,
closed by A_HASH).

### Label-propagation rules

The labeling discipline that makes non-interference a *single-run* safety
invariant (every state element carries a community label; the invariant is "no
high-labeled value flows into a low observation"):

- **L1 — Source label.** Every event row carries `community_id`, set by the
  server-side resolver at insert time via `ResolveTenant`. For a **channel-bearing**
  event the label is `resolve(channel_id)`; for a **channel-less** event
  (`kind:0` profiles, `1059` DMs, `30023`/`30174`/`30315`/`30078`, lists —
  `channel_id = NULL`) the label is the connection's host-bound community
  `resolve_host(connection.host)`, with the token stamp required to *agree* (never
  to *supply* it). The `h` tag is **not** the label source, and neither is the
  client-claimed community. (Resolution is a fence — see P-RESOLVE and
  P-RESOLVE-HOST.)
- **L2 — Projection inheritance.** Each projection row (`event_mentions`,
  `thread_metadata`, `reactions`, FTS) inherits its source event's label; rebuild
  = replay of labeled source rows, so rebuilds preserve labels by construction.
- **L3 — Audit partitioning.** N independent chains, one per community label;
  community-scoped writers only; no cross-chain reference, no global "latest" head.
- **L4 — Auth-verdict label.** The allow/deny verdict carries the **resolved**
  community label, never the **claimed** one.
- **L5 — Token stamp.** A NIP-98 token has exactly one community stamp, assigned
  at mint from the resolved channel set; a mint resolving to >1 community is
  rejected fail-closed (S2). The token's label *is* its stamp.
- **L6 — Connection scope.** A connection has exactly one resolved community at a
  time, **bound from its host** (`resolve_host(connection.host)`) at establishment
  before any handler runs; re-scoping requires a new connection to a different
  host; all its observations inherit that scope. An unmapped host binds to no
  community and is rejected fail-closed (P-RESOLVE-HOST), never defaulted.
- **L7 — Error label.** A finite, statically-declared alphabet `Σ_err` governs the
  *authenticated, tenant-scoped* WS error surface: every `O.WS.OK.message`,
  `O.WS.NOTICE`, and `O.WS.CLOSED` is drawn from it (the 9 NIP-01-reachable
  prefixes — `auth-required`, `restricted`, `invalid`, `duplicate`, `pow`,
  `rate-limited`, `blocked`, `error`, `frame-too-large`). Emitting a non-`Σ_err`
  string is a structural code violation (the C2.2 code-fence — a lint, not a model
  property). Today `RelayError::Database(#[from] buzz_db::DbError)` (`error.rs:11`)
  is the seam. The *unauthenticated/REST* error surface (`not-found`,
  `bad-request`) is a **distinct fence** — C2.4's typed-input constraint, not
  `Σ_err` — because it has no tenant scope and no label, so it sits outside the
  labeling invariant entirely. One Rust enum may back both for ergonomics, but the
  model treats them as two alphabets closed by two mechanisms.
- **L8 — No injection.** Per L7, A-labeled state cannot influence *which* `Σ_err`
  symbol B observes.

In one line: *for every reachable state `s`, every B-scoped connection `c`, and
every observation `o ∈ O.* ∪ Σ_err` emitted to `c`, `o` is a deterministic
function of (B-labeled state in `s`, `c`'s request history, relay-static config);
no A-labeled element is an input to `o`.* This is what the TLA+ model encodes —
strictly stronger than row-equality, because it forces enumeration of every
observation channel above.

## Axioms

The proof holds *relative to* the following. Each is a documented property of
Postgres / the crypto primitives, and a testable assumption admitted per
deployment (§Conformance).

### Row-level security (the fail-closed backstop)

Postgres RLS is fail-closed **only** under specific configuration (PostgreSQL
manual, "Row Security Policies"). We state the configuration as obligations:

- **(A-RLS-1)** Every queryable tenant-bearing table has RLS enabled with a
  restrictive policy `community_id = current_setting('app.community_id')::uuid`,
  and no permissive policy that admits cross-tenant rows.
- **(A-RLS-2)** The relay's request role is non-superuser, `NOBYPASSRLS`, and not
  the table owner unless `FORCE ROW LEVEL SECURITY` is set (owners and `BYPASSRLS`
  roles bypass policies).
- **(A-RLS-3)** `app.community_id` is set transaction-locally (`SET LOCAL`) before
  any query and cleared at transaction end. Pooled connections must not retain or
  combine tenant context across requests.
- **(A-RLS-4)** `SECURITY DEFINER` and `leakproof`/user-defined functions in the
  request path are audited as part of the trusted boundary: a `leakproof`
  function may be evaluated *ahead of* the RLS check, and a `SECURITY DEFINER`
  function can read data unavailable to the caller.
- **(A-RLS-5)** Uniqueness and foreign-key constraints include `community_id`, so
  a conflict outcome or a dangling reference cannot reveal or reach another
  community.

A query that fails to set `app.community_id` matches the policy predicate over
NULL → no rows, never all rows. This is what makes a missed *application*
predicate fail closed rather than leak (Theorem I4).

### Concurrency, crypto, and resolution

- **(P-APPEND)** `INSERT … ON CONFLICT (community_id, created_at, id) DO NOTHING`
  commits a row iff no row with that key exists; concurrent appends are
  serializable under MVCC; a committed row is never silently overwritten; a read
  sees a consistent snapshot.
- **(P-SIG)** An actor cannot produce a valid Schnorr signature (BIP-340) for a
  pubkey whose secret key it does not hold. A NIP-98 event's `u`/`method`/
  `payload` tags bind it to exactly one HTTP request and are non-transferable to a
  different request.
- **(P-RESOLVE)** `resolve : channel_id → community_id` is a total function over
  existing channels, computed from control-plane state under the operation's
  transaction snapshot. A channel belongs to exactly one community
  (`channels.community_id` NOT NULL); resolution never returns a community a
  channel does not belong to. **A channel's community is set at creation and never
  reassigned: `channels.community_id` is immutable after insert.** Both mechanized
  models encode this — Tamarin as the persistent `!ChannelCommunity` fact
  (`MultiTenantAuth.spthy:51`, once-true-always-true), TLA+ as the
  `ChannelCommunity` CONSTANT function (`MultiTenantRelay.tla:107`). Any future
  re-tenanting would be a separate axiomatic admission with its own audit
  discipline and re-verification of S1/S2 (and I1–I5).
- **(P-RESOLVE-HOST)** `resolve_host : host → community_id ∪ {⊥}` is the upstream
  binding for **every** connection, lifting today's per-relay URL identity one
  level up to the community. A connection's community is `resolve_host(host)`,
  fixed at establishment; the URL the client connects to *is* the selector,
  exactly as a relay URL is today. `ResolveTenant(req, event)` composes the two:
  if the event has an `h` tag, require `resolve(h) = resolve_host(host)` (the
  host/channel **agreement** fence — an A-host presenting a B-channel event is a
  confused deputy on the host axis and is rejected fail-closed, never acted on as
  B) and store that community; if it has none, store
  `community_id = resolve_host(host), channel_id = NULL`. Two fences hold for both
  paths. **Fail-closed:** a host/channel disagreement (incl. an unmapped host
  resolving to `⊥`, which can never equal a real channel community) is rejected
  generically (`auth-required`/`restricted`), never bound to a default tenant —
  `resolve_host` is partial and the absence/disagreement of a binding is a reject,
  not a fallback. **Host wins:** a NIP-98 token's community stamp (L5) must *agree
  with* the host-derived community; a token that disagrees is rejected, so the
  confused-deputy fence (I2) is intact with authority binding to the host-resolved
  object. Tamarin encodes this as the persistent `!HostCommunity` fact
  (`MultiTenantAuth.spthy`): the channel-less use rule fires only when token stamp
  and host community coincide (witness `ChannelLessResolved`, lemma
  `channelless_use_confined_to_host_community`), and the **channel-bearing** use
  rule fires only when the channel mapping and the host community coincide (witness
  `ChannelBearingResolved(tok, used_comm, host, host_comm)`, lemma
  `channelbearing_use_agrees_with_host` asserting `used_comm = host_comm`). TLA+
  encodes it as the `HostCommunity` resolver (with a `⊥` sentinel for unmapped
  hosts) and an `Inv_HostBindingFence` invariant quantifying over **every** accepted
  write — channel-bearing and channel-less — *and* every observable duplicate/no-op
  outcome, that its stored community equals its originating host's mapping. The
  duplicate/no-op path carries the same obligation because it is client-observable
  write surface (the `Duplicate` result exposes the scoped existence/conflict rows):
  an A-host presenting a B-channel id is fenced before any conflict lookup, so it
  cannot learn whether that id exists in B. At N = 1 this is byte-identical to
  today: one host → the one community, every connection lands there, nothing
  client-observable changes.
- **(A_HASH)** The event id `sha256(canonical event)` is second-preimage
  resistant: an actor cannot find a distinct event hashing to a chosen id. (NIP-01
  already relies on this; we cite it the way git-on-s3 cites its CAS axiom.)
- **(P3)** *NIP-98 mint freshness.* A NIP-98 mint event (kind:27235) is accepted
  at most once. The implementation enforces this with two checks: a `created_at`
  within ±60s of server time (`buzz-auth/src/nip98.rs:77-83`,
  `TIMESTAMP_TOLERANCE_SECS = 60`) **and** a seen-set keyed on event id
  (`buzz-relay/src/api/bridge.rs::check_nip98_replay`), whose cache TTL (120s,
  `state.rs:407`) is 2× the window so a mint valid at either edge stays tracked
  for the full window. The Tamarin model abstracts the window as a fresh nonce on
  `~time` (`MultiTenantAuth.spthy:91`), which over-approximates the
  implementation by treating every mint as structurally unique; the spthy comment
  at `:84-86` references this obligation as "P3."

P-RESOLVE is the load-bearing *application* assumption for channel-bearing events
and P-RESOLVE-HOST is its channel-less counterpart — together the fence the
`h`-tag and claimed-community adversary cannot circumvent. A-RLS-1..5 are the
load-bearing *backstop*.

## Safety Theorems

### Isolation (mechanized in TLA+)

- **NI (Non-interference, master).** For every reachable state and every B-scoped
  observation, the observed value is a function only of B-labeled state — no
  high-labeled value flows into a low-labeled observation. I1–I5 are the specific
  flows it rules out, each independently mutation-tested non-vacuous.
- **I1 (Read confinement).** Every row a `Serve` returns — including direct-id and
  `#e`/`#a` lookups — is `ctx.community`-labeled.
- **I2 (Resolution fence).** `ctx.community = resolve(channel_id)` for
  channel-bearing events and `resolve_host(host)` for channel-less ones, never the
  `h` tag, the claimed community, or the token stamp; an adversary `h = C' ≠
  resolve = C` cannot widen what is served or accepted. The **host axis** is fenced
  on both paths: a channel-less write over host A cannot land in community B, and a
  channel-bearing op over host A on a B-channel is rejected rather than acted on as
  B — including the **duplicate/no-op outcome**, so an A-host cannot use a B-channel
  id-conflict result as a cross-tenant existence oracle (`Inv_HostBindingFence`
  quantifies over accepted writes *and* recorded duplicates, making "default to C",
  "A-host drives a B-channel insert", and "A-host probes a B-channel duplicate"
  caught mutations, not invisible ones).
- **I3 (Write non-loss & no cross-contamination).** Every accepted append commits
  under the resolved label and no other; no committed message is lost or
  overwritten; two communities appending the same event id land as two rows under
  distinct labels (cross-community id collision is not a write conflict).
- **I4 (Fail-closed backstop).** A dropped application predicate yields ∅ under
  A-RLS, and NI still holds; removing the RLS guard makes the dropped predicate
  produce a cross-label row — proving RLS load-bearing, not decorative.
- **I5 (Admission fence).** Channel membership and channel-less read capability
  exist only for actors admitted to *that* community. The NIP-43 allowlist is the
  `admittedMembers` relation keyed on `(community, actor)`; `AddMembership` and
  every channel-less read are gated on `IsAdmitted(c, a)`, and `Inv_AdmissionFence`
  quantifies over every membership *and* every recorded channel-less read,
  requiring same-community admission on both — the channel-less branch additionally
  binding `HostCommunity[host] = community`, so the host axis is fenced here too.
  The same gate covers the **open-community** and **no-`#h`-read** surfaces: an
  open community auto-registers an authenticated npub into the host-resolved
  community (`AuthenticateOpenCommunity` recording an `authRegistration`), and the
  kinds-only **feed read** (`ReadHostFeedRows`) and `#e`-only **aux read**
  (`ReadHostAuxRows`) each record a witness only when the actor is `IsAdmitted` to
  the host community — `Inv_AdmissionFence` quantifies over those witness sets too,
  so an actor admitted only in B can neither open-register into A nor read A's
  no-`#h` feed/aux. **Channel creation** (`CreateChannel`) stamps a fresh channel
  from `HostCommunity[host]` and `Inv_ChannelCommunityImmutable` proves that stamp
  is never re-labeled — creation is an in-relay analog of S2's
  resolve-then-immutable discipline. The fence is about **current**
  capability: it is mutation-tested non-vacuous by
  M9 (re-keying the membership/read gate to any-community admission), which goes red
  on both a membership trace and a channel-less-read trace, and by M10–M13 (the
  open-AUTH, channel-create, feed-read, and aux-read stamp/gate mutations), each
  confirmed red — proving an admit-into-A then act-in-B escape is caught rather than
  invisible on every one of these surfaces. (See C3 for the explicit
  historical-write carve-out.)

### Authorization soundness (mechanized in Tamarin, Dolev-Yao adversary)

- **S1 (Token confinement).** A token accepted for a B-resolved operation was
  minted with stamped community B; a token stamped A never authorizes in B. A
  *leaked* token authorizes within its own community (blast radius is not zero and
  we do not pretend otherwise) but never another — containment, proven.
- **S2 (Mint integrity).** A token exists only as the output of a NIP-98 mint by
  the holder of `owner_pubkey`'s key (P-SIG); it carries exactly one stamped
  community; a mint whose channel set spans two communities yields no token.
  S2's trace-level mint-rejection closure relies on P-RESOLVE's totality,
  single-valuedness, **and immutability**: the Tamarin model encodes immutability
  via persistent-fact semantics (`!ChannelCommunity`), without which a
  retag-then-replay — reject a cross-community `req`, retag a channel, replay the
  original mint bytes (same `req` hash) — would mint a token for a request S2
  declares unmintable. This is the structural analog of A-RLS-5's
  `UNIQUE (community_id, id)` clause for I1: both turn stable scope into the
  disjointness witness.
- **S3 (Signing-key non-confusion + containment).** A community-B-signed system
  event (NIP-29 `39000`/`39001`/`39002`) is never accepted as an authentic
  community-A event, even when group ids collide; compromise of B's signing key
  does not let the adversary forge A's events.
- **S4 (Audit-chain unforgeability + containment).** No splice, reorder, or forge
  in community A's hash chain; compromise of B's chain does not break A's — N
  independent chains, N independent guarantees.
- **S5 (Channel-less host confinement).** A channel-less authorization (profiles,
  DMs, long-form, lists — no `h` tag) is confined to the community bound to the
  connection's **host**, not the token's stamp: host wins. The token must agree
  with the host community or the request is rejected; a B-stamped token presented
  over an A-host never authorizes for B. This is I2's host counterpart, mechanized
  as `channelless_use_confined_to_host_community`,
  `channelless_token_agrees_with_host`, and `host_token_mismatch_not_authorized`.
- **S6 (Channel-bearing host/channel agreement).** A channel-*bearing*
  authorization is confined to the community bound to the connection's **host**:
  the host and the channel mapping must agree. An A-host presenting a B-channel
  event never authorizes as B — the host axis of the confused-deputy fence, which
  the prior model proved only on the channel axis (claimed-community ignored). This
  closes the cross-tenant escape over a wildcard host route where the channel
  mapping alone would have been authoritative. Mechanized as
  `channelbearing_use_agrees_with_host` (the single-witness `ChannelBearingResolved`
  fact asserting `used_comm = host_comm`).
- **S7 (NIP-43 admission confinement).** A community's member-list (NIP-43)
  admission is confined to the community whose signing key signed it: B's signing
  key can never admit a pubkey into A. Modeled as a parallel rule pair —
  `Community_Signs_NIP43_MemberList` mints the signed list and
  `Relay_Accepts_NIP43_MemberList` re-verifies the signature against
  `!CommunitySigningKey(comm, sk)`, so `comm` is bound by unification to the
  resolved community (the same confused-deputy discipline as the S5/S6 host
  fence), emitting persistent `!Admitted(pk, comm)`.
  `nip43_admission_confined_to_signing_community` proves the confinement; the
  commented `MUTATION_Admit_Ignore_Community` (the dual of S6's
  `MUTATION_Use_Token_Ignore_Host`) falsifies it, confirming the green is
  non-vacuous. This is the authorization-world half of the same admission property
  TLA+'s I5 proves in the in-relay world: `!Admitted(pk, comm)` /
  `MemberAdmitted(pk, comm)` ⇔ `admittedMembers`/`IsAdmitted(c, a)` — one property,
  two worlds (Tamarin proves the admission *event* per-community unforgeable, TLA+
  proves the resulting capability in-relay scoped).
- **S8 (Open-community AUTH confinement).** When a community carries no NIP-43
  member-pubkey allowlist it is **open**: any authenticated npub auto-registers on
  AUTH. The registration is still confined to the **host-resolved** community —
  `Authenticate_To_Open_Community` stamps the registration from the connection's
  host binding, never a client-supplied selector, so "open" relaxes the *gate* on
  membership without relaxing the *boundary* it lands in. Mechanized as
  `open_auth_registration_confined_to_host_community` (a host-bound npub registers
  only into its host's community), with the exists-trace witness
  `executable_open_auth_registration` proving a legitimate open registration is
  producible so the confinement lemma is non-vacuous. This is S5/S6's host-binding
  discipline applied to the admission *event*: the same confused-deputy fence that
  stops a B-stamped token authorizing over an A-host stops a B-host AUTH
  registering into A. Its in-relay counterpart is I5's open-community branch
  (`AuthenticateOpenCommunity`, mutation M10).

Each Tamarin lemma is paired with an exists-trace sanity lemma (the honest
protocol can run), the Tamarin analog of the mutation test.

**Verification status.** S1–S8 are **machine-verified green** on
Tamarin 1.12.0 / Maude 3.5.1 — the full selected run verifies all 32 lemmas in
~12s with zero `analyzed` failures. S1/S2: `token_confinement`,
`cross_community_use_attempts_are_not_authorized`, the two
`minted_*_channels_match_stamp` lemmas, `token_stamp_matches_mint`,
`cross_community_mint_yields_no_token_for_that_request`, and the
`leaked_token_blast_radius_contained` / `leaked_token_can_authorize_within_its_community`
containment pair, with `MUTATION_Use_Token_Claimed_Community` confirmed red
(`falsified — found trace`). S3:
`system_event_acceptance_requires_same_community_key_or_compromise` (21 steps) and
`other_community_key_compromise_does_not_authorize` (147 steps). S4:
`audit_append_advances_same_community_head` (2 steps) and
`cross_community_audit_splice_attempt_is_not_append` (1 step). S5 (channel-less
host confinement): `channelless_use_confined_to_host_community` (2 steps),
`channelless_token_agrees_with_host` (3 steps), and
`host_token_mismatch_not_authorized` (6 steps), each paired with an exists-trace
probe (`executable_host_bound`, `executable_channelless_use`,
`executable_host_token_mismatch_attempt`). The S5 mutation
`MUTATION_Use_Token_ChannelLess_Ignore_Host` (the relay reading the token's stamp
and ignoring the host binding — the B-token-on-A-host confused deputy) is
confirmed red: it falsifies `channelless_use_confined_to_host_community` in 3.3s
with a 13-step trace. Each safety lemma is
paired with a verified exists-trace sanity lemma, and the S3/S4 mutations are
confirmed red: the bad-accept-with-other-community-key mutation falsifies both S3
lemmas (5 / 16 steps) and the splice-as-append mutation falsifies the S4 splice
lemma (8 steps). S6 (channel-bearing host/channel agreement):
`channelbearing_use_agrees_with_host` (2 steps), with the
`MUTATION_Use_Token_Ignore_Host` mutation (the relay resolving a channel-bearing
op from the channel mapping while ignoring the host binding — the A-host-on-a-
B-channel confused deputy) confirmed red: it falsifies
`channelbearing_use_agrees_with_host` in 2.6s with a 14-step trace. S7 (NIP-43
admission confinement): `nip43_admission_confined_to_signing_community` (19 steps)
and `other_community_key_compromise_does_not_admit` (79 steps), with the
exists-trace probe `executable_member_admitted` (7 steps) proving a legitimate
admission is producible — so the confinement lemma is non-vacuous, not trivially
true over an unreachable premise. The S7 mutation `MUTATION_Admit_Ignore_Community`
(the relay minting `!Admitted` for a community other than the one whose key
signed — the admission-side confused deputy, the dual of S6's
`MUTATION_Use_Token_Ignore_Host`) is confirmed red: it falsifies
`nip43_admission_confined_to_signing_community` in 1.57s with a 7-step trace.
S8 (open-community AUTH confinement): `open_auth_registration_confined_to_host_community`
(2 steps), paired with the exists-trace witness `executable_open_auth_registration`
(5 steps) proving a legitimate open-community registration is producible, so the
confinement lemma is non-vacuous; its in-relay counterpart is the M10 open-AUTH
stamp mutation, confirmed red in TLA+ (a 2-state `Inv_AdmissionFence` violation).

The S5 confinement lemma was deliberately framed to keep its mutation
*cheaply* refutable. An earlier framing joined two action facts
(`ChannelLessAuthorized` ⋈ `HostBoundFor`) on a shared host; the proof verified,
but the *mutation refutation* did not terminate — Tamarin chased which
`HostBoundFor` instance applied for a given host across both the real and mutated
rules. The fix emits a single combined witness
`ChannelLessResolved(tok, used_comm, host, host_comm)` from the authorizing rule
(in the real rule both communities are the same variable), so the confinement
lemma is a single-fact assertion `used_comm = host_comm` and the mutation that
breaks it is a one-rule-instance counterexample. The proof dropped to 2 steps and
the mutation falsifies in 3.3s — the same "make the bad case structurally cheap to
exhibit" discipline as the S1 claimed-community mutation.

The S3/S4 round corrected one vacuity bug in the committed
`1e7fb042…aceaacf24` artifact: `other_community_key_compromise_does_not_authorize`
bound `Neq(commA, commB)` to the *same* timepoint as `CommunityKeyCompromised(commB)`,
but no rule emits `Neq` at the compromise point, so that premise was unsatisfiable —
the lemma verified vacuously and asserted nothing. (Independently confirmed: an
exists-trace probe of the old premise returns `no trace found`.) The fix decouples
the inequality onto a separate witness timepoint `#k`; a new exists-trace lemma
`executable_other_key_compromise_plus_system_accept` (16 steps, verified) proves the
corrected premise is satisfiable, so the 147-step proof is non-vacuous. This is the
same hygiene class as F1/F3/F4 — an artifact relying on a fact the model never makes
reachable — but caught inside a safety lemma's premise rather than a comment. That
fix predates this milestone's host-binding additions and is carried forward
unchanged in the current `.spthy`.

## Conformance

Each axiom is *admitted* per deployment, not assumed universally:

- **A-RLS-1..5** are admitted by a startup/CI assertion suite: enumerate every
  tenant-bearing table and assert RLS enabled + restrictive policy present; assert
  the request role is `NOBYPASSRLS` and non-owner-or-FORCE; assert no
  `SECURITY DEFINER` function in the request path reads tenant tables without
  re-establishing context; assert every unique/FK constraint includes
  `community_id`. A failing assertion rejects the deployment.
- **P-RESOLVE** is admitted by the `channels.community_id NOT NULL` constraint
  plus a test that `resolve` is read under the operation's snapshot, plus a
  migration lint asserting `channels.community_id` is never mutated after insert
  (no `UPDATE`/`ALTER`/drop-recreate). A failing lint rejects the deployment.
- **P-SIG / A_HASH** are the standard Nostr crypto assumptions; admitted by using
  the audited libraries the rest of Buzz uses.
- **P3** is admitted by the NIP-98 handler enforcing *both* timestamp-range
  validation and the seen-event-id check (`check_nip98_replay`) before any mint.
  Two structural gates make the seen-set sound, and both are conformance checks
  because the implementation is silent if either is violated:
  1. **Capacity vs. rate.** The seen-set is bounded (capacity 10,000, TTL 120 s
     = 2× the ±60 s window). It must satisfy `capacity ≥ peak NIP-98 RPS × 120 s`
     (≈ 83 RPS sustained at the current capacity); above that, LRU eviction can
     release an entry while its signed `created_at` is still inside the window,
     and a replay slips through.
  2. **Per-pod scope.** The seen-set is `Arc<AppState>`-scoped, not cross-pod, so
     the same replayed event reaching two pods succeeds once on each. P3 therefore
     requires *either* NIP-98 mints be pod-sticky on `event_id` *or* the seen-set
     be shared across pods (e.g. Redis with the same atomic insert-if-absent
     semantics and TTL ≥ 120 s). The chart default (`replicaCount: 1`) satisfies
     this gate today; the shipped HA examples (`replicaCount: 3` in
     `deploy/charts/buzz/examples/argocd-app.yaml:27` and
     `deploy/charts/buzz/examples/flux-helmrelease.yaml:35`) are
     P3-non-conforming as shipped unless the operator adds one of:
     - **(a)** an ingress annotation hashing upstream selection on a header stable
       across replays — `nginx.ingress.kubernetes.io/upstream-hash-by:
       "$http_authorization"` works for today's NIP-98 HTTP path, since the signed
       event rides in `Authorization: Nostr <base64>` (`bridge.rs:34-46`) and is
       bit-identical across replays. Two caveats keep this from being the
       recommended fix: it couples replay-stickiness to literal-byte-identity of
       the auth header (any future header normalization — whitespace, casing,
       base64 padding — silently breaks it), and it does not extend to any mint
       path that moves off HTTP (a WS mint has no Authorization header to hash on).
     - **(b)** a shared seen-set backed by a store with atomic insert-if-absent and
       TTL ≥ 120 s (e.g. Redis, already present in the HA chart for git-pubsub).
       **This is the recommended path** — no new infra surface and none of (a)'s
       fragility.

  A regression test asserts a replayed mint within the window yields a single
  token under the deployment's routing/storage shape (and that the seen-set TTL
  covers the full ±60 s window). A failing test or an unmet gate rejects the
  deployment.

## Prior Art

The *pattern* (discriminator column + RLS) is established; the *formal treatment*
as label-flow non-interference is, to our knowledge, new for a Nostr relay.

- **Goguen & Meseguer, "Security Policies and Security Models" (IEEE S&P 1982)** —
  the origin of non-interference; the theorem shape ("A's actions do not affect
  B's observations"), with "community" for "security domain."
- **Sabelfeld & Myers, "Language-Based Information-Flow Security" (IEEE JSAC
  2003)** — the canonical label-based IFC survey; its declassification discipline
  is the model for our named C1 carve-out.
- **Jean Yang et al., "Precise, Dynamic Information Flow for Database-Backed
  Applications" (arXiv:1507.03513, Jacqueline)** and **Parker, Vazou, Hicks,
  "LWeb" (arXiv:1901.07665)** — the closest formal analogs: label-based per-row
  policy over a real relational store with a *mechanized* non-interference proof.
  They justify "RLS is a backstop axiom; the theorem is the composition."
- **Hardy, "The Confused Deputy" (ACM SIGOPS OSR 1988)** and **Miller et al.,
  "Capability Myths Demolished" (HPL-2003-222)** — the resolution-as-capability
  framing: bind authority to the resolved object, not the caller-supplied name.
- **NIP-29 (relay-based groups)** — confirms the relay is authoritative and group
  ids are not globally unique security domains; supports per-community signing
  keys and per-community audit chains, and motivates S3's "non-confusable even
  when group ids collide."
- **`fiatjaf/relay29`** — empirical prior art: isolation logic lives across read
  filters, direct-id lookups, metadata generation, in-memory state rebuilds, and
  `previous`-tag validation, not just insert/select predicates. The reason
  `Serve` must model the full observable surface, not just channel reads.
- **PostgREST / PostGraphile** — converge on the transaction-local-context fence
  (A-RLS-3); real systems install request-local identity into the DB transaction
  and let policies authorize. (See `RESEARCH/MULTITENANT_ISOLATION_PRIOR_ART.md`
  for citations and local checkout line references.)

## Mechanized Verification

- **`docs/spec/MultiTenantRelay.tla` + `.cfg`** — the TLA+ isolation model. Run:
  `java -cp tla2tools.jar tlc2.TLC -config MultiTenantRelay.cfg MultiTenantRelay.tla`.
  On the core finite harness (2 communities × 4 channels, 2 message ids, 1 actor,
  1 worker, 2 audit values, bounded observation set, symmetry over the permutable
  model-value sets) TLC **completes exhaustively**: *Model checking completed. No
  error has been found.* — 472,530,528 states generated, 16,226,016 distinct, 0 left
  on queue, depth 13 (8 workers, ~5m). The distinct-state count grew from the
  pre-host-binding baseline (4,350,464 → 5,091,328 with channel-less host binding →
  5,621,760 with channel-bearing host/channel agreement → 9,232,992 with the
  `admittedMembers` allowlist, `channelLessReads` capability rows, and the
  `AdmitMember`/`RevokeMember` actions → 16,226,016 with the open-community AUTH
  auto-registration, server-stamped channel creation, and the no-`#h` host
  feed/aux read paths) precisely because the
  channel-less write path, the fail-closed unmapped-host path, the
  channel-bearing host/channel-agreement (and its fail-closed disagreement) path,
  the admit/revoke/gated-membership/gated-read paths, and now the
  open-AUTH/channel-create/feed-read/aux-read paths
  are genuinely reachable — new behavior, not dead code. Threading the host through
  the duplicate/no-op path adds reachable fail-closed transitions without new
  distinct states: only the agreeing host can produce a
  recorded duplicate, so the host on that path is fully determined; layering the
  admission gate, then the open-AUTH/channel-create/feed/aux surfaces on top, is the
  growth to the figures above (admit-then-act, revoke-then-act-fails, gated
  reads/joins, open-community auto-registration, server-stamped creation, and the
  two host-fenced no-`#h` read shapes multiply the reachable space). That each new
  surface is reachable rather than dead is pinned by four intentionally-false
  reachability probes (`Probe_OpenAuthRegistration_Unreachable`,
  `Probe_CreatedChannel_Unreachable`, `Probe_HostFeedRead_Unreachable`,
  `Probe_HostAuxRead_Unreachable`): each asserts the corresponding witness set stays
  empty, so each must go red if its action fires — and all four do (open AUTH and
  channel-create at 2 states; host feed and host aux at 3 states, via open AUTH then
  read). A vacuously-passing new conjunct over an unfireable action is therefore
  ruled out, not assumed. Non-vacuity of the
  invariants themselves is shown by thirteen mutations (M1–M13), each
  confirmed to produce a counterexample: substituting the unscoped direct-by-id
  lookup (`UnscopedDirectIdRows`, the `get_accessible_channel_ids` landmine) →
  `Safety` violated at depth 4; widening the sanitized-error label to all
  communities (the raw-error leak) → `Safety` violated at depth 2; the
  global-id conflict key (M3: `WriteDuplicate` keyed on `id` alone via
  `GlobalConflictRows`, the missing-`community_id`-in-the-unique-index footgun)
  → `Safety` violated at depth 3, with a B-scoped `WriteResult` observation
  carrying `labels |-> {commA}` (the existence-oracle leak C2.1 closes); the
  host-default-tenant mutation (a channel-less write from an unmapped host landing
  in a default community instead of failing closed) → `Inv_HostBindingFence`
  violated at depth 2, the counterexample exhibiting `hostBad` writing into
  `commA`; the **M8** host/channel-agreement mutation (`WriteInsert` dropping
  the agreement fence so an A-host op on a B-channel is accepted) →
  `Inv_HostBindingFence` violated by a 2-state trace (`Init → WriteInsert`); and the
  **M8-duplicate** mutation (`WriteDuplicate` dropping the same fence so an A-host
  can probe a B-channel id-conflict) → `Inv_HostBindingFence` violated by a 3-state
  trace (`Init → WriteInsert → WriteDuplicate`), the counterexample exhibiting a
  foreign-host duplicate record whose stored community ≠ its host's mapping (the
  existence oracle the duplicate path would otherwise reopen); and the **M9**
  global-allowlist mutation (re-keying the admission gate from same-community
  `IsAdmitted(c, a)` to any-community `AdmittedInAnyCommunity(a)`) →
  `Inv_AdmissionFence` violated in two surfaces: a 5-state membership trace
  (`Init → WriteInsert → WriteInsert → AdmitMember(commA, alice) →
  AddMembership(commB/chanB1, alice)`) where alice, admitted to A, joins B's
  channel through the global hole; and a 4-state channel-less-read trace
  (`Init → WriteInsert → AdmitMember(commB, alice) →
  ReadMessageRows(commA, NoChannel, hostA)`). The two M9 variants prove both the
  `AddMembership` gate and the channel-less-read gate are independently
  load-bearing, not just one. The four newest surfaces — open-community AUTH
  auto-registration, server-stamped channel creation, and the two no-`#h` host
  read shapes (kinds-only feed, `#e`-only aux) — are each held by their own
  confirmed-red mutation: the **M10** open-AUTH stamp mutation (the relay stamping
  an open-community auto-registration into a default/claimed community instead of
  `HostCommunity[host]`) → `Inv_AdmissionFence` violated by a 2-state trace
  (`Init → AuthenticateOpenCommunity(hostB stamps commA)`), catching an
  `authRegistration` whose host maps elsewhere; the **M11** channel-create stamp
  mutation (a fresh channel stamped into a default/claimed community rather than
  the host's) → `Inv_HostBindingFence`/`Inv_ChannelCommunityImmutable` violated by a
  2-state trace (`Init → CreateChannel(hostB stamps commA)`); the **M12** feed
  global-admission mutation (re-keying the `ReadHostFeedRows` admission guard from
  same-community `IsAdmitted(c, a)` to relay-global `GloballyAdmitted(a)`) →
  `Inv_AdmissionFence` violated by a 3-state trace
  (`Init → AdmitMember(commB, alice) → ReadHostFeedRows(hostA)`), so an actor
  admitted only in B cannot read A's no-`#h` feed; and the **M13** aux
  global-admission mutation (the same guard re-key on `ReadHostAuxRows`) →
  `Inv_AdmissionFence` violated by a 3-state trace
  (`Init → AdmitMember(commB, alice) → ReadHostAuxRows(hostA)`). M10–M13 confirm the
  open-AUTH/create/feed/aux fences are load-bearing, not decorative — the same
  "every new conjunct earns a confirmed red" contract as M1–M9. (To reproduce M12/M13,
  the substitution that trips `Inv_AdmissionFence` is the action's admission *guard*
  (`IsAdmitted(c, a)` → `GloballyAdmitted(a)` in `ReadHostFeedRows`/`ReadHostAuxRows`),
  not the row-set helper alone, since the invariant quantifies over the recorded
  `feedReads`/`auxReads` witnesses rather than the returned row set — the `.tla`
  helper comments call this out.) The host-fence and new-surface
  figures above are counterexample **trace lengths** (the error-trace state count),
  which unlike TLC's run-dependent "depth of complete graph search" total are
  reproducible from the printed error trace. The
  `h`-tag mutation is the same shape (I2). The config is deliberately a
  fast non-vacuity harness, not the full deployment scale — widening workers,
  actors, and ids explodes the space; symmetry + bounded observations keep the
  core isolation surface exhaustively checkable.
- **`docs/spec/MultiTenantAuth.spthy`** — the Tamarin authorization model. Run:
  `tamarin-prover --prove docs/spec/MultiTenantAuth.spthy`. All 32 lemmas (S1–S8)
  verify green (Tamarin 1.12.0 / Maude 3.5.1, ~12 s) — each safety lemma paired with
  a verified exists-trace sanity lemma, and the documented mutations
  (`MUTATION_Use_Token_Claimed_Community` for S1, the S3 bad-accept and S4
  splice-as-append mutations, `MUTATION_Use_Token_ChannelLess_Ignore_Host`
  for S5's host fence, `MUTATION_Use_Token_Ignore_Host` for S6's channel-bearing
  host/channel-agreement fence, and `MUTATION_Admit_Ignore_Community` for S7's
  NIP-43 admission confinement) confirmed red. The 32 lemmas include the
  open-community AUTH pair added with the host-scoped-open-auth surfaces:
  `open_auth_registration_confined_to_host_community` (2 steps) proves an
  open-community auto-registration commits to the host-resolved community and never
  a client-claimed one, and its exists-trace witness
  `executable_open_auth_registration` (5 steps) proves a legitimate open-community
  registration is producible, so the confinement lemma is non-vacuous. See
  §Authorization soundness for the
  full lemma list, the S5/S6 single-witness framing, and the corrected
  `other_community_key_compromise_does_not_authorize` vacuity fix.

  **Machine-check hygiene.** S1–S8 lemmas close by two distinct shapes.
  **Rule-shape closure** means the lemma's conclusion follows by unification on a
  single rule's action multiset: `token_confinement`,
  `audit_append_advances_same_community_head`,
  `channelless_use_confined_to_host_community` (the S5 single-witness fact),
  `channelbearing_use_agrees_with_host` (the S6 single-witness fact), and
  the S2 supporting set
  (`minted_token_channels_match_stamp`, `minted_request_channels_match_stamp`,
  `token_stamp_matches_mint`). These are well-formedness guards on the model's
  action labels; the substantive security claim is carried by the corresponding
  rule design and mutation (for example, `MUTATION_Use_Token_Claimed_Community`
  falsifies `token_confinement` when authorization is rewritten to use a claimed
  community, `MUTATION_Use_Token_ChannelLess_Ignore_Host` falsifies
  `channelless_use_confined_to_host_community` when the relay reads the token
  stamp instead of the host binding, and `MUTATION_Use_Token_Ignore_Host`
  falsifies `channelbearing_use_agrees_with_host` when the relay resolves a
  channel-bearing op from the channel mapping while ignoring the host).
  **Substantive closure** requires cross-rule reasoning over
  persistent-fact invariance (`cross_community_mint_yields_no_token_for_that_request`,
  `leaked_token_blast_radius_contained`,
  `cross_community_use_attempts_are_not_authorized`), linear-fact lifecycle
  (`cross_community_audit_splice_attempt_is_not_append`), or signed-preimage
  unification (`system_event_acceptance_requires_same_community_key_or_compromise`).
  Tamarin proves both kinds identically; the distinction is for reviewer hygiene,
  not a weakened theorem claim. This paragraph is prose-only to preserve the
  `.spthy` byte hash above.

## Implementation Correspondence

The model's obligations map to concrete code seams:

- **P-RESOLVE / I2** — `resolve(channel_id)` must be the *only* source of
  `ctx.community_id`; the `h` tag is never written into tenancy. Today there is no
  community layer; `channel_id` is the only locality.
- **P-RESOLVE (immutability) / S2** — `channels.community_id` must be immutable
  after insert. No migration may `UPDATE channels SET community_id = …`,
  `ALTER TABLE channels … community_id …`, or drop-and-recreate the column without
  an explicit re-admission of P-RESOLVE and re-verification of S1/S2. This is the
  load-bearing assumption behind S2's trace-level mint-rejection (a retag-then-
  replay breaks it) and behind the TLA `ChannelCommunity` CONSTANT; it is
  invisible to both the labeling invariant and the Tamarin lemmas (the proofs
  would silently weaken, not fail), so it is enforced by a migration lint — the
  same gate-on-the-migration class as the C2.1 composite-index and C2.4
  `RelayInfo::build` signature lints.
- **I1 / I4** — every DB entry point takes `TenantContext` and `SET LOCAL
  app.community_id`; the unscoped `get_accessible_channel_ids()`
  (`crates/buzz-db/src/channel.rs:545-560`, which unions every open channel in the
  DB) must not exist in any tenant-scoped path. RLS is the backstop.
- **C2.1 / A-RLS-5** — the message-uniqueness constraint must be composite over
  `(community_id, …, id)`, never `UNIQUE (id)` alone. This is the closure for the
  existence-oracle (M3 goes red at depth 3 under a global key). It is one bad
  migration away from breaking and is invisible to the labeling invariant, so it
  is enforced by the conformance schema assertion (§Conformance: "every unique/FK
  constraint includes `community_id`") — the same gate-on-the-migration class as
  the C2.4 `RelayInfo::build` signature lint.
- **S3 / S4** — the relay keypair becomes a per-community signing key
  (`communities.signing_key`), distinct from relay-instance identity; the single
  global audit chain (`crates/buzz-audit/src/service.rs`) becomes N per-community
  chains `AuditEntry(community, seq, prev, hash)`.
- **P3 / S2** — the NIP-98 mint freshness obligation the Tamarin model abstracts
  as a fresh `~time` nonce is carried by two code seams: the ±60s window in
  `crates/buzz-auth/src/nip98.rs:77-83` and the event-id seen-set
  `check_nip98_replay` in `crates/buzz-relay/src/api/bridge.rs:76-94`, called
  before every mint (`bridge.rs:181`, `:254`, `:514`). The seen-set
  (`state.nip98_seen`, `state.rs:249`/`:407`) is the structural analog of the
  model's nonce: it makes a replayed mint within the window non-fresh, so the
  implementation matches the "every mint is structurally unique" world the model
  proves S2 in. This correspondence is deployment-conditional: today's in-process
  moka cache carries P3 for the chart default (`replicaCount: 1`) and for any
  deployment that routes all mints for the same event id to the same pod, but the
  shipped HA examples (`replicaCount: 3`) do **not** carry P3 as shipped because
  there is no sticky routing and no shared seen-set. HA conformance requires a
  Redis/shared-store seen-set with atomic insert-if-absent and TTL ≥ 120 s
  (recommended), or a header-stable sticky-routing layer — see §Conformance (P3)
  for the two operator options and the caveats on the routing workaround.
- **C2.2** — the client-facing error path must map all DB errors to a fixed
  sanitized alphabet; no `sqlx::Error::to_string()` reaches a tenant connection.
- **C2.4** — the NIP-11 builder `RelayInfo::build`
  (`crates/buzz-relay/src/nip11.rs:122`) must keep its relay-static-only signature
  (no `&PgPool`, no tenant context, no audit service); a signature lint enforces
  the typed-input fence on the unauthenticated `/` surface.
- **P-RESOLVE-HOST / row-zero conformance** — every externally reachable
  relay-global surface consumes the host-derived `TenantContext` before reading
  or mutating tenant data. This is the implementation seam for NIP-11/community
  relay identity, NIP-98/API-token REST calls, media upload/serve, git Smart
  HTTP, workflow webhooks/schedules/manual triggers, search, presence, and Redis
  fan-out. Tokens, signed NIP-98 `u` URLs, webhook ids, workflow ids, repo names,
  media hashes, and event ids are subordinate names; none may select a community
  that disagrees with the request host.
- **NIP-11 / S3** — tenant-observable relay identity is per-community. Static
  software/version fields may be operator-global, but `self`/relay-signed group,
  membership, audit, and system events use the community signing key. The
  unauthenticated info path may reveal facts about the addressed host/community
  only; unknown hosts fail closed generically rather than returning another
  community's info document.
- **API tokens / P3** — `api_tokens` is a community-scoped namespace. Token hash
  lookup, channel claims, scopes, revocation, and NIP-98 replay checks are
  evaluated under `(community_id, token_hash/event_id)`. HA deployments require a
  shared atomic seen-set keyed by community and NIP-98 event id, or an explicitly
  admitted sticky/single-replica deployment; otherwise S2's freshness premise is
  not carried in production.
- **Search / C2.1** — Typesense is shared infrastructure, not a shared result
  space. Indexed documents carry `community_id`; document ids are collision-safe
  across communities (for example `community_id:event_id`) or every upsert/delete
  path includes a community filter. All query `filter_by` strings include
  `community_id`, and Postgres refetch by hit id is `(community_id, event_id)`.
  The existing `__global__` channel sentinel means channel-less within one
  community, never operator-global.
- **Redis / subscription refinement** — Redis pub/sub keys, presence keys, typing
  keys, cache invalidation channels, and local-echo dedup labels include
  community context in any shared multi-tenant deployment. The safe shape is
  `buzz:{community}:channel:{channel_id}`,
  `buzz:{community}:presence:{pubkey}`, and
  `buzz:{community}:typing:{channel_id}`. The current unprefixed keys are
  admissible only for the degenerate single-community deployment or physically
  isolated Redis.
- **Media / Blossom** — raw blob bytes may remain content-addressed and
  operator-deduplicated, but descriptors, upload authorization, quotas, audit
  rows, and any future read policy are community-scoped. A media hash collision or
  pre-existing blob in another community must not become an existence oracle via
  metadata, status code, quota accounting, or audit output.
- **Git / NIP-34** — git Smart HTTP resolves the repository namespace from the
  host-derived community before consulting owner/repo names, branch protection,
  NIP-34 repo announcements, manifests, or object-store pointers. Pointer keys
  include community (for example `repos/{community}/{owner}/{repo}/pointer`);
  pack/object CAS may be shared only below community-scoped refs/manifests and
  authorization metadata.
- **Workflows / system events** — workflow definitions, runs, approval hashes,
  webhook/manual trigger routes, cron scheduling, and relay-signed workflow events
  inherit `community_id`. A workflow id or approval token hash alone is never a
  lookup key. Trigger evaluation sees events in the same community only, and
  schedule coordination must preserve that label across pods.
- **Relay membership / pubkey admission** — relay membership, pubkey allowlist,
  and archived identities are community-global admission facts. The portable
  value is the pubkey; the stored membership/archive fact is
  `(community_id, pubkey, ...)`. No deployment-global user gate is
  tenant-observable unless it is modeled as a separate operator surface. This is
  no longer asserted-only: the `(community_id, pubkey)` admission key and the
  *absence* of a deployment-global gate are both mechanized. TLA+ carries the
  allowlist as the `admittedMembers` relation
  (`MultiTenantRelay.tla:149`), keyed on `[community, actor]`; `IsAdmitted(c, a)`
  (`:317`) gates `AddMembership` and every channel-less read, and
  `Inv_AdmissionFence` proves no membership or channel-less read capability
  survives that is not same-community-admitted (Theorem I5). The
  deployment-global gate is exactly mutation M9: replacing `IsAdmitted(c, a)`
  with the any-community `AdmittedInAnyCommunity(a)` (`:324`) makes the model go
  red — so admit-into-A-then-act-in-B is a *caught* escape, not an invisible one.
  On the authorization side, NIP-43 member-list events are signed and accepted
  per-community in Tamarin (`Community_Signs_NIP43_MemberList` /
  `Relay_Accepts_NIP43_MemberList`, `MultiTenantAuth.spthy:403`/`:413`), and
  `nip43_admission_confined_to_signing_community` proves B's signing key can
  never admit a pubkey into A (Theorem S7).

### Subscription-pipeline abstraction

The mechanized models abstract one structural seam: the **subscription
pipeline** (`REQ → register → match → fan-out → access-filter → EVENT/EOSE`).
The TLA+ isolation model represents this pipeline as the synchronous `Read*`
actions, indexed by `(worker, actor, community, channel)`; it has no
`sub_id`, no `Register`, no `Match`, no `FanOut`, no `EOSE`, no filter state.
This is sound — the model proves `Inv_LabelPropagation` over the **aggregate**
row-set delivered to a B-scoped worker, and the prose observational interface
(§The typed observational interface) presents the same property over
**per-sub streams**. The refinement from aggregate to per-stream is *coarser
than the interface, not wrong* — but it is not mechanized, and it is closed
here, by code-fence and obligation, against the implementation.

**Governing rule.** Every observation kind enumerated in §The typed
observational interface must either (i) be discharged by a TLA+ invariant or
Tamarin lemma, or (ii) appear by name in this subsection with a code-fence
and a closure obligation. New observation kinds added to §The typed
observational interface require a new entry here in the same commit. This
rule is what surfaced F1 (A_HASH closure mis-attribution) and F2 (the
subscription-pipeline abstraction itself).

#### G1 — establishment (`crates/buzz-relay/src/handlers/req.rs:79-204`)

A `REQ` from a connection authenticated under pubkey *p* and token *t*
registers a subscription only after:

1. `accessible_channels ← get_accessible_channel_ids_cached(p)` (`:79`) —
   the DB-derived UUID set the connection's pubkey is a member of.
2. If *t* carries a `channel_ids` claim, intersect with it (`:88-90`). This
   is the one-token-one-community enforcement at the WS surface.
3. `extract_channel_id_from_filters(filters)` (`:92`, body at `:795-822`)
   returns `Some(uuid)` **only if every filter pins the same `#h=<uuid>`**;
   any mixed-`#h` or missing-`#h` filter yields `None`, routing the
   subscription to the global indexes (tests at `:1045-1083`).
4. Channel-scoped path: if the returned `ch_id ∉ accessible_channels`,
   re-confirm via `is_member` against the DB (`:112`); on `Ok(false)` or
   `Err(_)` emit `CLOSED "restricted: …"` (`:127-132`).
5. Global path (`channel_id = None`): per-filter p/engram/author gates must
   hold against *p* (`:144-167`); otherwise `CLOSED`.
6. Only then is `sub_registry.register(conn_id, sub_id, filters, channel_id)`
   called (`:202-204`). `rg -U -n "sub_registry\s*\n?\s*\.register\(" crates/buzz-relay/src`
   (`-U` is required — the prod call splits `.sub_registry` and `.register(`
   across `:203-204`, so the single-line pattern would miss it) returns
   exactly three sites: `req.rs:204` is the **sole** production caller; the
   two others (`mesh_signaling.rs:550`, `event.rs:1058`) are inside
   `#[cfg(test)]` modules.

#### G2 — delivery (`crates/buzz-relay/src/handlers/event.rs:59-113`)

Every candidate from `sub_registry.fan_out` passes through
`filter_fanout_by_access` before any `send_to`. The function (`:59`) and its
doc comment (`:117-124`) state the invariant: *a registered subscription is
never sufficient for delivery — delivery always revalidates access on the
sending pod*. Three checks, in order:

- **Author-only kinds** (`:70-83`) — filter to recipients whose
  `pubkey_for_conn` equals the event author.
- **Channel visibility** (`:85-97`) — `channel_visibility_cached(channel_id)`.
  Non-private → pass through; `"private"` → continue. **Lookup error →
  `return Vec::new()`** (`:91-96`): visibility short-circuit, fail-closed
  for the whole fan-out. The cache discipline at `state.rs:560-568` caches
  only `"private"`, so a stale entry can only over-restrict (≤10s), never
  leak.
- **Membership** (`:99-111`) — `is_member_cached(channel_id, pubkey)` per
  recipient; `Ok(false)` or `Err(_)` drops that recipient.

#### Non-mechanized obligations

The following obligations close the per-sub stream properties the TLA+
`Inv_LabelPropagation` does not reach. Each names its code-fence and the
gates (G1, G2) that carry the closure.

1. **EOSE cardinality.** The count of events preceding `O.WS.EOSE(sub_id)`
   must equal `|{m ∈ messages : matches(m, F) ∧ m ∈ ResolvedScope(conn)}|`,
   where `F` is the sub's declared filter set. Delivery: `req.rs:281`
   (per-event `EVENT` send); EOSE emission: `req.rs:292`. Closure: G1
   admits the subscription only with a `ResolvedScope(conn)`-consistent
   filter set, and G2 drops any candidate not in `ResolvedScope(conn)` at
   delivery; the EOSE count is therefore the sum of events that passed
   both gates.
2. **EOSE → late-EVENT temporal pairing.** No `O.WS.EVENT(sub_id, …)`
   delivered after the sub's EOSE may reveal state withheld by G2 during
   the historical dump. Closure: G2 re-validates visibility and membership
   on every live fan-out, against the same `ResolvedScope(conn)` predicate
   used at EOSE time. The **primary closure is the visibility
   short-circuit at `event.rs:91-96`** — a transient DB error during the
   late-EVENT window returns an empty fan-out for the whole event, not a
   relaxed predicate; the per-recipient membership branch at
   `event.rs:107-110` is the secondary backstop.
3. **`sub_id` reuse and collisions.** The `sub_id` namespace is
   **per-connection, not global**. Cross-connection collisions are
   structurally impossible: `SubRegistry.subs` is keyed
   `entry(conn_id).or_default().insert(sub_id, …)` (`subscription.rs:66-69`)
   and every index entry stores `(conn_id, sub_id)`. Same-connection reuse
   (`REQ` with `sub_id="x"` superseding a prior `sub_id="x"`) is closed by
   `subscription.rs::register` calling `remove_subscription(conn_id, &sub_id)`
   at `:64` before re-insert, and by the new subscription re-running G1
   against the connection's current `ResolvedScope(conn)`.

## Summary

One shared Postgres, one canonical `community_id`-keyed message log, stateless
relay workers, a relational tenant-scoped control plane, and disposable
tenant-scoped projections — with isolation stated as label-flow non-interference
(TLA+), authorization soundness stated as trace lemmas under a Dolev-Yao
adversary (Tamarin), every shared logical channel enumerated and closed, and
every invariant mutation-tested. Safety is machine-checkable relative to the RLS,
crypto, and resolution axioms, each admitted per deployment by a conformance gate.
