-------------------------- MODULE GitOnObjectStore --------------------------
(***************************************************************************)
(* Formal model of git refs over object storage, accompanying             *)
(* docs/git-on-object-storage.md.  Model-checks the three safety           *)
(* properties under the conditional-write (CAS) axiom A3 by construction:  *)
(* the PUT/If-Match action is the only writer of the pointer and is atomic *)
(* per step (TLC interleaves at action granularity), modeling A3 directly. *)
(*                                                                         *)
(* Pushers race to advance a single manifest pointer holding a ref value.   *)
(* We assert (see SAFETY PROPERTIES for the full set and per-invariant docs):*)
(*   T1  fence: observed success => the obligated push is durably published  *)
(*   T2  closure: a published manifest either covers its parent's packs or   *)
(*       names a trusted full-closure compaction pack                         *)
(*   T3  ref linearizability: installs form a fork-free chain, each commits  *)
(*       exactly the value it proposed, derived from the pointer it read     *)
(* Each invariant is mutation-tested non-vacuous; see docs/ Mechanized §.    *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS Pushers,        \* set of concurrent pusher ids
          MaxManifests    \* bound on distinct manifests (model finiteness)

VARIABLES
    pointer,    \* current manifest id held by M_R (a natural; 0 = empty repo)
    published,  \* set of manifest ids ever installed as pointer (durable history)
    packs,      \* function: manifest id -> set of pack ids it names
    pc,         \* pusher id -> control state
    readEtag,   \* pusher id -> pointer value it last read (its CAS precondition)
    staged,     \* pusher id -> manifest id it intends to install
    parent,     \* manifest id -> the manifest id it was derived from (history)
    refs,       \* manifest id -> objectId that this manifest binds the ref "main" to
    compacted,  \* manifests whose own pack is a full closure of their refs
    newVal,     \* pusher id -> objectId this push proposes for "main" (its effect)
    snapErr,    \* pusher id -> did either ref-snapshot read fail? (BOOLEAN)
    observed    \* set of pusher ids that have observed success (fence passed)

vars == <<pointer, published, packs, pc, readEtag, staged,
          parent, refs, compacted, newVal, snapErr, observed>>

\* We model a single ref, "main", whose value is an objectId in ObjIds. This is
\* enough to exhibit ref-update linearizability: a lost update is the published
\* value of "main" reverting or skipping a committed predecessor's value.
\* (Dawn's point: prove ref VALUES survive, not just that effect tokens are
\* monotone.) refs[m] is the value "main" holds in manifest m.
ObjIds == 0..MaxManifests

\* A push CHANGES refs iff the value it proposes differs from the value in the
\* manifest it READ. This is now DERIVED from real ref state, not a free boolean.
DidChange(p) == newVal[p] # refs[readEtag[p]]

ManifestIds == 0..MaxManifests

TypeOK ==
    /\ pointer \in ManifestIds
    /\ published \subseteq ManifestIds
    /\ packs \in [ManifestIds -> SUBSET ManifestIds]
    /\ pc \in [Pushers -> {"idle","staged","done","lost"}]
    /\ readEtag \in [Pushers -> ManifestIds]
    /\ staged \in [Pushers -> ManifestIds]
    /\ parent \in [ManifestIds -> ManifestIds]
    /\ refs \in [ManifestIds -> ObjIds]
    /\ compacted \subseteq ManifestIds
    /\ newVal \in [Pushers -> ObjIds]
    /\ snapErr \in [Pushers -> BOOLEAN]
    /\ observed \subseteq Pushers

Init ==
    /\ pointer = 0
    /\ published = {0}
    /\ packs = [m \in ManifestIds |-> {}]
    /\ pc = [p \in Pushers |-> "idle"]
    /\ readEtag = [p \in Pushers |-> 0]
    /\ staged = [p \in Pushers |-> 0]
    /\ parent = [m \in ManifestIds |-> 0]
    /\ refs = [m \in ManifestIds |-> 0]   \* "main" starts at objectId 0 (empty)
    /\ compacted = {}
    /\ newVal = [p \in Pushers |-> 0]
    /\ snapErr = [p \in Pushers |-> FALSE]
    /\ observed = {}

\* A fresh manifest id, distinct from every published manifest AND every
\* concurrently-staged one (Perci): distinct pushes mint distinct content-addressed
\* manifests, so two concurrent stages never alias the same id. This keeps the
\* no-lost-update counterexamples about CAS serialization, not id collision.
StagedIds == { staged[q] : q \in Pushers }
FreshId == CHOOSE m \in ManifestIds : m \notin published /\ m \notin StagedIds /\ m # 0

CanStage == \E m \in ManifestIds : m \notin published /\ m \notin StagedIds /\ m # 0

\* The publish-skip decision (the fallible-snapshot fence, Quinn #2 / Dawn's case).
\* A push skips publish ONLY if its snapshots succeeded AND showed no ref change.
\* If either snapshot errored (snapErr), it must NOT skip -- it falls through to CAS.
\* This is "Ok(b) = Ok(a)", never "b = a" with errors silently equal.
MustPublish(p) == DidChange(p) \/ snapErr[p]

\* Steps 3-6: read pointer; nondeterministically this push either changes refs or
\* is a no-op, and its ref-snapshot reads either succeed or fail. Stage a manifest.
Begin(p) ==
    /\ pc[p] = "idle"
    /\ CanStage
    \* This push proposes some value v for "main" (v = current value models a
    \* no-op push; v # current models a real ref change); its snapshot reads may
    \* fail (e). The staged manifest binds "main" to v and is derived from the
    \* manifest the push READ -- so a stale reader builds on stale ref state, and
    \* only the CAS guard stops it from clobbering a newer published value.
    /\ \E v \in ObjIds, e \in BOOLEAN, compact \in BOOLEAN :
         /\ newVal'  = [newVal  EXCEPT ![p] = v]
         /\ snapErr' = [snapErr EXCEPT ![p] = e]
         /\ LET m == FreshId IN
              /\ readEtag' = [readEtag EXCEPT ![p] = pointer]
              /\ staged'   = [staged   EXCEPT ![p] = m]
              /\ parent'   = [parent   EXCEPT ![m] = pointer]
              \* A compact stage models `pack-objects` over every post-push
              \* ref tip. Its own pack is therefore trusted to cover the full
              \* reachable closure; a normal stage extends the parent pack set.
              /\ packs'    = [packs EXCEPT
                                  ![m] = IF compact
                                        THEN {m}
                                        ELSE packs[pointer] \union {m}]
              /\ refs'     = [refs     EXCEPT ![m] = v]
              /\ compacted' = IF compact
                              THEN compacted \union {m}
                              ELSE compacted \ {m}
    /\ pc' = [pc EXCEPT ![p] = "staged"]
    /\ UNCHANGED <<pointer, published, observed>>

\* No-op fast path: a push that must NOT publish (no change, snapshots ok) goes
\* straight to done WITHOUT touching the pointer -- zero CAS/publish latency.
SkipPublish(p) ==
    /\ pc[p] = "staged"
    /\ ~MustPublish(p)
    /\ pc' = [pc EXCEPT ![p] = "done"]
    /\ UNCHANGED <<pointer, published, packs, readEtag, staged, parent, refs, compacted, newVal, snapErr, observed>>

\* Step 7: CAS.  Succeeds iff pointer still equals the etag this pusher read (A3).
CasSucceed(p) ==
    /\ pc[p] = "staged"
    /\ MustPublish(p)
    /\ pointer = readEtag[p]
    /\ pointer' = staged[p]
    /\ published' = published \union {staged[p]}
    /\ pc' = [pc EXCEPT ![p] = "done"]
    /\ UNCHANGED <<packs, readEtag, staged, parent, refs, compacted, newVal, snapErr, observed>>

CasFail(p) ==
    /\ pc[p] = "staged"
    /\ MustPublish(p)
    /\ pointer # readEtag[p]
    /\ pc' = [pc EXCEPT ![p] = "lost"]   \* will retry from idle
    /\ UNCHANGED <<pointer, published, packs, readEtag, staged, parent, refs, compacted, newVal, snapErr, observed>>

\* Step 8: the fence.  Observe success ONLY after the push reached "done"
\* (either via successful CAS or a legitimate skip).
Observe(p) ==
    /\ pc[p] = "done"
    /\ observed' = observed \union {p}
    /\ UNCHANGED <<pointer, published, packs, pc, readEtag, staged, parent, refs, compacted, newVal, snapErr>>

\* A loser retries: back to idle, ready to re-read the advanced pointer.
Retry(p) ==
    /\ pc[p] = "lost"
    /\ pc' = [pc EXCEPT ![p] = "idle"]
    /\ UNCHANGED <<pointer, published, packs, readEtag, staged, parent, refs, compacted, newVal, snapErr, observed>>

Next ==
    \E p \in Pushers :
        Begin(p) \/ SkipPublish(p) \/ CasSucceed(p) \/ CasFail(p)
          \/ Observe(p) \/ Retry(p)

Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

------------------------------------------------------------------------------
\* SAFETY PROPERTIES

\* T1 (Durability-Ordering): any observed push that was obligated to publish
\* (it changed refs, or its snapshot reads errored) has its staged manifest in
\* the durable published history before the client observes success. A
\* legitimately-skipped no-op push (no change, snapshots ok) is exempt -- it
\* publishes nothing and is correct to do so.
Inv_Fence ==
    \A p \in observed : MustPublish(p) => staged[p] \in published

\* The bite for the fallible-snapshot case (Quinn #2 / Dawn): if a push actually
\* changed refs and was observed, its change is durably published -- regardless of
\* snapshot outcome. This is what breaks if SkipPublish ignores snapErr (i.e. if
\* the skip predicate were "b = a" instead of "Ok(b) = Ok(a) /\ no change").
\*
\* NOTE (Dawn): this is NOT redundant with Inv_Fence, even though
\* MustPublish == DidChange \/ snapErr makes Inv_Fence look strictly stronger.
\* Inv_Fence is predicated on the OPERATOR MustPublish; mutate that operator (the
\* skip-on-error bug) and Inv_Fence's own predicate moves with it, so the mutated
\* Inv_Fence stops catching the bug. Inv_ChangedPublished is predicated on
\* DidChange directly, independent of MustPublish, so it stays load-bearing under
\* exactly the mutation we care about. Do not delete it as "redundant."
Inv_ChangedPublished ==
    \A p \in observed : DidChange(p) => staged[p] \in published

Installed(p) == (p \in observed) /\ MustPublish(p) /\ (staged[p] \in published)

\* (A former Inv_NoLost -- "distinct installs never share a manifest id" -- was
\* removed: with FreshId excluding in-flight staged ids, Inv_NoFork implies it, so
\* it caught only a model aliasing artifact, not a real failure mode. Verified by
\* checking that no mutation trips it without also tripping Inv_NoFork.)

\* T3b (Ref-update linearizability -- Dawn's user-visible theorem): the model
\* now carries the REAL ref value (refs[m] = the objectId "main" holds in manifest
\* m), not just effect tokens. Two properties bind the proof to ref VALUES:
\*
\* (i) Every installed push's own proposed value is exactly what its manifest
\*     commits -- the push's effect is applied, not dropped.
Inv_RefEffectApplied ==
    \A p \in Pushers : Installed(p) => (refs[staged[p]] = newVal[p])

\* (ii) An installed push computed its new value from the manifest that was the
\*     pointer AT INSTALL TIME (its parent is the pointer it read, and the CAS
\*     guard forced read == current). So no install builds "main" on top of a
\*     value that a concurrent winner already superseded -- the lost-update of a
\*     ref value. Operationally: an installed manifest's parent is published and
\*     its value was derived from that parent, giving a single serial line of ref
\*     values. (The fork ban, Inv_NoFork, plus this, is ref linearizability.)
Inv_RefDerivedFromParent ==
    \A p \in Pushers :
        Installed(p) => (parent[staged[p]] = readEtag[p] /\ readEtag[p] \in published)

\* T2 (Reconstruction coverage -- non-vacuous): every published non-root
\* manifest either names its trusted full-closure compaction pack, or covers its
\* published parent's pack set plus its own delta pack. The model abstracts
\* Git's reachability walk as the `compacted` marker; production earns that
\* marker only by feeding every post-push ref tip to `git pack-objects --revs`.
Inv_Closed ==
    \A m \in published :
        (m # 0 /\ parent[m] \in published) =>
            (m \in packs[m] /\
                (m \in compacted \/ packs[parent[m]] \subseteq packs[m]))

\* Parent integrity: every published non-root manifest's parent is also published
\* (the install chain is grounded in durable history, never in vapor).
Inv_ParentPublished ==
    \A m \in published : (m = 0) \/ (parent[m] \in published)

\* The pointer is always itself a published manifest (never points at vapor).
Inv_PointerPublished ==
    pointer \in published

\* T3c (Linear history -- the real no-lost-update): the published manifests form
\* a single chain ending at the current pointer; there is no fork. A lost update
\* is precisely a fork: two installs sharing a parent, so one's effects are
\* dropped from the surviving line. Reachability of every published manifest from
\* the pointer via parent edges rules that out. With MaxManifests bound, we check
\* the contrapositive directly: no two distinct published non-root manifests share
\* a parent (a shared parent = a fork = a lost update). The A3 CAS guard is what
\* makes this hold; removing it lets two pushers install off the same parent.
Inv_NoFork ==
    \A m1, m2 \in published :
        (m1 # m2 /\ m1 # 0 /\ m2 # 0) => (parent[m1] # parent[m2])

\* Finiteness bound: at most MaxManifests distinct manifests may be published.
\* Without it the Retry loop lets pushers churn newVal/FreshId unboundedly.
BoundedManifests == Cardinality(published) <= MaxManifests

Safety ==
    /\ TypeOK
    /\ Inv_Fence
    /\ Inv_ChangedPublished
    /\ Inv_RefEffectApplied
    /\ Inv_RefDerivedFromParent
    /\ Inv_NoFork
    /\ Inv_Closed
    /\ Inv_ParentPublished
    /\ Inv_PointerPublished
=============================================================================
