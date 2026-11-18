/-!
# Conditional pointer writes for Git on an object store

This file proves four safety properties of the storage protocol that
`../GitOnObjectStore.tla` checks with TLC for small finite instances. The
proofs here hold for any number of clients, refs, objects and packs. Only core
Lean is used.

## What is modeled

A repository is stored as immutable manifests and packs plus one mutable
pointer that names the current manifest. A writer reads the pointer (`begin`),
stages a successor manifest computed from the value it read, uploads that
manifest and its packs (`upload`), and then performs a conditional write of
the pointer (`cas`), which succeeds only if the ETag of the pointer equals the
ETag of the value the writer read. A writer may stop at any time (`abort`): a
lost race, a crash, or a timeout. Writers interleave arbitrarily.

`Model.Staged` is the set of manifests a writer may stage from a pointer value
`p`: the manifest records `p` as its parent, its ref state is a legal successor
of the ref state of `p`, and its pack list follows one of three rules (no new
pack, one delta pack added to the packs of `p`, or one pack that replaces them
all).

Two fields are ghost state, used only to state the results: `State.writes`
counts successful pointer writes, and `Writer.readAt` records the value of that
counter when the writer read the pointer.

## Assumptions

* `Model.etag_inj`: the ETag of a pointer body determines the body. This
  stands for two facts about the implementation: the object store assigns
  different ETags to different bodies, and SHA-256 has no collision among the
  values that occur.
* A digest is modeled by the value it names. `Manifest.parent` therefore holds
  the predecessor manifest itself, and the predecessor chain is finite because
  `Manifest` is an inductive type.
* Nothing is removed from the manifest store or the pack store. Garbage
  collection is outside this model.
* `Model.Legal`, `Model.reach` and `Model.objs` are arbitrary relations.
  Nothing is assumed about which ref updates are legal or which objects a ref
  state reaches.

## Results (for every reachable state)

* `fence`: the manifest the pointer names is stored, and so is every pack that
  manifest lists.
* `closed`: the packs listed by the pointed-to manifest contain every object
  reachable from its ref state. This is derived from the three pack rules.
* `pointer_step_legal`: a step either leaves the pointer unchanged or replaces
  it by a manifest whose parent is the replaced value and whose ref state is a
  legal successor of the replaced ref state. No update is lost.
* `cas_sees_no_intervening_write`: when a conditional write is enabled, no
  pointer write has happened since the writer read the pointer, although the
  ETag depends only on the body.
* `pointer_depth_eq_writes`: the length of the predecessor chain of the pointer
  equals the number of pointer writes, so a published manifest is never
  published again.

`write_is_possible` exhibits an instance in which a conditional write
succeeds, so the results are not vacuous.

## What is not covered

The proofs concern the pointer discipline only. The decision whether a push is
published, the reports sent to clients, the default-branch setter, readers and
garbage collection are covered by the TLA+ model for finite instances and are
not formalized here. `Model.Legal` stands for everything `git receive-pack` and
the push policy permit; this file does not say what that is.
-/

namespace GitPointer

/-- A manifest: a published ref state, the set of packs that hold its objects,
and the manifest it replaced (`none` for the first manifest of a repository). -/
structure Manifest (R Pk : Type) where
  refs : R
  packs : Pk → Prop
  parent : Option (Manifest R Pk)

variable {R Obj Pk Client E : Type}

/-- The length of the predecessor chain of a manifest, counting the manifest. -/
def Manifest.depth : Manifest R Pk → Nat
  | ⟨_, _, none⟩ => 1
  | ⟨_, _, some p⟩ => p.depth + 1

/-- The depth of a pointer value; an absent pointer has depth 0. -/
def depthOpt : Option (Manifest R Pk) → Nat
  | none => 0
  | some m => m.depth

theorem Manifest.depth_eq (m : Manifest R Pk) : m.depth = depthOpt m.parent + 1 := by
  cases m with
  | mk r p par => cases par <;> simp [Manifest.depth, depthOpt]

/-- The parameters of the protocol. `objs pk o`: pack `pk` contains object `o`.
`reach r o`: object `o` is reachable from ref state `r`. `Legal b r`: ref state
`r` may follow base `b`, where `none` is a repository that does not exist.
`etag` is the ETag of a pointer body, assumed to determine the body. -/
structure Model (R Obj Pk E : Type) where
  objs : Pk → Obj → Prop
  reach : R → Obj → Prop
  Legal : Option R → R → Prop
  etag : Manifest R Pk → E
  etag_inj : ∀ a b, etag a = etag b → a = b

/-- The local state of a writer: the pointer value it read, the manifest it
staged, whether it has uploaded that manifest, and (ghost) the write count at
the time of its read. -/
structure Writer (R Pk : Type) where
  base : Option (Manifest R Pk)
  staged : Manifest R Pk
  uploaded : Bool
  readAt : Nat

/-- The global state: the pointer, the manifest store, the pack store, the
writers in progress, and (ghost) the number of successful pointer writes. -/
structure State (R Pk Client : Type) where
  pointer : Option (Manifest R Pk)
  mstore : Manifest R Pk → Prop
  pstore : Pk → Prop
  writers : Client → Option (Writer R Pk)
  writes : Nat

/-- The initial state: no pointer, empty stores, no writers. -/
def State.init : State R Pk Client :=
  ⟨none, fun _ => False, fun _ => False, fun _ => none, 0⟩

/-- `pk` is listed by the manifest `p` names. False when `p` is absent. -/
def basePacks (p : Option (Manifest R Pk)) (pk : Pk) : Prop :=
  ∃ b, p = some b ∧ b.packs pk

/-- `g` is `f` with the entry of client `c` replaced by `v`. Stated as a
relation so that equality of clients need not be decidable. -/
def Upd (f : Client → Option (Writer R Pk)) (c : Client) (v : Option (Writer R Pk))
    (g : Client → Option (Writer R Pk)) : Prop :=
  g c = v ∧ ∀ c', c' ≠ c → g c' = f c'

/-- A writer present after an update is the new entry or was present before. -/
theorem Upd.cases {f g : Client → Option (Writer R Pk)} {c : Client} {v : Option (Writer R Pk)}
    (h : Upd f c v g) {c' : Client} {w : Writer R Pk} (hw : g c' = some w) :
    v = some w ∨ f c' = some w := by
  rcases Classical.em (c' = c) with rfl | hne
  · exact Or.inl (h.1.symm.trans hw)
  · exact Or.inr ((h.2 c' hne).symm.trans hw)

variable (M : Model R Obj Pk E)

/-- `o` is reachable from the ref state of the manifest `p` names. -/
def Model.baseReach (p : Option (Manifest R Pk)) (o : Obj) : Prop :=
  ∃ b, p = some b ∧ M.reach b.refs o

/-- The manifests a writer may stage after reading pointer value `p`. -/
def Model.Staged (p : Option (Manifest R Pk)) (m : Manifest R Pk) : Prop :=
  m.parent = p ∧ M.Legal (p.map Manifest.refs) m.refs ∧
  (-- no new pack
   ((∀ o, M.reach m.refs o → M.baseReach p o) ∧ ∀ pk, m.packs pk ↔ basePacks p pk) ∨
   -- delta pack
   (∃ d, (∀ o, M.reach m.refs o → M.baseReach p o ∨ M.objs d o) ∧
      ∀ pk, m.packs pk ↔ (basePacks p pk ∨ pk = d)) ∨
   -- compaction
   (∃ f, (∀ o, M.reach m.refs o → M.objs f o) ∧ ∀ pk, m.packs pk ↔ pk = f))

/-- The packs a manifest lists contain every object reachable from its refs. -/
def Model.Closed (m : Manifest R Pk) : Prop :=
  ∀ o, M.reach m.refs o → ∃ pk, m.packs pk ∧ M.objs pk o

/-- The steps of the protocol. Each constructor lists its enabling conditions;
the post-state is the pre-state with the named fields replaced. -/
inductive Model.Step : State R Pk Client → State R Pk Client → Prop
  /-- Client `c` reads the pointer and stages a manifest `m`. -/
  | begin (s : State R Pk Client) (c : Client) (m : Manifest R Pk) (ws) :
      s.writers c = none → M.Staged s.pointer m →
      Upd s.writers c (some ⟨s.pointer, m, false, s.writes⟩) ws →
      Model.Step s { s with writers := ws }
  /-- Client `c` stores its staged manifest and the packs it lists. -/
  | upload (s : State R Pk Client) (c : Client) (w : Writer R Pk) (ws) :
      s.writers c = some w →
      Upd s.writers c (some { w with uploaded := true }) ws →
      Model.Step s { s with
        mstore := fun x => s.mstore x ∨ x = w.staged
        pstore := fun pk => s.pstore pk ∨ w.staged.packs pk
        writers := ws }
  /-- The conditional write. The comparison is on `Option E`, so an absent
  pointer matches only an absent base (`If-None-Match: *`). -/
  | cas (s : State R Pk Client) (c : Client) (w : Writer R Pk) (ws) :
      s.writers c = some w → w.uploaded = true →
      s.pointer.map M.etag = w.base.map M.etag →
      Upd s.writers c none ws →
      Model.Step s { s with pointer := some w.staged, writes := s.writes + 1, writers := ws }
  /-- Client `c` stops: a lost race, a crash, or a timeout. -/
  | abort (s : State R Pk Client) (c : Client) (w : Writer R Pk) (ws) :
      s.writers c = some w → Upd s.writers c none ws →
      Model.Step s { s with writers := ws }

/-- The states reachable from the initial state by finitely many steps. -/
inductive Model.Reachable : State R Pk Client → Prop
  | init : Model.Reachable State.init
  | step {s s' : State R Pk Client} : Model.Reachable s → M.Step s s' → Model.Reachable s'

/-! ## Auxiliary lemmas and the inductive invariant -/

/-- A manifest staged from a closed base is closed. This is where the three
pack rules are used. -/
theorem Model.Staged.closed {p : Option (Manifest R Pk)} {m : Manifest R Pk}
    (h : M.Staged p m) (hp : ∀ b, p = some b → M.Closed b) : M.Closed m := by
  intro o ho
  have base : M.baseReach p o → ∃ pk, basePacks p pk ∧ M.objs pk o := by
    rintro ⟨b, rfl, hb⟩
    obtain ⟨pk, hpk, hobj⟩ := hp b rfl o hb
    exact ⟨pk, ⟨b, rfl, hpk⟩, hobj⟩
  rcases h.2.2 with ⟨hr, hpk⟩ | ⟨d, hr, hpk⟩ | ⟨f, hr, hpk⟩
  · obtain ⟨pk, h1, h2⟩ := base (hr o ho)
    exact ⟨pk, (hpk pk).2 h1, h2⟩
  · rcases hr o ho with hb | hd
    · obtain ⟨pk, h1, h2⟩ := base hb
      exact ⟨pk, (hpk pk).2 (Or.inl h1), h2⟩
    · exact ⟨d, (hpk d).2 (Or.inr rfl), hd⟩
  · exact ⟨f, (hpk f).2 rfl, hr o ho⟩

/-- Equal ETags (as `Option E`) imply equal pointer values, by `etag_inj`. -/
theorem Model.eq_of_etag_eq {p q : Option (Manifest R Pk)}
    (h : p.map M.etag = q.map M.etag) : p = q := by
  cases p <;> cases q <;> simp at h
  · rfl
  · exact congrArg some (M.etag_inj _ _ h)

/-- The invariant of one writer: its staged manifest was computed from its
base, its base is closed, the depth of its base is the write count it read,
and once it has uploaded, its staged manifest and packs are stored. -/
structure Model.WriterInv (s : State R Pk Client) (w : Writer R Pk) : Prop where
  staged : M.Staged w.base w.staged
  baseClosed : ∀ b, w.base = some b → M.Closed b
  readAt : depthOpt w.base = w.readAt
  stored : w.uploaded = true → s.mstore w.staged ∧ ∀ pk, w.staged.packs pk → s.pstore pk

/-- The inductive invariant of the protocol. -/
structure Model.Inv (s : State R Pk Client) : Prop where
  fence : ∀ m, s.pointer = some m → s.mstore m ∧ ∀ pk, m.packs pk → s.pstore pk
  closed : ∀ m, s.pointer = some m → M.Closed m
  depth : depthOpt s.pointer = s.writes
  writers : ∀ c w, s.writers c = some w → M.WriterInv s w

/-- The writer invariant survives any step that only adds to the stores. -/
theorem Model.WriterInv.mono {s s' : State R Pk Client} {w : Writer R Pk}
    (h : M.WriterInv s w) (hm : ∀ x, s.mstore x → s'.mstore x)
    (hp : ∀ pk, s.pstore pk → s'.pstore pk) : M.WriterInv s' w :=
  ⟨h.staged, h.baseClosed, h.readAt, fun hu =>
    ⟨hm _ (h.stored hu).1, fun pk hpk => hp pk ((h.stored hu).2 pk hpk)⟩⟩

theorem Model.Inv.init : M.Inv (State.init : State R Pk Client) :=
  ⟨fun _ h => (nomatch h), fun _ h => (nomatch h), rfl, fun _ _ h => (nomatch h)⟩

theorem Model.Inv.step {s s' : State R Pk Client} (inv : M.Inv s) (st : M.Step s s') :
    M.Inv s' := by
  cases st with
  | begin c m ws hc hm hu =>
    refine ⟨inv.fence, inv.closed, inv.depth, fun c' w' hw' => ?_⟩
    rcases hu.cases hw' with hnew | hold
    · cases hnew
      exact ⟨hm, inv.closed, inv.depth, fun h => nomatch h⟩
    · exact (inv.writers c' w' hold).mono M (fun _ h => h) (fun _ h => h)
  | upload c w ws hc hu =>
    refine ⟨fun m hm => ?_, inv.closed, inv.depth, fun c' w' hw' => ?_⟩
    · exact ⟨Or.inl (inv.fence m hm).1, fun pk h => Or.inl ((inv.fence m hm).2 pk h)⟩
    · rcases hu.cases hw' with hnew | hold
      · cases hnew
        have hw := inv.writers c w hc
        exact ⟨hw.staged, hw.baseClosed, hw.readAt, fun _ => ⟨Or.inr rfl, fun _ h => Or.inr h⟩⟩
      · exact (inv.writers c' w' hold).mono M (fun _ h => Or.inl h) (fun _ h => Or.inl h)
  | cas c w ws hc hup he hu =>
    have hw := inv.writers c w hc
    have hbase : s.pointer = w.base := M.eq_of_etag_eq he
    refine ⟨fun m hm => ?_, fun m hm => ?_, ?_, fun c' w' hw' => ?_⟩
    · cases hm
      exact hw.stored hup
    · cases hm
      exact hw.staged.closed M hw.baseClosed
    · show w.staged.depth = s.writes + 1
      rw [w.staged.depth_eq, hw.staged.1, ← hbase, inv.depth]
    · rcases hu.cases hw' with hnew | hold
      · cases hnew
      · exact (inv.writers c' w' hold).mono M (fun _ h => h) (fun _ h => h)
  | abort c w ws hc hu =>
    refine ⟨inv.fence, inv.closed, inv.depth, fun c' w' hw' => ?_⟩
    rcases hu.cases hw' with hnew | hold
    · cases hnew
    · exact (inv.writers c' w' hold).mono M (fun _ h => h) (fun _ h => h)

/-- Every reachable state satisfies the invariant. -/
theorem Model.Reachable.inv {s : State R Pk Client} (hs : M.Reachable s) : M.Inv s := by
  induction hs with
  | init => exact Model.Inv.init M
  | step _ st ih => exact ih.step M st

/-! ## Main results -/

/-- The pointer names only what is stored: the manifest it names is in the
manifest store, and every pack that manifest lists is in the pack store. -/
theorem fence {s : State R Pk Client} (hs : M.Reachable s) {m : Manifest R Pk}
    (hm : s.pointer = some m) : s.mstore m ∧ ∀ pk, m.packs pk → s.pstore pk :=
  hs.inv.fence m hm

/-- The packs listed by the pointed-to manifest contain every object reachable
from its ref state. -/
theorem closed {s : State R Pk Client} (hs : M.Reachable s) {m : Manifest R Pk}
    (hm : s.pointer = some m) : ∀ o, M.reach m.refs o → ∃ pk, m.packs pk ∧ M.objs pk o :=
  hs.inv.closed m hm

/-- Every step from a reachable state leaves the pointer unchanged or replaces
it by a manifest whose parent is the replaced value and whose ref state is a
legal successor of the replaced ref state. Each change of the published ref
state is thus one step of an atomic ref database, computed against the state
it replaces. -/
theorem pointer_step_legal {s s' : State R Pk Client} (hs : M.Reachable s) (st : M.Step s s') :
    s'.pointer = s.pointer ∨
    ∃ m, s'.pointer = some m ∧ m.parent = s.pointer ∧
      M.Legal (s.pointer.map Manifest.refs) m.refs := by
  cases st with
  | begin => exact Or.inl rfl
  | upload => exact Or.inl rfl
  | abort => exact Or.inl rfl
  | cas c w ws hc hup he hu =>
    have hw := hs.inv.writers c w hc
    have hbase : s.pointer = w.base := M.eq_of_etag_eq he
    exact Or.inr ⟨w.staged, rfl, hbase ▸ hw.staged.1, hbase ▸ hw.staged.2.1⟩

/-- The length of the predecessor chain of the pointer equals the number of
pointer writes. Each write therefore publishes a manifest of a depth that no
earlier published manifest had. -/
theorem pointer_depth_eq_writes {s : State R Pk Client} (hs : M.Reachable s) :
    depthOpt s.pointer = s.writes :=
  hs.inv.depth

/-- If the conditional write of writer `w` is enabled in a reachable state (the
three hypotheses are the enabling conditions of `Model.Step.cas`), then no
pointer write has happened since `w` read the pointer. -/
theorem cas_sees_no_intervening_write {s : State R Pk Client} (hs : M.Reachable s)
    {c : Client} {w : Writer R Pk} (hc : s.writers c = some w) (_hup : w.uploaded = true)
    (he : s.pointer.map M.etag = w.base.map M.etag) : w.readAt = s.writes := by
  have hw := hs.inv.writers c w hc
  rw [← hw.readAt, ← M.eq_of_etag_eq he, hs.inv.depth]

/-! ## Non-vacuity -/

/-- The enabling conditions can be met: there is an instance of the model in
which a writer reads the absent pointer, uploads, and wins the conditional
write. The results above therefore do not hold merely because no pointer write
is ever possible. -/
theorem write_is_possible :
    ∃ (M : Model Unit Unit Unit (Manifest Unit Unit)) (s : State Unit Unit Unit),
      M.Reachable s ∧ s.writes = 1 ∧ s.pointer ≠ none := by
  let M : Model Unit Unit Unit (Manifest Unit Unit) :=
    ⟨fun _ _ => True, fun _ _ => True, fun _ _ => True, id, fun _ _ h => h⟩
  let m : Manifest Unit Unit := ⟨(), fun _ => True, none⟩
  have upd : ∀ (f : Unit → Option (Writer Unit Unit)) v, Upd f () v (fun _ => v) :=
    fun _ _ => ⟨rfl, fun _ h => absurd rfl h⟩
  have staged : M.Staged none m :=
    ⟨rfl, trivial, Or.inr (Or.inr ⟨(), fun _ _ => trivial, fun _ => ⟨fun _ => rfl, fun _ => trivial⟩⟩)⟩
  have s1 := Model.Reachable.step Model.Reachable.init
    (Model.Step.begin (M := M) State.init () m _ rfl staged (upd _ _))
  have s2 := Model.Reachable.step s1 (Model.Step.upload _ () _ _ rfl (upd _ _))
  have s3 := Model.Reachable.step s2 (Model.Step.cas _ () _ _ rfl rfl rfl (upd _ _))
  exact ⟨M, _, s3, rfl, fun h => nomatch h⟩

end GitPointer

#print axioms GitPointer.fence
#print axioms GitPointer.closed
#print axioms GitPointer.pointer_step_legal
#print axioms GitPointer.cas_sees_no_intervening_write
#print axioms GitPointer.pointer_depth_eq_writes
#print axioms GitPointer.write_is_possible
