/-
Approval mechanics: how a block accumulates validator approvals toward
finality via ratification and super-ratification.

## Design

`Approves` is the same predicate as `VouchesFor` from `Equivocation.lean`.
The Rust `approves` function in `consensus/approval.rs` (Definition 18 of
arXiv:2205.09174) checks exactly that: a block observes the target and has
no observed incomparable same-creator competitor. Naming it separately
here lets `Finality.lean` and `Ordering.lean` import the approval concept
without depending on `Equivocation.lean`'s internals.

`Ratifies` and `SuperRatifies` are the two-level weighted-supermajority
chain of Definition 22–24 in the paper, implemented in Rust as
`weighted_ratifies` / `weighted_super_ratifies` in `cordiality.rs`.

## Why not `OpenTheta` from `Weights.lean`?

`OpenTheta` requires `[Fintype Validator]`. `NodeId = Nat` is infinite,
so `Fintype NodeId` does not hold. We therefore carry an explicit finite
universe `validators : Finset NodeId` and express the 2/3 threshold via
`Finset.sum` directly — the arithmetic is identical to `OpenTheta w 2 3`
for a `Fintype` type.

Owned by Issue 04 (KR4 — Finalized Leader Safety).
-/
import LeanVerification.Equivocation
import LeanVerification.Weights

open scoped BigOperators

namespace CordialMiners

/-! ### Approval -/

/-- A block `approver` approves a `target` block when it observes the
target and does not also observe any incomparable block by the same
creator. Definitionally equal to `VouchesFor` from `Equivocation.lean`.

Rust: `consensus/approval.rs:approves` (Definition 18, arXiv:2205.09174). -/
def Approves (B : Blocklace) (approver target : BlockId) : Prop :=
  VouchesFor B approver target

/-! ### Bond-weighted supermajority -/

/-- Aggregate bond weight of a set of validators.
`Finset.sum` over `bonds` — no `Fintype` instance required. -/
abbrev bondOf (bonds : NodeId → ℕ) (S : Finset NodeId) : ℕ :=
  ∑ v ∈ S, bonds v

/-- `validators` contains every node with positive bond weight.
Without this, a caller could hide bonded validators from the universe,
inflating any subset's apparent share. Rust always passes the full
validator set from the epoch config.  -/
def ValidBonds (bonds : NodeId → ℕ) (validators : Finset NodeId) : Prop :=
  ∀ v, 0 < bonds v → v ∈ validators

/-- Strict two-thirds weighted supermajority of `validators` held by `S`:
  `3 * bondOf bonds S > 2 * bondOf bonds validators`.
Cross-multiplication avoids division and matches `strict_two_thirds` in
`consensus/cordiality.rs`. -/
def StrictTwoThirdsMaj (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (S : Finset NodeId) : Prop :=
  3 * bondOf bonds S > 2 * bondOf bonds validators

/-! ### Quorum-intersection helpers (Finset-based) -/

/-- A non-empty Finset with positive total bond weight. -/
lemma bondOf_pos_nonempty (bonds : NodeId → ℕ) (S : Finset NodeId)
    (h : 0 < bondOf bonds S) : S.Nonempty := by
  by_contra hemp
  simp [Finset.not_nonempty_iff_eq_empty.mp hemp, bondOf] at h

/-- Bond weight is monotone: a subset weighs at most the superset. -/
lemma bondOf_mono (bonds : NodeId → ℕ) {A B : Finset NodeId} (h : A ⊆ B) :
    bondOf bonds A ≤ bondOf bonds B :=
  Finset.sum_le_sum_of_subset h

/-- Inclusion-exclusion for bond weights. -/
lemma bondOf_union_inter (bonds : NodeId → ℕ) (A B : Finset NodeId) :
    bondOf bonds (A ∪ B) + bondOf bonds (A ∩ B) = bondOf bonds A + bondOf bonds B :=
  Finset.sum_union_inter

/-- If `H`, `A`, and `B` each hold a strict two-thirds majority of
`validators`, their triple intersection is non-empty.
This is the Finset-based analog of `Weights.honest_triple_intersection`. -/
theorem finset_honest_triple_intersection
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (H A B : Finset NodeId)
    (hH : H ⊆ validators) (hA : A ⊆ validators) (hB : B ⊆ validators)
    (hHmaj : StrictTwoThirdsMaj bonds validators H)
    (hAmaj : StrictTwoThirdsMaj bonds validators A)
    (hBmaj : StrictTwoThirdsMaj bonds validators B) :
    (A ∩ B ∩ H).Nonempty := by
  -- Name the sums as local Nat variables so omega can reason about them.
  set u := bondOf bonds validators
  set a := bondOf bonds A
  set b := bondOf bonds B
  set h := bondOf bonds H
  set ab_union := bondOf bonds (A ∪ B)
  set ab_inter := bondOf bonds (A ∩ B)
  set abh_union := bondOf bonds (A ∩ B ∪ H)
  set abh_inter := bondOf bonds (A ∩ B ∩ H)
  -- Subset bounds
  have hu_A : a ≤ u := bondOf_mono bonds hA
  have hu_B : b ≤ u := bondOf_mono bonds hB
  have hu_H : h ≤ u := bondOf_mono bonds hH
  have hu_AB : ab_union ≤ u := bondOf_mono bonds (Finset.union_subset hA hB)
  have hu_ABH : abh_union ≤ u :=
    bondOf_mono bonds (Finset.union_subset (Finset.inter_subset_left.trans hA) hH)
  -- Inclusion-exclusion
  have hAB_ie : ab_union + ab_inter = a + b := bondOf_union_inter bonds A B
  have hABH_ie : abh_union + abh_inter = ab_inter + h := bondOf_union_inter bonds (A ∩ B) H
  -- Supermajority hypotheses (cross-multiplied, so no division)
  unfold StrictTwoThirdsMaj at hAmaj hBmaj hHmaj
  -- From the above, derive positivity by linear arithmetic.
  have hABH_pos : 0 < abh_inter := by omega
  exact bondOf_pos_nonempty bonds _ hABH_pos

/-! ### Ratification -/

/-- `r` ratifies `b` with respect to bond weights `bonds` over universe
`validators`: within `r`'s observation closure there exists a set of
validators `S` that (a) each have an approving block in that closure and
(b) collectively hold a strict two-thirds bond supermajority.

Mirrors `consensus/cordiality.rs:weighted_ratifies` (lines 252–303). -/
def Ratifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (r b : BlockId) : Prop :=
  ∃ S : Finset NodeId,
    S ⊆ validators ∧
    (∀ v ∈ S, ∃ a, Observes B r a ∧ creatorOf B a = some v ∧ Approves B a b) ∧
    StrictTwoThirdsMaj bonds validators S

/-- `witness` super-ratifies `b`: within the block set `witness` there is a
set of validators `R` that (a) each have a ratifying block in `witness` and
(b) collectively hold a strict two-thirds bond supermajority.

Mirrors `consensus/cordiality.rs:weighted_super_ratifies` (lines 311–340). -/
def SuperRatifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (witness : Finset BlockId) (b : BlockId) : Prop :=
  ∃ R : Finset NodeId,
    R ⊆ validators ∧
    (∀ v ∈ R, ∃ r ∈ witness, creatorOf B r = some v ∧ Ratifies bonds validators B r b) ∧
    StrictTwoThirdsMaj bonds validators R

/-! ### Key bridge lemmas -/

/-- `Approves` is definitionally `VouchesFor`, so the implication is the
identity function. This is the proof obligation that `Finality.lean` and
`Ordering.lean` plug into `equivocation_not_approved`. -/
theorem approves_implies_vouchesFor (B : Blocklace) (approver target : BlockId) :
    Approves B approver target → VouchesFor B approver target := id

/-- An acknowledging block cannot approve either branch of an equivocation.
Direct corollary of `Equivocation.lean:equivocation_not_approved`,
specialised to `Approves = VouchesFor`. -/
theorem approves_exclusion (B : Blocklace) (hV : ValidBlocklace B)
    (b₁ b₂ approver : BlockId)
    (heq : Equivocation B hV b₁ b₂)
    (hack : Acknowledges B approver b₁ b₂) :
    ¬ Approves B approver b₁ ∧ ¬ Approves B approver b₂ :=
  equivocation_not_approved B hV b₁ b₂ approver heq hack id id

/-- Ratification is monotone in the ratifier: if `d` observes `r` and
`r` ratifies `b`, then `d` ratifies `b` too. The witnessing approver
set for `r` is a valid approver set for `d` because `Observes` is
transitive (`observes_trans`). -/
theorem ratifies_mono (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (r d b : BlockId)
    (hdr : Observes B d r) (hr : Ratifies bonds validators B r b) :
    Ratifies bonds validators B d b := by
  obtain ⟨S, hSsub, hSwitn, hSmaj⟩ := hr
  exact ⟨S, hSsub,
    fun v hv => by
      obtain ⟨a, haobs, hacreator, haapprove⟩ := hSwitn v hv
      exact ⟨a, observes_trans B hdr haobs, hacreator, haapprove⟩,
    hSmaj⟩

/-- Super-ratification is monotone in the witness block set: if `witness ⊆ witness'`
and `witness` super-ratifies `b`, then `witness'` super-ratifies `b` too. -/
theorem superRatifies_mono (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (witness witness' : Finset BlockId) (b : BlockId)
    (hss : witness ⊆ witness') (hsr : SuperRatifies bonds validators B witness b) :
    SuperRatifies bonds validators B witness' b := by
  obtain ⟨R, hRsub, hRwitn, hRmaj⟩ := hsr
  exact ⟨R, hRsub,
    fun v hv => by
      obtain ⟨r, hrw, hrcreator, hrrat⟩ := hRwitn v hv
      exact ⟨r, hss hrw, hrcreator, hrrat⟩,
    hRmaj⟩

end CordialMiners
