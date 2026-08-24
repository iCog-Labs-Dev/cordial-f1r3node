import LeanVerification.Observe
import Mathlib.Order.Antichain
import Mathlib.Order.Preorder.Chain
import Mathlib.Data.Set.Basic

namespace CordialMiners

/--
Projections for block identity.

`Validator` is a parameter of the class, so all projections are
explicitly associated with the corresponding validator type.
-/
class CreatorRound (Validator : Type*) where
  creatorOf : Blocklace → BlockId → Option Validator
  roundOf : Blocklace → BlockId → Option ℕ

  /--
  If `a` observes `b` and they are distinct, then the round of `b`
  is strictly smaller than the round of `a`.
  -/
  round_lt_of_observes :
    ∀ (B : Blocklace) (a b : BlockId),
      Observes B a b →
      a ≠ b →
      ∀ ra rb : ℕ,
        roundOf B a = some ra →
        roundOf B b = some rb →
        rb < ra

  /--
  Every block with a defined creator has a defined round.
  -/
  round_defined_of_creator_defined :
    ∀ (B : Blocklace) (b : BlockId) (v : Validator),
      creatorOf B b = some v →
      ∃ r, roundOf B b = some r


variable {Validator : Type*} [CreatorRound Validator]


/-!
======================================================================
  Creator / Round Definitions
======================================================================
-/

/--
The set of blocks created by validator `v` at round `r` in blocklace `B`.
-/
def creatorBlocksAtRound
    (B : Blocklace)
    (v : Validator)
    (r : ℕ) : Set BlockId :=
  {
    b |
      CreatorRound.creatorOf (Validator := Validator) B b = some v ∧
      CreatorRound.roundOf (Validator := Validator) B b = some r
  }


/-!
======================================================================
  Equivocation Definitions
======================================================================
-/

/--
`b1` and `b2` constitute an equivocation if:

* both were created by `v`,
* both were created at round `r`, and
* they are distinct blocks.
-/
def Equivocation
    (B : Blocklace)
    (v : Validator)
    (r : ℕ)
    (b1 b2 : BlockId) : Prop :=
  b1 ∈ creatorBlocksAtRound B v r ∧
  b2 ∈ creatorBlocksAtRound B v r ∧
  b1 ≠ b2


/--
A validator equivocates at a round when the set of its blocks at
that round is nontrivial.
-/
def EquivocatesAt
    (B : Blocklace)
    (v : Validator)
    (r : ℕ) : Prop :=
  (creatorBlocksAtRound B v r).Nontrivial


/--
Characterization of `EquivocatesAt` in terms of two distinct blocks.
-/
theorem equivocatesAt_iff_exists_equivocation
    (B : Blocklace)
    (v : Validator)
    (r : ℕ) :
    EquivocatesAt B v r ↔
      ∃ b1 b2, Equivocation B v r b1 b2 := by
  constructor

  · rintro ⟨b1, hb1, b2, hb2, hne⟩
    exact ⟨b1, b2, hb1, hb2, hne⟩

  · rintro ⟨b1, b2, hb1, hb2, hne⟩
    exact ⟨b1, hb1, b2, hb2, hne⟩


/--
A validator is an equivocator if it equivocates at some round.
-/
def Equivocator
    (B : Blocklace)
    (v : Validator) : Prop :=
  ∃ r, EquivocatesAt B v r


/--
A validator is honest in `B` when it never equivocates.
-/
def HonestIn
    (B : Blocklace)
    (v : Validator) : Prop :=
  ¬ Equivocator B v


/-!
======================================================================
  Structural Properties
======================================================================
-/

/--
Blocks created by the same validator at the same round form an
antichain under observation.

If two distinct blocks are at the same round, neither can observe
the other because observation strictly increases round number.
-/
theorem creatorBlocksAtRound_isAntichain
    (B : Blocklace)
    (v : Validator)
    (r : ℕ) :
    IsAntichain
      (Observes B)
      (creatorBlocksAtRound B v r) := by

  intro b1 hb1 b2 hb2 hne hobs

  have hr1 := hb1.2
  have hr2 := hb2.2

  exact
    (
      CreatorRound.round_lt_of_observes
        (Validator := Validator)
        B
        b1
        b2
        hobs
        hne
        r
        r
        hr1
        hr2
    ).ne rfl


/--
An equivocation consists of incomparable blocks.
-/
theorem equivocation_incomparable
    (B : Blocklace)
    {v : Validator}
    {r : ℕ}
    {b1 b2 : BlockId}
    (heq : Equivocation B v r b1 b2) :
    ¬ Observes B b1 b2 ∧
    ¬ Observes B b2 b1 := by

  have hanti :=
    creatorBlocksAtRound_isAntichain B v r

  obtain ⟨hb1, hb2, hne⟩ := heq

  exact
    ⟨
      fun h =>
        hanti hb1 hb2 hne h,

      fun h =>
        hanti hb2 hb1 hne.symm h
    ⟩


/--
For an honest validator, two blocks belonging to the same round
must be equal.

This is the round-level uniqueness property induced by honesty.
-/
theorem honest_round_injective
    (B : Blocklace)
    (v : Validator)
    (hHonest : HonestIn B v)
    {b1 b2 : BlockId}
    {r : ℕ}
    (hb1 : b1 ∈ creatorBlocksAtRound B v r)
    (hb2 : b2 ∈ creatorBlocksAtRound B v r) :
    b1 = b2 := by

  by_contra hne

  exact
    hHonest
      ⟨
        r,
        b1,
        hb1,
        b2,
        hb2,
        hne
      ⟩


/-!
======================================================================
  Honest Validator Chain
======================================================================
-/

/--
All blocks created by `v` form a chain when every later round block
observes every earlier round block.

This is an explicit chain-extension assumption.
-/
def ExtendsOwnChain
    (B : Blocklace)
    (v : Validator) : Prop :=
  ∀ b b' : BlockId,
    CreatorRound.creatorOf (Validator := Validator) B b = some v →
    CreatorRound.creatorOf (Validator := Validator) B b' = some v →
    ∀ r r' : ℕ,
      CreatorRound.roundOf (Validator := Validator) B b = some r →
      CreatorRound.roundOf (Validator := Validator) B b' = some r' →
      r' < r →
      Observes B b b'


/--
Under honesty and the own-chain extension assumption, all blocks
created by a validator are linearly ordered by observation.
-/
theorem honest_chain_linear
    (B : Blocklace)
    (v : Validator)
    (hHonest : HonestIn B v)
    (hExtends : ExtendsOwnChain B v) :
    IsChain
      (Observes B)
      {
        b : BlockId |
          CreatorRound.creatorOf
            (Validator := Validator)
            B
            b = some v
      } := by

  intro b1 hb1 b2 hb2 hne

  simp only [Set.mem_ofPred_eq] at hb1 hb2

  obtain ⟨r1, hr1⟩ :=
    CreatorRound.round_defined_of_creator_defined
      (Validator := Validator)
      B
      b1
      v
      hb1

  obtain ⟨r2, hr2⟩ :=
    CreatorRound.round_defined_of_creator_defined
      (Validator := Validator)
      B
      b2
      v
      hb2

  rcases lt_trichotomy r1 r2 with hlt | heqr | hgt

  · /-
      r1 < r2.
      Therefore b1 is the earlier block and b2 observes b1.
    -/
    exact
      Or.inr
        (
          hExtends
            b2
            b1
            hb2
            hb1
            r2
            r1
            hr2
            hr1
            hlt
        )

  · /-
      Same round.

      Both blocks belong to the same validator and round, so
      honesty implies they are equal, contradicting `hne`.
    -/
    have hb1' :
        b1 ∈ creatorBlocksAtRound B v r1 :=
      ⟨hb1, hr1⟩

    have hb2' :
        b2 ∈ creatorBlocksAtRound B v r1 :=
      ⟨
        hb2,
        by
          rw [heqr]
          exact hr2
      ⟩

    exact
      absurd
        (honest_round_injective B v hHonest hb1' hb2')
        hne

  · /-
      r2 < r1.
      Therefore b1 is the later block and b1 observes b2.
    -/
    exact
      Or.inl
        (
          hExtends
            b1
            b2
            hb1
            hb2
            r1
            r2
            hr1
            hr2
            hgt
        )


/-!
======================================================================
  Acknowledgement
======================================================================
-/

/--
`w` acknowledges validator `v` at round `r` when `w` observes every
block created by `v` at that round.
-/
def Acknowledges
    (B : Blocklace)
    (w : BlockId)
    (v : Validator)
    (r : ℕ) : Prop :=
  ∀ b ∈ creatorBlocksAtRound B v r,
    Observes B w b


/--
`w` hides the round when it does not acknowledge the round.
-/
def Hides
    (B : Blocklace)
    (w : BlockId)
    (v : Validator)
    (r : ℕ) : Prop :=
  ¬ Acknowledges B w v r


/--
Acknowledgement is monotone along observation.

If `w'` observes `w`, and `w` acknowledges a round, then `w'`
also acknowledges that round.
-/
theorem acknowledges_mono
    (B : Blocklace)
    {w w' : BlockId}
    {v : Validator}
    {r : ℕ}
    (h : Acknowledges B w v r)
    (hobs : Observes B w' w) :
    Acknowledges B w' v r := by

  intro b hb

  exact
    observes_trans
      B
      hobs
      (h b hb)


/-!
======================================================================
  Vouching / Approval Exclusion
======================================================================
-/

/--
`w` cleanly vouches for block `b`.

The first conjunct says that `w` observes `b`.

The second says that if another block `b'` has the same creator as
`b`, is distinct from `b`, and is also observed by `w`, then `b`
and `b'` cannot be incomparable.

This captures the idea that a clean vouch for `b` excludes a
competing block from the same creator.
-/
def VouchesFor
    (V : Type*)
    [CreatorRound V]
    (B : Blocklace)
    (w b : BlockId) : Prop :=
  Observes B w b ∧
  ∀ (b' : BlockId),
    CreatorRound.creatorOf
        (Validator := V)
        B
        b'
      =
    CreatorRound.creatorOf
        (Validator := V)
        B
        b →
    b' ≠ b →
    Observes B w b' →
    ¬ (
      ¬ Observes B b b' ∧
      ¬ Observes B b' b
    )


/--
If `w` acknowledges an equivocating validator's round, then `w`
cannot cleanly vouch for any block from that equivocation round.

This is the central equivocation-exclusion theorem.
-/
theorem acknowledges_no_vouch_for_equivocation
    (B : Blocklace)
    {v : Validator}
    {r : ℕ}
    {w b : BlockId}
    (heq : EquivocatesAt B v r)
    (hb : b ∈ creatorBlocksAtRound B v r)
    (hack : Acknowledges B w v r) :
    ¬ VouchesFor Validator B w b := by

  obtain ⟨b1, hb1, b2, hb2, hne⟩ := heq

  have hanti :=
    creatorBlocksAtRound_isAntichain B v r

  /-
    Find another block in the same round that is distinct from `b`.
  -/
  obtain ⟨s, hs, hsb⟩ :
      ∃ s,
        s ∈ creatorBlocksAtRound B v r ∧
        s ≠ b := by

    by_cases hcase : b = b1

    · exact
        ⟨
          b2,
          hb2,
          hcase.symm ▸ hne.symm
        ⟩

    · exact
        ⟨
          b1,
          hb1,
          Ne.symm hcase
        ⟩

  rintro ⟨_, hvouch⟩

  /-
    `s` and `b` have the same creator because they belong to the
    same validator's round.
  -/
  have h_same_creator :
      CreatorRound.creatorOf
          (Validator := Validator)
          B
          s
        =
      CreatorRound.creatorOf
          (Validator := Validator)
          B
          b := by

    rw [hs.1, hb.1]

  /-
    Same-round distinct blocks are incomparable.
  -/
  have h_incomp :
      ¬ Observes B b s ∧
      ¬ Observes B s b :=
    ⟨
      hanti hb hs hsb.symm,
      hanti hs hb hsb
    ⟩

  /-
    But acknowledgement gives `w` observation of `s`, and therefore
    the vouching condition contradicts the incomparability.
  -/
  exact
    hvouch
      s
      h_same_creator
      hsb
      (hack s hs)
      h_incomp


/--
Convenient pairwise form of the equivocation exclusion theorem.

If `b1` and `b2` are two distinct blocks in the same validator/round
and `w` acknowledges that round, then `w` cannot vouch for either
block.
-/
theorem equivocation_vouch_exclusion
    (B : Blocklace)
    {v : Validator}
    {r : ℕ}
    {w b1 b2 : BlockId}
    (heq : Equivocation B v r b1 b2)
    (hack : Acknowledges B w v r) :
    ¬ VouchesFor Validator B w b1 ∧
    ¬ VouchesFor Validator B w b2 := by

  obtain ⟨hb1, hb2, hne⟩ := heq

  constructor

  · exact
      acknowledges_no_vouch_for_equivocation
        B
        ⟨b1, hb1, b2, hb2, hne⟩
        hb1
        hack

  · exact
      acknowledges_no_vouch_for_equivocation
        B
        ⟨b1, hb1, b2, hb2, hne⟩
        hb2
        hack


/--
General approval-exclusion theorem.

Any approval relation that implies `VouchesFor` automatically
inherits the equivocation exclusion property.
-/
theorem equivocation_not_approved
    (B : Blocklace)
    {v : Validator}
    {r : ℕ}
    {w b : BlockId}
    {Approves : Blocklace → BlockId → BlockId → Prop}
    (heq : EquivocatesAt B v r)
    (hb : b ∈ creatorBlocksAtRound B v r)
    (hack : Acknowledges B w v r)
    (hApproveImpliesVouch :
      Approves B w b →
      VouchesFor Validator B w b) :
    ¬ Approves B w b := by

  intro happ

  exact
    acknowledges_no_vouch_for_equivocation
      B
      heq
      hb
      hack
      (hApproveImpliesVouch happ)


/-!
======================================================================
  Worked Example
======================================================================
-/

section WorkedExample

variable
  (B : Blocklace)
  (honest cheater : Validator)
  (g e1 e2 : BlockId)
  (rg re : ℕ)
  (w : BlockId)

variable
  (hg :
    CreatorRound.creatorOf
      (Validator := Validator)
      B
      g
      =
    some honest)

variable
  (hrg :
    CreatorRound.roundOf
      (Validator := Validator)
      B
      g
      =
    some rg)

variable
  (he1 :
    CreatorRound.creatorOf
      (Validator := Validator)
      B
      e1
      =
    some cheater)

variable
  (hre1 :
    CreatorRound.roundOf
      (Validator := Validator)
      B
      e1
      =
    some re)

variable
  (he2 :
    CreatorRound.creatorOf
      (Validator := Validator)
      B
      e2
      =
    some cheater)

variable
  (hre2 :
    CreatorRound.roundOf
      (Validator := Validator)
      B
      e2
      =
    some re)

variable
  (hne : e1 ≠ e2)

variable
  (honly :
    ∀ b,
      CreatorRound.creatorOf
          (Validator := Validator)
          B
          b
        =
      some honest →
      b = g)

variable
  (hackw :
    Acknowledges B w cheater re)


/-!
----------------------------------------------------------------------
  Honest validator example
----------------------------------------------------------------------
-/

/--
The honest validator cannot equivocate when every block it creates
is the single block `g`.
-/
example : HonestIn B honest := by

  rintro
    ⟨
      r,
      b1,
      hb1,
      b2,
      hb2,
      hne'
    ⟩

  rw
    [
      honly b1 hb1.1,
      honly b2 hb2.1
    ]
    at hne'

  exact hne' rfl


/-!
----------------------------------------------------------------------
  Cheater example
----------------------------------------------------------------------
-/

/--
The cheater is not honest because it creates two distinct blocks
at the same round.
-/
example : ¬ HonestIn B cheater := by

  intro h

  exact
    h
      ⟨
        re,
        e1,
        ⟨he1, hre1⟩,
        e2,
        ⟨he2, hre2⟩,
        hne
      ⟩


/-!
----------------------------------------------------------------------
  Equivocation witness
----------------------------------------------------------------------
-/

/--
The two cheater blocks form an explicit equivocation.
-/
example :
    Equivocation B cheater re e1 e2 := by

  exact
    ⟨
      ⟨he1, hre1⟩,
      ⟨he2, hre2⟩,
      hne
    ⟩


/-!
----------------------------------------------------------------------
  Acknowledgement excludes vouching for e1
----------------------------------------------------------------------
-/

/--
A node acknowledging the cheater's round cannot vouch for `e1`.
-/
example :
    ¬ VouchesFor Validator B w e1 := by

  exact
    acknowledges_no_vouch_for_equivocation
      B
      (
        ⟨
          e1,
          ⟨he1, hre1⟩,
          e2,
          ⟨he2, hre2⟩,
          hne
        ⟩
      )
      ⟨he1, hre1⟩
      hackw


/-!
----------------------------------------------------------------------
  Acknowledgement excludes vouching for e2
----------------------------------------------------------------------
-/

/--
A node acknowledging the cheater's round cannot vouch for `e2`.
-/
example :
    ¬ VouchesFor Validator B w e2 := by

  exact
    acknowledges_no_vouch_for_equivocation
      B
      (
        ⟨
          e1,
          ⟨he1, hre1⟩,
          e2,
          ⟨he2, hre2⟩,
          hne
        ⟩
      )
      ⟨he2, hre2⟩
      hackw


/-!
----------------------------------------------------------------------
  Pairwise equivocation exclusion
----------------------------------------------------------------------
-/

/--
The pairwise theorem simultaneously excludes both equivocation
blocks from clean vouching.
-/
example :
    ¬ VouchesFor Validator B w e1 ∧
    ¬ VouchesFor Validator B w e2 := by

  exact
    equivocation_vouch_exclusion
      B
      (
        ⟨
          ⟨he1, hre1⟩,
          ⟨he2, hre2⟩,
          hne
        ⟩
      )
      hackw


/-!
----------------------------------------------------------------------
  Approval exclusion example
----------------------------------------------------------------------
-/

/--
Any approval relation that implies `VouchesFor` is also excluded
for the equivocating block.
-/
example
    {Approves : Blocklace → BlockId → BlockId → Prop}
    (hApproveImpliesVouch :
      Approves B w e1 →
      VouchesFor Validator B w e1) :
    ¬ Approves B w e1 := by

  exact
    equivocation_not_approved
      B
      (
        ⟨
          e1,
          ⟨he1, hre1⟩,
          e2,
          ⟨he2, hre2⟩,
          hne
        ⟩
      )
      ⟨he1, hre1⟩
      hackw
      hApproveImpliesVouch


end WorkedExample

end CordialMiners