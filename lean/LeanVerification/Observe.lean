/-
The `Observes` relation between blocks in the blocklace.

This formalizes the observation/reachability layer of Cordial Miners.

Rust correspondence:
  - `Blocklace::predecessors`
  - `Blocklace::ancestors`
  - `Blocklace::observe`
  - `Blocklace::ancestors_inclusive`
  - `Blocklace::precedes`
  - `Blocklace::precedes_or_equals`
-/

import LeanVerification.Blocklace
import Mathlib.Logic.Relation
import Mathlib.Data.Finset.Basic

namespace CordialMiners

open Relation Finmap

/--
`DirectPred B b p` means that `p` is a direct predecessor of `b`
in blocklace `B`.
  Rust: `Blocklace::predecessors`
  Paper: `{a | a ← b}`
-/
def DirectPred (B : Blocklace) (b p : BlockId) : Prop :=
  ∃ blk : Block,
    B.lookup b = some blk ∧
    p ∈ blk.content.predecessors

/--
`Observes B a b` means that `a` can observe `b` by following
zero or more predecessor links.
-/
def Observes (B : Blocklace) : BlockId → BlockId → Prop :=
  ReflTransGen (DirectPred B)

/- every block observes itself (reflexivity) -/
theorem observes_refl (B : Blocklace) (a : BlockId) : Observes B a a :=
  ReflTransGen.refl

/- a block observes its direct predecessors in one/single step -/
theorem observes_step
    (B : Blocklace) (b p : BlockId) (h : DirectPred B b p) :
    Observes B b p :=
  ReflTransGen.single h

/- if a observes b, and b observes c then a observes c -/
theorem observes_trans
    (B : Blocklace) {a b c : BlockId}
    (hab : Observes B a b) (hbc : Observes B b c) :
    Observes B a c :=
  ReflTransGen.trans hab hbc

/-! ### Constructive Blocklace induction -/

/-- The empty blocklace. -/
def emptyBlocklace : Blocklace := ⟨0, Multiset.nodup_zero⟩

/-Inductive characterization of a validly constructed blocklace:
either empty, or built by inserting an `Insertable` (closure-respecting),
fresh block into a valid one.

A valid blocklace starts empty and can only be built
by repeatedly inserting a valid, closure-respecting, new block.-/
inductive ValidBlocklace : Blocklace → Prop where
  | empty : ValidBlocklace emptyBlocklace
  | insert (B : Blocklace) (blk : Block)
      (hValid : ValidBlocklace B)
      (hInsertable : Insertable B blk)
      (hNew : blk.id ∉ B.keys) :
      ValidBlocklace (blocklaceInsert B blk)

/-- Every validly constructed blocklace satisfies `Closed`. -/
theorem closed_of_valid (B : Blocklace) (h : ValidBlocklace B) : Closed B := by
  induction h with
  | empty =>
    intro id blk hlookup
    have h : (emptyBlocklace : Blocklace).lookup id = none := by
      dsimp [emptyBlocklace, Finmap.lookup]; rfl
    simp [h] at hlookup
  | insert B blk hValid hInsertable hNew ih =>
    exact insertPreservesClosed B blk ih hInsertable
/-! ### Edge-agreement lemmas between `B` and `blocklaceInsert B blk` -/

/-- Proves that inserting a new block blk does not alter or add any predecessor edges between
existing blocks $x$ and $y$. Used in directPred_acc to transfer accessibility of
existing blocks from $B$ to the new blocklace. -/
theorem directPred_insert_of_ne
    (B : Blocklace) (blk : Block) {x : BlockId} (hx : x ≠ blk.id) (y : BlockId) :
    DirectPred (blocklaceInsert B blk) x y ↔ DirectPred B x y := by
  constructor
  · rintro ⟨b, hb, hp⟩
    have hb' : lookup x (blocklaceInsert B blk) = some b := hb
    dsimp [blocklaceInsert] at hb'
    rw [lookup_insert_of_ne B hx] at hb'
    exact ⟨b, hb', hp⟩
  · rintro ⟨b, hb, hp⟩
    refine ⟨b, ?_, hp⟩
    dsimp [blocklaceInsert]
    rw [lookup_insert_of_ne B hx]
    exact hb

/-- Isolates the predecessor edges coming out of the newly inserted block blk.id.
Used in directPred_acc to prove that blk.id itself is accessible because all
of its predecessors $y$ are already accessible in $B$. -/
theorem directPred_insert_self
    (B : Blocklace) (blk : Block) (y : BlockId) :
    DirectPred (blocklaceInsert B blk) blk.id y ↔ y ∈ blk.content.predecessors := by
  constructor
  · rintro ⟨b, hb, hp⟩
    have hb' : lookup blk.id (blocklaceInsert B blk) = some b := hb
    dsimp [blocklaceInsert] at hb'
    rw [lookup_insert B] at hb'
    cases hb'
    exact hp
  · intro hp
    refine ⟨blk, ?_, hp⟩
    dsimp [blocklaceInsert]
    rw [lookup_insert B]

/-! ### Well-foundedness, proved structurally -/

/--
Every id is accessible under the "has a direct predecessor" relation,
for any validly constructed blocklace.

Proof strategy: induction on the `ValidBlocklace` derivation.
- Empty blocklace: trivially accessible (no blocks, no edges).
- Insert case: predecessors of the fresh block are already in `B`
  (`hInsertable`) and hence ≠ `blk.id` (`hNew`), so they inherit
  accessibility from the smaller blocklace `B` via `ih`. Everything
  else behaves exactly as in `B`, transferred through the closure
  invariant (every predecessor stays inside `B.keys`, hence ≠ `blk.id`).
-/
theorem directPred_acc
    (B : Blocklace) (hV : ValidBlocklace B) :
    ∀ id, Acc (fun a b => DirectPred B b a) id := by
  induction hV with
  | empty =>
    intro id
    refine Acc.intro id ?_
    intro p hp
    obtain ⟨blk, hblk, _⟩ := hp
    dsimp [emptyBlocklace, lookup] at hblk
    cases hblk
  | insert B blk hValid hInsertable hNew ih =>
    have hClosed : Closed B := closed_of_valid B hValid
    have transfer : ∀ x, Acc (fun a b => DirectPred B b a) x → x ≠ blk.id →
        Acc (fun a b => DirectPred (blocklaceInsert B blk) b a) x := by
      intro x hacc
      induction hacc with
      | intro x _ ihx =>
        intro hxne
        refine Acc.intro x ?_
        intro y hy
        have hy' : DirectPred B x y := (directPred_insert_of_ne B blk hxne y).mp hy
        obtain ⟨bblk, hbblk, hpy⟩ := hy'
        have hyKeys : y ∈ B.keys := hClosed x bblk hbblk y hpy
        have hyNe : y ≠ blk.id := fun h => hNew (h ▸ hyKeys)
        exact ihx y ⟨bblk, hbblk, hpy⟩ hyNe
    intro id
    by_cases hid : id = blk.id
    · subst hid
      refine Acc.intro blk.id ?_
      intro y hy
      have hy' : y ∈ blk.content.predecessors := (directPred_insert_self B blk y).mp hy
      have hyKeys : y ∈ B.keys := hInsertable y hy'
      have hyNe : y ≠ blk.id := fun h => hNew (h ▸ hyKeys)
      exact transfer y (ih y) hyNe
    · exact transfer id (ih id) hid

/-- Every validly constructed blocklace has a well-founded direct-predecessor relation. -/
theorem directPred_wf_of_valid (B : Blocklace) (h : ValidBlocklace B) :
    WellFounded (fun a b => DirectPred B b a) :=
  ⟨directPred_acc B h⟩

/-! ### Generic well-founded-relation facts -/

/-- A well-founded relation is irreflexive.
Provides the contradiction step for acyclicity:
if a path could lead from $a$ back to $a$, it would violate irreflexivity. -/
theorem wf_irrefl {α} {r : α → α → Prop} (hwf : WellFounded r) : ∀ a, ¬ r a a := by
  intro a
  have hacc := hwf.apply a
  induction hacc with
  | intro a _ ih =>
    intro h
    exact ih a h h

/-- `TransGen` commutes with flipping the underlying relation and swapping endpoints. -/
theorem transGen_flip_iff {α} {r : α → α → Prop} {a b : α} :
    TransGen r a b ↔ TransGen (fun x y => r y x) b a := by
  constructor
  · intro h
    induction h with
    | single hr => exact TransGen.single hr
    | tail _ hr ih => exact TransGen.trans (TransGen.single hr) ih
  · intro h
    induction h with
    | single hr => exact TransGen.single hr
    | tail _ hr ih => exact TransGen.trans (TransGen.single hr) ih

/-- No block can observe itself through a nontrivial chain of predecessors. -/
theorem directPred_acyclic
    (B : Blocklace) (hV : ValidBlocklace B)
    {a : BlockId} (hcycle : TransGen (DirectPred B) a a) : False := by
  have hwf : WellFounded (fun a b => DirectPred B b a) := directPred_wf_of_valid B hV
  have hwf' : WellFounded (TransGen (fun a b => DirectPred B b a)) := hwf.transGen
  have hflip : TransGen (fun x y => DirectPred B y x) a a := transGen_flip_iff.mp hcycle
  exact wf_irrefl hwf' a hflip

/-! ### `Observes` is a partial order -/

/-Antisymmetry of `Observes` on a closed blocklace.
If `a` observes `b` and `b` observes `a`, then `a = b`.-/
theorem observes_antisymm
    (B : Blocklace) (hV : ValidBlocklace B)
    {a b : BlockId}
    (hab : Observes B a b) (hba : Observes B b a) :
    a = b := by
  by_contra hne
  rcases ReflTransGen.cases_head hab with hEq | ⟨c, hac, hcb⟩
  · exact hne hEq
  · have hca : Observes B c a := ReflTransGen.trans hcb hba
    have hcycle : TransGen (DirectPred B) a a := TransGen.head' hac hca
    exact directPred_acyclic B hV hcycle

/-`Observes` is a partial order on a closed blocklace.-/
theorem observes_partialOrder
    (B : Blocklace) (hV : ValidBlocklace B) :
    (∀ a, Observes B a a) ∧
    (∀ a b c, Observes B a b → Observes B b c → Observes B a c) ∧
    (∀ a b, Observes B a b → Observes B b a → a = b) :=
  ⟨ observes_refl B,
    fun _ _ _ hab hbc => observes_trans B hab hbc,
    fun _ _ hab hba => observes_antisymm B hV hab hba ⟩

/-If `B` is extended to `B'` without changing the existing blocks,
anything observable in `B` remains observable in `B'`.
This is the monotonicity of the observation cone.-/
theorem observes_mono
    (B B' : Blocklace)
    (hSub : ∀ id blk, B.lookup id = some blk → B'.lookup id = some blk)
    {a b : BlockId}
    (h : Observes B a b) :
    Observes B' a b :=
  ReflTransGen.lift (p := DirectPred B') id
    (fun x y (hstep : DirectPred B x y) =>
      let ⟨blk, hblk, hpred⟩ := hstep
      (⟨blk, hSub x blk hblk, hpred⟩ : DirectPred B' x y))
    a b h

/-! ### Computable observeSet via well-founded recursion -/

/--
Computable observation set, defined by well-founded recursion on the
"has direct predecessor" relation. Terminates because `hwf` (derived
from `ValidBlocklace`) guarantees no infinite predecessor chains.
-/
def observeSetWF
    (B : Blocklace) (hwf : WellFounded (fun a b => DirectPred B b a)) :
    BlockId → Finset BlockId :=
  hwf.fix (fun x rec =>
    match hB : B.lookup x with
    | none => {x}
    | some blk =>
        insert x (blk.content.predecessors.attach.biUnion
          (fun p => rec p.1 ⟨blk, hB, p.2⟩)))

/-- Unfolding equation for `observeSetWF`. -/
theorem observeSetWF_eq
    (B : Blocklace) (hwf : WellFounded (fun a b => DirectPred B b a)) (x : BlockId) :
    observeSetWF B hwf x =
      match _hB : B.lookup x with
      | none => {x}
      | some blk =>
          insert x (blk.content.predecessors.attach.biUnion
            (fun p => observeSetWF B hwf p.1)) := by
  unfold observeSetWF
  rw [WellFounded.fix_eq]

/--
The executable `observeSet`, requiring proof `B` is validly constructed.
-/
def observeSet (B : Blocklace) (hV : ValidBlocklace B) (a : BlockId) : Finset BlockId :=
  observeSetWF B (directPred_wf_of_valid B hV) a

/-- Every element of `observeSet` is observed by `a`. -/
theorem observeSet_sound
    (B : Blocklace) (hV : ValidBlocklace B) (a : BlockId) :
    ∀ p, p ∈ observeSet B hV a → Observes B a p := by
  have hwf := directPred_wf_of_valid B hV
  have hacc := hwf.apply a
  induction hacc with
  | intro a _ ih =>
    intro p hp
    rw [observeSet, observeSetWF_eq] at hp
    split at hp
    · rw [Finset.mem_singleton] at hp; subst hp; exact ReflTransGen.refl
    · rename_i blk hB
      rw [Finset.mem_insert] at hp
      rcases hp with rfl | hp
      · exact ReflTransGen.refl
      · rw [Finset.mem_biUnion] at hp
        obtain ⟨q, _, hpmem⟩ := hp
        have hstep : DirectPred B a q.1 := ⟨blk, hB, q.2⟩
        have hq_obs : Observes B q.1 p := ih q.1 hstep p hpmem
        exact ReflTransGen.head hstep hq_obs

/-- Every block observed by `a` and present in `B` belongs to `observeSet`. -/
theorem observeSet_complete
    (B : Blocklace) (hV : ValidBlocklace B) (a p : BlockId)
    (_hp : p ∈ B.keys) (h : Observes B a p) :
    p ∈ observeSet B hV a := by
  induction h using ReflTransGen.head_induction_on with
  | refl =>
    rw [observeSet, observeSetWF_eq]
    cases hB : B.lookup p with
    | none => exact Finset.mem_singleton_self p
    | some blk => rw [Finset.mem_insert]; exact Or.inl rfl
  | head hstep _ ih =>
    rw [observeSet, observeSetWF_eq]
    obtain ⟨blk, hB, hmem⟩ := hstep
    rw [hB, Finset.mem_insert]
    exact Or.inr (Finset.mem_biUnion.mpr ⟨⟨_, hmem⟩, Finset.mem_attach _ _, ih⟩)

/-- `observeSet` computes exactly the observation relation for blocks in `B`. -/
theorem observeSet_equiv
    (B : Blocklace) (hV : ValidBlocklace B) (a p : BlockId) (hp : p ∈ B.keys) :
    p ∈ observeSet B hV a ↔ Observes B a p :=
  ⟨observeSet_sound B hV a p, observeSet_complete B hV a p hp⟩

/-! ### `Precedes` / `PrecedesOrEquals` -/

/-- `Precedes` — strict observation: `a` is reachable from `b` via ≥1 edge. -/
def Precedes (B : Blocklace) (a b : BlockId) : Prop :=
  Relation.TransGen (DirectPred B) b a

/-`PrecedesOrEquals` corresponds to Rust
`Blocklace::precedes_or_equals`. This is exactly the `Observes` relation.-/
def PrecedesOrEquals (B : Blocklace) (a b : BlockId) : Prop :=
  Observes B b a

/-Every strict predecessor is observed.-/
theorem precedes_implies_observes
    (B : Blocklace) {a b : BlockId} (h : Precedes B a b) :
    Observes B b a :=
  h.to_reflTransGen

end CordialMiners
