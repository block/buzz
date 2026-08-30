# Redis/Valkey Cluster Mode for the Buzz Relay: A Formal Specification

`draft`

## Abstract

This document specifies the migration of the Buzz relay's Redis/Valkey usage
from a single-shard, cluster-mode-disabled ElastiCache replication group to a
cluster-mode-enabled, N-shard topology, and gives a formal proof of the
migration's safety-critical core. Valkey executes commands on one engine
thread per node; the deployed primary's engine core is the relay's hard
scaling ceiling, and it is filling on a measured doubling curve. Sharding the
keyspace across N primaries is the durable fix — **but only for keyed
commands**: classic pub/sub broadcasts every message to every node regardless
of shard count, so converting the relay's event fan-out to *sharded* pub/sub
(`SPUBLISH`/`SSUBSCRIBE`) is a prerequisite that feeds the sizing math, not a
migration detail.

The safety-critical core is the **fenced session directory**: a per-session
`{lease, generation}` key pair mutated atomically by Lua, whose generation
counter is the relay mesh's fencing authority — non-expiring, never reset,
never forked. Cluster mode forces these keys into one hash slot, which forces
a key rename, which forces a live migration of fencing authority under a
rolling deploy. We prove three safety theorems over that migration —
**single-authority** (at no instant do two live leases exist for one session,
across both old and new keyspaces, including under emergency rollback),
**generation-monotonicity** (no generation is ever issued twice across the
counter handoff or a rollback re-merge), and **slot-rules safety** (cluster
protocol modes are entered only after old-keyspace authority is provably
drained and the fleet runs only slot-clean scripts) — mechanized in TLA+
(`docs/spec/RedisClusterFencingMigration.tla`) with every invariant shown
non-vacuous by mutation (9 mutants, all killed). The proof is scoped
precisely: it covers acquisition authority, generation issuance, lease
migration, and the mode doors, under a **deployment-gate assumption (E1)
that must be discharged operationally** — it is *not* a property of our
Kubernetes rollout machinery (§Deployment Gate).

Everything else in the migration is specified as ordered, gated engineering
work with named failure modes: the split client architecture, the sharded
pub/sub conversion, the silently-wrong `SCAN` in mesh peer discovery, and the
staged ElastiCache mode change whose final step is irreversible.

## Scope and Non-Goals

This specification proves **safety** of the fencing-key migration and states
the rest of the migration as gated obligations. It deliberately does **not**
prove:

- **Liveness or performance.** That N shards hold the growth curve, or that
  resharding completes within a window, is empirical — characterized by the
  profiling gate (G0) and load tests, not by theorem.
- **Shard count.** The shardable-keyspace vs. unshardable-broadcast ratio is
  a measured input, not a theorem (§Open Decisions, D1 — composed
  2026-07-30). Committing a shard count before that profile existed would
  have been fiction with units; with it, initial N is an ops/cost trade.
- **redis-rs / deadpool-redis / ElastiCache internals.** The cluster client's
  MOVED/ASK routing, RESP3 push delivery, and ElastiCache's online resharding
  are trusted vendor components, admitted per-deployment by conformance gates
  (§Conformance), not reproven.
- **Valkey cluster correctness.** Hash-slot assignment (CRC16 mod 16384, hash
  tags), single-slot Lua atomicity, and slot migration semantics are stated
  as axioms with documentation cites.

Stating this boundary is part of the claim, in the house style of
`docs/git-on-object-storage.md`: "provably safe migration" without naming the
trust boundary does not survive scrutiny; "the fencing handoff is
machine-checked relative to four stated axioms, and every vendor behavior is
gated by an explicit conformance test" does.

## Problem Statement (measured, 2026-07-30)

ElastiCache replication group `buzz-001` (acct 433851229429, us-west-2):
Valkey 8.0.1, `cache.r7g.8xlarge`, 1 primary + 1 replica, cluster mode
disabled, auto-failover **enabled**, Multi-AZ **enabled** (verified via
`describe-replication-groups` / `describe-cache-clusters`,
profile `bb-public-operations-ro`).

- `EngineCPUUtilization` (primary): 0.3% (Jul 21) → 8.4% (Jul 27) → 19.5%
  (Jul 30). **Fitted post-onset clock (P3, log-linear, r²=0.92): doubling
  every ~3.4–3.5 days; ~7 days to 80%, ~8 to saturation.** (Kickoff's
  eyeballed "~3.5–4 days / 8–10 days" spanned the pre-onset regime — see the
  fit-window bullet below — and understated the urgency; the fitted number
  is the citable one.)
- Driver is command volume — product growth (~80k users added), not a leak.
  Fitted 9-day post-onset growth is x9.6–12.4 across all command families
  (kickoff's "~150x / ~400x in two weeks" were coarser eyeballs over a
  window spanning the onset).
- Whole-instance CPU ~4%, memory 0.09%, replication lag <1ms. Vertical
  scaling is dead: same-family bigger instances have identical single-core
  speed; r8g (Graviton4) buys ~25–30% single-thread ≈ 1.1–1.3 days at the
  fitted 3.45-day doubling (1.25–1.30x → Td·log₂(M); vendor range, not
  benchmarked by anyone in this effort).
- **Fit window (measured, P3; onset attributed 2026-07-30):** the curve has a
  sharp onset at 2026-07-21 **16:40Z** (5-min bins; the earlier ~18:00Z was a
  15-min-bin artifact) — flat ~0.55% for the preceding five days, then
  exponential (subscribed channels 74→1303 in four hours at a pinned
  connection count). Growth fits must start after the onset or they mix two
  regimes. `curr_connections` moves in pool-config plateaus (40.0 → 74.9 →
  ~280) and must not be read as demand. **The onset is not a deploy** — it is
  organic community creation (58 → 425 communities in two hours, first move in
  the same 5-min bin as the Redis breakout; one pub/sub topic per community
  makes the chain mechanical, and subscriptions-per-connection stayed flat
  across the onset, which excludes a subscription-behavior code change).
  Community creation has since gone **linear** (~3,100/day) while Redis load
  remains **exponential** — the exponential is carried by intensification
  within communities, so the curve-break indicator to watch is
  **channels-per-community** (rising as of Jul 30), not community count.
  Receipts: `RESEARCH/BB_PUBLIC_REDIS_ONSET_ATTRIBUTION.md`. Measurement
  trap: `cacheclusterid:buzz-001-001` exists in **four** AWS accounts, so
  every ElastiCache query must scope `aws_account:433851229429` — a bare-tag
  `avg:` dilutes engine CPU ~3.4x.

## System Model

A **relay pod** is stateless compute (15 pods, HPA to 25). All pods share one
Redis endpoint through two client surfaces (today): a `deadpool-redis`
command pool built from one URL (`crates/buzz-relay/src/main.rs`), and three
dedicated single-node pub/sub subscriber connections
(`crates/buzz-pubsub/src/{subscriber,cache_invalidation,conn_control}.rs`).

The relay's Redis state divides into **families with distinct consistency
obligations**. Making this distinction explicit is load-bearing for the whole
design; the two Lua-fenced keys and the discovery index must not be held to
the same standard:

| Family | Keys | Obligation | Cluster-mode status |
|---|---|---|---|
| **Fencing directory** | `buzz:<community>:tunnel:<session>:{lease,generation}` (`tunnel/directory.rs`) | **Strict**: atomic two-key Lua; generation is non-expiring fencing authority — never fork, never repeat | **Blocker (loud)**: keys share no hash tag; every script fails cross-slot. Migration is the mechanized core of this spec |
| **Mesh ready registry** | `mesh:ready:<runtime_id>` + `SCAN` discovery (`relay-mesh/registry.rs`) | **Hint, by its own contract** (`registry.rs:3-6`): entries are membership hints; the fenced directory arbitrates ownership. Bounded-staleness discovery is in-contract | **Blocker (silent)**: `SCAN` has no key, routes to one random node, returns a partial keyspace with no error |
| **Presence** | `buzz:<community>:presence:<pubkey>`, TTL 90s (`presence.rs`) | Ephemeral, self-healing | Safe: single-key ops; bulk `MGET` is auto-split per node by redis-rs (`MultipleNodeRoutingInfo::MultiSlot`, `cluster_handling/routing.rs`) |
| **Replay guard / rate limit** | `SET NX EX` (`nip98_replay.rs`); single-key Lua (`rate_limiter.rs`) | Per-key atomic | Safe once routed |
| **Event pub/sub** | `buzz:<community>:channel:<uuid>`, `buzz:<community>:global` — exact topics (`topic.rs`) | At-most-once transport hint; relay re-checks access on fan-out | **Unsharded by default** (Axiom A4). Exact topics convert to `SSUBSCRIBE`/`SPUBLISH`; conversion is prerequisite P2 |
| **Broadcast pub/sub** | `PSUBSCRIBE buzz:*:cache-invalidate`, `buzz:*:conn-control` | Must-not-miss control signals (cache coherence, live bans) | No sharded equivalent (`SPSUBSCRIBE` does not exist). **Decision D2**: stays classic — a missed transition is worse than a low-volume broadcast |

Command inventory provenance: deployed image `sha-22be8bb` = Buzz commit
`22be8bb35`, scoped `git grep` over `crates/buzz-pubsub`, `crates/buzz-relay`,
`crates/buzz-relay-mesh`; no `MULTI`/`EXEC`/pipeline found in that scope
(`RESEARCH/BB_PUBLIC_REDIS_CLUSTER_MODE_INVENTORY.md`). The two hot event-path
caches (`channel_visibility_cached`, `is_member_cached`) are in-process, not
Redis — they are not the get/set driver.

## Axioms

- **(A1) Deterministic slot assignment.** A key's slot is `CRC16(key) mod
  16384`; if the key contains a `{...}` hash tag, only the tagged substring is
  hashed. Two keys sharing a tag always share a slot, on every topology.
  (Valkey cluster spec, key distribution model.)
- **(A2) Single-slot Lua atomicity, declared-keys slot check.** A Lua script
  whose declared `KEYS` all map to one slot executes atomically on that
  slot's primary, exactly as on a standalone node. A script whose declared
  keys span slots is **rejected with `CROSSSLOT`** (loud failure) on any
  cluster-enabled node — **including a single-shard node that owns all
  16384 slots**: the check is on declared slot equality, not slot
  ownership (falsified live: single-shard cluster-enabled node, all slots
  assigned, `cluster_state:ok`, two-key `EVAL` on the un-tagged fencing
  pair → `CROSSSLOT`; raw RESP, no client library involved). Scripts must
  declare all keys via `KEYS`; ours do — and this is an **enforced
  prohibition, not an observation**: keys accessed via `redis.call` but
  not declared in `KEYS` bypass the slot check entirely and succeed on a
  single shard, then break silently the moment slots split. That is the
  deadline-pressure "fix" this spec explicitly forbids (verified live:
  `numkeys=0` script reading a tagged + untagged key succeeds). The G2
  validator (§Conformance) mechanically rejects undeclared key access.
- **(A3) Staged mode change; slot rules start at compatible.** ElastiCache
  supports `cluster-mode: disabled → compatible → enabled`. In *compatible*,
  one shard speaks both protocols and exposes a configuration endpoint;
  `compatible → disabled` is a supported revert. **`enabled` is
  irreversible** ("Reverting this configuration is not possible" —
  modify-cluster-mode.md). While *compatible*: scaling and engine-version
  changes are blocked. **Working assumption: slot rules (A2's `CROSSSLOT`
  rejection) are enforced from `compatible` onward** — compatible sets the
  cluster-enabled parameter, and the OSS behavior A2 documents attaches to
  cluster-enabled, not to shard count. We have not tested AWS compatible
  mode itself (it could conceivably relax slot checks for
  standalone-protocol clients); a one-command probe (`EVAL` on an
  un-tagged two-key pair) is a **pre-G3 checklist item**, and even if AWS
  turns out to permit it, the plan does not rely on that — vendor-tested
  leniency is not a protocol property, and it would silently vanish at
  `enabled`.
- **(A4) Classic pub/sub is broadcast.** `PUBLISH` to any node is propagated
  to every node in the cluster; each node's engine thread processes every
  message. Sharded pub/sub (`SPUBLISH`/`SSUBSCRIBE`) confines propagation to
  the channel's slot owner. (cluster-spec.md §pub/sub; pubsub.md §sharded.)

A3 and A4 are the two facts that shape the entire plan: A4 makes sharded
pub/sub a sizing prerequisite; A3 makes compatible mode the last revert point.

## Client Architecture (the split)

Verified constraints, from vendored `redis 1.2.4` / `deadpool-redis 0.23.0`
source:

1. `ssubscribe`/`sunsubscribe`/cluster `subscribe` **require RESP3**
   (`check_resp3!`, hard error on RESP2). We run RESP2 today.
2. The cluster async connection has **no `get_async_pubsub()` and no
   `split()`** — push messages arrive via an mpsc `push_sender` handed to
   `ClusterClientBuilder` at build time, and the client **auto-resubscribes**
   after disconnect, emitting duplicate subscription-confirmation pushes.
3. `deadpool-redis`'s cluster `Manager` builds its client internally and
   exposes **no push-sender hook** — subscriptions need their own client
   regardless of protocol choice.
4. Protocol is **per-connection**, set by URL (`?protocol=resp3`) — the two
   clients can run different protocols with no global switch.

Therefore the target architecture is a **split**:

- **Command pool**: `deadpool-redis` cluster pool (feature `cluster` →
  `redis/cluster-async`) for all routed traffic — `GET`/`SET`/`MGET`/Lua/
  `SPUBLISH`. Protocol (RESP2 vs RESP3) is a **test-backed decision**, not a
  forced rewrite: typed command APIs may remain source-compatible, but the
  choice is admitted only by the conformance gate (G2), never assumed.
- **Subscription client**: a separate raw `ClusterClientBuilder` with
  `push_sender` on RESP3, owning all three subscriber loops. The loops are
  **rearchitected** around push delivery (the `select!`-over-`split()` shape
  does not exist here); reconnect handling must tolerate auto-resubscribe and
  duplicate `SSubscribe` pushes.

**Version floor:** `redis` ≥ **1.4.1** (target **1.5.0**), for:
non-blocking replica connection repair (#2120, 1.4.0), cluster retry-backoff
clamp (#2158, 1.3.0), and READONLY-path sleep removal (#2223, 1.4.1).
**Residual failure mode, not a fixed bug:** on every released version, a
**dead primary halts dispatch** pending topology refresh. Any degradation
argument that assumes graceful behavior during primary loss is wrong; this
belongs in shard-count risk math (more shards = smaller blast radius but more
primaries that can die). **The 1.2.4 → 1.5.0 bump is verified free in the
current single-node world** (Dawn, on `origin/main` `73589408d`):
`cargo update -p redis --precise 1.5.0` resolves cleanly with no deadpool
bump and no other crate moved; `cargo build --workspace --all-targets` is
clean with zero source changes; all five redis-touching crates' tests pass
(buzz-core, buzz-pubsub, buzz-relay-mesh, buzz-admin, buzz-relay — 0
failed), **including buzz-pubsub's 11 live-Redis `#[ignore]` tests**
(pub/sub roundtrip, cache invalidation, presence, nip98 replay, cross-
community isolation). The floor can therefore ship ahead of and
independently from the cluster work — one less thing moving during the
migration. That run proves nothing about cluster behavior (#2120/#2223 are
cluster-only paths); the multi-shard suite (G2) still owns those. One
additional known gap: async pub/sub reconnect fix redis-rs#2242 merged
*after* 1.5.0's release — G2 must force-disconnect the subscription client
and assert resubscription + delivery, never infer it from command tests.

## The Fencing Migration (mechanized core)

### Why it exists

The directory's four Lua scripts (acquire/renew/release/validate) each
atomically touch `…:lease` and `…:generation`. Under cluster mode those keys
must share a slot (A1, A2), so they must be **renamed** to carry a shared hash
tag — e.g. `buzz:tunnel:{<community>:<session>}:lease` / `…:generation`. The
generation counter is deliberately non-expiring and is the mesh's fencing
authority (`directory.rs:1-6`): renaming it is a live handoff of authority
under a rolling deploy, where old-script and new-script pods coexist. Done
naively, two failure classes appear: **fork** (old-key lease and new-key lease
alive simultaneously — two owners for one session) and **generation reuse**
(new counter re-issues a generation the old counter already issued — a stale
frame passes the fence).

### The protocol

Script versions, deployed as a phased rollout. **The fleet spanning at most
two adjacent versions is NOT something Kubernetes gives us — it is
environment assumption E1, discharged by the Deployment Gate below.** All of
O/A/B run in **`disabled` mode only**: A and B are cross-slot scripts, and
cross-slot access is illegal on *any* cluster-enabled topology including a
single shard (A2). There is no "compatible-mode grace period" for the
migration — compatible comes after C.

- **O (legacy, deployed today):** old keys only.
- **A:** acquire checks **both** lease keys (union check); lease still
  written to the **old** key; generation issued as `max(oldGen, newGen) + 1`,
  written to the old counter. (The union *read* of both counters is the
  load-bearing part — mutation M5. An earlier draft also dual-wrote the
  generation to the new counter; mutation testing showed that write
  redundant, and it was removed: the smallest sufficient script wins.)
- **B:** union lease check; lease written to the **new** (hash-tagged) key;
  generation `max(oldGen, newGen) + 1` written to the **new** counter.
  **B's renewer migrates**: when a B-pod's renew tick finds its own lease
  still on the old key (acquired before the pod upgraded), it moves the
  lease to the new key — same owner, **same generation** (no new issuance),
  one atomic cross-slot Lua script, legal because we are in `disabled`
  mode. This is what makes the drain *reachable* (see C-gate below).
- **Backfill (once, after full-B):** fold `oldGen` into `newGen` by
  max-merge. Idempotent.
- **C:** new keys only. No cross-slot access exists anywhere. **C-gate
  (participation-based, not temporal):** backfill has run **and** no live
  old-key lease remains, verified **by direct observation** (count
  old-keyspace leases = 0), *never* by waiting out a TTL. A fixed wait is
  wrong on its face: `RENEW_SCRIPT` is `PEXPIRE` (`directory.rs`) on a 10s
  cadence against the 30s TTL (`reliable.rs`), so a renewed lease **never
  expires while its pod lives** — "wait one TTL after full-B" drains
  nothing, and no session-lifetime bound exists in `tunnel/` or `audio/`
  to save it. What actually drains the old keyspace: B-pods migrate their
  own leases at the next renew tick (≤10s), and leases whose owners died
  expire by TTL because nobody renews them. Full-B therefore implies
  drain within bounded time — but the gate checks the *state*, not the
  clock. (The huddle lane, `audio/join.rs`, mirrors the reliable lane's
  10s/30s renewer against the same directory and is expected to behave
  identically; that expectation is read, not traced, and the G2 drill
  covers both lanes explicitly.)
- **Enter compatible (G3):** only with the whole fleet on C. Slot rules
  turn on here (A3); C is the only slot-clean script version. Revert to
  `disabled` remains available.
- **Enable (G4):** `compatible → enabled` — the one-way door. Old keys
  become unreachable garbage; the fence never consults them again.

### Per-phase operation table

"Renew/release operate on whichever key the caller's lease names" is not
implementable with today's `SessionLease` — it carries no keyspace
discriminator (`directory.rs`, `SessionLease` fields). The migration does
not add one: instead each script version has a **fixed, version-local key
rule**, and the table below is the normative statement per operation. A
pod always addresses the keyspace its own script version dictates; the one
cross-version case (a B-pod holding a lease acquired while it ran A-code
on the old key) is handled by B's renew-migrate, not by lease-carried
state. This version-local rule is an assumption the conformance suite
tests across pod termination and rollback, not something the type system
enforces.

| Op | O | A | B | C |
|---|---|---|---|---|
| `acquire` | old key; `INCR` old ctr | union check; write old key; `max(old,new)+1` → old ctr | union check; write new key; `max(old,new)+1` → new ctr | new key; `INCR` new ctr |
| `renew` | old key `PEXPIRE` | old key `PEXPIRE` | **migrate-then-renew**: own lease on old key → move to new key (same gen, atomic); then new key `PEXPIRE` | new key `PEXPIRE` |
| `release` | old key | old key | try new key; fall back old key (owner+gen guarded, so wrong-key release is a no-op) | new key |
| `lookup` | old key | union (new wins) | union (new wins) | new key |
| `known_generation` | old ctr | `max(old, new)` ctrs | `max(old, new)` ctrs | new ctr |
| `validate_fenced_header` | old pair | union pair, `max` of counters | union pair, `max` of counters | new pair |

`validate_fenced_header` is the hop-by-hop fence (`directory.rs`); its A/B
variants read both pairs in one cross-slot script (legal: disabled mode)
and fence against the max — a frame that passes validation under A/B
would pass under whichever single keyspace currently holds authority,
because T1 guarantees at most one does.

### Scope of the mechanized proof

The TLA+ model covers **acquisition authority, generation issuance, lease
migration (B's renew-migrate), lease loss, phase machinery including
emergency rollback, and the mode doors**. It does not model renew, release,
lookup, or validate as distinct transitions, and that is a scoping decision,
not an omission:

- **renew** (TTL extension) is subsumed by the model's untimed
  nondeterminism — a renewed lease is exactly one where `Expire` has not
  yet fired. The renew-*migrate* step, which does change authority
  location, **is** modeled (`MigrateB`).
- **release** is state-identical to expiry (lease loss, generation
  preserved) — `Expire` covers both.
- **lookup / known_generation / validate** are read-only; they cannot
  violate T1/T2. Their per-phase read rules (table above) are specified
  and belong to the G2 test matrix.

So the honest claim is: **T1/T2 are proved for every write to fencing
state, under assumption E1; the read-side per-phase rules are specified
and tested, not proved.**

### Deployment Gate (discharging E1)

The model's phase machinery assumes: the fleet chases one target phase,
a phase advances only when every pod runs it, and rollback never skips a
phase or overlaps a running downgrade. **Kubernetes gives none of this.**
bb-public deploys a plain `RollingUpdate` (`maxSurge: 1`,
`maxUnavailable: 0`) via GitOps; nothing prevents applying B while A is
still rolling, rolling back two phases at once, or a new ReplicaSet
coexisting with two older ones. E1 is therefore an **operational gate**,
with these rules (each maps to a mutation that shows what its absence
costs):

1. **One image per phase.** Each of A, B, C is a separately released,
   separately tagged image. No phase is a config flag on a shared image.
2. **Advance gate:** the next image change is blocked until (a) every
   live pod reports the expected phase and (b) all older ReplicaSets are
   at zero. Pods expose their script phase as a **metric/readiness fact**
   (e.g. a `buzz_fencing_phase` gauge), so the gate checks *behavior*,
   not an image tag. (Absence → mutants M1/M2-class forks: O+B coexist
   and fork authority.)
3. **C-gate addition:** backfill complete **and** observed old-keyspace
   lease count = 0 (direct observation, per §The protocol). (Absence →
   M4.)
4. **Rollback matrix** (emergency path, modeled as `RollbackPhase`):
   - Roll back **one adjacent phase at a time**; never start a second
     rollback while pods are still above the current target (absence →
     M8).
   - Any rollback **invalidates the backfill**; it must re-run before the
     C-gate can ever pass again.
   - Rolling back past A into O additionally requires the **new keyspace
     lease-drained** (observed zero; absence → M6) and an operator-run
     **reverse max-merge** of the new counter into the old one — O issues
     from the old counter alone and would otherwise re-issue generations
     B already issued (absence → M7).
   - No phase rollback while in `compatible`; revert the mode to
     `disabled` first (the mode doors and phase machinery never move in
     the same step).
   - **Rolling back B→A drops every session B has already migrated —
     prefer forward-fix.** An A-pod's renew addresses the *old* key
     (§Per-phase operation table), so a lease `MigrateB` moved to the new
     key is renewed by nobody after the downgrade: it dies within one TTL
     and the renewer treats the loss as fatal (session torn down, verified
     live against `spawn_lease_renewer_with_interval`'s `Lost` handling).
     Safety is unaffected — A's union check sees the new-key lease and
     refuses to fork — but rollback from B is **not free**, and an
     operator reaching for it should expect to shed exactly the sessions
     the migration had already carried forward. (Liveness, so outside the
     safety model's scope; found and reproduced live by review round 2.)
5. **Pre-G3 checklist:** compute `CLUSTER KEYSLOT` on both members of a
   real hash-tagged pair and require equality **before** entering
   compatible — a mis-tagged pair is invisible in disabled mode (slot
   rules unenforced) and becomes a `CROSSSLOT` outage at G3. This is the
   only point the bug is catchable cheaply. Also run the one-command AWS
   compatible-mode probe from A3.

### Safety theorems

> **T1 (Single Authority).** At every instant of the migration — including
> under emergency rollback — at most one live lease exists per session
> across both keyspaces.
>
> **T2 (Generation Monotonicity).** Generations issued to leases are strictly
> increasing per session across the old→new counter handoff, the
> renew-migrate, and any rollback re-merge; no generation is ever issued
> twice.
>
> **T3 (Slot-Rules Safety).** A cluster-protocol mode (compatible or
> enabled) is entered only in states where old-keyspace authority is
> drained (backfill complete, no live old-key lease) and the whole fleet
> runs slot-clean C scripts — so no post-door execution ever attempts a
> cross-slot fencing script or consults a stale fencing key. That
> `enabled` is one-way is an environment axiom (A3), encoded as an action
> guard, not a theorem.

All three hold **under environment assumption E1** (Deployment Gate).

**Proof sketch.** T1: every acquire version checks the union of both lease
keys before creating authority; the renew-migrate moves a lease atomically
(delete-old and write-new in one script) so no interleaving observes two;
E1's phase adjacency means no pod that skips a keyspace coexists with a pod
that writes it. T2: A and B issue `max(oldGen,newGen)+1`; the migrate
carries an existing generation without issuing; the backfill max-merge makes
C's counter dominate every generation ever issued; the rollback reverse
max-merge restores that dominance to the old counter before O can issue
again. T3: the mode door requires `backfilled ∧ oldOwner = None ∧ fleet
fully C`, and phase rollback is disallowed while any cluster-protocol mode
is active. ∎

The sketch is not the proof; the model is.

### Mechanized verification

`docs/spec/RedisClusterFencingMigration.tla` models one session (sessions are
independent — per-session keys), 3 pods, phased deployment with per-pod
upgrade **and downgrade** interleaving, emergency rollback, TTL expiry,
B's renew-migrate, backfill, and the three-position mode variable
(disabled/compatible/enabled) with slot-rule enforcement from compatible
onward. TLC checks five invariants; the history variables (`lastIssued`,
`monoOk`) encode T2 as a single-run safety invariant.

```
$ java -cp tla2tools.jar tlc2.TLC RedisClusterFencingMigration.tla \
    -config RedisClusterFencingMigration.cfg -deadlock
Model checking completed. No error has been found.
12636 states generated, 3637 distinct states found.

$ java -cp tla2tools.jar tlc2.TLC RedisClusterFencingMigration.tla \
    -config RedisClusterFencingMigrationDrain.cfg -deadlock
Error: Invariant Probe_NotC is violated.   # INVERTED verdict: this is PASS
```

(State counts on the drain probe's violating run are **not reproducible**:
TLC halts at the first violation, so the count varies with worker count
and scheduling — 9/8 at `-workers 1`, ~100/~45 at `-workers 4`. Only the
verdict is the check. Exhaustive-pass counts, like the main model's
12636/3637 and M10's green run, are deterministic.)

(`-deadlock` disables deadlock reporting because the model has *intended*
terminal states — migration complete, or the MaxGen finiteness bound reached
— which TLC would otherwise report as errors. Pods = {p1,p2,p3}, MaxGen = 4.
Three pods exercise every adjacent-version race — old/old, old/new, new/new
writers; a fourth adds no qualitatively new interleaving. Bounded check,
mutation-shown non-vacuous — the standard claim for a TLC-checked safety
spec.)

**Every invariant is non-vacuous by mutation** (10 mutants, each run in
isolation, each distinguished from the true model — M1–M9 by a violated
safety invariant, M10 by the drain probe's inverted verdict):

| Mutation | Models the real bug | Trips |
|---|---|---|
| M1: `AcquireB` drops the old-lease check | new-script pod ignores legacy leases → two owners | `Inv_SingleAuthority` |
| M2: `AcquireA` drops the new-lease check | old-keyspace writer ignores new leases during rollback/mixed fleet | `Inv_SingleAuthority` |
| M3: `AcquireB` issues `newGen+1` without max-merge | fresh counter re-issues generation 1 → stale frame passes fence | `Inv_Monotonic` |
| M4: C-gate dropped (no backfill/drain requirement) | C deployed while an old-key lease is live → C-pod acquires over it | `Inv_SingleAuthority` |
| M5: `AcquireA` drops the union max-merge read | A never sees B's issues → reuse | `Inv_Monotonic` |
| M6: rollback into O drops the new-keyspace drain check | O-pod acquires on the old key while a B-era lease lives on the new key → fork | `Inv_SingleAuthority` |
| M7: rollback into O drops the reverse max-merge | O re-issues generations B already issued | `Inv_Monotonic` |
| M8: rollback allowed past a running downgrade | skipped-phase rollback: O and B pods coexist | `Inv_SingleAuthority` |
| M9: compatible entered without fleet fully on C | A/B cross-slot scripts meet live slot rules | `Inv_SlotRulesSafe` |
| M10: `MigrateB` deleted (round-1 behavior: B renews old-key leases instead of migrating) | migration stalls at the C-gate forever | `Probe_NotC` **stays green** (inverted verdict — see below) |

M10 needs a word, because it defends a fix the safety invariants cannot
see. The safety model lets leases expire spontaneously (`ExpireOld`
unguarded) — correct as the adversarial worst case for safety, but for
*drain* it is the friendly best case: it lets the C-gate's
`oldOwner = None` be discharged by luck, so deleting `MigrateB` from
`Next` leaves all five safety invariants green (found by review round 2).
The **drain-reachability probe** (`DrainSpec` +
`RedisClusterFencingMigrationDrain.cfg`) closes the hole: it starts TLC in
the stall state (fleet fully on B, backfilled, one live B-pod owning an
old-key lease), removes spontaneous expiry (faithful to a lease whose
owner is alive and renewing — renew is `PEXPIRE`), and checks
`Probe_NotC == phase /= 3` with **inverted verdict semantics**: TLC
reporting the "invariant" *violated* proves C is *reachable* (pass; state
counts on a violating run vary with worker scheduling — the verdict is
the check); TLC green means the migration deadlocks at the gate forever.
With `MigrateB`: violated (C reachable). M10 (without it): green — the
exact round-1 bug, now permanently visible to the model.

Both configs run in CI (`scripts/check-tla-specs.sh`, job `TLA+ Spec
Checks`, triggered by `docs/spec/**`), so the inverted verdict is
machinery rather than discipline: the script asserts on the specific
string `Invariant Probe_NotC is violated` rather than on TLC's exit
status, because a parse error also exits non-zero and must fail the build
instead of counting as the expected violation. tla2tools is pinned by
release tag *and* sha256. Deleting `MigrateB` fails the job.

M4 is the operationally scary one: it is exactly "someone deploys the final
script version early because everything looks green." M6–M8 are the price
of admitting that rollbacks happen; they are what the Deployment Gate's
rollback matrix discharges. Mutation testing also *removed* a mechanism:
A's dual-write of the generation counter survived every invariant when
deleted, so the protocol no longer carries it (§The protocol).

## Sharded Pub/Sub Conversion (prerequisite P2)

By A4, shard count does not touch the pub/sub term; conversion does. It is
the fastest-growing command family (x12.4 over the fitted 9-day window; the
kickoff "~400x" was a coarser 14-day eyeball — cite the fitted number) and
the only one whose per-op cost is rising (1.18x/9d, tracking channel count
x12.7).

- **Exact event topics** (`buzz:<community>:channel:<uuid>`,
  `buzz:<community>:global`) convert to `SSUBSCRIBE`/`SPUBLISH`. The existing
  dynamic per-topic subscribe pattern maps directly (separate `SSUBSCRIBE`
  calls may span slots). Note: hash-tagging is deliberately **not** applied to
  topic names — each topic hashing to its own slot is what spreads
  propagation cost across shards; a `{community}` tag would recreate the hot
  node per community.
- **The effective unit of spread is per-community, not per-channel**
  (measured, G0 profile 2026-07-30): 87.8% of publish volume targets
  `buzz:<community>:global` (presence kind 20001 = 60.8%, agent observer
  frames kind 24200 = 27.0%); only typing and channel messages use
  per-channel topics. Slot spreading still works — no community exceeded
  ~4% of publish volume — but it is sub-linear: CRC16 simulation over real
  topic names and measured weights puts the hottest shard at ~1.5x its fair
  share at 4 shards (p95), growing with shard count. D1's arithmetic must
  size for the hottest shard, not `current/N`.
- **Conversion payoff is bounded by measured amplification, not worst-case
  broadcast**: cross-pod deliveries run ~5.3 pod-deliveries per publish on a
  15-pod fleet (dynamic interest subscription already avoids full broadcast),
  so P2 buys back at most that flowing cost. Real but bounded; see
  `RESEARCH/BB_PUBLIC_REDIS_G0_PROFILE.md` for receipts.
- **The two pattern subscribers stay classic** (Decision D2, argued in-thread
  and accepted): enumerating concrete channels would require a new
  subscribe-before-reachable / unsubscribe-after-last protocol with discovery
  races, and for cache invalidation and live-ban control a missed transition
  is strictly worse than a low-volume broadcast path. This accepts a
  permanently unsharded broadcast channel, gated by a volume measurement (G0
  must confirm these two channels are noise; if they are not, D2 reopens).

## Mesh Registry Discovery (the silent one)

`scan_ready()` (`relay-mesh/registry.rs`) discovers peers by `SCAN MATCH
mesh:ready:* COUNT 100`. `SCAN` carries no key; redis-rs routes it to **one
random node** (`RouteBy::Undefined` → `SingleNodeRoutingInfo::Random`) and the
cursor legitimately reaches 0 on that node — so under cluster mode discovery
**silently returns a per-call-varying subset** of peers. No error is ever
raised. This is the only cluster-unsafe site that fails silently, and it must
be tested by asserting **completeness** against a multi-shard cluster, not by
checking for errors.

**Fix:** replace keyspace scanning with a **scored-expiry index** in one slot:
heartbeat does `ZADD <index> <expiry_ts> <runtime_id>` alongside the existing
`SET EX` record; discovery reads live-scored members, `MGET`s their records,
filters, and opportunistically prunes dead scores. Staleness becomes
*representable* (a score in the past) rather than an agreement property
between two structures. The registry's own contract makes this sound: entries
are **membership hints** — the fenced directory arbitrates ownership — and the
read path already skips missing/undecodable/unattested records by design. The
invariant is **bounded discoverability** (a live runtime is discoverable
within one heartbeat interval; no unauthenticated record is ever returned),
*not* set equality. The single-slot concentration clears on measured load:
one `ZADD` per runtime per 15s across 15–25 pods ≈ 1–2 ops/s, three orders
below the hot path — deployment-global, so re-check if fleet size explodes.

**This spec is explicit that the registry's consistency obligations are
weaker than the tunnel directory's *by design*.** Holding both to the fencing
standard would gold-plate a hint; holding the fence to the hint standard
would corrupt sessions. (Table in §System Model.)

## Migration Plan (gates, in order)

The ordering headline (changed from the first draft, per Dawn's live
falsification): **the entire fencing sequence A→B→backfill→drain→C happens
in `disabled` mode.** Compatible mode is not a grace period for cross-slot
work — slot rules are assumed live from compatible onward (A3). This is
strictly safer than the original ordering and costs nothing; it also means
the vertical escape hatch (r8g) stays available for the whole of G1, since
the scaling freeze only starts at G3.

**G0 — Profile the hot path** *(informs everything; owner: option-3 owner)*.
Attribute the 150x get/set and 400x pub/sub growth to code paths; measure the
shardable-vs-broadcast ratio (this is the shard-count input) and the two
pattern-subscriber volumes (gates D2). Known: in-process caches are not the
driver; suspects are presence and per-event rate-limiter Lua — hypothesis,
not finding. If the driver is the rate limiter, "batch it" is not available
(atomic per-event counter); volume reduction takes a different shape.

**G1 — Code prerequisites, all in `disabled` mode** *(can start now,
independent of G0)*:
  1. `redis` 1.2.4 → 1.5.0 (verified free; ship first, separately).
  2. Fencing migration phases A→B→backfill→C per the mechanized protocol
     and the Deployment Gate (one image per phase, phase metric, advance
     gates, rollback matrix).
  3. Registry scored-expiry index replacing `SCAN`.
  4. Split client architecture + feature flags.
  5. Subscriber loops rearchitected (push_sender, RESP3, auto-resubscribe).
  6. Sharded pub/sub conversion for exact topics (D2 leaves patterns classic).

**G2 — Conformance against a real multi-shard Valkey cluster** (test matrix
below). The A/B/backfill/C staged deploy gets its own **disabled-mode
integration/chaos gate** (it can never legally run on a cluster-enabled
topology); everything else tests C-scripts on multi-shard. No date is
committed before this compiles and passes.

**G3 — `cluster-mode=compatible`** (revert available). Entered only with
the fleet fully on C (T3). Pre-G3 checklist: `CLUSTER KEYSLOT` equality
assertion on a real tagged pair; the AWS compatible-mode cross-slot probe
(A3). Client cutover to the configuration endpoint; validate under real
traffic. No scaling/engine changes while here (A3) — **the r8g tourniquet
and compatible-onward are mutually exclusive in flight**; entering G3
forecloses the vertical escape hatch for the duration.

**G4 — `cluster-mode=enabled`** — **the one-way door** (A3). Requires:
G2/G3 green and a key-size sanity pass (slots holding items >256MB silently
refuse to migrate; our 0.09% memory makes this unlikely — "unlikely" is not
"checked").

**G5 — Add shards** (online resharding). Do it **early**: AWS guidance says
keep CPU <80% during resharding — resharding is compute-intensive, and doing
it while the engine core saturates is the worst time. The countdown clock is
the argument for starting G1 now, not for heroics at 90%.

**Fallback (curve outruns the plan):** replica read offload. Downgraded but
not dead (P3, measured 2026-07-30): the replica is **not idle** — its 7.8%
engine CPU is its own write-apply + propagation load, unsheddable, on its own
exponential (doubling ~3.8d; saturates alone in ~14 days serving zero reads).
Offload still buys primary time, but the destination is a node ~6 days behind
the primary on the same clock — size against its remaining headroom, not
"93% free". Note the coupling: in cluster mode, replica reads lean on exactly
the replica-repair path fixed in redis-rs #2120/#2223, so the version floor
is a prerequisite for the fallback too, not just hygiene.

## Conformance (test matrix, gate G2)

Run against a real multi-shard Valkey 8 cluster (not a mock, not a single
node in cluster mode) — except the fencing-migration row, which by
construction runs on a standalone node (disabled mode is the only place
A/B legally execute). Two verdict columns — **errors** and **silently
wrong** — because three of our bugs would pass an error-only harness:

| Surface | Must verify |
|---|---|
| Routing | MOVED/ASK redirects under slot migration; TLS/auth on the configuration endpoint |
| **Locality contract** | Atomic-looking work is never silently scattered: multi-key ops either share a slot or are explicitly per-key pipelines with partial-failure handling declared at the call site (the ioredis production resolution; Stripe's narrow-tag rule). A **GitLab-style executable cross-slot validator** in CI computes slots for every Lua `KEYS` declaration, pipeline, and multi-key command in the workspace, and rejects undeclared `redis.call` key access (A2's prohibition) |
| Fencing scripts | Hash-tagged pairs execute atomically; `CROSSSLOT` observed for un-tagged pairs (negative test); `CLUSTER KEYSLOT` equality asserted on real tagged pairs; per-node script cache / NOSCRIPT handling |
| Fencing migration | A/B/backfill/drain/C staged deploy against live traffic **on a standalone (disabled-mode) node** with chaos (pod kill mid-phase, rollback per the matrix, both tunnel lanes — reliable and huddle); generation strictly increases across the handoff and the renew-migrate (assert, don't assume); old-keyspace lease count observed to reach zero |
| Presence `MGET` | Nil/order preservation when split per node |
| Registry index | **Completeness**: every live runtime discoverable within one heartbeat; assert against known population, not absence of errors |
| Subscriptions | RESP3 push delivery; dynamic ssubscribe/sunsubscribe; **forced disconnect** of the subscription client with asserted resubscription + delivery (redis-rs#2242 merged after 1.5.0 — never infer this from command tests); duplicate subscription-confirmation pushes handled; message routing correct across shards |
| Failure drills | Replica connection repair **under load** (the #2120 path); primary failover — characterize the dispatch halt window; both clients (command pool + subscription client) have independent recovery behavior — drill both |
| Protocol choice | Command pool compiled and integration-tested under its chosen protocol (RESP2 or RESP3) — admitted by test, never assumed |
| Telemetry | Per-shard/slot/channel metrics exist before G5: aggregate CPU cannot attribute a hot slot, and the phase gauge (`buzz_fencing_phase`) feeds the Deployment Gate |

## Failure Modes (named, with dispositions)

| Failure | Loud/Silent | Disposition |
|---|---|---|
| Cross-slot Lua (fencing keys, pre-fix) | Loud | Fixed by design (hash tag + mechanized migration) |
| Mis-tagged fencing pair (looks fine in disabled mode) | **Silent until G3**, then loud outage | Pre-G3 `CLUSTER KEYSLOT` equality assertion; G2 validator |
| Undeclared `redis.call` key access (bypasses slot check) | **Silent** until slots split | Prohibited by A2; G2 validator rejects mechanically |
| Deployment gate breach (phase skip, overlapping rollback, early phase apply) | **Silent** (authority fork / generation reuse) | E1 discharged operationally (§Deployment Gate); mutants M6–M8 show the cost |
| Old-keyspace drain assumed temporal ("wait one TTL") | **Silent** (renew = `PEXPIRE`; renewed leases never expire) | C-gate is participation-based + observed-zero; B renew-migrates |
| `SCAN` partial discovery | **Silent** | Fixed by design (scored-expiry index); completeness-asserting test |
| Dead primary halts client dispatch pending topology refresh | Loud-ish (latency wall) | Residual on all redis-rs versions; goes in shard-count risk math; drilled in G2 |
| Missed cache-invalidate / conn-control message | Silent | Avoided by D2 (patterns stay classic broadcast) |
| Generation fork/reuse during migration | Silent (fence passes stale frame) | Excluded by T1/T2 (mechanized, mutation-tested) |
| Premature `compatible`/`enabled` | Loud (`CROSSSLOT`) / Irreversible | Excluded by T3 gate + A3 named as one-way door |
| Subscription client fails to resubscribe after disconnect | **Silent** (missed events) | Forced-disconnect drill in G2 (#2242 postdates 1.5.0) |
| >256MB items refuse slot migration | Silent (permanent imbalance) | Key-size pass at G4 |
| Resharding under CPU pressure | Loud | G5 scheduled early, <80% CPU rule |

## Implementation Correspondence

Code pins are at deployed commit `22be8bb35` (verified against image tag
`sha-22be8bb`); symbols, not line numbers, are normative after refactors.

| Spec element | Code |
|---|---|
| Fencing scripts (acquire/renew/release/validate) | `crates/buzz-relay/src/tunnel/directory.rs` (`ACQUIRE_SCRIPT` etc.) |
| Fencing keys needing hash tag | `SessionKeys::new` (`directory.rs`) |
| Lease TTL (drain window for C-gate) | `DEFAULT_LEASE_TTL` = 30s (`directory.rs`) |
| Registry `SCAN` to replace | `scan_ready` (`relay-mesh/registry.rs`) |
| Registry heartbeat to extend with `ZADD` | `publish_ready` (`registry.rs`), `spawn_registry_heartbeat` (`runtime.rs`) |
| Exact topics for sharded pub/sub | `EventTopicKey::redis_channel` (`buzz-pubsub/src/topic.rs`) |
| Pattern subscribers staying classic | `cache_invalidation.rs`, `conn_control.rs` (`psubscribe`) |
| Subscriber loops to rearchitect | `subscriber.rs` (`get_async_pubsub` + `split`) |
| Huddle-lane lease renewer (mirrors reliable lane; drilled in G2) | `audio/join.rs` |
| Presence bulk read (auto-split, no change) | `get_presence_bulk` (`presence.rs`) |
| Command pool construction | `deadpool_redis::Config::from_url` (`buzz-relay/src/main.rs`) |
| Feature flags to add | workspace `Cargo.toml` `redis`/`deadpool-redis` entries |
| Phase gauge for the Deployment Gate | new metric (`buzz_fencing_phase`), exported per pod |

## Prior Art (what this spec adopts, and from where)

Three source-linked briefs inform this design (`RESEARCH/
REDIS_CLUSTER_PRODUCTION_PRIOR_ART.md`, `…_OPEN_SOURCE_PRIOR_ART.md`,
`…_OPERATIONAL_PRIOR_ART.md`):

- **Stripe** (brandur.org/redis-cluster): a single hot engine core at scale,
  fixed by cluster mode with **narrow, entity-scoped hash tags** — adopted
  as the per-session fencing tag and the no-`{community}`-tag rule for
  topics. Their hardest part was hardening the client, not operating the
  cluster; this spec's weight distribution (client architecture + G2 over
  cluster ops) follows.
- **GitLab** (`multi_store.rb`, `redis_cluster_validator`): treats the move
  as an application migration with an old/dual/new state machine and an
  executable cross-slot validator — adopted as the O/A/B/C protocol shape
  (with the fencing-specific monotonic-generation handling GitLab doesn't
  need) and the G2 validator row.
- **BullMQ**: smallest-atomic-domain hash tags, spread across shards —
  corroborates per-session tagging; its years of cluster-fix changelog is
  the cautionary tale behind the version floor and G2's chaos rows.
- **Sidekiq** (negative prior art): refuses cluster configuration except a
  narrow cluster-safe subsystem — adopted as the per-family obligations
  table: cluster readiness is a capability boundary, not a client flag.
- **Grafana Alertmanager HA**: classic pub/sub coexisting with a cluster
  client — supports D2 (patterns stay classic) as viable, while proving
  nothing about fan-out capacity.
- **ioredis #1842** (maintainer resolution): the locality contract — atomic
  work gets a narrow tag; unrelated multi-key work becomes per-slot
  pipelines with *visible* partial-failure semantics — adopted verbatim as
  the G2 locality row.
- **No published production precedent** exists for sharded pub/sub at
  Buzz-like fan-out scale (searched; Stripe validates command sharding
  only). The `SPUBLISH`/`SSUBSCRIBE` conversion therefore carries its own
  Buzz-specific load/reconnect gate rather than an appeal to prior art.
- **AWS guidance** (modify-cluster-mode.md, online-resharding best
  practices): compatible-as-revert-point (A3) and the <80% CPU resharding
  rule (G5).

## Open Decisions

- **D1 — Shard count.** Blocked on G0's shardable/broadcast ratio. The spec
  deliberately refuses a number until the ratio exists. First G0 inputs are
  in (2026-07-30): keyed-command growth is uniform (no hot path in the
  command mix), and the hottest-shard imbalance factor is ~1.5x fair share
  at 4 shards (see §Sharded Pub/Sub Conversion) — do not size by
  `current/N`. The earlier caveat that per-family `rate x latency` explains
  under a quarter of engine CPU is now resolved by measurement (P3,
  replica-subtraction natural experiment: the never-read replica carries
  7.8pp of the primary's 19.5pp): **~40% of primary engine CPU is
  write-apply + publish propagation; ~60% is client-facing** (reads, Lua,
  connection I/O, subscriber delivery). That split answers *what the
  replica pays*, not *what sharding divides* — reads and Lua are keyed and
  divide by N exactly like writes, so the genuinely non-dividing terms are
  narrower: classic pub/sub propagation (pre-P2 only — by A4 every node
  processes every publish until P2 lands, after which propagation confines
  to the owning shard) and client connection I/O (every pod dials every
  shard). Estimated shardable fraction: **f ≈ 0.60 pre-P2 (band
  0.45–0.70), f ≈ 0.75 post-P2 (band 0.60–0.85)**. Two further cautions
  from the same measurement set: engine CPU cannot be split
  pub/sub-vs-keyed by regression (rates collinear at r=0.997; subsampled
  shares swing 4%→37%), and the apparent sublinearity of CPU vs. rate is a
  ~1.1% fixed baseline, not economies of scale — do not project on the
  exponent. **Composed arithmetic (2026-07-30,
  `RESEARCH/BB_PUBLIC_REDIS_D1_SHARD_COUNT_ARITHMETIC.md`):** with
  `scale(N,f) = (1-f) + f·IMB(N)/N` applied to the variable load only (the
  ~1.1% baseline must not be scaled), at the central 3.45-day doubling the
  do-nothing runway is 7.2d — **band 5.2–9.0d across fit windows; plan
  against the band, not the point** — and N=4 buys **+2.3d pre-P2 / +3.1d
  post-P2** (N=8 post-P2: +4.3d). Any one-time capacity gain of factor M
  buys exactly `Td·log₂(M)` days, so no N reachable this week buys two
  doublings, and every available lever — sharding, r8g (+1.3d at the
  optimistic 1.30x edge), presence
  reduction, all stacked (~5.0d; 4.8d interaction-aware) — is smaller than the migration's own
  duration estimate (14d aggressive / 21d likely / 35d conservative; soft,
  LOC-derived, not a bottom-up plan). **Shard count is therefore not a
  runway purchase, and the honest case for cluster mode is optionality,
  not headroom**: G5 online resharding makes N a repeatable dial — each
  doubling of N buys another Td on demand, without another one-way door —
  while single-shard has no dial left (r8g is one-shot and forecloses G3
  while in flight). This case does not depend on the growth curve holding;
  the runway numbers do, and at 3.45d the curve implies unreachable user
  counts within a month, so *when it flattens* is a product question, and
  if it does not flatten on its own, the gap closes with product-side load
  reduction, not any infra lever. D1's remaining open item is the choice
  of initial N — an ops/cost trade within 2–8, no longer a capacity
  computation.
- **D2 — Pattern subscribers stay classic.** Accepted in design; re-opens
  only if G0 shows their volume is non-trivial. **Measured 2026-07-30: D2
  holds.** All publishers are lifecycle/admin-rate (code trace, verified
  independently); the events-stored ceiling (131/s fleet-summed) sits ~10x
  under the total pub/sub term. Per-pattern publish counters land with G2
  per-shard telemetry.
- **D3 — Command-pool protocol (RESP2 vs RESP3).** Test-backed decision at
  G2; the split architecture makes it independent of the subscription
  client's hard RESP3 requirement.
- **D4 — G0 ownership.** Profiling was kicked off as option 3; needs an
  owner and telemetry access. It gates D1, so it is on the critical path for
  shard count but *not* for starting G1.

## Summary

| Property | Status | Discharged by |
|---|---|---|
| Fencing single-authority (T1) | Proved (write-side, under E1) | TLA+ model, 9 mutants killed |
| Generation monotonicity (T2) | Proved (write-side, under E1) | TLA+ model, incl. rollback re-merge |
| Slot-rules safety (T3) | Proved (under E1) | Mode-door gate + A3, mechanized |
| Deployment gate E1 | **Operational obligation** | §Deployment Gate (phase images, advance gates, rollback matrix, phase metric) |
| Read-side per-phase rules (lookup/validate/renew/release) | Specified + tested | §Per-phase operation table; G2 |
| `redis` 1.2.4 → 1.5.0 | Verified free (single-node) | Dawn's build+test run on `73589408d`; cluster paths remain G2's |
| Vendor cluster behaviors | Empirical | Conformance matrix (G2) |
| Sharded pub/sub necessity | Documented fact | Axiom A4 (Valkey cluster spec) |
| Sharded pub/sub at scale | **No prior art** | Buzz-specific load/reconnect gate in G2 |
| Shard count | **Arithmetic composed** — N=4 buys +2.3d pre-P2 / +3.1d post-P2; value is the repeatable dial, not runway. Open only on initial N (ops/cost, 2–8) | D1 composed arithmetic |
| Timeline | ~7 days to 80% on the primary (fitted central; band 5.2–9.0d across fit windows) | G1 starts now; fallback = replica offload (degraded — replica not idle, ~14d to its own saturation) |
