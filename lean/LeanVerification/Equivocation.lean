/-
Equivocation and exclusion (KR2): the one Byzantine behavior Cordial
Miners cares about — a validator producing two same-depth blocks that sit
side by side in the DAG, neither observing the other — and the structural
guarantee that no single block can vouch for both sides of it.

Owned by Issue 03 (KR2 — Equivocation and Exclusion).

## Rounds are derived, not assumed

Rust defines a round as a block's DAG depth. This file computes that depth
by well-founded recursion over `DirectPred`, using Issue 02's proved
`directPred_wf_of_valid`; it does not assume a `round_lt_of_observes` axiom
or introduce a typeclass projection. `Fork` records the round-independent
same-creator incomparability used by approval and the chain invariant.
`Equivocation` adds equality of the two computed depths, matching
`equivocation_blocks_at_round` and `all_equivocations` in Rust.

## Concrete types now, not a placeholder signature

Issue 02's `Block.lean`/`Blocklace.lean`/`Observe.lean` have all landed,
so this file no longer develops against an abstract stand-in signature —
`Blocklace`, `BlockId`, `Block`, and `Observes` are the real, concrete
definitions. In particular `Block.creator : NodeId` (with
`NodeId := Nat`) means there is no generic `Validator` type parameter
here at all; `NodeId` is used directly. `creatorOf` below is a plain,
ordinary function (`Blocklace → BlockId → Option NodeId`), not a class
field, so calling it is never ambiguous.

The distinction matters: Rust's detector reports same-depth forks, while
`approves` rejects every observed incomparable block by the same creator.
Keeping both predicates makes each Lean-to-Rust mapping exact instead of
silently broadening the detector.
-/
import LeanVerification.Observe
import Mathlib.Order.Preorder.Chain


namespace CordialMiners

/-! ### Creator lookup

Plain, ordinary function — not a typeclass field, so there is nothing for
Lean to have to infer here. Mirrors `BlockIdentity.creator`
(`types/identity_id.rs`) via `Block.lookup`. -/

/-- The creator of `b` in `B`, if `b` is present. `none` if `b ∉ B.keys`. -/
def creatorOf (B : Blocklace) (b : BlockId) : Option NodeId :=
  (B.lookup b).map Block.creator

/-! ### DAG depth -/

/-- DAG depth computed by well-founded recursion. Missing identifiers have
depth zero; all uses in `Equivocation` separately require membership. -/
def blockDepthWF
    (B : Blocklace)
    (hwf : WellFounded (fun a b => DirectPred B b a)) :
    BlockId → Nat :=
  hwf.fix fun b rec =>
    match hlookup : B.lookup b with
    | none => 0
    | some blk =>
        blk.content.predecessors.attach.sup fun p =>
          rec p.1 ⟨blk, hlookup, p.2⟩ + 1

/-- Unfolding equation for `blockDepthWF`. -/
theorem blockDepthWF_eq
    (B : Blocklace)
    (hwf : WellFounded (fun a b => DirectPred B b a))
    (b : BlockId) :
    blockDepthWF B hwf b =
      match _hlookup : B.lookup b with
      | none => 0
      | some blk =>
          blk.content.predecessors.attach.sup fun p =>
            blockDepthWF B hwf p.1 + 1 := by
  unfold blockDepthWF
  rw [WellFounded.fix_eq]

/-- Rust-compatible block depth for a validly constructed blocklace. -/
def blockDepth (B : Blocklace) (hV : ValidBlocklace B) (b : BlockId) : Nat :=
  blockDepthWF B (directPred_wf_of_valid B hV) b

/-! ### Depth monotonicity along predecessor edges -/

/-- A direct predecessor is strictly shallower than its successor. -/
theorem blockDepth_directPred (B : Blocklace) (hV : ValidBlocklace B) (b p : BlockId)
    (h : DirectPred B b p) : blockDepth B hV p + 1 ≤ blockDepth B hV b := by
  obtain ⟨blk, hblk, hp⟩ := h
  unfold blockDepth
  rw [blockDepthWF_eq B (directPred_wf_of_valid B hV) b, hblk]
  exact Finset.le_sup (f := fun q =>
    blockDepthWF B (directPred_wf_of_valid B hV) q.1 + 1) (Finset.mem_attach _ ⟨p, hp⟩)

/-- Depth strictly increases along every transitive predecessor path. -/
theorem depth_strict_of_precedes (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ : BlockId)
    (h : Relation.TransGen (DirectPred B) b₁ b₂) :
    blockDepth B hV b₂ < blockDepth B hV b₁ :=
  h.head_induction_on
    (fun {a} hac => Nat.lt_of_add_one_le (blockDepth_directPred B hV a b₂ hac))
    (fun {a b} hab _hbc ihb =>
      Nat.lt_trans ihb (Nat.lt_of_add_one_le (blockDepth_directPred B hV a b hab)))

/-- Two blocks with the same depth and distinct ids cannot observe each other. -/
theorem same_depth_incomparable (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ : BlockId)
    (hne : b₁ ≠ b₂) (hd : blockDepth B hV b₁ = blockDepth B hV b₂) :
    ¬ Observes B b₁ b₂ ∧ ¬ Observes B b₂ b₁ := by
  have aux : ∀ x y : BlockId, x ≠ y → blockDepth B hV x = blockDepth B hV y →
      ¬ Observes B x y := by
    intro x y hxy hdxy hobs
    rcases (Relation.reflTransGen_iff_eq_or_transGen.mp hobs) with rfl | ht
    · exact hxy rfl
    · exact Nat.lt_irrefl _ (hdxy ▸ depth_strict_of_precedes B hV x y ht)
  exact ⟨aux b₁ b₂ hne hd, aux b₂ b₁ (Ne.symm hne) hd.symm⟩

/-! ### Forks, equivocation, and honesty -/

/-- A structural fork: two present, distinct, incomparable blocks with the
same creator. This is the conflict relation checked by Rust `approves`. -/
def Fork (B : Blocklace) (b₁ b₂ : BlockId) : Prop :=
  b₁ ∈ B.keys ∧ b₂ ∈ B.keys ∧
  creatorOf B b₁ = creatorOf B b₂ ∧ b₁ ≠ b₂ ∧
  ¬ Observes B b₁ b₂ ∧ ¬ Observes B b₂ b₁

/-- A Rust/Issue-186 equivocation: a structural fork at one DAG depth. -/
def Equivocation
    (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ : BlockId) : Prop :=
  Fork B b₁ b₂ ∧ blockDepth B hV b₁ = blockDepth B hV b₂

/-- `v` is an equivocator in `B`: some pair of `v`'s blocks in `B`
equivocate at one computed depth. Mirrors `all_equivocations`. -/
def Equivocator (B : Blocklace) (hV : ValidBlocklace B) (v : NodeId) : Prop :=
  ∃ b₁ b₂, Equivocation B hV b₁ b₂ ∧ creatorOf B b₁ = some v

/-- Chain honesty is the absence of every structural fork, including forks
whose blocks have different depths. This is the premise needed by the
all-pairs chain theorem and corresponds to Rust `satisfies_chain_axiom`. -/
def HonestIn (B : Blocklace) (v : NodeId) : Prop :=
  ¬ ∃ b₁ b₂, Fork B b₁ b₂ ∧ creatorOf B b₁ = some v

/-- The set of blocks `v` has created that are present in `B`. -/
def blocksBy (B : Blocklace) (v : NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧ creatorOf B b = some v}

/-- **Honest chain linearity.** If `v` is honest in `B`, `v`'s blocks form
a chain under `Observes B`: any two are comparable. This is what makes
"the chain of validator `v`" a well-defined, unambiguous concept
elsewhere in the protocol — an honest validator's history is always
linear, never a DAG with a fork in it. The Rust counterpart is the
closure-axiom-adjacent invariant `satisfies_chain_axiom`
(`blocklace.rs`, lines 252–279), which this theorem is the formal,
general statement of. -/
theorem honest_chain_linearity (B : Blocklace) (v : NodeId) (h : HonestIn B v) :
    IsChain (Observes B) (blocksBy B v) := by
  intro b₁ hb₁ b₂ hb₂ hne
  by_contra hcontra
  push Not at hcontra
  exact h ⟨b₁, b₂, ⟨hb₁.1, hb₂.1, hb₁.2.trans hb₂.2.symm, hne, hcontra.1, hcontra.2⟩, hb₁.2⟩

/-- Chain honesty rules out the narrower same-depth detector predicate. -/
theorem honest_not_equivocator
    (B : Blocklace) (hV : ValidBlocklace B) (v : NodeId)
    (h : HonestIn B v) :
    ¬ Equivocator B hV v := by
  rintro ⟨b₁, b₂, ⟨hfork, -⟩, hcreator⟩
  exact h ⟨b₁, b₂, hfork, hcreator⟩

/-! ### Acknowledgement and hiding -/

/-- `c` acknowledges the equivocation between `b₁` and `b₂`: `c`'s causal
history includes both branches. Mirrors `acknowledges_equivocation`
(`consensus/cordiality.rs`, lines 130–145). -/
def Acknowledges (B : Blocklace) (c b₁ b₂ : BlockId) : Prop :=
  Observes B c b₁ ∧ Observes B c b₂

/-- `c` hides (at least part of) the equivocation between `b₁` and `b₂`:
`c` fails to acknowledge it, i.e. is missing at least one branch. Mirrors
`hidden_equivocations` (`consensus/cordiality.rs`, lines 147–177)
reporting a nonempty set of missing branches for `c` — note that, exactly
as in the Rust, observing one branch but not the other still counts as
hiding, not as acknowledging. -/
def Hides (B : Blocklace) (c b₁ b₂ : BlockId) : Prop :=
  ¬ Acknowledges B c b₁ b₂

/-- The causal view reconstructed from a candidate block's declared
predecessors before that candidate is inserted into `B`. -/
def CandidateObserves (B : Blocklace) (candidate : Block) (b : BlockId) : Prop :=
  ∃ p ∈ candidate.content.predecessors, Observes B p b

/-- Pre-insertion acknowledgement, matching Rust
`acknowledges_equivocation(blocklace, candidate, ...)`. -/
def CandidateAcknowledges
    (B : Blocklace) (candidate : Block) (b₁ b₂ : BlockId) : Prop :=
  CandidateObserves B candidate b₁ ∧ CandidateObserves B candidate b₂

/-- Pre-insertion hiding, matching Rust `hidden_equivocations`. -/
def CandidateHides
    (B : Blocklace) (candidate : Block) (b₁ b₂ : BlockId) : Prop :=
  ¬ CandidateAcknowledges B candidate b₁ b₂

/-- Once a candidate is present under its own identifier, its reconstructed
pre-insertion view is included in ordinary `Observes`. -/
theorem candidateAcknowledges_of_inserted
    (B : Blocklace) (candidate : Block) (b₁ b₂ : BlockId)
    (hlookup : B.lookup candidate.id = some candidate)
    (hack : CandidateAcknowledges B candidate b₁ b₂) :
    Acknowledges B candidate.id b₁ b₂ := by
  constructor
  · obtain ⟨p, hp, hpb⟩ := hack.1
    exact observes_trans B (observes_step B candidate.id p ⟨candidate, hlookup, hp⟩) hpb
  · obtain ⟨p, hp, hpb⟩ := hack.2
    exact observes_trans B (observes_step B candidate.id p ⟨candidate, hlookup, hp⟩) hpb

/-- **Equivocation-visibility monotonicity.** If `d` observes `c`, and `c`
already acknowledges the equivocation between `b₁` and `b₂`, then `d`
acknowledges it too. Visibility of a known equivocation only grows as the
blocklace grows and later blocks reference deeper into the causal past —
it never shrinks. This is a direct corollary of `Observe.lean`'s
`observes_trans`, which holds unconditionally (no `Closed B` needed). -/
theorem acknowledgement_monotone (B : Blocklace) (c d b₁ b₂ : BlockId) (hdc : Observes B d c)
    (hack : Acknowledges B c b₁ b₂) :
    Acknowledges B d b₁ b₂ :=
  ⟨observes_trans B hdc hack.1, observes_trans B hdc hack.2⟩

/-! ### Exclusion -/

/-- The equivocation-relevant fragment of the paper's approval relation
(Definition 18, as implemented by `approves` in `consensus/approval.rs`):
`c` cleanly vouches for `b` if it observes `b` and does not also observe
any block, by the same creator as `b`, that is incomparable with `b`. The
real `approves` checks this against *every* other block by the creator;
this fragment only needs the equivocation case, which is all Issue 04's
Agreement proof requires from this file — Issue 04 is expected to build
its full `Approves` on top of this rather than duplicate it. -/
def VouchesFor (B : Blocklace) (c b : BlockId) : Prop :=
  Observes B c b ∧
    ∀ b', creatorOf B b' = creatorOf B b → b' ≠ b → Observes B c b' →
      ¬ (¬ Observes B b b' ∧ ¬ Observes B b' b)

/-- **Exclusion property.** If `b₁` and `b₂` equivocate and `c`
acknowledges the equivocation (observes both), `c` cleanly vouches for
*neither* branch. This is the structural fact that keeps both sides of a
cheat out of any finalized output built from `VouchesFor`/`Approves`-style
acceptance — the sentence to say out loud is: no single approving block
can vouch for both sides of the same lie, because as soon as it has seen
both sides, `VouchesFor`'s own "no incomparable sibling observed"
condition fails for each side in turn. This is the core proof Issue 04's
Agreement theorem rests on; see `equivocation_not_approved` immediately
below for the form actually meant to be imported as a hypothesis, phrased
against an arbitrary `Approves` rather than this file's own `VouchesFor`. -/
theorem equivocation_exclusion
    (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ c : BlockId)
    (heq : Equivocation B hV b₁ b₂) (hack : Acknowledges B c b₁ b₂) :
    ¬ VouchesFor B c b₁ ∧ ¬ VouchesFor B c b₂ := by
  obtain ⟨⟨-, -, hcreator, hne, hnob12, hnob21⟩, -⟩ := heq
  refine ⟨fun hv => ?_, fun hv => ?_⟩
  · exact hv.2 b₂ hcreator.symm (Ne.symm hne) hack.2 ⟨hnob12, hnob21⟩
  · exact hv.2 b₁ hcreator hne hack.1 ⟨hnob21, hnob12⟩

/-- **Bridge for Issue 04.** `Approves` is left as an arbitrary relation
here, not defined in this file — Issue 04 owns the real `Approves`
formalization. Given a proof that `Approves` implies `VouchesFor` for the
specific `c`/`b₁` (resp. `c`/`b₂`) in question — i.e. that `VouchesFor`
really is a sound over-approximation of Issue 04's eventual `Approves`,
as `VouchesFor`'s own doc comment already claims — acknowledging the
equivocation rules out `Approves` too, not just `VouchesFor`.

This, not `equivocation_exclusion` above, is the form the issue's
acceptance criteria ask for literally: Issue 04 supplies its own
`Approves` and the two implication lemmas and gets the exclusion result
directly, with zero changes to this file, regardless of what `Approves`
ends up looking like. -/
theorem equivocation_not_approved
    (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ c : BlockId)
    {Approves : Blocklace → BlockId → BlockId → Prop}
    (heq : Equivocation B hV b₁ b₂) (hack : Acknowledges B c b₁ b₂)
    (hApproveImpliesVouch₁ : Approves B c b₁ → VouchesFor B c b₁)
    (hApproveImpliesVouch₂ : Approves B c b₂ → VouchesFor B c b₂) :
    ¬ Approves B c b₁ ∧ ¬ Approves B c b₂ :=
  let ⟨hv₁, hv₂⟩ := equivocation_exclusion B hV b₁ b₂ c heq hack
  ⟨fun h => hv₁ (hApproveImpliesVouch₁ h), fun h => hv₂ (hApproveImpliesVouch₂ h)⟩

section WorkedExample

/-- The honest validator. -/
def honestId : NodeId := 0

/-- The cheater. -/
def cheaterId : NodeId := 1

/-- A third validator whose block acknowledges both equivocation branches. -/
def observerId : NodeId := 2

theorem honestId_ne_cheaterId : honestId ≠ cheaterId := by decide
theorem honestId_ne_observerId : honestId ≠ observerId := by decide
theorem cheaterId_ne_observerId : cheaterId ≠ observerId := by decide

def contentG : BlockContent := { payload := [], predecessors := ∅ }
def contentE1 : BlockContent := { payload := [0], predecessors := ∅ }
def contentE2 : BlockContent := { payload := [1], predecessors := ∅ }

/-- The honest validator's only block. -/
def g : Block :=
  { id := hashContent honestId contentG
    creator := honestId
    content := contentG
    id_eq := rfl }

/-- The cheater's first block. -/
def e1 : Block :=
  { id := hashContent cheaterId contentE1
    creator := cheaterId
    content := contentE1
    id_eq := rfl }

/-- The cheater's second, conflicting block. -/
def e2 : Block :=
  { id := hashContent cheaterId contentE2
    creator := cheaterId
    content := contentE2
    id_eq := rfl }

/-- A real acknowledging block whose declared predecessors are both branches. -/
def contentC : BlockContent :=
  { payload := [2], predecessors := {e1.id, e2.id} }

def c : Block :=
  { id := hashContent observerId contentC
    creator := observerId
    content := contentC
    id_eq := rfl }

theorem e1_ne_e2 : e1.id ≠ e2.id := by
  intro h
  have hcontent := (hashInj h).2
  simp [contentE1, contentE2] at hcontent

theorem g_ne_e1 : g.id ≠ e1.id := by
  intro h
  exact honestId_ne_cheaterId (hashInj h).1

theorem g_ne_e2 : g.id ≠ e2.id := by
  intro h
  exact honestId_ne_cheaterId (hashInj h).1

theorem g_ne_c : g.id ≠ c.id := by
  intro h
  exact honestId_ne_observerId (hashInj h).1

theorem e1_ne_c : e1.id ≠ c.id := by
  intro h
  exact cheaterId_ne_observerId (hashInj h).1

theorem e2_ne_c : e2.id ≠ c.id := by
  intro h
  exact cheaterId_ne_observerId (hashInj h).1

/-! Build the example by valid insertions so DAG depth is available without
an axiom. `demoBeforeC` is exactly the state used by Rust while validating
the not-yet-inserted candidate `c`; `demoB` is the state after insertion. -/

def demoB0 : Blocklace := emptyBlocklace
def demoB1 : Blocklace := blocklaceInsert demoB0 e1
def demoB2 : Blocklace := blocklaceInsert demoB1 e2
def demoBeforeC : Blocklace := blocklaceInsert demoB2 g
def demoB : Blocklace := blocklaceInsert demoBeforeC c

theorem no_mem_demoB0 (x : BlockId) : x ∉ demoB0.keys := by
  intro h
  have hm := Finmap.mem_keys.mp h
  simp [demoB0, emptyBlocklace, Finmap.mem_def] at hm

theorem key_mem_insert_self (B : Blocklace) (blk : Block) :
    blk.id ∈ (blocklaceInsert B blk).keys := by
  exact Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inl rfl))

theorem key_mem_insert_of_mem
    (B : Blocklace) (blk : Block) {x : BlockId} (h : x ∈ B.keys) :
    x ∈ (blocklaceInsert B blk).keys := by
  exact Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inr (Finmap.mem_keys.mp h)))

theorem key_not_mem_insert
    (B : Blocklace) (blk : Block) {x : BlockId}
    (hne : x ≠ blk.id) (hnot : x ∉ B.keys) :
    x ∉ (blocklaceInsert B blk).keys := by
  intro h
  rcases Finmap.mem_insert.mp (Finmap.mem_keys.mp h) with heq | hmem
  · exact hne heq
  · exact hnot (Finmap.mem_keys.mpr hmem)

theorem e1_mem_before_c : e1.id ∈ demoBeforeC.keys :=
  key_mem_insert_of_mem demoB2 g
    (key_mem_insert_of_mem demoB1 e2 (key_mem_insert_self demoB0 e1))

theorem e2_mem_before_c : e2.id ∈ demoBeforeC.keys :=
  key_mem_insert_of_mem demoB2 g (key_mem_insert_self demoB1 e2)

theorem demoB0_valid : ValidBlocklace demoB0 := by
  exact ValidBlocklace.empty

theorem demoB1_valid : ValidBlocklace demoB1 := by
  apply ValidBlocklace.insert demoB0 e1 demoB0_valid
  · intro p hp
    simp [e1, contentE1] at hp
  · exact no_mem_demoB0 e1.id

theorem demoB2_valid : ValidBlocklace demoB2 := by
  apply ValidBlocklace.insert demoB1 e2 demoB1_valid
  · intro p hp
    simp [e2, contentE2] at hp
  · exact key_not_mem_insert demoB0 e1 e1_ne_e2.symm (no_mem_demoB0 e2.id)

theorem demoBeforeC_valid : ValidBlocklace demoBeforeC := by
  apply ValidBlocklace.insert demoB2 g demoB2_valid
  · intro p hp
    simp [g, contentG] at hp
  · exact key_not_mem_insert demoB1 e2 g_ne_e2
      (key_not_mem_insert demoB0 e1 g_ne_e1 (no_mem_demoB0 g.id))

theorem demoB_valid : ValidBlocklace demoB := by
  apply ValidBlocklace.insert demoBeforeC c demoBeforeC_valid
  · intro p hp
    simp [c, contentC] at hp
    rcases hp with rfl | rfl
    · exact e1_mem_before_c
    · exact e2_mem_before_c
  · exact key_not_mem_insert demoB2 g g_ne_c.symm
      (key_not_mem_insert demoB1 e2 e2_ne_c.symm
        (key_not_mem_insert demoB0 e1 e1_ne_c.symm (no_mem_demoB0 c.id)))

theorem lookup_g : demoB.lookup g.id = some g := by
  unfold demoB blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoBeforeC g_ne_c]
  unfold demoBeforeC blocklaceInsert
  exact Finmap.lookup_insert demoB2

theorem lookup_e1 : demoB.lookup e1.id = some e1 := by
  unfold demoB blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoBeforeC e1_ne_c]
  unfold demoBeforeC blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoB2 g_ne_e1.symm]
  unfold demoB2 blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoB1 e1_ne_e2]
  unfold demoB1 blocklaceInsert
  exact Finmap.lookup_insert demoB0

theorem lookup_e2 : demoB.lookup e2.id = some e2 := by
  unfold demoB blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoBeforeC e2_ne_c]
  unfold demoBeforeC blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoB2 g_ne_e2.symm]
  unfold demoB2 blocklaceInsert
  exact Finmap.lookup_insert demoB1

theorem lookup_c : demoB.lookup c.id = some c := by
  unfold demoB blocklaceInsert
  exact Finmap.lookup_insert demoBeforeC

theorem lookup_before_c_e1 : demoBeforeC.lookup e1.id = some e1 := by
  unfold demoBeforeC blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoB2 g_ne_e1.symm]
  unfold demoB2 blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoB1 e1_ne_e2]
  unfold demoB1 blocklaceInsert
  exact Finmap.lookup_insert demoB0

theorem lookup_before_c_e2 : demoBeforeC.lookup e2.id = some e2 := by
  unfold demoBeforeC blocklaceInsert
  rw [Finmap.lookup_insert_of_ne demoB2 g_ne_e2.symm]
  unfold demoB2 blocklaceInsert
  exact Finmap.lookup_insert demoB1

theorem g_mem : g.id ∈ demoB.keys := by
  exact key_mem_insert_of_mem demoBeforeC c (key_mem_insert_self demoB2 g)

theorem e1_mem : e1.id ∈ demoB.keys := by
  exact key_mem_insert_of_mem demoBeforeC c e1_mem_before_c

theorem e2_mem : e2.id ∈ demoB.keys := by
  exact key_mem_insert_of_mem demoBeforeC c e2_mem_before_c

theorem c_mem : c.id ∈ demoB.keys := by
  exact key_mem_insert_self demoBeforeC c

theorem demoB_keys_cases {b : BlockId} (hb : b ∈ demoB.keys) :
    b = g.id ∨ b = e1.id ∨ b = e2.id ∨ b = c.id := by
  simp [demoB, demoBeforeC, demoB2, demoB1, demoB0, blocklaceInsert,
    emptyBlocklace, Finmap.mem_keys, Finmap.mem_insert] at hb
  tauto

theorem creatorOf_g : creatorOf demoB g.id = some honestId := by
  simp only [creatorOf, lookup_g]; rfl

theorem creatorOf_e1 : creatorOf demoB e1.id = some cheaterId := by
  simp only [creatorOf, lookup_e1]; rfl

theorem creatorOf_e2 : creatorOf demoB e2.id = some cheaterId := by
  simp only [creatorOf, lookup_e2]; rfl

theorem creatorOf_c : creatorOf demoB c.id = some observerId := by
  simp only [creatorOf, lookup_c]; rfl

theorem directPred_e1_false (y : BlockId) : ¬ DirectPred demoB e1.id y := by
  rintro ⟨blk, hlookup, hp⟩
  rw [lookup_e1] at hlookup
  cases hlookup
  simp [e1, contentE1] at hp

theorem directPred_e2_false (y : BlockId) : ¬ DirectPred demoB e2.id y := by
  rintro ⟨blk, hlookup, hp⟩
  rw [lookup_e2] at hlookup
  cases hlookup
  simp [e2, contentE2] at hp

theorem not_observes_e1_e2 : ¬ Observes demoB e1.id e2.id := by
  intro h
  rcases Relation.ReflTransGen.cases_head h with hEq | ⟨p, hstep, -⟩
  · exact e1_ne_e2 hEq
  · exact directPred_e1_false p hstep

theorem not_observes_e2_e1 : ¬ Observes demoB e2.id e1.id := by
  intro h
  rcases Relation.ReflTransGen.cases_head h with hEq | ⟨p, hstep, -⟩
  · exact e1_ne_e2.symm hEq
  · exact directPred_e2_false p hstep

theorem depth_e1 : blockDepth demoB demoB_valid e1.id = 0 := by
  unfold blockDepth
  rw [blockDepthWF_eq, lookup_e1]
  simp [e1, contentE1]

theorem depth_e2 : blockDepth demoB demoB_valid e2.id = 0 := by
  unfold blockDepth
  rw [blockDepthWF_eq, lookup_e2]
  simp [e2, contentE2]

theorem e1_e2_fork : Fork demoB e1.id e2.id :=
  ⟨e1_mem, e2_mem, creatorOf_e1.trans creatorOf_e2.symm, e1_ne_e2,
    not_observes_e1_e2, not_observes_e2_e1⟩

/-- `e1` and `e2` are an explicit same-depth equivocation. -/
theorem e1_e2_equivocate : Equivocation demoB demoB_valid e1.id e2.id :=
  ⟨e1_e2_fork, depth_e1.trans depth_e2.symm⟩

/-- `HonestIn` evaluates false for the cheater. -/
theorem cheater_not_honest : ¬ HonestIn demoB cheaterId := fun h =>
  h ⟨e1.id, e2.id, e1_e2_fork, creatorOf_e1⟩

/-- `HonestIn` evaluates true for the honest validator: it only ever
created one block, `g`, so no pair of "its" blocks can possibly
equivocate.

The `Option.some.inj` calls below fix a separate, third bug: comparing
`hb1 : some cheaterId = some honestId` (an `Option NodeId` equality)
directly against `honestId_ne_cheaterId : honestId ≠ cheaterId` (a raw
`NodeId` inequality) is a type mismatch — `some a = some b` and `a = b`
are different propositions, even though logically equivalent.
`Option.some.inj` (Lean's auto-generated constructor-injectivity lemma)
strips the `some` first. -/
theorem honest_is_honest : HonestIn demoB honestId := by
  rintro ⟨b₁, b₂, ⟨hb1mem, hb2mem, hcreator, hne, -, -⟩, hb1⟩
  have hb1g : b₁ = g.id := by
    rcases demoB_keys_cases hb1mem with rfl | rfl | rfl | rfl
    · rfl
    · rw [creatorOf_e1] at hb1
      exact absurd (Option.some.inj hb1.symm) honestId_ne_cheaterId
    · rw [creatorOf_e2] at hb1
      exact absurd (Option.some.inj hb1.symm) honestId_ne_cheaterId
    · rw [creatorOf_c] at hb1
      exact absurd (Option.some.inj hb1.symm) honestId_ne_observerId
  have hb2g : b₂ = g.id := by
    rw [hb1g, creatorOf_g] at hcreator
    rcases demoB_keys_cases hb2mem with rfl | rfl | rfl | rfl
    · rfl
    · rw [creatorOf_e1] at hcreator
      exact absurd (Option.some.inj hcreator) honestId_ne_cheaterId
    · rw [creatorOf_e2] at hcreator
      exact absurd (Option.some.inj hcreator) honestId_ne_cheaterId
    · rw [creatorOf_c] at hcreator
      exact absurd (Option.some.inj hcreator) honestId_ne_observerId
  exact hne (hb1g.trans hb2g.symm)

/-- Before insertion, Rust reconstructs `c`'s view from its declared
predecessors; both branches are present in that view. -/
theorem candidate_c_acknowledges_before_insert :
    CandidateAcknowledges demoBeforeC c e1.id e2.id := by
  constructor
  · exact ⟨e1.id, by simp [c, contentC], observes_refl demoBeforeC e1.id⟩
  · exact ⟨e2.id, by simp [c, contentC], observes_refl demoBeforeC e2.id⟩

theorem candidate_c_acknowledges_after_insert :
    CandidateAcknowledges demoB c e1.id e2.id := by
  constructor
  · exact ⟨e1.id, by simp [c, contentC], observes_refl demoB e1.id⟩
  · exact ⟨e2.id, by simp [c, contentC], observes_refl demoB e2.id⟩

/-- The pre-insertion view bridges to ordinary observation after `c` is
inserted under its own identifier. This witness makes the exclusion demo
non-vacuous. -/
theorem c_acknowledges : Acknowledges demoB c.id e1.id e2.id :=
  candidateAcknowledges_of_inserted demoB c e1.id e2.id lookup_c
    candidate_c_acknowledges_after_insert

theorem demo_exclusion :
    ¬ VouchesFor demoB c.id e1.id ∧ ¬ VouchesFor demoB c.id e2.id :=
  equivocation_exclusion demoB demoB_valid e1.id e2.id c.id e1_e2_equivocate c_acknowledges

/-- The same demo, through `equivocation_not_approved` instead: a
stand-in `Approves` (here, literally `VouchesFor` itself, so the
implication hypotheses are trivial `id`s) shows the shape Issue 04 would
actually plug its own, real `Approves` and implication proofs into. -/
theorem demo_exclusion_via_approves :
    ¬ VouchesFor demoB c.id e1.id ∧ ¬ VouchesFor demoB c.id e2.id :=
  equivocation_not_approved (Approves := VouchesFor) demoB demoB_valid e1.id e2.id c.id
    e1_e2_equivocate c_acknowledges id id

end WorkedExample

end CordialMiners
