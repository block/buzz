/- Specification equivalences and machine-checked descending-scan proofs. -/
import Aeneas
import RelayFilter.Spec
import RelayFilter.Vocab
import RelayFilter.Code.Funs

open Aeneas Std Result
open RelayFilter.Spec
open Djinn.Vocab

namespace RelayFilter.Proofs

def Ids (ids : Option (List (List U8))) (id : List U8) : Prop :=
  match ids with | none => True | some xs => id ∈ xs

def Kinds (ks : Option (List U16)) (k : U16) : Prop :=
  match ks with | none => True | some xs => k ∈ xs

def Since (bound : Option U64) (time : U64) : Prop :=
  match bound with | none => True | some b => b.val ≤ time.val

def Until (bound : Option U64) (time : U64) : Prop :=
  match bound with | none => True | some b => time.val ≤ b.val

def Tag (tag : input.EventTag) (key : List U8) (values : List (List U8)) : Prop :=
  tag.key.val = key ∧ ∃ value, tag.value = some value ∧ value.val ∈ values

def Constraint (query : input.TagQuery) (event : input.EventInput) : Prop :=
  (∃ tag ∈ event.tags.val, Tag tag query.key.val (query.values.val.map (·.val))) ∨
  (query.key.val = [104#u8] ∧
    (∀ tag ∈ event.tags.val, tag.key.val ≠ [104#u8]) ∧
    ∃ channel, event.channel = some channel ∧ channel.val ∈ query.values.val.map (·.val))

def One (filter : input.FilterInput) (event : input.EventInput) : Prop :=
  Ids (filter.ids.map (fun xs => xs.val.map (·.val))) event.id.val ∧
  Ids (filter.authors.map (fun xs => xs.val.map (·.val))) event.author.val ∧
  Kinds (filter.kinds.map (·.val)) event.kind ∧
  Since filter.since event.created_at ∧ Until filter.until event.created_at ∧
  ∀ query ∈ filter.tags.val, Constraint query event

theorem bool_ext (a b : Bool) (h : (a = true ↔ b = true)) : a = b := by
  cases a <;> cases b <;> simp_all

theorem identities_eq (ids : Option (List (List U8))) (id : List U8) (r : Bool) :
    identities_spec ids id r ↔ (r = true ↔ Ids ids id) := by
  cases ids <;> simp [identities_spec, identities_pre, Ids]

theorem kinds_eq (ks : Option (List U16)) (k : U16) (r : Bool) :
    kinds_spec ks k r ↔ (r = true ↔ Kinds ks k) := by
  cases ks <;> simp [kinds_spec, kinds_pre, Kinds]

theorem since_eq (bound : Option U64) (time : U64) (r : Bool) :
    since_spec bound time r ↔ (r = true ↔ Since bound time) := by
  cases bound <;> simp [since_spec, since_pre, Since]

theorem until_eq (bound : Option U64) (time : U64) (r : Bool) :
    until_spec bound time r ↔ (r = true ↔ Until bound time) := by
  cases bound <;> simp [until_spec, until_pre, Until]

theorem tag_eq (tag : input.EventTag) (key : List U8) (values : List (List U8)) (r : Bool) :
    tag_match_spec tag key values r ↔ (r = true ↔ Tag tag key values) := by
  cases h : tag.value <;> cases r <;>
    simp [tag_match_spec, tag_match_pre, Tag, h]

theorem constraint_eq (query : input.TagQuery) (event : input.EventInput) (r : Bool) :
    tag_constraint_spec query event r ↔ (r = true ↔ Constraint query event) := by
  classical
  cases h : event.channel <;> cases r <;>
    simp [tag_constraint_spec, tag_constraint_pre, tag_eq, Constraint, h,
      Bool.exists_bool] <;> first | tauto | grind

theorem one_eq (filter : input.FilterInput) (event : input.EventInput) (r : Bool) :
    one_filter_spec filter event r ↔ (r = true ↔ One filter event) := by
  classical
  cases r <;>
    simp [one_filter_spec, one_filter_pre, constraint_eq, identities_eq,
      kinds_eq, since_eq, until_eq, One, Bool.exists_bool] <;> first | tauto | grind

theorem filters_eq (filters : List input.FilterInput) (event : input.EventInput) (r : Bool) :
    filters_match_spec filters event r ↔ (r = true ↔ ∃ f ∈ filters, One f event) := by
  classical
  cases r <;> simp [filters_match_spec, filters_match_pre, one_eq, Bool.exists_bool]

theorem option_wf {α : Type} (o : Option α) :
    ¬(o.isNone = true) → o.isSome = true := by
  cases o <;> simp


/-! ## identities -/

theorem identities.wf :
    ∀ (ids : Option (List (List U8))) (id : List U8) (r : Bool),
      identities_spec ids id r →
      ¬(ids.isNone = true) → (ids.isSome = true) := by
  intros; apply option_wf; assumption

theorem identities.exists :
    ∀ (ids : Option (List (List U8))) (id : List U8), identities_pre ids id → ∃ r, identities_spec ids id r := by
  classical
  intro ids id _
  exact ⟨decide (Ids ids id), (identities_eq _ _ _).mpr (by simp)⟩

/-! ## kinds -/

theorem kinds.wf :
    ∀ (kinds : Option (List U16)) (kind : U16) (r : Bool),
      kinds_spec kinds kind r →
      ¬(kinds.isNone = true) → (kinds.isSome = true) := by
  intros; apply option_wf; assumption

theorem kinds.exists :
    ∀ (kinds : Option (List U16)) (kind : U16), kinds_pre kinds kind → ∃ r, kinds_spec kinds kind r := by
  classical
  intro ks k _
  exact ⟨decide (Kinds ks k), (kinds_eq _ _ _).mpr (by simp)⟩

/-! ## since -/

theorem since.wf :
    ∀ (bound : Option U64) (time : U64) (r : Bool),
      since_spec bound time r →
      ¬(bound.isNone = true) → (bound.isSome = true) := by
  intros; apply option_wf; assumption

theorem since.exists :
    ∀ (bound : Option U64) (time : U64), since_pre bound time → ∃ r, since_spec bound time r := by
  classical
  intro bound time _
  exact ⟨decide (Since bound time), (since_eq _ _ _).mpr (by simp)⟩

/-! ## until -/

theorem until.wf :
    ∀ (bound : Option U64) (time : U64) (r : Bool),
      until_spec bound time r →
      ¬(bound.isNone = true) → (bound.isSome = true) := by
  intros; apply option_wf; assumption

theorem until.exists :
    ∀ (bound : Option U64) (time : U64), until_pre bound time → ∃ r, until_spec bound time r := by
  classical
  intro bound time _
  exact ⟨decide (Until bound time), (until_eq _ _ _).mpr (by simp)⟩

/-! ## tag_match -/

theorem tag_match.wf :
    ∀ (tag : RelayFilter.input.EventTag) (key : List U8) (values : List (List U8)) (r : Bool),
      tag_match_spec tag key values r →
      ¬((tag.value.map (fun y => y.val)).isNone = true) → ∀ value : List U8, (value = (tag.value.map (fun y => y.val)).get! ∧ (r = true ↔ (tag.key.val = key ∧ (∃ v, v ∈ values ∧ v = value)))) → ((tag.value.map (fun y => y.val)).isSome = true) := by
  intros; apply option_wf; assumption

theorem tag_match.exists :
    ∀ (tag : RelayFilter.input.EventTag) (key : List U8) (values : List (List U8)), tag_match_pre tag key values → ∃ r, tag_match_spec tag key values r := by
  classical
  intro tag key values _
  exact ⟨decide (Tag tag key values), (tag_eq _ _ _ _).mpr (by simp)⟩

/-! ## tag_constraint -/

theorem tag_constraint.wf :
    ∀ (query : RelayFilter.input.TagQuery) (event : RelayFilter.input.EventInput) (r : Bool),
      tag_constraint_spec query event r →
      ∀ explicit : Bool, (((explicit = true → (∃ (tag : RelayFilter.input.EventTag), (tag ∈ event.tags.val ∧ tag_match_spec tag query.key.val (query.values.val.map (fun y => y.val)) true))) ∧ (¬(explicit = true) → (∀ tag, tag ∈ event.tags.val → tag_match_spec tag query.key.val (query.values.val.map (fun y => y.val)) false))) ∧ (((event.channel.map (fun y => y.val)).isNone = true → r = explicit) ∧ (¬((event.channel.map (fun y => y.val)).isNone = true) → (∃ (channel : List U8), (channel = (event.channel.map (fun y => y.val)).get! ∧ (r = true ↔ (explicit = true ∨ ((query.key.val = [104#u8] ∧ ¬((∃ tag, tag ∈ event.tags.val ∧ tag.key.val = [104#u8]))) ∧ (∃ value, value ∈ (query.values.val.map (fun y => y.val)) ∧ value = channel))))))))) → ¬((event.channel.map (fun y => y.val)).isNone = true) → ∀ channel : List U8, (channel = (event.channel.map (fun y => y.val)).get! ∧ (r = true ↔ (explicit = true ∨ ((query.key.val = [104#u8] ∧ ¬((∃ tag, tag ∈ event.tags.val ∧ tag.key.val = [104#u8]))) ∧ (∃ value, value ∈ (query.values.val.map (fun y => y.val)) ∧ value = channel))))) → ((event.channel.map (fun y => y.val)).isSome = true) := by
  intros; apply option_wf; assumption

theorem tag_constraint.exists :
    ∀ (query : RelayFilter.input.TagQuery) (event : RelayFilter.input.EventInput), tag_constraint_pre query event → ∃ r, tag_constraint_spec query event r := by
  classical
  intro query event _
  exact ⟨decide (Constraint query event), (constraint_eq _ _ _).mpr (by simp)⟩

/-! ## one_filter -/

theorem one_filter.exists :
    ∀ (filter : RelayFilter.input.FilterInput) (event : RelayFilter.input.EventInput), one_filter_pre filter event → ∃ r, one_filter_spec filter event r := by
  classical
  intro filter event _
  exact ⟨decide (One filter event), (one_eq _ _ _).mpr (by simp)⟩

/-! ## filters_match -/

theorem filters_match.unambiguous :
    ∀ (filters : List RelayFilter.input.FilterInput) (event : RelayFilter.input.EventInput) (o₁ : Bool) (o₂ : Bool),
      filters_match_spec filters event o₁ → filters_match_spec filters event o₂ → o₁ = o₂ := by
  intro filters event o₁ o₂ h₁ h₂
  exact bool_ext _ _ (((filters_eq _ _ _).mp h₁).trans ((filters_eq _ _ _).mp h₂).symm)


open RelayFilter.solution
open ControlFlow

-- A descending scan preserves the proposition of its unexamined prefix.
theorem descending_scan (body : Usize → Result (ControlFlow Usize Bool))
    (P : Nat → Prop) (n : Usize)
    (hbody : ∀ i, i.val ≤ n.val → body i ⦃ r => match r with
      | .done b => b = true ↔ P i.val
      | .cont j => j.val < i.val ∧ (P j.val ↔ P i.val) ⦄) :
    loop body n ⦃ b => b = true ↔ P n.val ⦄ := by
  apply loop.spec_decr_nat (fun i => i.val)
    (fun i => i.val ≤ n.val ∧ (P i.val ↔ P n.val))
  · intro i hi
    apply WP.spec_mono (hbody i hi.1)
    intro r hr
    cases r with
    | done b => exact hr.trans hi.2
    | cont j => exact ⟨⟨by omega, hr.2.trans hi.2⟩, hr.1⟩
  · exact ⟨Nat.le_refl _, Iff.rfl⟩

def PrefixAny {α : Type} (xs : List α) (P : α → Prop) (n : Nat) : Prop :=
  ∃ j, ∃ hj : j < xs.length, j < n ∧ P (xs[j]'hj)

def PrefixAll {α : Type} (xs : List α) (P : α → Prop) (n : Nat) : Prop :=
  ∀ j, ∀ hj : j < xs.length, j < n → P (xs[j]'hj)

theorem prefix_any_step {α : Type} (xs : List α) (P : α → Prop) (n : Nat)
    (hn : n < xs.length) :
    PrefixAny xs P (n+1) ↔ P (xs[n]'hn) ∨ PrefixAny xs P n := by
  simp only [PrefixAny]
  constructor
  · rintro ⟨j, hj, hjn, hp⟩
    by_cases h : j = n
    · subst j; exact Or.inl hp
    · exact Or.inr ⟨j, hj, by omega, hp⟩
  · rintro (hp | ⟨j, hj, hjn, hp⟩)
    · exact ⟨n, hn, by omega, hp⟩
    · exact ⟨j, hj, by omega, hp⟩

theorem prefix_all_step {α : Type} (xs : List α) (P : α → Prop) (n : Nat)
    (hn : n < xs.length) :
    PrefixAll xs P (n+1) ↔ P (xs[n]'hn) ∧ PrefixAll xs P n := by
  simp only [PrefixAll]
  constructor
  · intro h; exact ⟨h n hn (by omega), fun j hj hjn => h j hj (by omega)⟩
  · rintro ⟨hp, h⟩ j hj hjn
    by_cases he : j = n
    · subst j; exact hp
    · exact h j hj (by omega)

@[simp] theorem prefix_any_full {α : Type} (xs : List α) (P : α → Prop) :
    PrefixAny xs P xs.length ↔ ∃ x ∈ xs, P x := by
  simp only [PrefixAny]
  constructor
  · rintro ⟨j, hj, _, hp⟩; exact ⟨_, List.getElem_mem hj, hp⟩
  · rintro ⟨x, hx, hp⟩
    obtain ⟨j, hj, rfl⟩ := List.mem_iff_getElem.mp hx
    exact ⟨j, hj, hj, hp⟩

@[simp] theorem prefix_all_full {α : Type} (xs : List α) (P : α → Prop) :
    PrefixAll xs P xs.length ↔ ∀ x ∈ xs, P x := by
  simp only [PrefixAll]
  constructor
  · intro h x hx
    obtain ⟨j, hj, rfl⟩ := List.mem_iff_getElem.mp hx
    exact h j hj hj
  · intro h j hj _; exact h _ (List.getElem_mem hj)

theorem byte_list_eq (xs ys : List U8) (hlen : xs.length = ys.length) :
    List.allM (fun (p : U8 × U8) => match p with
      | (x, y) => do
        let b ← core.cmp.PartialEqU8.ne x y
        ok (decide (¬ b = true))) (xs.zip ys) = ok (decide (xs = ys)) := by
  induction xs generalizing ys with
  | nil =>
    cases ys <;> simp_all [List.allM]
    rfl
  | cons x xs ih =>
    cases ys with
    | nil => simp at hlen
    | cons y ys =>
      have ht : xs.length = ys.length := by simpa using hlen
      by_cases hxy : x = y
      · subst y
        simpa [List.zip, List.allM, core.cmp.PartialEqU8, liftFun2,
          core.cmp.impls.PartialEqU8.ne, lift] using ih ys ht
      · have hval : x.val ≠ y.val := by
          intro h
          apply hxy
          scalar_tac
        simp [List.zip, List.allM, core.cmp.PartialEqU8, liftFun2,
          core.cmp.impls.PartialEqU8.ne, lift, hxy, hval]
        rfl

@[step] theorem slice_bytes_eq_spec (left right : Aeneas.Std.Slice U8) :
    core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 left right
      ⦃ b => b = true ↔ left.val = right.val ⦄ := by
  unfold core.slice.cmp.PartialEqSlice.eq
  split
  · rename_i hlen
    rw [WP.spec_equiv_exists]
    refine ⟨decide (left.val = right.val), ?_, by simp⟩
    exact byte_list_eq left.val right.val hlen
  · rename_i hlen
    simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
    intro heq
    exact hlen (by simp only [Slice.length, heq])

@[step] theorem bytes_equal_spec (left right : Aeneas.Std.Slice U8) :
    bytes_equal left right ⦃ b => b = true ↔ left.val = right.val ⦄ := by
  unfold bytes_equal
  step*
  have hl : left.val.length = 1 := by scalar_tac
  have hr : right.val.length = 1 := by scalar_tac
  obtain ⟨x, hx⟩ := List.length_eq_one_iff.mp hl
  obtain ⟨y, hy⟩ := List.length_eq_one_iff.mp hr
  simp_all

@[step] theorem contains_kind_loop_spec (values : Aeneas.Std.Slice U16) (kind : U16)
    (i : Usize) (hi : i.val ≤ values.val.length) :
    contains_kind_loop values kind i ⦃ b => b = true ↔
      PrefixAny values.val (fun x => x = kind) i.val ⦄ := by
  unfold contains_kind_loop
  apply descending_scan (P := PrefixAny values.val (fun x => x = kind))
  intro k hk
  unfold contains_kind_loop.body
  step*
  · simp only [true_iff, PrefixAny]
    exact ⟨i1.val, by omega, by omega, by grind⟩
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < values.val.length := by omega
    rw [heq, prefix_any_step values.val _ i1.val hbound]
    clear heq
    simp_all
    omega
  · have hz : k.val = 0 := by scalar_tac
    simp [PrefixAny, hz]

@[step] theorem contains_kind_spec (values : Aeneas.Std.Slice U16) (kind : U16) :
    contains_kind values kind ⦃ b => b = true ↔ kind ∈ values.val ⦄ := by
  unfold contains_kind
  step*
  simpa only [Slice.len_val, prefix_any_full, exists_eq_right] using b_post

@[step] theorem contains_id_loop_spec (values : Aeneas.Std.Slice (Array U8 32#usize))
    (id : Array U8 32#usize) (i : Usize) (hi : i.val ≤ values.val.length) :
    contains_id_loop values id i ⦃ b => b = true ↔
      PrefixAny values.val (fun x => x.val = id.val) i.val ⦄ := by
  unfold contains_id_loop
  apply descending_scan (P := PrefixAny values.val (fun x => x.val = id.val))
  intro k hk
  unfold contains_id_loop.body
  step*
  · simp only [true_iff, PrefixAny]
    refine ⟨i1.val, by omega, by omega, ?_⟩
    simp_all [alloc.vec.Vec.deref, Array.to_slice]
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < values.val.length := by omega
    rw [heq, prefix_any_step values.val _ i1.val hbound]
    clear heq
    simp_all [alloc.vec.Vec.deref, Array.to_slice]
    omega
  · have hz : k.val = 0 := by scalar_tac
    simp [PrefixAny, hz]


@[step] theorem contains_id_spec (values : Aeneas.Std.Slice (Array U8 32#usize))
    (id : Array U8 32#usize) :
    contains_id values id ⦃ b => b = true ↔ id.val ∈ values.val.map (·.val) ⦄ := by
  unfold contains_id
  step*
  simpa [Slice.len_val] using b_post

@[step] theorem contains_value_loop_spec (values : Aeneas.Std.Slice (alloc.vec.Vec U8))
    (value : Aeneas.Std.Slice U8) (i : Usize) (hi : i.val ≤ values.val.length)
    (hfirst : ∀ h0 : 0 < values.val.length, (values.val[0]'h0).val ≠ value.val) :
    contains_value_loop values value i ⦃ b => b = true ↔
      PrefixAny values.val (fun x => x.val = value.val) i.val ⦄ := by
  unfold contains_value_loop
  apply descending_scan (P := PrefixAny values.val (fun x => x.val = value.val))
  intro k hk
  unfold contains_value_loop.body
  step*
  · simp only [true_iff, PrefixAny]
    refine ⟨i1.val, by omega, by omega, ?_⟩
    simp_all [alloc.vec.Vec.deref, Array.to_slice]
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < values.val.length := by omega
    rw [heq, prefix_any_step values.val _ i1.val hbound]
    clear heq
    simp_all [alloc.vec.Vec.deref, Array.to_slice]
    omega
  · have hz : k.val ≤ 1 := by scalar_tac
    simp only [PrefixAny, Bool.false_eq_true, false_iff]
    rintro ⟨j, hj, hjk, hp⟩
    have hj0 : j = 0 := by omega
    subst j
    exact hfirst hj hp


@[step] theorem contains_value_spec (values : Aeneas.Std.Slice (alloc.vec.Vec U8))
    (value : Aeneas.Std.Slice U8) :
    contains_value values value ⦃ b => b = true ↔ value.val ∈ values.val.map (·.val) ⦄ := by
  unfold contains_value
  step*
  · have hz : values.val = [] := by
      simpa using (show values.val.length = 0 by scalar_tac)
    simp [hz]
  · simp only [true_iff, List.mem_map]
    refine ⟨values.val[0]'(by scalar_tac), List.getElem_mem (by scalar_tac), ?_⟩
    simp_all [alloc.vec.Vec.deref]
  · intro h0
    simp_all [alloc.vec.Vec.deref]
  · simpa [Slice.len_val] using b_post


@[step] theorem is_h_spec (key : Aeneas.Std.Slice U8) :
    is_h key ⦃ b => b = true ↔ key.val = [104#u8] ⦄ := by
  unfold is_h
  step*
  have hlen : key.val.length = 1 := by scalar_tac
  obtain ⟨a, ha⟩ := List.length_eq_one_iff.mp hlen
  simp_all [Array.make]

@[step] theorem tag_matches_spec (query : input.TagQuery) (tag : input.EventTag) :
    tag_matches query tag ⦃ b => b = true ↔
      Tag tag query.key.val (query.values.val.map (·.val)) ⦄ := by
  unfold tag_matches
  step*
  all_goals simp_all [Tag, alloc.vec.Vec.deref]

@[step] theorem explicit_match_loop_spec (query : input.TagQuery)
    (v : alloc.vec.Vec input.EventTag) (i : Usize) (hi : i.val ≤ v.val.length)
    (hfirst : ∀ h0 : 0 < v.val.length,
      ¬ Tag (v.val[0]'h0) query.key.val (query.values.val.map (·.val))) :
    explicit_match_loop query v i ⦃ b => b = true ↔
      PrefixAny v.val (fun tag => Tag tag query.key.val (query.values.val.map (·.val))) i.val ⦄ := by
  unfold explicit_match_loop
  apply descending_scan (P := PrefixAny v.val
    (fun tag => Tag tag query.key.val (query.values.val.map (·.val))))
  intro k hk
  unfold explicit_match_loop.body
  step*
  · simp only [true_iff, PrefixAny]
    refine ⟨i1.val, by omega, by omega, ?_⟩
    simp_all
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < v.val.length := by omega
    rw [heq, prefix_any_step v.val _ i1.val hbound]
    clear heq
    simp_all
    omega
  · have hz : k.val ≤ 1 := by scalar_tac
    simp only [PrefixAny, Bool.false_eq_true, false_iff]
    rintro ⟨j, hj, hjk, hp⟩
    have hj0 : j = 0 := by omega
    subst j
    exact hfirst hj hp

@[step] theorem explicit_match_spec (query : input.TagQuery) (event : input.EventInput) :
    explicit_match query event ⦃ b => b = true ↔
      ∃ tag ∈ event.tags.val, Tag tag query.key.val (query.values.val.map (·.val)) ⦄ := by
  unfold explicit_match
  step*
  · have hz : event.tags.val = [] := by
      simpa using (show event.tags.val.length = 0 by scalar_tac)
    simp [hz]
  · simp only [true_iff]
    refine ⟨event.tags.val[0]'(by scalar_tac), List.getElem_mem (by scalar_tac), ?_⟩
    simp_all
  · intro h0
    simp_all
  · simpa using b_post

@[step] theorem has_h_loop_spec (v : alloc.vec.Vec input.EventTag)
    (i : Usize) (hi : i.val ≤ v.val.length) :
    has_h_loop v i ⦃ b => b = true ↔
      PrefixAny v.val (fun tag => tag.key.val = [104#u8]) i.val ⦄ := by
  unfold has_h_loop
  apply descending_scan (P := PrefixAny v.val (fun tag => tag.key.val = [104#u8]))
  intro k hk
  unfold has_h_loop.body
  step*
  · simp only [true_iff, PrefixAny]
    refine ⟨i1.val, by omega, by omega, ?_⟩
    simp_all [alloc.vec.Vec.deref]
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < v.val.length := by omega
    rw [heq, prefix_any_step v.val _ i1.val hbound]
    clear heq
    simp_all [alloc.vec.Vec.deref]
    omega
  · have hz : k.val = 0 := by scalar_tac
    simp [PrefixAny, hz]

@[step] theorem has_h_spec (event : input.EventInput) :
    has_h event ⦃ b => b = true ↔ ∃ tag ∈ event.tags.val, tag.key.val = [104#u8] ⦄ := by
  unfold has_h
  step*
  simpa using b_post

@[step] theorem tag_constraint_impl_spec (query : input.TagQuery) (event : input.EventInput) :
    solution.tag_constraint query event ⦃ b => b = true ↔ Constraint query event ⦄ := by
  unfold solution.tag_constraint
  step*
  all_goals simp_all [Constraint, alloc.vec.Vec.deref]


@[step] theorem tags_match_loop_spec (v : alloc.vec.Vec input.TagQuery)
    (event : input.EventInput) (i : Usize) (hi : i.val ≤ v.val.length) :
    tags_match_loop v event i ⦃ b => b = true ↔
      PrefixAll v.val (fun query => Constraint query event) i.val ⦄ := by
  unfold tags_match_loop
  apply descending_scan (P := PrefixAll v.val (fun query => Constraint query event))
  intro k hk
  unfold tags_match_loop.body
  step*
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < v.val.length := by omega
    rw [heq, prefix_all_step v.val _ i1.val hbound]
    clear heq
    simp_all
    omega
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < v.val.length := by omega
    rw [heq, prefix_all_step v.val _ i1.val hbound]
    clear heq
    simp_all
  · have hz : k.val = 0 := by scalar_tac
    simp [PrefixAll, hz]

@[step] theorem tags_match_spec (filter : input.FilterInput) (event : input.EventInput) :
    tags_match filter event ⦃ b => b = true ↔ ∀ query ∈ filter.tags.val, Constraint query event ⦄ := by
  unfold tags_match
  step*
  simpa using b_post

@[step] theorem one_filter_impl_spec (filter : input.FilterInput) (event : input.EventInput) :
    solution.one_filter filter event ⦃ b => b = true ↔ One filter event ⦄ := by
  unfold solution.one_filter
  step*
  all_goals simp_all [One, Ids, Kinds, Since, Until, alloc.vec.Vec.deref, UScalar.lt_equiv]

@[step] theorem filters_match_loop_spec (filters : Aeneas.Std.Slice input.FilterInput)
    (event : input.EventInput) (i : Usize) (hi : i.val ≤ filters.val.length) :
    filters_match_loop filters event i ⦃ b => b = true ↔
      PrefixAny filters.val (fun filter => One filter event) i.val ⦄ := by
  unfold filters_match_loop
  apply descending_scan (P := PrefixAny filters.val (fun filter => One filter event))
  intro k hk
  unfold filters_match_loop.body
  step*
  · simp only [true_iff, PrefixAny]
    refine ⟨i1.val, by omega, by omega, ?_⟩
    simp_all
  · have heq : k.val = i1.val + 1 := by omega
    have hbound : i1.val < filters.val.length := by omega
    rw [heq, prefix_any_step filters.val _ i1.val hbound]
    clear heq
    simp_all
    omega
  · have hz : k.val = 0 := by scalar_tac
    simp [PrefixAny, hz]

theorem filters_match.impl_correct :
    ∀ (filters : Aeneas.Std.Slice input.FilterInput) (event : input.EventInput),
      filters_match_pre filters.val event →
      RelayFilter.spec.filters_match filters event ⦃ r => filters_match_spec filters.val event r ⦄ := by
  intro filters event _
  unfold RelayFilter.spec.filters_match solution.filters_match
  apply WP.spec_mono (filters_match_loop_spec filters event (Slice.len filters) (by simp))
  intro r hr
  apply (filters_eq _ _ _).mpr
  simpa only [Slice.len_val, prefix_any_full] using hr

end RelayFilter.Proofs
