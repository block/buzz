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
`community_id` is **resolved by the relay** from the channel the operation names
(`resolve : channel_id → community_id`, an indexed lookup the relay owns under
the same transaction snapshot as the operation). It is **never** read from the
client-supplied `h` tag.

Two operation classes act on the store:

- **Serve(ctx, q)** — a read (REQ / REST GET, including direct `ids` lookup,
  `#e`/`#a` tag filters, metadata/member discovery, and projection reads). Returns
  rows and derived results matching `q`, confined to `ctx.community_id`.
- **Accept(ctx, e)** — a write (EVENT / REST POST). Appends `e` to `L` (or mutates
  control-plane state) under `ctx.community_id`, after an authorization decision
  over current control-plane state.

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
  server-side resolver from `resolve(channel_id)` at insert time. The `h` tag is
  **not** the label source. (Resolution is a fence.)
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
  time; re-scoping requires re-auth; all its observations inherit that scope.
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
  `ChannelCommunity` CONSTANT function (`MultiTenantRelay.tla:85`). Any future
  re-tenanting would be a separate axiomatic admission with its own audit
  discipline and re-verification of S1/S2 (and I1–I4).
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

P-RESOLVE is the load-bearing *application* assumption — the fence the `h`-tag
adversary cannot circumvent. A-RLS-1..5 are the load-bearing *backstop*.

## Safety Theorems

### Isolation (mechanized in TLA+)

- **NI (Non-interference, master).** For every reachable state and every B-scoped
  observation, the observed value is a function only of B-labeled state — no
  high-labeled value flows into a low-labeled observation. I1–I4 are the specific
  flows it rules out, each independently mutation-tested non-vacuous.
- **I1 (Read confinement).** Every row a `Serve` returns — including direct-id and
  `#e`/`#a` lookups — is `ctx.community`-labeled.
- **I2 (Resolution fence).** `ctx.community = resolve(channel_id)`, never the `h`
  tag; an adversary `h = C' ≠ resolve = C` cannot widen what is served or
  accepted.
- **I3 (Write non-loss & no cross-contamination).** Every accepted append commits
  under the resolved label and no other; no committed message is lost or
  overwritten; two communities appending the same event id land as two rows under
  distinct labels (cross-community id collision is not a write conflict).
- **I4 (Fail-closed backstop).** A dropped application predicate yields ∅ under
  A-RLS, and NI still holds; removing the RLS guard makes the dropped predicate
  produce a cross-label row — proving RLS load-bearing, not decorative.

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

Each Tamarin lemma is paired with an exists-trace sanity lemma (the honest
protocol can run), the Tamarin analog of the mutation test.

**Verification status (this draft).** S1 and S2 are **machine-verified green** on
Tamarin 1.12.0 / Maude 3.5.1 (`token_confinement`,
`cross_community_use_attempts_are_not_authorized`, the two
`minted_*_channels_match_stamp` lemmas, `token_stamp_matches_mint`,
`cross_community_mint_yields_no_token_for_that_request`, and the
`leaked_token_blast_radius_contained` / `leaked_token_can_authorize_within_its_community`
containment pair), each with its exists-trace sanity lemma also verified, and the
`MUTATION_Use_Token_Claimed_Community` mutation confirmed red
(`falsified — found trace`). S3 and S4 lemmas are **authored in the model but not
yet verified**; this draft claims S1/S2 as the proven milestone and tracks S3/S4
as the next proof round. The committed `.spthy` is byte-identical
(SHA-256 `1e7fb042…aceaacf24`) to the artifact behind the green S1/S2 run.

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
  error has been found.* — 102,742,532 states generated, 4,350,464 distinct, 0 left
  on queue, depth 13 (~5 min 45 s, single-threaded). Non-vacuity is shown by three mutations, each
  confirmed to produce a counterexample: substituting the unscoped direct-by-id
  lookup (`UnscopedDirectIdRows`, the `get_accessible_channel_ids` landmine) →
  `Safety` violated at depth 4; widening the sanitized-error label to all
  communities (the raw-error leak) → `Safety` violated at depth 2; and the
  global-id conflict key (M3: `WriteDuplicate` keyed on `id` alone via
  `GlobalConflictRows`, the missing-`community_id`-in-the-unique-index footgun)
  → `Safety` violated at depth 3, with a B-scoped `WriteResult` observation
  carrying `labels |-> {commA}` (the existence-oracle leak C2.1 closes). The
  `h`-tag mutation is the same shape (I2). The config is deliberately a
  fast non-vacuity harness, not the full deployment scale — widening workers,
  actors, and ids explodes the space; symmetry + bounded observations keep the
  core isolation surface exhaustively checkable.
- **`docs/spec/MultiTenantAuth.spthy`** — the Tamarin authorization model. Run:
  `tamarin-prover --prove docs/spec/MultiTenantAuth.spthy`. S1/S2 lemmas verify
  green (Tamarin 1.12.0 / Maude 3.5.1) — each paired with a verified exists-trace
  sanity lemma, and the commented `MUTATION_Use_Token_Claimed_Community` (authorize
  from a client-supplied tag) confirmed to falsify `token_confinement` when
  uncommented. S3/S4 lemmas are authored but not yet verified (see §Authorization
  soundness). The committed file is SHA-256 `1e7fb042…aceaacf24`.

  **Machine-check hygiene.** S1–S4 lemmas close by two distinct shapes.
  **Rule-shape closure** means the lemma's conclusion follows by unification on a
  single rule's action multiset: `token_confinement`,
  `audit_append_advances_same_community_head`, and the S2 supporting set
  (`minted_token_channels_match_stamp`, `minted_request_channels_match_stamp`,
  `token_stamp_matches_mint`). These are well-formedness guards on the model's
  action labels; the substantive security claim is carried by the corresponding
  rule design and mutation (for example, `MUTATION_Use_Token_Claimed_Community`
  falsifies `token_confinement` when authorization is rewritten to use a claimed
  community). **Substantive closure** requires cross-rule reasoning over
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
