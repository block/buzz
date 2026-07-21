# Git Refs over Object Storage: A Formal Specification

`draft`

## Abstract

This document specifies a protocol for hosting git repositories on an object
store (S3 and S3-compatible backends such as MinIO) with **no persistent
filesystem**, and gives a formal proof of its safety properties. Repository
content is stored as create-only, content-addressed pack objects (immutable by
protocol discipline — see A1, not by assuming an immutable store); the current
state of every ref is captured by a single mutable *manifest pointer* updated
by atomic compare-and-swap (CAS). We prove two safety theorems —
**durability-ordering** (a client never observes success for a ref change that
is not yet durable) and **manifest reconstruction** (hydrating a published
manifest reconstructs every object reachable from its refs; the named pack set is
a superset of that closure) — and one
**linearizability** theorem (concurrent ref-changing pushes never lose an
update). All three reduce to three explicitly stated object-store axioms.

The protocol is not novel as an *algorithm*: it is git's post-reftable ref
model — immutable content artifacts plus an atomic pointer swap — with the
atomic primitive substituted from POSIX `rename()` to an S3 conditional `PUT`.
The contribution of this document is the **formal characterization**: to our
knowledge, no prior formal treatment of git refs over conditional-write object
storage exists (see Prior Art). The proof is *parametric over the atomic
primitive*: it holds for any backend satisfying the three axioms, and a concrete
backend is *admitted for deployment* by a single conformance gate (a finite probe
cannot prove a universal axiom; it can only admit or reject a backend).

## Scope and Non-Goals

This specification proves **safety** ("nothing bad happens"). It deliberately
does **not** prove:

- **Liveness or performance.** That a hydrate completes within a latency budget
  is empirical, not formal; it is characterized by benchmark, not theorem.
- **Git's internal correctness.** `git index-pack`, `upload-pack`, and
  `receive-pack` are trusted upstream components. We prove only that our
  *composition* feeds them well-formed inputs and surfaces their outputs
  faithfully.
- **The object-store axioms themselves.** S3's durability, read-after-write
  consistency, and conditional-write linearizability are stated as axioms
  (§Axioms); each backend is *admitted* per-deployment by an empirical conformance
  gate (§Conformance), which rejects non-conforming backends — it does not prove
  the axiom universally.

Stating this boundary is part of the claim. "Provably sound" without naming the
trust boundary does not survive scrutiny; "safety is machine-checkable relative
to three stated axioms, each empirically gated per backend" does.

### v1 deployment architecture

The implementation has *no authoritative per-repo filesystem state*. Every
request hydrates an ephemeral working tree from the published manifest, runs
the appropriate git subprocess against it, and drops the tree on scope exit:
read paths (`info/refs`, `upload-pack`) via `hydrate_for_read`, the write path
(`receive-pack`) via `hydrate_for_write`, which also returns the `ParentState`
the CAS at §Push step 7 predicates on. The relay is multi-instance-ready by
construction: nothing on local disk needs to be coordinated between instances.
Each process may retain a byte-bounded, process-lifetime cache of immutable
pack/index pairs keyed by verified digest. Deployment mounts that cache on a
per-pod ephemeral volume, so accounting and single-flight coordination stay
local. Cache misses, restarts, and evictions only affect performance; object
storage remains the source of truth, and per-request refs/HEAD are still
materialized from the current manifest.

The accepted v1 tradeoff: under concurrent same-repo pushes, every contender
hydrates and runs receive-pack, and the CAS losers' subprocess work is
discarded. This is wasted CPU/IO under contention, not a correctness bug —
`Inv_NoFork` (Theorem 3) holds because the CAS is the only writer
serialization. Same-ref concurrent push is rare; the alternative (a
cross-instance lock service) is the kind of dependency this protocol exists
to avoid. If contention ever shows up in metrics the fix is a short
best-effort *local* lock as a latency optimization, never a correctness
dependency.

A bounded retry layer on classified-terminal-vs-transport errors is **parked,
not closed.** The checked-in regression fence is the 8-way live CAS race
(`e2e_git::git_concurrent_push_one_wins_and_repo_recovers`), which passes
against MinIO with no retry layer; we ship v1 without one. (A one-off 16-way
local run against MinIO also passed, as separate calibration evidence that
the property holds at greater width — the regression test stays at 8 because
each contender clones/commits/pushes through real `git` and the cost grows
with width.) The open question — "is the no-retry default safe past MinIO
and beyond the widths so far exercised?" — re-opens on a different backend
or a sustained-load regime the conformance probe (§Conformance) doesn't
already exercise. The non-negotiable rule: retry, if added, lives in the
store layer and retries *only* pre-classification network errors — never
`Ok(2xx)`, `LostRace(412)`, or `NotFound(404)`. Retrying a classified
outcome would change the TLA action and break the proof.

## System Model

A **repository** `R` has the following state in the object store:

- A set `P_R` of **pack objects**. Each is *content-addressed* (its key is a
  cryptographic digest of its bytes) and *treated as immutable by protocol*:
  written create-only, so the same key is never overwritten, and verified by
  digest on read (see A1 — this is protocol discipline, not an assumed store
  property). Writing the same content twice is idempotent.
- A single **manifest pointer** `M_R`: a mutable object holding the digest (and
  ETag) of the *current manifest*.

A **manifest** `m` is itself an immutable, content-addressed object containing:

- `m.packs` — the set of pack-object keys that constitute the repository.
- `m.refs` — a total map `refname ↦ object_id` (the published ref state).

We write `pointer(R) = (e, d)` for the current pointer state: `e` is the
object-store ETag of `M_R`, `d` is the manifest digest it holds. `manifest(d)`
denotes the (immutable) manifest object with digest `d`.

Two operations act on `R`:

- **Read(R)** — clone / fetch / ls-remote. Resolve `pointer(R) → d`, read
  `manifest(d)`, download `m.packs`, reconstruct the object graph, serve the
  requested refs.
- **Push(R, Δ)** — receive-pack. Δ is a set of requested ref updates
  `refname ↦ (old_id, new_id)`.

**The manifest pointer is the sole source of truth.** In Buzz, a successful
push may also publish a relay event (kind:30618) so subscribers learn refs moved.
That event is a *derived notification*, never the commit point: a push has
happened iff `M_R` was CAS-swapped (A3), regardless of whether — or when — any
event is published. Relay events can lag, duplicate, or replay; subscribers must
treat them only as a signal to re-read `pointer(R)`, never as ref state. The
dangerous inversion — "the push succeeded because the event published" — would
substitute relay ordering for manifest ordering and reintroduce the lost-update
hole A3 closes. (This is a real behavioral obligation, not a restatement: today
the commit point is the local filesystem — `receive-pack` updating the bare repo,
with the relay event published best-effort afterward — so making manifest-CAS the
commit, with the event derived from it, is the change, not just a storage swap.
See §Implementation Correspondence.)

## Axioms

The protocol's safety is proved *relative to* the following properties of the
object store. Each is a documented property of AWS S3 (2024+) and a testable
assumption for any S3-compatible backend.

- **(A1) Durable write.** A `PUT` that returns success is durable. S3 does *not*
  by itself make an object immutable — that is enforced by **protocol rule**, not
  assumed of the store: pack and manifest objects use content-addressed keys and
  are written create-only (`If-None-Match: *`), so a key is never overwritten,
  and readers verify the object's bytes against its key digest. Any deviation is
  therefore *detectable* (digest mismatch), not silent. (We do not rely on S3
  Object Lock or bucket immutability policy; the create-only + content-address
  discipline is sufficient and backend-portable.) **No deletion under the
  protocol.** Pack and manifest objects are never deleted by the protocol. This
  is what makes Read a consistent snapshot in the presence of concurrent writers:
  a reader holding an old manifest digest can always GET every pack it names,
  because no writer removes packs. Physical pruning of unreachable packs is a
  *backend retention concern* outside this proof boundary; any such sweep must
  honor in-flight readers (e.g. a retention window longer than the max hydrate
  time), and proving that bound is future GC work, not part of the safety
  argument here. (Without this rule, a GC that prunes packs a winning push
  orphaned could 404 a concurrent reader mid-hydrate — see Theorem 2's reliance
  on every named pack being GETtable.)

  The relay includes an opt-in, leader-elected **dry-run inventory** for this
  future GC (`BUZZ_GIT_GC_ENABLED=true`). It lists only `manifests/`, `packs/`,
  and `idx/`, computes reachability from every current `repos/*/pointer`, and
  exports observed candidate counts and bytes. Pointer count, listed objects,
  manifest bytes, and scan duration are bounded; truncated prefix inventories
  are explicitly reported as incomplete. It never deletes. A candidate report
  is not a deletion proof: production reclamation additionally requires
  durable evidence that an object stayed unreachable longer than the maximum
  read lifetime and coordination preventing a concurrent publisher from
  making an old content-addressed object reachable during sweep.

- **(A2) Strong read-after-write.** A read issued after a successful `PUT`
  observes that write. (AWS S3 provides this for all regions and all
  PUT/DELETE.)

- **(A3) Linearizable conditional write (CAS).** `PUT M_R If-Match: e` succeeds
  iff the current ETag of `M_R` equals `e`; otherwise it fails with a
  precondition error and does not modify `M_R`. Among any set of conditional
  PUTs predicated on the same `e`, **at most one succeeds**, and all PUTs are
  linearizable (there is a single total order consistent with observed
  successes/failures). "Linearizable conditional write" is *our* formal term for
  the axiom. The supporting AWS evidence: S3 documents `If-Match` as comparing the
  supplied ETag against the current object ETag (match → `200`, mismatch → `412`)
  and strong read-after-write consistency in all regions; the Nov 2024
  conditional-update announcement states this offloads compare-and-swap to S3.
  (`If-None-Match: *` for create-only PUT shipped Aug 2024.) AWS does not use the
  word "linearizable" in the user guide — we treat the documented CAS + strong
  consistency as evidence *for* the axiom, not as AWS asserting our term.

A3 is the single load-bearing backend assumption. It replaces the POSIX
`rename()` atomicity that reftable relies on. See §Conformance for how a backend
is *admitted* against it.

## Protocol

### Read

1. Resolve `pointer(R) = (e, d)`.
2. Fetch `manifest(d)`; let `m = manifest(d)`.
3. For each key in `m.packs`, GET the pack object (A2 guarantees visibility;
   content-addressing lets the reader verify each, given A1).
4. Hydrate a bare repository from the packs; serve refs from `m.refs`.

Read takes no locks and never writes. It observes a single committed manifest
`d` and is therefore a consistent snapshot by construction, even under concurrent
writers, because: (i) `manifest(d)` and the packs it names are immutable
[A1 + content-addressing], so a writer that advances the pointer past `d` cannot
alter what `d` resolves to; and (ii) **no pack `d` names is ever deleted**
[A1, no-deletion rule] and every named pack was durably written before `d` was
published [A1 + A2, §Push step order], so a reader holding an older `d` can always
GET every pack — a concurrent GC that prunes packs unreachable from the *new*
pointer never 404s this reader. This read-consistency-during-writer property is the
reason the no-deletion rule is load-bearing, not cosmetic; it is asserted in the
prose here because the mechanized model has no Read action (it checks the writer
side; this is the reader-side complement).

### Push

```
1. receive-pack: accept the pack, index it, derive new object set O.
2. for each o in O: PUT pack-object(o)            # content-addressed, idempotent (A1)
3. (e, d_before) := pointer(R); m_before := manifest(d_before)
4. validate Δ against m_before.refs               # fast-forward / push rules
       on rejection -> respond non-ff, STOP (no write, no fence cost)
5. m_after := m_before with refs updated per Δ; packs := m_before.packs ∪ keys(O)
6. d_after := PUT manifest-object(m_after)         # content-addressed (A1)
7. result := PUT M_R (value = d_after)          # CAS (A3)
       If-Match: e         if a pointer already exists (e from step 3)
       If-None-Match: *    if the repo has no pointer yet (first push / repo init)
       on 412 (lost race): re-read pointer, GOTO 3 (retry) or respond non-ff
       on success: the ref change is PUBLISHED
8. construct success response   # ONLY after step 7 succeeds  -- the FENCE
```

**The fence (step 8 after step 7).** The success response is not constructed
until the CAS in step 7 returns success. This is the publish-ordering
guarantee. It is *conditional on refs changing*: a no-op or rejected push
(step 4) never reaches step 7 and pays zero CAS/fence latency.

**No advisory lock.** Reftable serializes writers with `tables.list.lock` *and*
the rename. This protocol has **no advisory lock in v1**: writer serialization
is provided entirely by the CAS in step 7. §Theorem 3 proves this is
sufficient — the lock reftable uses is an optimization (it avoids wasted work
under contention), not a correctness requirement, *given A3*.

**Retry is policy, not safety.** The "GOTO 3" loop on a 412 is a *liveness/policy*
choice — retry count, backoff, or immediate non-ff rejection are all sound. Safety
(Theorems 1–3) holds for any of them, because a losing push that retries simply
re-runs steps 3–7 against the advanced pointer; it never writes `M_R` while
predicated on a stale ETag. We deliberately make no liveness claim here (§Scope).

## Safety Theorems

Let a push `p` be **ref-changing** if it reaches step 7. Let `observe(p) =
success` mean a client received `p`'s success response (step 8 executed).

### Theorem 1 (Durability-Ordering)

> If `observe(p) = success` for a ref-changing push `p`, then at the moment of
> observation, `pointer(R)` has held a value `d_after` whose manifest reflects
> `p`'s ref updates, and that manifest and all packs it names are durable.

**Proof.** Step 8 executes only after step 7 returns success (program order;
enforced by type in the implementation — see §Implementation Correspondence).
By A3, step 7's success means `M_R` was atomically set to `d_after`. By A1,
`manifest(d_after)` (written step 6) and every pack in `O` (written step 2,
before step 6) are durable. The fence orders the client-visible success strictly
after the durable pointer swap. Therefore observation implies durability. ∎

*Pointer integrity:* the pointer object holds a manifest *digest*, not inline
state. A1+A2 mean a successful pointer write is durable and read-after-write
consistent; a bit-flip in the stored digest yields a value that resolves to no
manifest (or a digest-mismatched one), so Read fails *closed* (error, not a
wrong-but-plausible history). The protocol never trusts an unresolvable or
mismatched pointer.

Corollary (crash safety): if the process crashes between any two steps, no
client has observed success unless step 7 completed; an incomplete push leaves
orphan packs and an unchanged pointer — wasted bytes, never a visible-but-lost
ref change.

### Theorem 2 (Manifest Reconstruction)

> Read(R) resolving to manifest digest `d` can reconstruct, in full, the object
> graph reachable from every ref in `manifest(d).refs` — no reachable object is
> missing. (The named pack set is a *superset* of that reachable closure; see
> the remark on force-push/delete below.)

**Proof.** By the push order, `d` is published (step 7) only after all packs in
`manifest(d).packs` are durably written (step 2, before step 6). By A2 a reader
that resolves `d` (step 1) can GET every pack in `manifest(d).packs` (step 3).
By A1 + content-addressing, each pack's bytes are exactly those written, and any
deviation is detected by digest. Git's `index-pack` over a complete, verified
pack set reconstructs the object graph (trusted upstream, §Non-Goals; reproducible
against a pinned minimum `git` ≥ 2.31, the first release with `git index-pack
--fsck-objects` defaults relied on here). It remains
to show `manifest(d).packs` *covers* the reachable closure of `m.refs`. By
induction on the push chain: the empty repo's manifest names ∅ and refs ∅
(covered vacuously). A normal step sets
`m_after.packs = m_before.packs ∪ keys(O)` where `O` is every object this push
introduced; and `m_after.refs` only points at objects in `m_before.refs`'
closure (unchanged or deleted refs) or in `O` (new or force-moved refs). A
compaction step instead feeds every `m_after.refs` tip to `git pack-objects
--revs` with no negative revisions, then names only the resulting bounded pack
set. The normal case preserves coverage by induction; the compaction case
re-establishes coverage directly from the complete post-push ref closure. ∎

**Remark (force-push, delete, and GC).** Coverage is a *superset*, not equality,
and that is the correct invariant. A delete-ref drops a key from `m.refs`; a
force-push repoints a ref off its old history. Neither normal ref operation
removes packs from `m.packs`, so objects reachable only from the old/deleted ref become unreachable
but remain named. This is safe — reconstruction of the *current* refs is
unaffected. Before the bounded manifest reaches its pack limit, an accepted push
proactively captures the complete post-push reachable closure and CAS-publishes
a replacement manifest that normally has fewer packs. At the hard cap, an
equal-count replacement is also valid when it incorporates the newly reachable
objects while remaining within the bound. The old immutable objects are not
deleted, so readers holding an earlier manifest remain valid. Physical
object-store deletion remains a separate retention concern outside this proof
boundary.

### Theorem 3 (Linearizable Refs / No Lost Update)

> If two ref-changing pushes `p₁`, `p₂` execute concurrently, the published
> history of `M_R` contains both effects in some serial order, or rejects one;
> neither silently overwrites the other.

**Proof.** Both read the pointer (step 3) and CAS predicated on the ETag they
read (step 7). Suppose both read ETag `e`. By A3 at most one CAS predicated on
`e` succeeds; WLOG `p₁` succeeds, `M_R` advances to `e′`. `p₂`'s CAS predicated
on `e` then fails (412): `p₂` re-reads (now `e′`, `d_after(p₁)`), re-validates Δ
against `p₁`'s published refs (step 4), and either composes onto `p₁`'s state or
is rejected non-ff. `p₂` never writes `M_R` while predicated on the stale `e`.
By A3's linearizability, the successful CAS sequence is a single total order;
each push's `m_after` is computed from its immediate predecessor's published
state. Hence no update is lost and the published ref history is serial. The
absence of an advisory lock changes only *efficiency under contention* (a loser
may do wasted indexing before its 412), not correctness. ∎

The mechanized model makes "no update is lost" concrete in checked forms over
**real ref values**: the published manifests form a chain with no two sharing a
parent (`Inv_NoFork` — a shared parent *is* a lost update); an installed push's
committed ref value equals the value it proposed (`Inv_RefEffectApplied` — your
write is what lands); each install is derived from the pointer it actually read
(`Inv_RefDerivedFromParent` — never built on superseded state). Removing the CAS
guard makes the model fork; that is the precise sense in which A3 is load-bearing.

## Conformance (Admitting a Backend for A3)

A3 is the only axiom not guaranteed by the protocol itself; for AWS S3 it is
documented, for any other backend (MinIO, Ceph RGW, ...) it is an empirical
claim. A backend is **admitted** against it by a **conformance probe** run at
startup against the target backend. The probe is a **deployment admission gate**: a backend is
trusted only if it passes. Passing does *not* prove the universal axiom — a
finite probe yields operational confidence for *this backend, build, and
config*; failure invalidates the design against that backend, exactly as
non-atomic `rename()` would invalidate reftable.

The probe has both a sequential half (semantic correctness) and a concurrent
half (linearizability under contention). The concurrent half is the load-bearing
part — A3 is a claim about *races*, so a probe that only checks sequential
conditional writes cannot admit a backend against it.

1. **Sequential semantics.** create-if-absent succeeds; duplicate
   `If-None-Match: *` fails; `If-Match: E` with current ETag succeeds; stale
   `If-Match` fails; `If-Match` on a missing key does not create; read-after-write
   returns the written value.
2. **N-way `If-Match` race (required).** Write key with body `base`; read its
   ETag `E` via the *same code path production uses for the pointer*. Spawn N
   (e.g. 32–64) concurrent `PUT key If-Match: E` with unique bodies. **Pass** iff
   exactly one succeeds, the other N−1 classify as precondition-failed, a
   subsequent read returns the winner's body and a new ETag, and the final body is
   never one of the *failed* payloads. Repeat for R rounds (configurable; default
   trades boot time against confidence).
3. **N-way `If-None-Match: *` race (required).** Same shape on a missing key:
   exactly one create wins, N−1 precondition failures, final body is the winner.
4. **ETag-token consistency.** Verify HEAD-path and GET-path ETag extraction
   agree byte-for-byte (quoting included). `If-Match` compares tokens literally;
   a quote mismatch between the read path and the write path silently tests the
   wrong thing. The probe must use the exact token format the pointer write uses.

**Proof surface (explicit non-goals of the probe and the design).** The protocol
depends only on conditional writes of *small single objects* (the manifest
pointer). It does **not** depend on, and the probe does **not** test:
conditional *multipart* uploads (packs are content-addressed plain PUTs, staged
without conditionals) or conditional *delete* (GC uses retention/sweep over a
republished manifest, not `If-Match` delete). Keeping the proof surface to
single-object conditional PUT is what makes A3 both sufficient and cheaply
checkable.

If the probe passes, A3 is admitted for that backend and Theorems 1–3 transfer
unchanged.

## Prior Art

The *algorithm* is established; the *formal characterization* is, to our
knowledge, new.

- **JGit `DfsRefDatabase`** reduces backend ref consistency to two **per-ref**
  CAS hooks, `compareAndPut(oldRef, newRef)` and `compareAndRemove(oldRef)`
  (`org.eclipse.jgit/.../dfs/DfsRefDatabase.java`). This *is* the model "git
  refs = CAS over ref state," spelled in Java — at per-ref granularity.
- **JGit/Google `reftable`** (`Documentation/technical/reftable.md`): immutable
  reftable files plus a single mutable `tables.list` pointer swapped atomically
  via `tables.list.lock` + POSIX `rename()`. Note this is *pointer*-granularity,
  not the per-ref CAS of `DfsRefDatabase` — reftable CASes one stack pointer.
  This protocol is the same shape: it substitutes an S3 conditional `PUT` for
  reftable's `rename()` and (v1) omits the advisory lock. Our granularity is one
  **repo manifest pointer** — the same single-pointer granularity as reftable,
  not the per-ref granularity of `DfsRefDatabase`.
- **`awslabs/git-remote-s3`** uses per-ref lock objects via S3 conditional
  writes (advisory lock substitute). **`mattn/git-remote-s3`** uses a
  `latest.json` pointer with `If-Match`/`If-None-Match` optimistic locking —
  closest to this design; advertises MinIO compatibility.
- **`johnny0917/jgit-aws`** stores packs in S3 but refs in DynamoDB
  (`compareAndPut` → Dynamo conditional update), the canonical pre-2024 *punt*:
  before S3 had conditional writes, the CAS-needing state went elsewhere. This
  protocol is what becomes possible once S3 itself offers CAS.
- **Gitaly (GitLab)** supports only local Git storage for refs; it punts ref
  consistency to the local filesystem and does not attempt object-store CAS.
- **arXiv:** no formal treatment of git refs over object storage found.
  `arXiv:1904.06584` ("GoT: Git, but for Objects") is a git-*inspired*
  replicated-object model, not a formalization of this problem.

## Implementation Correspondence

The fence (Theorem 1) maps to a single structural obligation on the
implementation, stated here as a requirement the code must meet for the proof to
transfer:

- **Unique constructor seam.** There must be exactly one path that builds a
  *push* `Response`, and it is `finalize_push(PushContext) -> Response`. The
  discriminator is the `PushContext`: only `finalize_push` consumes one, and a
  push subprocess's output reaches a response only by being wrapped in a
  `PushContext`. (The lower-level `build_git_response` helper that does the literal
  `Body::from(stdout)` conversion is *shared* with the read paths — info_refs,
  upload_pack — so it has two call sites; but those carry no `PushContext` and no
  fence obligation, so push-side uniqueness is structural, not a property of the
  body conversion itself. A reader auditing the code will see `build_git_response`
  reached twice and should check the discriminator is `PushContext`, not the
  conversion.) If any other path could build a push response without going through
  `finalize_push`, the fence would be convention, not structure, and Theorem 1
  would not hold. This is a checkable code property, verified by reviewers against
  the actual seam (`finalize_push`).
- **Parent observed once.** `hydrate_for_write` reads the pointer, fetches and
  verifies the parent manifest, materializes the workspace from it, and returns
  a `(HydratedRepo, ParentState)` pair where `ParentState` carries the exact
  `(ETag, digest, Manifest)` triple the workspace was hydrated against. That
  same `ParentState` rides on the `PushContext` through receive-pack, and
  `cas_publish` predicates the CAS on `parent_state.if_match` — it never re-reads
  the pointer. The "build on `d_old`, publish against `d_new`" hazard is closed
  at the type system: a concurrent writer that advances the pointer between
  hydrate and CAS surfaces as `CasError::Conflict`/HTTP 409, not as a manifest
  whose `parent` disagrees with the refs the workspace produced. This is the
  Rust analogue of `Inv_RefDerivedFromParent`.
- **412 → 409, terminal.** The CAS lost-race outcome maps to a typed
  `CasError::Conflict { winner_manifest, winner_manifest_key }`. The variant is
  distinct from `Backend(StoreError)` so `?`-bubbling cannot turn 412 into 500.
  There is no in-handler retry: the loser's receive-pack output was derived
  against a now-superseded parent, so reusing it would change the TLA action
  and break `Inv_RefDerivedFromParent`. The client re-pushes, which re-hydrates
  against the advanced state — that is the only safe retry, and `git` itself
  drives it.
- **kind:30618 derived after CAS.** Emission is conditional on
  `manifest_changed = (parent_digest != committed_digest)` — `Manifest::
  canonical_bytes` is deterministic, so equal published state ⇒ equal digest by
  construction (no-op pushes pay no 30618 cost). The event is built by
  `manifest_event::build_ref_state_event` from `CasSuccess.manifest` — the
  values that *physically landed* via CAS, by `Inv_RefEffectApplied`. The event
  is relay-signed (the relay is authoritative for ref state of repos it hosts);
  the pusher's pubkey rides in a `p` tag (buzz extension; NIP-34 does not
  define one). 30618 emission happens after `cas_publish` returns `Ok` and
  before the success `Response` is constructed — so 30618 is a strict
  consequence of a committed CAS, never the commit itself. A failed 30618
  insert is non-fatal: the push remains durable in the object store, and the
  next read/push surfaces the committed state from the manifest.
- **No advisory lock.** Writer serialization is the CAS. The per-repo mutex the
  legacy persistent-disk path used would only have spanned a single process and
  is incompatible with the multi-instance v1 architecture (§Scope). Dropping it
  was strictly more correct, not a risk — same-repo concurrent pushes each
  hydrate + run receive-pack, and the CAS losers' work is discarded (an
  accepted v1 tradeoff named in §Scope).

**Current code status (verified provenance).** The full S3-CAS implementation
exists in code at PR #726's tip (`crates/buzz-relay/src/api/git/`), with the
relay lib green, clippy `--tests -D warnings` clean, fmt clean, and the live
MinIO e2e — clone/push/fetch/force-push roundtrip + N-way concurrent-push
no-fork — green on the assembled tip. (Line numbers below are pinned at
landing time; reviewers checking after subsequent refactors should consult
symbol search, not line counts.)

| Spec element | Code |
|---|---|
| `Manifest { version, head, refs, packs, parent }` + `canonical_bytes` | `manifest.rs` |
| `Manifest::validate()` (pre-CAS rejection: refs/HEAD/OIDs/parent-shape) | `manifest.rs` |
| `GitStore::{put_pack, put_manifest, put_pointer}` (create-only + CAS) | `store.rs` |
| `run_conformance_probe` (A1/A3 fail-closed startup gate) | `store.rs` + `main.rs` |
| `hydrate_for_read` / `hydrate_for_write` | `hydrate.rs` |
| Bounded digest-keyed pack/index cache and single-flight population | `pack_cache.rs` |
| Proactive full-closure pack compaction before manifest capacity | `cas_publish.rs` |
| `ParentState { if_match, parent_digest, parent }` + `from_loaded`/`fresh` | `cas_publish.rs:154` |
| `cas_publish(.., &parent_state) -> Result<CasSuccess, CasError>` | `cas_publish.rs:410` |
| `CasError::Conflict { winner_manifest, winner_manifest_key }` (typed 412) | `cas_publish.rs:92` |
| `build_ref_state_event(&RefStateInputs, &Keys)` (NIP-34 kind:30618) | `manifest_event.rs` |
| `PushContext { pack, parent_state, repo_handle, … }` | `transport.rs:643` |
| `finalize_push(state, ctx) -> Response` — **the seam** | `transport.rs:674` |
| `build_git_response` (sole `Body::from(stdout)` site) | `transport.rs:627` |

The push path reaches `build_git_response` *only* through `finalize_push`,
which consumes a `PushContext`; the compiler enforces "no `PushContext` ⇒ no
push `Response`." Read paths reach `build_git_response` independently after
hydrating the published state via `hydrate_for_read` — pointer-absent → 404,
any below-pointer failure → 5xx, never a synthesized empty repo (A1
detectability holds in the read direction too). The 404 invariant is
unambiguous because kind:30617 announce seeds an empty-manifest pointer
*before* the announcement event is published: an announced repo is always
cloneable (empty refs, but a valid pointer), and pointer-absence means "never
announced."

One named follow-up: a behavioral integration test for runtime ordering
(publish-before-response) — currently enforced by `finalize_push` being a
single sequential async fn (no detached `tokio::spawn`) — is the
belt-and-suspenders item to add once a mockable-CAS seam exists. The
mechanical no-fork claim is empirically gated by the live N-way
concurrent-push e2e (`e2e_git::git_concurrent_push_one_wins_and_repo_recovers`).

## Mechanized Verification

The safety theorems are model-checked, not only argued in prose. The companion
TLA+ module `docs/spec/GitOnObjectStore.tla` models concurrent pushers racing to
advance the manifest pointer, with the CAS action (`If-Match`) as the sole
pointer writer — directly encoding axiom A3. Each push models a proposed ref
value (`newVal`, the objectId it wants `main` to hold) and whether its ref
snapshot reads succeed (`snapErr`); whether it *changes* refs is then **derived**
(`DidChange == newVal ≠ value-in-the-manifest-it-read`), not a free boolean. The
skip predicate is `MustPublish(p) == DidChange(p) \/ snapErr(p)` — "publish unless
we *observed* no change," never "publish unless `b == a`" with failed reads
compared equal. A `compacted` marker distinguishes normal delta-pack stages
from stages whose own pack is the trusted full closure produced from every
post-push ref tip.

Crucially the model carries the **real ref value** per manifest (`refs[m]` = the
objectId `main` holds in manifest `m`) and explicit **history** (`parent[m]`).
This is what lets the invariants prove ref-*update* linearizability — that *your*
ref write is what gets committed and survives — not merely pointer-id bookkeeping.
TLC checks eight invariants (a finiteness constraint, `BoundedManifests`, caps published manifests at `MaxManifests` so the retry loop terminates):

| Invariant | Theorem | Statement |
|---|---|---|
| `Inv_Fence` | T1 | an obligated push's manifest is published before success is observed |
| `Inv_ChangedPublished` | T1 | a ref-changing push is always published (fallible-snapshot bite) |
| `Inv_Closed` | T2 | a normal manifest covers its parent's packs; a compacted manifest names its trusted full-closure pack |
| `Inv_NoFork` | T3 | no two published manifests share a parent (a fork = a lost update) |
| `Inv_RefEffectApplied` | T3 | an installed push's committed ref value equals the value it proposed |
| `Inv_RefDerivedFromParent` | T3 | an install is derived from the pointer it read (no build on superseded state) |
| `Inv_ParentPublished` | T2/T3 | every published manifest's parent is published (grounded history) |
| `Inv_PointerPublished` | T1 | the pointer always names a published manifest |

```
$ tlc GitOnObjectStore.tla -config GitOnObjectStore.cfg
Model checking completed. No error has been found.
```

**Every invariant is proven non-vacuous** by a mutation that trips it (each
checked in isolation against each mutant). This is the discipline that catches
"green but vacuous" specs:

| Mutation | Trips |
|---|---|
| skip predicate `DidChange /\ ~snapErr` (ref change + both snapshots fail → silently skipped) | `Inv_ChangedPublished` |
| a normal stage uses `packs[m] = {m}` without the full-closure marker | `Inv_Closed` |
| drop CAS guard (two pushers install off one parent — a fork) | `Inv_NoFork` |
| install records the *read* ref value, not the push's proposal (effect dropped) | `Inv_RefEffectApplied` |
| record parent as root instead of the pointer actually read | `Inv_RefDerivedFromParent` (+ `Inv_NoFork`) |

The unmarked `packs[m]={m}` and CAS-guard mutations are the two core closure and
serialization vacuity tests. The full-closure marker is only assigned by the
compaction branch; reusing it for a normal delta stage would make the closure
claim vacuous, so the model explicitly clears stale markers when manifest ids
are reused.
The ref-value mutations (effect-dropped, wrong-parent) are what close the gap from
"pointer CAS serializes" to "ref *updates* are linearizable" — the user-visible
theorem the title promises. (A weaker mutation, `MustPublish == DidChange` alone,
does *not* break safety — it over-publishes, safe-but-wasteful; noted so the
mutation set is honest about which mutations are true safety regressions.
`Inv_ChangedPublished` and `Inv_Fence` look redundant but are not: see the .tla
comment — mutating the `MustPublish` operator weakens `Inv_Fence`'s own predicate,
so the `DidChange`-predicated `Inv_ChangedPublished` is the one that stays
load-bearing under exactly that mutation.)

The model is checked at `Pushers = {p1,p2,p3}`, `MaxManifests = 3`, under the
`BoundedManifests` constraint (`|published| ≤ MaxManifests`) — required because
the retry loop otherwise lets pushers churn fresh manifest ids and ref values
without bound, so the model is *finite-state only with the bound*. Three
concurrent pushers exercise every CAS race relevant to these invariants (a fourth
adds no qualitatively new interleaving). This is a *bounded* model check, not an
unbounded proof: it exhaustively verifies the invariants within the bound and is mutation-
shown non-vacuous, which is the standard claim for a TLC-checked safety spec.

## Summary

| Property | Status | Discharged by |
|---|---|---|
| Durability-ordering (T1) | Proved | Fence + A1, A2 |
| Manifest reconstruction (T2) | Proved | Content-addressing + A1, A2; git upstream |
| No lost update (T3) | Proved | A3 (no advisory lock needed) |
| A1/A2/A3 hold on backend | Empirical | Conformance probe (gate) |
| Liveness / latency | Empirical | Benchmark (out of scope) |
