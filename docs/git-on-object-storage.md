# Git Refs over Object Storage: A Formal Specification

`draft`

## Abstract

This document specifies a protocol for hosting git repositories on an object
store (S3 and S3-compatible backends such as MinIO) with **no persistent
filesystem**, and states what has been verified about it. Repository
content is stored as create-only, content-addressed pack objects (immutable by
protocol discipline — see A1, not by assuming an immutable store); the current
state of every ref is captured by a single mutable *manifest pointer* updated
by atomic compare-and-swap (CAS).

The correctness statement is a refinement. Every behavior of the protocol is a
behavior of an *atomic ref database* (`docs/spec/GitRefDB.tla`), in which a push
takes effect in one step, is applied only if it is authorized and only against
the ref values it names, and is reported to the client truthfully. This covers
**linearizability** (concurrent pushes never lose an update) and the client-side
half of **durability-ordering** (a client is never told an update succeeded
unless it was published). Three further properties concern storage: the pointer
never names an object that is not stored, the packs a published manifest lists
contain every object reachable from its refs (**manifest reconstruction**), and
whatever a reader has resolved stays fetchable.

Three kinds of evidence support these statements, and §Mechanized Verification
says which claim rests on which. The refinement and the storage properties are
model-checked exhaustively for small instances. The pointer discipline is
proved in Lean for any number of clients, refs and objects. The correspondence
between the models and the Rust code rests on tests and review. All of it is
relative to three object-store axioms and to a stated contract for
`git receive-pack`. Two further behaviors of the implementation, the
serving-write lease and the order of the derived kind:30618 events, are checked
in a separate bounded model.

The protocol is not novel as an *algorithm*: it is git's post-reftable ref
model — immutable content artifacts plus an atomic pointer swap — with the
atomic primitive substituted from POSIX `rename()` to an S3 conditional `PUT`.
The contribution of this document is the **formal characterization**: to our
knowledge, no prior formal treatment of git refs over conditional-write object
storage exists (see Prior Art). The argument is *parametric over the atomic
primitive*: it holds for any backend satisfying the three axioms, and a concrete
backend is *admitted for deployment* by a single conformance gate (a finite probe
cannot prove a universal axiom; it can only admit or reject a backend).

## Scope and Non-Goals

This specification concerns **safety** ("nothing bad happens"). It deliberately
does **not** establish:

- **Liveness or performance.** That a hydrate completes within a latency budget
  is empirical, not formal; it is characterized by benchmark, not theorem.
- **Git's internal correctness.** `git index-pack`, `upload-pack`, and
  `receive-pack` are trusted upstream components. The models represent
  `receive-pack` by a stated contract (assumption G1, §Mechanized Verification)
  and check that our *composition* publishes exactly what git did and reports
  git's outputs faithfully.
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
outcome would change the modeled action, and the checked results would no
longer apply to the code.

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

### Default-branch management

`buzz repos default-branch get/set` reads or changes the published manifest's
symbolic `head`. It selects an **existing** `refs/heads/<branch>`; it never moves
branch tips, creates/deletes refs, or changes packs. A subsequent push preserves
that HEAD while the branch exists, and fresh clones check it out.

This is a narrow, intentional exception to the Nostr-first API preference:
`GET`/`POST /git/{owner}/{repo}/default-branch` completes against the host-local
Git pointer transaction. A kind:30617 metadata edit is not equivalent: normal
event ingest persists before side effects and duplicate acknowledgements do not
rerun them. A new signed command/result lifecycle could drive the same CAS, but
adds a second completion/retry protocol for this one Git-host operation. Repo
metadata and ACLs remain in Nostr; kind:30618 remains a derived notification,
never authoritative Git state. This exception is not a general repository-settings
HTTP API.

- Both methods require request-specific NIP-98 (exact host-derived URL/method,
  timestamp, signature, shared fail-closed replay check). POST also requires a
  payload hash. Reusable Smart HTTP credentials are insufficient.
- GET requires current membership of the channel bound by the current 30617
  announcement. POST additionally requires a non-archived channel and the repo
  author, a named maintainer, or the recorded human owner of an agent-authored
  repo. Push permission, channel admin status and project membership alone do
  not grant management. Relay admission and signer/attested-owner bans apply.
- An agent may inherit management from a verified NIP-OA owner who is also a
  current channel member and manager. Kind-restricted credentials cannot grant
  this HTTP authority (there is no event kind); temporal restrictions still
  apply. Direct signer authority does not depend on an optional owner's channel
  membership. A stored agent-owner relationship is an ownership lookup, not a
  live delegation credential.
- Authorization is **admission-time**: later membership/maintainer removal,
  rebinding or archival does not cancel an already admitted write. This is
  narrower than the vision's instantaneous-revocation aspiration. The shared
  serving-write lease separately fences community deletion through the write.

GET returns `{branch, head, manifest}`. POST accepts only
`{branch, expected_manifest}` (maximum 4096 bytes) and returns the same snapshot
plus `changed`. The caller's digest must match the loaded pointer. A changed
manifest records the prior digest as its parent, then commits with that pointer's
observed ETag. Even a no-op checks the ETag. A concurrent push or default change
returns 409 instead of silently reloading and overwriting it.

The pointer CAS is the commit point. Notification failure after commit returns
an explicit error, not a rollback claim. The CLI sends POST once, with redirects
disabled. Ambiguous delivery (including server errors or a lost response) is
non-retryable `delivery_unknown` and retains the original expected digest. Read
current state before deciding on another write; blindly running `set` without
`--expected-manifest` would observe a fresh version and can override someone
else's later decision. A conflict is exit code 5.

Deploy relay support **before** using an updated CLI against that relay. This
requires no schema migration and changes no live repository merely by deploying.
See the [CLI examples](../crates/buzz-cli/README.md#default-branch).

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
reason the no-deletion rule is load-bearing, not cosmetic. The model has a
reader process and checks the property as `ReaderComplete`; a seeded deletion of
an unlisted pack violates it (§Mechanized Verification).

### Push

```
1. (e, d_before) := pointer(R); m_before := manifest(d_before)
   hydrate an ephemeral workspace W from m_before      # one pointer observation
2. receive-pack on W with the pre-receive hook installed:
       the hook submits the whole command list Δ to the push policy;
           if the policy declines, git applies no command
       git applies a command only if its old_id is the ref's value in W
3. refs_after := refs(W)                              # what git actually did
       refs_after = m_before.refs -> relay git's output, STOP (nothing to publish)
       git exited abnormally      -> respond error, STOP (nothing is published)
4. O := a pack of the objects reachable from refs_after but not from
        m_before.refs (delta), or of all objects reachable from refs_after
        (compaction)
5. PUT pack-object(O)                                 # content-addressed, create-only (A1)
6. m_after := (refs_after, head, packs, parent = d_before)
        packs = m_before.packs ∪ keys(O), or keys(O) alone after compaction
   d_after := PUT manifest-object(m_after)            # content-addressed (A1)
7. result := PUT M_R (value = d_after)                # CAS (A3)
       If-Match: e         if a pointer already exists (e from step 1)
       If-None-Match: *    if the repo has no pointer yet
       on 412 (lost race): respond conflict (HTTP 409), STOP
       on success: the ref change is PUBLISHED
8. relay git's output to the client   # ONLY after step 7 succeeds  -- the FENCE
```

**The fence (step 8 after step 7).** Git's status report for a push that
changed refs is not sent until the CAS in step 7 returns success. This is the
publish-ordering guarantee. It applies only when refs changed: a push that git
declined, or one that changed nothing, stops at step 3 and performs no write.

**Publication is decided from the workspace (step 3).** The relay does not infer
what git did from git's exit status or from the status report in its output.
Both are unreliable for this purpose. `git receive-pack` exits 0 when the
pre-receive hook declines a push and when a request carries no commands, it
reports a declined push only to a client that asked for a status report, and a
push with several commands can end with some applied and others refused. The
relay therefore compares the refs in the workspace after `receive-pack` with
the refs the workspace was hydrated from. If they are equal, nothing is
published and the client receives git's own answer. That answer contains `ok`
only for a command whose new value equals its old value: git accepts such a
command and changes nothing, and its report is true of the published state the
workspace was hydrated from. If they differ and git
exited normally, exactly the refs now in the workspace are published, so the
`ok` and `ng` lines git reported per ref are true of the published state. If
they differ and git exited abnormally, the request fails and nothing is
published. Under the contract G1 for `receive-pack` (§Mechanized Verification),
a changed workspace implies that the push policy accepted the command list, so
a publication always acts for an authorized push.

**No advisory lock.** Reftable serializes writers with `tables.list.lock` *and*
the rename. This protocol has **no advisory lock in v1**: writer serialization
is provided entirely by the CAS in step 7. Theorem 3 states that this is
sufficient — the lock reftable uses is an optimization (it avoids wasted work
under contention), not a correctness requirement, *given A3*.

**No retry inside a request.** A push that loses the race at step 7 is answered
with a conflict. Its workspace was computed from a manifest that is no longer
current, so nothing derived from it may be published; the client pushes again,
which starts at step 1 against the new pointer. Whether and when a client
retries is a liveness matter and affects none of the safety properties. We
deliberately make no liveness claim here (§Scope).

## Safety Theorems

This section states the properties the protocol is required to have. The
statements are numbered as theorems because the implementation's comments cite
them by number. The paragraphs marked *Argument* explain why each property
holds; they are explanations and not the evidence. The evidence is in
§Mechanized Verification: each statement is checked exhaustively by TLC for
small instances, and the parts of Theorems 1–3 that concern the pointer are
proved in Lean without a bound on clients, refs or objects.

Two definitions are used throughout. A push is **published** when its
conditional write in step 7 succeeds. A client is **told `ok` for a ref** when
the response it receives contains git's status line `ok <ref>`.

### Theorem 1 (Durability-Ordering)

> (a) Whenever the pointer names a manifest, that manifest and every pack it
> lists are stored. (b) If a client is told `ok` for a command that changes a
> ref, the push was published before the response was sent, and the published
> manifest holds the value the client asked for.

**Argument.** For (a), a manifest becomes the pointer's value only in step 7,
and the same request stored that manifest in step 6 and its new packs in step
5. The packs it inherits were listed by `m_before`, which was the pointer's
value when read, so they were already stored; nothing is deleted (A1). For (b),
the response is built in step 8, which runs only after step 7 succeeded, and
`refs_after` in the published manifest is the workspace state from which git
produced its `ok` lines. A request that stops at step 3 relays git's output
without publishing. In that case every `ok` git reported belongs to a command
that asked for the value the ref already had, because a command that changes a
ref and is reported `ok` leaves the workspace different from `m_before`. Such a
report is true of the state the request read in step 1, and that read is where
the request takes effect.

*Model:* (a) is the invariant `Fence`; (b) is part of the refinement `Refines`
(the abstract specification allows a response to report only the outcome fixed
when the push took effect) and is also stated directly as `NoSilentDrop`.
The check `relayed-ok` confirms that the model reaches the case of an `ok`
that is relayed without a publication.

*Pointer integrity:* the pointer object holds a manifest *digest*, not inline
state. A1+A2 mean a successful pointer write is durable and read-after-write
consistent; a bit-flip in the stored digest yields a value that resolves to no
manifest (or a digest-mismatched one), so Read fails *closed* (error, not a
wrong-but-plausible history). The protocol never trusts an unresolvable or
mismatched pointer.

Corollary (crash safety): if the process crashes between any two steps, no
client has been told `ok` unless step 7 completed; an incomplete push leaves
orphan packs and an unchanged pointer — wasted bytes, never a visible-but-lost
ref change. The converse does not hold and is not claimed: a push can be
published and the client can still receive an error, for example when the
response is lost. The client then learns the outcome by reading the refs.

### Theorem 2 (Manifest Reconstruction)

> Read(R) resolving to manifest digest `d` can reconstruct, in full, the object
> graph reachable from every ref in `manifest(d).refs` — no reachable object is
> missing. (The named pack set is a *superset* of that reachable closure; see
> the remark on force-push/delete below.)

**Argument.** By Theorem 1(a) every pack in `manifest(d).packs` is stored, and
by A2 a reader that resolves `d` can GET each of them. By A1 +
content-addressing, each pack's bytes are exactly those written, and any
deviation is detected by digest. Git's `index-pack` over a complete, verified
pack set reconstructs the object graph (trusted upstream, §Non-Goals; reproducible
against a pinned minimum `git` ≥ 2.31, the first release with `git index-pack
--fsck-objects` defaults relied on here). It remains
to show `manifest(d).packs` *covers* the reachable closure of `m.refs`. By
induction on the sequence of publications: the empty repo's manifest names ∅ and
refs ∅ (covered vacuously). A normal step sets
`m_after.packs = m_before.packs ∪ keys(O)` where `O` holds every object
reachable from `refs_after` that is not reachable from `m_before.refs`, so an
object reachable from `refs_after` is either in `O` or covered by the induction
hypothesis. A compaction step instead feeds every `refs_after` tip to
`git pack-objects --revs` with no negative revisions, then names only the
resulting bounded pack set. The normal case preserves coverage by induction; the
compaction case re-establishes coverage directly from the complete post-push ref
closure. The induction applies to `m_before` because the conditional write
succeeds only if `m_before` is still the pointer's value (Theorem 3).

The step "`O` holds every object reachable from `refs_after` and not from
`m_before.refs`" requires those objects to be present in the workspace. That is
part of the contract G1: `receive-pack` refuses a ref update whose new value is
not fully connected.

*Model:* the invariant `PublishedClosed`, over an object graph in which one
object refers to another, so that a pack can lack an ancestor.

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
object-store deletion remains a separate retention concern outside this
boundary.

### Theorem 3 (Linearizable Refs / No Lost Update)

> Every behavior of the protocol is a behavior of the atomic ref database
> `GitRefDB` under the mapping that reads the database state from the manifest
> the pointer names. In particular: each change of the published refs applies
> commands of one push, each applied command names as its old value the value
> the ref held immediately before the change, and a push whose conditional
> write fails changes nothing.

**Argument.** A request observes the pointer once, in step 1, and every later
step works from that observation: git validates each command against the refs
hydrated from `m_before`, and the conditional write in step 7 is predicated on
the ETag `e` read in step 1. By A3 the write succeeds only if the pointer's
current ETag is `e`. Under the ETag assumption E1 (§Mechanized Verification)
that implies the pointer still holds `d_before`, so at the instant of the
write the published refs are exactly the refs git validated against. The whole
push therefore takes effect at that instant as one step of the atomic database:
the applied commands have the right old values, and the refs they did not touch
keep their published values. A request whose write fails publishes nothing and
is answered with a conflict. The default-branch setter (§Default-branch
management) follows the same discipline and is one further kind of atomic step.

The comparison in step 7 is on the ETag, which may be a function of the pointer
body alone. That is sufficient. If the pointer's body returns to an earlier
value, the manifest it names is the same manifest, with the same refs, HEAD and
packs, so a push computed from the earlier observation is also correct against
the current state. Because a manifest records its predecessor, a body in fact
never recurs (the Lean theorem `pointer_depth_eq_writes`), and a successful
write implies that no other write that changed the pointer happened since the
read (`cas_sees_no_intervening_write`; the Lean model omits the default-branch
setter, whose same-value write rewrites the same body and so preserves the
argument of the previous paragraph).

*Model:* the temporal property `Refines`. Removing the CAS guard, predicating
the write on a later observation than the one the workspace was built from, a
store whose ETag does not distinguish bodies, and sending the response before
the write each violate it.

### Theorem 4 (Authorized Publication)

> A request reaches step 4 only if the push policy accepted its command list
> and `receive-pack` changed at least one ref. A request that is answered from
> git's output without a publication reported `ok` only for commands that
> asked for no change.

**Argument.** Step 3 continues to step 4 only if the workspace refs differ from
`m_before.refs`. By G1 `receive-pack` changes a ref only after the pre-receive
hook accepted the command list, and the hook accepts only what the push policy
accepts. The second sentence restates the observation used for Theorem 1(b): a
command that changes a ref and is reported `ok` changes the workspace.

*Model:* the invariants `PublishIsAuthorizedChange` and `NoSilentDrop`. Both
fail for the rule this protocol used before (publish when git exits 0 and
reports no `ng`), which the model keeps as the seeded error
`Decision = "legacy"`.

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
   (e.g. 32–64) concurrent `PUT key If-Match: E` with unique bodies. A racer
   whose outcome is *unknown* (a socket or send failure, or a 5xx answer) is
   dropped from the
   round; every other answer is *classified*. **Pass** iff at least two racers
   classify, exactly one classified racer succeeds, every other classified racer
   is precondition-failed (any other answer, such as a 403, fails the probe),
   and a read right after the round returns the winner's body under the
   winner's ETag. That read is what keeps a dropped racer's write from passing:
   its outcome is unknown, so a backend that committed it and then answered 5xx
   must still leave the winner as the stored pointer. Repeat for R rounds
   (configurable; default trades boot time against confidence).
3. **N-way `If-None-Match: *` race (required).** Same shape on a missing key,
   with the same drop rule: at least two classified racers, exactly one create
   wins among them, the rest are precondition failures, and the final body is
   the racers' (identical, content-addressed) bytes.
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
  against a now-superseded parent, so nothing computed from it may be
  published (the seeded error `stale-base` is that mistake). The client
  re-pushes, which re-hydrates against the advanced state — that is the only
  safe retry, and `git` itself drives it.
- **Publication decided from observed refs.** `observe_push` takes the snapshot
  of the workspace after `receive-pack` (`snapshot_workspace`) and passes it,
  with the hydrated parent refs and git's exit status, to `push_disposition`,
  which returns `RelayOnly`, `Publish` or `Abort` exactly as §Push step 3
  prescribes. `finalize_push` calls `cas_publish` only for `Publish` and hands
  it the same snapshot, so the refs that are compared are the refs that are
  published. The snapshot parser fails closed: output it cannot read is an
  error, never an empty or partial ref set. The table-driven test
  `push_is_published_exactly_when_receive_pack_changed_refs` runs real `git`
  through the production `run_git_at` and `observe_push` for a request with no
  commands, a declined push with and without a status report, an accepted push,
  a command that asks for the value a ref already has, a push in which one ref
  is updated and another refused for its old value, and a push that creates
  two refs whose names conflict, of which git creates one and refuses the
  other. Which of the two git creates differs between git versions; the test
  accepts either and requires that each ref is in the snapshot exactly when
  git reported `ok` for it. The test is Unix-only because its hook fixture
  sets file modes. Two
  tests cover `finalize_push` itself and need Postgres and an object store:
  `finalize_push_publishes_nothing_when_receive_pack_changed_no_ref` checks
  that an unchanged workspace leaves the pointer unwritten and stores no
  kind:30618 event, and `e2e_git::git_partial_push_publishes_accepted_refs`
  drives a partially refused push through a live relay.
- **kind:30618 derived after CAS.** `cas_publish` is reached only for a push
  that changed refs, so every successful publication is followed by one
  kind:30618 event and a push that changed nothing emits none. The event is
  built by `manifest_event::build_ref_state_event` from `CasSuccess.manifest` —
  the values that were written by the CAS. The event
  is relay-signed (the relay is authoritative for ref state of repos it hosts);
  the pusher's pubkey rides in a `p` tag (buzz extension; NIP-34 does not
  define one). 30618 emission happens after `cas_publish` returns `Ok` and
  before the success `Response` is constructed — so 30618 is a strict
  consequence of a committed CAS, never the commit itself. A failed 30618
  insert is non-fatal: the push remains durable in the object store, and the
  next read/push surfaces the committed state from the manifest.
- **kind:30618 order follows the manifest chain.** kind:30618 events are
  stored side by side, and readers take the one with the greatest
  `created_at` (lowest id on a tie). Stamping each event with the emitting
  instance's clock let an older state win: two pushes in one second tie, and a
  later push on a slower clock sorts first. Each published manifest therefore
  carries `published_at`, strictly greater than its parent's (`now + 1` after
  an unstamped parent, such as the announce-time seed), and the derived event
  uses it as `created_at`. Since the manifests form a chain under CAS, the
  newest event is the newest manifest regardless of which instance emits it
  or when. The field is omitted when absent, so the shared empty seed and
  manifests written before the field existed keep their digests. The argument
  for a manifest whose parent carries no stamp rests on an assumption about
  clocks; see "Order of ref-state events" in §Mechanized Verification.
- **A landed CAS stands when the serving lease lapses after it.** The pointer
  CAS runs under the community serving-write lease, and the lease check after
  the operation can fail because the heartbeat missed a renewal, not only
  because deletion has fenced the community. The CAS cannot be undone at that
  point, so `finalize_push` (and the default-branch setter) keeps its outcome
  instead of answering 503, and lets the lease-guarded 30618 insert decide
  whether publication may continue. Lease loss *during* the operation still
  drops it and answers 503; whether the pointer PUT landed is then unknown.
- **No advisory lock.** Writer serialization is the CAS. The per-repo mutex the
  legacy persistent-disk path used would only have spanned a single process and
  is incompatible with the multi-instance v1 architecture (§Scope). Dropping it
  was strictly more correct, not a risk — same-repo concurrent pushes each
  hydrate + run receive-pack, and the CAS losers' work is discarded (an
  accepted v1 tradeoff named in §Scope).

**Code correspondence.** The implementation is in
`crates/buzz-relay/src/api/git/`. The table names the symbol for each element
of the specification; symbols are given without line numbers because those
drift.

| Spec element | Code |
|---|---|
| `Manifest { version, head, refs, packs, parent, published_at }` + `canonical_bytes` | `manifest.rs` |
| `next_published_at` (stamp strictly after the parent's) | `manifest.rs` |
| `Manifest::validate()` (pre-CAS rejection: refs/HEAD/OIDs/parent-shape) | `manifest.rs` |
| `GitStore::{put_pack, put_manifest, put_pointer}` (create-only + CAS) | `store.rs` |
| `run_conformance_probe` (A1/A3 fail-closed startup gate) | `store.rs` + `main.rs` |
| §Push step 1: `hydrate_for_write` → `(HydratedRepo, ParentState)` | `hydrate.rs` |
| §Read: `hydrate_for_read` | `hydrate.rs` |
| Bounded digest-keyed pack/index cache and single-flight population | `pack_cache.rs` |
| §Push step 2: `receive_pack`, `install_hook`, the policy callback | `transport.rs`, `hook.rs`, `policy.rs` |
| §Push step 3: `snapshot_workspace`, `push_disposition`, `observe_push` | `cas_publish.rs`, `transport.rs` |
| §Push steps 4–7: `cas_publish(.., &parent_state, &workspace, ..)` | `cas_publish.rs` |
| Proactive full-closure pack compaction before manifest capacity | `cas_publish.rs` |
| `ParentState { if_match, parent_digest, parent }` + `from_loaded`/`fresh` | `cas_publish.rs` |
| `CasError::Conflict { winner_manifest, winner_manifest_key }` (typed 412) | `cas_publish.rs` |
| `build_ref_state_event(&RefStateInputs, &Keys)` (NIP-34 kind:30618) | `manifest_event.rs` |
| `PushContext { pack, parent_state, repo_handle, … }` | `transport.rs` |
| §Push step 8: `finalize_push(state, ctx) -> Response` — **the seam** | `transport.rs` |
| `build_git_response` (sole `Body::from(stdout)` site) | `transport.rs` |
| Default-branch setter: `DefaultBranchSnapshot::set` | `settings.rs` |
| `protect_landed`: a landed conditional write stands when the lease lapses after it | `buzz-deletion`; called from `transport.rs`, `settings.rs` |

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
single sequential async fn (no detached `tokio::spawn`) — is the item to add
once a mockable-CAS seam exists. The no-lost-update claim is exercised
end-to-end by the live N-way concurrent-push test
(`e2e_git::git_concurrent_push_one_wins_and_repo_recovers`), which needs
Postgres and an object store.

## Mechanized Verification

This section separates what is defined, what is assumed, what has been checked
by a tool, what is supported by tests of the implementation, and what is only
proposed. The TLC and Lean results can be reproduced with two commands:
`docs/spec/check.sh` (TLC, pinned to version 1.7.4 by SHA-256; needs Java) and
`lake build` in `docs/spec/lean` (Lean 4.33.1, no dependencies). Both run in CI
when a file under `docs/spec/` changes. The Quint results are reproduced with
the commands given in "Results checked with Quint"; they are not run in CI.

### Definitions

Two TLA+ modules define the claim.

`GitRefDB.tla` is the specification. It describes a database of refs and a HEAD
with no object store and no concurrency control. A push is an invocation (the
client submits commands, and it is recorded whether the push policy authorizes
them), one atomic step in which a subset of the commands is applied, and a
response. A command may be applied only if the push is authorized and only if
its old value is the ref's current value. A command whose new value equals its
old value can be applied; it confirms the value of the ref and changes nothing. A
response may report the true per-ref outcome, report nothing, or be an error;
it may not report an outcome that did not occur. The module also records which
actor the published state is attributed to, which stands for the pusher named
in the kind:30618 event.

`GitOnObjectStore.tla` is the protocol of §Push together with a default-branch
setter, a reader, and repository creation. Its state is the pointer, the
manifest store, the pack store, and the local state of each request. A manifest
is a record of refs, HEAD, packs and (optionally) predecessor. A pack is a set
of objects, and objects refer to other objects, so reachability and closure are
computed, not asserted. `receive-pack` is one step that chooses which commands
to apply, subject to G1 below, and may exit abnormally after applying some.

The correctness statement `Refines` says that every behavior of
`GitOnObjectStore` is a behavior of `GitRefDB`, where the abstract database is
read from the manifest the pointer names. The abstract outcome of a push is
fixed at its conditional write, or, for a request that publishes nothing, at
its read of the pointer. TLC checks this as a temporal property: every
step of the protocol must be a step of the specification or leave the abstract
state unchanged.

### Assumptions

The checked results hold under the following assumptions. None is verified
here.

- **A1–A3** (§Axioms): durable create-only writes, read-after-write, and a
  linearizable conditional write on the pointer.
- **E1 (ETag).** If the pointer's ETag equals the ETag a request read, the
  pointer's body equals the body that request read. A store whose ETag changes
  on every write satisfies E1. So does a store whose ETag is a digest of the
  body, provided that digest does not collide on two pointer bodies that occur.
  On S3 the ETag of such an object is an MD5 digest, and the body is the
  hexadecimal SHA-256 digest of a manifest; E1 then asks that no two manifest
  digests published for one repository have the same MD5.
- **H1 (digests).** SHA-256 does not collide on the manifests and packs that
  occur. The models identify a digest with the value it names.
- **G1 (`git receive-pack`).** `receive-pack` changes a ref only by applying a
  command the client submitted; only after the pre-receive hook accepted the
  whole command list; only if the command's old value is the ref's current
  value in the workspace; and only if every object reachable from the new value
  is present. It may refuse any command and may exit abnormally after applying
  some. The hook accepts exactly what the push policy accepts. This is git's
  documented behavior, and the test named in §Implementation Correspondence
  exercises the cases that matter here against the installed git, but git
  itself is not verified.
- **Policy.** Whether the push policy's decision is the right one is outside
  these models. They record its answer as a boolean per push.

### Results checked by TLC

TLC explores every reachable state of a finite instance. The base instance has
two concurrent pushing clients, two refs, the default-branch setter, repository
creation, and compaction when a manifest lists two packs. The ETag is a
function of the body and manifests do not record a predecessor. This is the
weakest comparison a conforming store can provide (the write succeeds in the
most situations), and it makes the state space finite without any bound on the
number of pushes, so the base check covers behaviors of unbounded length. The
other instances vary one dimension each.

| Check | Instance | Distinct states | Result |
|---|---|---|---|
| `refdb` | the specification alone, one client; its stated consequences | 11,262 | pass |
| `push` | base instance | 2,349,839 | pass |
| `objects` | one client, two objects (one refers to the other), reader | 531,099 | pass |
| `three-clients` | three clients, one ref | 91,039 | pass |
| `stale-commands` | a client may name an old value other than the hydrated one | 43,314 | pass |
| `parent-links` | manifests record their predecessor; at most 3 pointer writes | 1,329,939 | pass |
| `version-etag` | ETag changes on every write; at most 3 pointer writes | 505,468 | pass |
| `no-view` | base instance without the setter and without state identification | 6,773,286 | pass |

Each passing check establishes, for its instance, `Refines` and the invariants
`Fence`, `ReaderComplete`, `PublishedClosed`, `HeadPublished`, `NoSilentDrop`
and `PublishIsAuthorizedChange`, and the action property `ParentIsPredecessor`
(a pointer write that changes the manifest names the manifest it replaced,
where predecessors are recorded). `ReaderComplete` constrains only `objects`,
the instance with a reader. The whole suite takes about three minutes on a
16-core machine and about ten minutes on a GitHub-hosted runner.

The instances identify states that differ only in stored objects that nothing
reads (a TLC `VIEW`), and treat clients as interchangeable (`SYMMETRY`). The
argument that the first loses nothing is given in `MCGitOnObjectStore.tla`, and
`no-view` repeats the check without it.

### Seeded design errors

A property that cannot fail constrains nothing. Each row below changes the
design in one way, and `check.sh` requires TLC to report a violation of the
named property; the suite fails if a seeded error goes undetected. The view is
not used for these runs.

| Seeded error | Violates |
|---|---|
| publish when git exits 0 and reports no `ng` (the original rule): a declined or empty push is published | `PublishIsAuthorizedChange` |
| the original rule, mixed outcome: the client is told `ok` for a ref that is not published | `NoSilentDrop` |
| the original rule, git exits abnormally after applying a command | `NoSilentDrop` |
| the original rule, as a whole | `Refines` |
| publish when git exits 0 and reports an `ok` or no `ng` (the status-report rule): a declined or empty push is published | `PublishIsAuthorizedChange` |
| the status-report rule, git exits abnormally after applying a command | `NoSilentDrop` |
| the status-report rule, as a whole | `Refines` |
| no pre-receive hook | `PublishIsAuthorizedChange` |
| pointer write without the condition | `Refines` |
| condition taken from a later pointer read than the one the workspace was built from | `Refines`, `ParentIsPredecessor` |
| ETag that does not distinguish bodies (against a push; against the setter) | `Refines` |
| response sent before the pointer write | `Refines` |
| pointer write before the uploads | `Fence` |
| deletion of a pack that the current manifest does not list | `Fence`, `ReaderComplete` |
| a delta push that drops the packs it inherits | `PublishedClosed` |
| `receive-pack` without the connectivity check | `PublishedClosed` |

The first seven rows concern the two rules the relay used before the present
one. Both decided from git's exit status and the lines of its status report.
The original rule treated any `ng` line as "nothing happened". The status-report
rule repaired the mixed outcome by publishing whenever an `ok` line is present.
Neither sees a status report from a client that did not negotiate one, and
neither sees any line for a request without commands, so both publish in those
cases. The model was written against what git can do and not against the status
report, and for either rule it produces a counterexample of four states: the
repository is created, a request arrives, `receive-pack` declines it, and the
request is on the publish path, where the published state would be attributed
to the caller.

One further check, `relayed-ok`, asserts something that is not required (that a
request answered without a publication never carries an `ok`) and must be
violated by the protocol as specified. It shows that the model reaches the case
of a command that asks for no change, which Theorem 1(b) and Theorem 4 make an
exception for.

### Results proved in Lean

`docs/spec/lean/GitPointer.lean` proves the pointer discipline for arbitrary
types of clients, refs, objects and packs, with no finiteness assumption. The
model there has four steps (read the pointer and stage a successor, upload,
conditional write, abort) and arbitrary relations for which ref states may
follow which, which objects a ref state reaches, and which objects a pack
holds. Its one hypothesis about the store is E1. For every reachable state it
proves:

- `fence`: the manifest the pointer names and the packs it lists are stored
  (Theorem 1a).
- `closed`: those packs contain every object reachable from the manifest's
  refs, derived from the three rules for composing a pack list (Theorem 2).
- `pointer_step_legal`: every step leaves the pointer unchanged or replaces it
  with a manifest computed as a legal successor of the value it replaces
  (the pointer part of Theorem 3).
- `pointer_depth_eq_writes` and `cas_sees_no_intervening_write`: a pointer body
  never recurs, and a successful conditional write implies that no other write
  that changed the pointer happened since the read (the Lean model omits the
  default-branch setter, whose same-value write rewrites the same body and so
  preserves the argument of the previous paragraph).

`write_is_possible` shows that a successful write is reachable, so the results
do not hold vacuously. The proofs use no axioms beyond Lean's standard three
(`propext`, `Classical.choice`, `Quot.sound`).

The Lean model is narrower than the TLA+ model. It does not contain the publish
decision, the responses to clients, the default-branch setter, the reader, or
`receive-pack`; "legal successor" is a parameter. The two models are related by
reading, not by a mechanical link.

### Results checked with Quint

The TLA+ and Lean models contain neither the serving-write lease nor the
kind:30618 events. `docs/spec/GitPushPath.qnt` is a Quint model of the push
path that contains both. Two pushers each fetch the ref advertisement and then
send a receive-pack request, which hydrates from whatever pointer is current
when it runs. A request names at most two of three refs (which include the
directory/file pair `feature` and `feature/x`), may name none, and may come
from a client that did not negotiate a status report. The publish decision,
the lease and the event timestamps follow `finalize_push`. Each relay instance
reads a clock that may run one second ahead. The invariants are stated over
what clients observe:

| Invariant | Statement |
|---|---|
| `noFalseSuccess` | a ref the client printed as `ok` reached the published pointer |
| `noFalseFailure` | a ref the client was told failed did not reach the pointer |
| `refStateMatchesPointer` | once pushes settle, the kind:30618 event that readers select matches the pointer |
| `publicationChangesRefs` | every published manifest differs in its refs from the one it replaced |

The module `fixed` is the implementation: the publish decision of §Push step
3, a landed conditional write kept when the lease lapses after it, and events
stamped with the manifest's `published_at`. The other modules each restore one
earlier behavior, so that a counterexample is attributed to one defect.

| Module | Restores | Result |
|---|---|---|
| `fixed` | nothing | all four invariants hold |
| `partialBugOnly` | the original publish rule | violates `noFalseSuccess` and `publicationChangesRefs` |
| `reportRuleOnly` | the status-report publish rule | violates `publicationChangesRefs`; `noFalseSuccess` and `noFalseFailure` hold |
| `leaseBugOnly` | a landed write reported as a failure when the lease lapses | violates `noFalseFailure` |
| `timestampBugOnly` | events stamped with the emitting instance's clock | violates `refStateMatchesPointer` |
| `current` | all three original behaviors | violates `safety` |

The results were obtained with Quint 0.33.0 in two ways. Random simulation
explores runs of at most 14 steps:

```
quint run docs/spec/GitPushPath.qnt --main fixed --invariant safety \
  --max-steps 14 --max-samples 2000000 --seed 0x1
```

For `fixed` it reports no violation of `safety` (the conjunction of the four
invariants) in 2,000,000 runs. For `reportRuleOnly`, the same command with one
invariant at a time reports no violation of `noFalseSuccess`, `noFalseFailure`
or `refStateMatchesPointer` in 2,000,000 runs each, and a counterexample for
`publicationChangesRefs`. Each of the other modules produces a counterexample
for the invariant named in the table. Simulation samples runs; the absence of a
violation is evidence, not a proof.

The second way is a bounded symbolic check, which considers every run up to
the bound:

```
quint verify docs/spec/GitPushPath.qnt --main fixed --invariant safety \
  --max-steps 8
```

With Apalache 0.62.1 this reports no violation for `fixed` (81 seconds), and
the same command for `reportRuleOnly` with `publicationChangesRefs` returns a
counterexample. A push that publishes takes four steps (advertisement,
receive-pack, conditional write, event), so the bound of eight covers every
interleaving of the two pushes in which the clock does not advance. Runs in
which it advances between steps of both pushes are longer, and are covered by
simulation only.

A `reportRuleOnly` counterexample is a request that changes no ref and whose
visible report has no `ng` line: a request with no commands, or a declined push
from a client that did not negotiate a status report. The status-report rule
publishes it. In the Quint model the published refs equal
their predecessor's, so the client-facing invariants still hold; what the
publication adds is a pointer write and a kind:30618 event that names the
caller as the pusher. The TLA+ model, which has the push policy, reports the
same behavior as a violation of `PublishIsAuthorizedChange` (the seeded error
`report-unauthorized`).

### Evidence about the implementation

The models are about the design. Three things connect them to the code. The
publish decision is a pure function, `push_disposition`, whose three outcomes
are the three branches of the model's `Disposition`, and a unit test covers its
cases. The test `push_is_published_exactly_when_receive_pack_changed_refs` runs
the production subprocess path against real git for seven ways a push can
end, and fails if the decision is changed to depend on the exit status alone.
The test `finalize_push_publishes_nothing_when_receive_pack_changed_no_ref`
calls `finalize_push` with an unchanged workspace against Postgres and an
object store, and fails in the same way under that change. The remaining
correspondence (one pointer observation per request, uploads before the
conditional write, the response built only in `finalize_push`) is by the
structure described in §Implementation Correspondence and by the other
storage-backed tests.

The storage-backed tests of `finalize_push` and `e2e_git` are marked ignored
and are not selected by any CI job at the time of writing, so they are evidence
only when someone runs them. The seven-case test needs no services and runs in
the Unix unit-test jobs.

### Assumptions and limitations

- The TLC results are exhaustive for the instances listed and say nothing
  directly about larger ones. Two clients with two objects and two refs is
  beyond what the suite can check in reasonable time. The Lean results remove
  the bound for the pointer discipline only.
- Hydration is one atomic step in the push model. The reader process covers
  reads that are interleaved with writes.
- Only safety is covered. Nothing here shows that a push eventually succeeds.
- Deletion of stored objects is modeled only as a seeded error. A garbage
  collector would need its own model, with the reader and the upload-then-write
  window as the cases it must respect.
- The TLA+ and Lean models do not include the relay's database, the
  serving-write lease, request authentication, or the kind:30618 event stream.
  The Quint model includes the lease and the events, with the bounds stated in
  its section, and is not connected to the other two by any mechanical link.

### Order of ref-state events

Two pushes can win their conditional writes in the order A, B and then store
their kind:30618 events in the order B, A, because event storage happens after
the write and nothing orders the two requests. When each event carried the
time at which its relay instance built it, a consumer that reads ref state from
the newest event could then see A's refs although the pointer holds B's. Two
pushes within one second, or a later push handled by an instance with a slower
clock, had the same effect.

The implementation removes the dependence on storage order and on the emitting
clock. Every manifest written by a push or by the default-branch setter carries
`published_at`, computed by `next_published_at` as the later of the current
time and one second after the parent's stamp, and the derived event uses that
value as its `created_at`. Stamps therefore increase strictly along the chain
of manifests, and the chain is totally ordered by the conditional write
(Theorem 3), so the event with the greatest `created_at` is the event of the
manifest the pointer names. `refStateMatchesPointer` in the Quint model checks
this for stamped manifests, and `timestampBugOnly` shows the earlier behavior
violating it.

**Assumption C1 (clocks, unstamped parent).** Two manifests carry no stamp: the
empty manifest written when a repository is announced, whose bytes are shared
by all repositories, and any manifest written before the field existed. The
event of such a manifest was stamped with the clock of the instance that
emitted it. Its successor takes the current time plus one second on the
instance that writes it. The successor's event is selected over the
parent's only if that value exceeds the parent event's `created_at`. This
holds when one instance handles both, and otherwise when the writing
instance's clock is not behind the other's by more than the time that passed
between the two events. C1 states that it holds. It concerns one write per
repository (the first after creation, or the first after the field was
introduced); every later manifest has a stamped parent. C1 is established by
reading the code. It is not modeled: the Quint model starts from a stamped
manifest. If C1 fails, readers of the newest event see the parent's refs until
the next push, and the pointer is unaffected.

### Proposed extensions

- Bind publication to the policy decision itself: have the policy endpoint
  record, per request, the command list it approved, and have step 3 require
  that the observed ref changes are a subset of an approved list. This would
  replace assumption G1's clause about the hook with a check in the relay.
- Remove assumption C1, for example by having the first push after an
  unstamped manifest read the `created_at` of the stored event and exceed it,
  and model the unstamped case in Quint.
- Run the Quint checks in CI with a pinned Quint and Apalache.
- Run the storage-backed `finalize_push` tests in CI. Their fixture applies the
  migrations itself, which the CI database (built from the desired-state
  schema) does not expect, so the fixture needs the schema-mode switch the
  other storage-backed suites use.
- A mechanical link between the Lean model and the TLA+ model, or a proof of
  the full refinement for unbounded parameters.

### Names used by earlier revisions

Code comments and a database migration cite invariant names from an earlier
model. They are kept as aliases in `GitOnObjectStore.tla`: `Inv_NoFork` and
`Inv_RefEffectApplied` stand for `Refines`, `Inv_RefDerivedFromParent` for
`ParentIsPredecessor`, and `Inv_Closed` for `PublishedClosed`.

## Summary

| Property | Evidence | Relative to |
|---|---|---|
| Durability-ordering (T1) | TLC, finite instances; Lean (`fence`), unbounded | A1, A2 |
| Manifest reconstruction (T2) | TLC, finite instances; Lean (`closed`), unbounded | A1, A2, G1; git upstream |
| Linearizable refs (T3) | TLC refinement, finite instances; Lean (`pointer_step_legal`), unbounded, pointer discipline only | A3, E1, H1, G1 |
| Authorized publication (T4) | TLC, finite instances | G1; policy correctness not covered |
| Landed write reported as landed; ref-state events ordered as the manifests | Quint, bounded | C1 |
| Model matches the code | Tests and review | — |
| A1/A2/A3 hold on backend | Empirical | Conformance probe (gate) |
| Liveness / latency | Empirical | Benchmark (out of scope) |
