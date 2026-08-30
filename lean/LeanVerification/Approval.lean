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

/-- Strict two-thirds weighted supermajority of `validators` held by `S`:
  `3 * bondOf bonds S > 2 * bondOf bonds validators`.
Cross-multiplication avoids division and matches `strict_two_thirds` in
`consensus/cordiality.rs`. -/
def StrictTwoThirdsMaj (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (S : Finset NodeId) : Prop :=
  3 * bondOf bonds S > 2 * bondOf bonds validators

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
