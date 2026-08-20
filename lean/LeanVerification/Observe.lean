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
The relation is reflexive-transitive closure of `DirectPred`.
-/
def Observes (B : Blocklace) : BlockId → BlockId → Prop :=
  ReflTransGen (DirectPred B)

/-- Every block observes itself. -/
theorem observes_refl
    (B : Blocklace)
    (a : BlockId) :
    Observes B a a :=
  ReflTransGen.refl

/-- A block directly observes each of its direct predecessors. -/
theorem observes_step
    (B : Blocklace)
    (b p : BlockId)
    (h : DirectPred B b p) :
    Observes B b p :=
  ReflTransGen.single h

/-- Observation is transitive. -/
theorem observes_trans
    (B : Blocklace)
    {a b c : BlockId}
    (hab : Observes B a b)
    (hbc : Observes B b c) :
    Observes B a c :=
  ReflTransGen.trans hab hbc

/--
Well-foundedness of the direct-predecessor relation.
-/
axiom directPred_wf
    (B : Blocklace)
    (hClosed : Closed B) :
    WellFounded (DirectPred B)

/--
A well-founded direct-predecessor relation cannot contain a non-trivial
cycle.
-/
theorem directPred_acyclic
    (B : Blocklace)
    (hClosed : Closed B)
    {a : BlockId}
    (hcycle : TransGen (DirectPred B) a a) :
    False :=
    (directPred_wf B hClosed).transGen.irrefl.irrefl a hcycle

/--
Antisymmetry of `Observes` on a closed blocklace.
If `a` observes `b` and `b` observes `a`, then `a = b`.
-/
theorem observes_antisymm
    (B : Blocklace)
    (hClosed : Closed B)
    {a b : BlockId}
    (hab : Observes B a b)
    (hba : Observes B b a) :
    a = b := by
  by_contra hne
  rcases ReflTransGen.cases_head hab with hEq | ⟨c, hac, hcb⟩
  · exact hne hEq
  · have hca : Observes B c a :=
      ReflTransGen.trans hcb hba
    have hcycle :
        TransGen (DirectPred B) a a :=
      TransGen.head' hac hca
    exact directPred_acyclic B hClosed hcycle

/--
`Observes` is a partial order on a closed blocklace.
-/
theorem observes_partialOrder
    (B : Blocklace)
    (hClosed : Closed B) :
    (∀ a, Observes B a a) ∧
    (∀ a b c,
      Observes B a b →
      Observes B b c →
      Observes B a c) ∧
    (∀ a b,
      Observes B a b →
      Observes B b a →
      a = b) :=
  ⟨
    observes_refl B,
    fun _ _ _ hab hbc => observes_trans B hab hbc,
    fun _ _ hab hba => observes_antisymm B hClosed hab hba
  ⟩

/--
If `B` is extended to `B'` without changing the existing blocks,
anything observable in `B` remains observable in `B'`.
This is the monotonicity of the observation cone.
-/
theorem observes_mono
    (B B' : Blocklace)
    (hSub :
      ∀ id blk,
        B.lookup id = some blk →
        B'.lookup id = some blk)
    {a b : BlockId}
    (h : Observes B a b) :
    Observes B' a b :=
  ReflTransGen.lift (p := DirectPred B') id
    (fun x y (hstep : DirectPred B x y) =>
      let ⟨blk, hblk, hpred⟩ := hstep
      (⟨blk, hSub x blk hblk, hpred⟩ : DirectPred B' x y))
    a b h

/--
The set of all block IDs in `B` that are observable from `a`.
This definition is the finite-set view of the observation relation.
-/
noncomputable def observeSet
    (B : Blocklace)
    (a : BlockId) :
    Finset BlockId :=
  letI : DecidablePred (Observes B a) := fun _ => Classical.propDecidable _
  B.keys.filter (Observes B a)


/-- Every element of `observeSet` is observed by `a`. -/
theorem observeSet_sound
    (B : Blocklace)
    (a p : BlockId)
    (h : p ∈ observeSet B a) :
    Observes B a p := by
  simp only [observeSet, Finset.mem_filter] at h
  exact h.2

/--
Every block in the blocklace observed by `a` belongs to `observeSet`.
-/
theorem observeSet_complete
    (B : Blocklace)
    (a p : BlockId)
    (hp : p ∈ B.keys)
    (h : Observes B a p) :
    p ∈ observeSet B a := by
  simp only [observeSet, Finset.mem_filter]
  exact ⟨hp, h⟩

/--
For blocks belonging to the blocklace, membership in `observeSet`
is equivalent to the `Observes` relation.
-/
theorem observeSet_equiv
    (B : Blocklace)
    (_hClosed : Closed B)
    (a p : BlockId)
    (hp : p ∈ B.keys) :
    p ∈ observeSet B a ↔ Observes B a p := by
  simp [observeSet, hp]

/--
`Precedes` corresponds to Rust `Blocklace::precedes`.
It is the strict version of observation: `a` precedes `b` when
`a` is reachable from `b` through at least one predecessor edge.
-/
def Precedes
    (B : Blocklace)
    (a b : BlockId) :
    Prop :=
  Relation.TransGen (DirectPred B) b a

/--
`PrecedesOrEquals` corresponds to Rust
`Blocklace::precedes_or_equals`.
This is exactly the `Observes` relation.
-/
def PrecedesOrEquals
    (B : Blocklace)
    (a b : BlockId) :
    Prop :=
  Observes B b a

/--
Every strict predecessor is observed.
-/
theorem precedes_implies_observes
    (B : Blocklace)
    {a b : BlockId}
    (h : Precedes B a b) :
    Observes B b a :=
  h.to_reflTransGen

end CordialMiners
