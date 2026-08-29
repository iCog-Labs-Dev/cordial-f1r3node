/-
Equivocation and exclusion (KR2): the one Byzantine behavior Cordial
Miners cares about — a validator producing two blocks that sit side by
side in the DAG, neither observing the other — and the structural
guarantee that no single block can vouch for both sides of it.

Owned by Issue 03 (KR2 — Equivocation and Exclusion).

## Round-free by design (see review history)

An earlier version of this file introduced a `CreatorRound` typeclass
bundling a `roundOf : Blocklace → BlockId → Option ℕ` projection plus an
axiom, `round_lt_of_observes`, asserting (unproved) that round strictly
decreases along `Observes`. Review feedback on that version was direct:
since `round` was a new concept being introduced *by this file*, it
should either be proved, not assumed — or better, not introduced at all
if the file doesn't actually need it. It doesn't: equivocation only needs
same creator, distinct blocks, and incomparability under `Observes`, none
of which requires a round number anywhere. So this version drops
`CreatorRound`/`roundOf` entirely rather than trying to prove
`round_lt_of_observes` — there is no "round" concept left in this file to
have opinions about.

This also happens to fix a real, distinct bug the round-based version had
that showed up as `lake build` failures once the file finally got past an
earlier (unrelated) `lake update`-caused build stall: `roundOf`'s own
argument types (`Blocklace`, `BlockId`) never mentioned `Validator`, so
Lean had no way to infer which `CreatorRound Validator` instance a bare
call like `roundOf B b` should resolve against. That surfaced as
"Application type mismatch ... `@roundOf B`" and "typeclass instance
problem is stuck" errors at several call sites, and very likely explains
the earlier multi-hour build hang too (unconstrained typeclass search on
`CreatorRound ?m` backtracking extensively before ever reporting
anything). Going round-free removes the typeclass entirely, so there is
no instance-resolution ambiguity left to hang or misfire on.

## Concrete types now, not a placeholder signature

Issue 02's `Block.lean`/`Blocklace.lean`/`Observe.lean` have all landed,
so this file no longer develops against an abstract stand-in signature —
`Blocklace`, `BlockId`, `Block`, and `Observes` are the real, concrete
definitions. In particular `Block.creator : NodeId` (with
`NodeId := Nat`) means there is no generic `Validator` type parameter
here at all; `NodeId` is used directly. `creatorOf` below is a plain,
ordinary function (`Blocklace → BlockId → Option NodeId`), not a class
field, so calling it is never ambiguous.

Equivocation is defined structurally, via incomparability under
`Observes`, rather than via a round number — this matches how
`consensus/approval.rs`'s `approves` actually excludes conflicting blocks
(any observed, incomparable block by the same creator, not only
same-round ones) more closely than a same-round-only definition would.
It is phrased directly in terms of `Observes` rather than `Precedes`/
`PrecedesOrEquals`: since `b₁ ≠ b₂` is already tracked as a separate
conjunct, `Observes` alone is enough to express incomparability, and
using it directly avoids depending on an (as yet unproved, in this
project) bridge lemma connecting `Precedes` to `Observes` for distinct
blocks.
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

/-! ### Equivocation -/

/-- `b₁` and `b₂` equivocate in `B`: both present, same creator, distinct,
and incomparable under `Observes B` — neither is in the other's causal
history. This is the structural counterpart of
`equivocation_blocks_at_round` (`consensus/cordiality.rs`, lines 60–71),
generalized from "same round" to "incomparable", which is the condition
`approves` (`consensus/approval.rs`) actually tests. No round number is
involved. -/
def Equivocation (B : Blocklace) (b₁ b₂ : BlockId) : Prop :=
  b₁ ∈ B.keys ∧ b₂ ∈ B.keys ∧
  creatorOf B b₁ = creatorOf B b₂ ∧ b₁ ≠ b₂ ∧
  ¬ Observes B b₁ b₂ ∧ ¬ Observes B b₂ b₁

/-- `v` is an equivocator in `B`: some pair of `v`'s blocks in `B`
equivocate. Mirrors `all_equivocations` (`consensus/cordiality.rs`, lines
74–111) reporting a nonempty branch set for `v`. -/
def Equivocator (B : Blocklace) (v : NodeId) : Prop :=
  ∃ b₁ b₂, Equivocation B b₁ b₂ ∧ creatorOf B b₁ = some v

/-- `v` is honest in `B`: `v` has not equivocated. Mirrors
`equivocation_blocks_at_round` returning the empty set for `v` at every
round it's checked, generalized (as `Equivocation` generalizes it) to
"everywhere in `B`", not "at every round". -/
def HonestIn (B : Blocklace) (v : NodeId) : Prop :=
  ¬ Equivocator B v

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
theorem equivocation_exclusion (B : Blocklace) (b₁ b₂ c : BlockId)
    (heq : Equivocation B b₁ b₂) (hack : Acknowledges B c b₁ b₂) :
    ¬ VouchesFor B c b₁ ∧ ¬ VouchesFor B c b₂ := by
  obtain ⟨-, -, hcreator, hne, hnob12, hnob21⟩ := heq
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
    (B : Blocklace) (b₁ b₂ c : BlockId)
    {Approves : Blocklace → BlockId → BlockId → Prop}
    (heq : Equivocation B b₁ b₂) (hack : Acknowledges B c b₁ b₂)
    (hApproveImpliesVouch₁ : Approves B c b₁ → VouchesFor B c b₁)
    (hApproveImpliesVouch₂ : Approves B c b₂ → VouchesFor B c b₂) :
    ¬ Approves B c b₁ ∧ ¬ Approves B c b₂ :=
  let ⟨hv₁, hv₂⟩ := equivocation_exclusion B b₁ b₂ c heq hack
  ⟨fun h => hv₁ (hApproveImpliesVouch₁ h), fun h => hv₂ (hApproveImpliesVouch₂ h)⟩

section WorkedExample

/-- The honest validator. -/
def honestId : NodeId := 0

/-- The cheater. -/
def cheaterId : NodeId := 1

theorem honestId_ne_cheaterId : honestId ≠ cheaterId := by decide

/-- Distinct payloads are only needed to keep `contentE1 ≠ contentE2`,
which (via `hashInj`) is what makes `e1.id ≠ e2.id`. -/
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

/-- The demo blocklace: exactly `g`, `e1`, `e2`, nothing else.

NOTE: `(∅ : Blocklace)` does not elaborate directly. `Blocklace` is a
plain `def` (`Finmap (fun _ : BlockId => Block)`, not `abbrev`), and
typeclass search for `EmptyCollection Blocklace` does not unfold plain
`def`s to find `Finmap`'s own instance. Ascribing the empty value at the
underlying `Finmap` type first, and letting it unify against the expected
`Blocklace` type via `demoB`'s own `: Blocklace` signature, sidesteps
that: term-level unification against an expected type *does* unfold
plain `def`s, unlike instance search. -/
def demoB : Blocklace :=
  (((∅ : Finmap (fun _ : BlockId => Block)).insert e1.id e1).insert e2.id e2).insert g.id g

theorem lookup_g : demoB.lookup g.id = some g := by
  simp [demoB, Finmap.lookup_insert]

theorem lookup_e1 : demoB.lookup e1.id = some e1 := by
  simp [demoB, Finmap.lookup_insert, Finmap.lookup_insert_of_ne,
    g_ne_e1.symm, e1_ne_e2]

theorem lookup_e2 : demoB.lookup e2.id = some e2 := by
  simp [demoB, Finmap.lookup_insert, Finmap.lookup_insert_of_ne, g_ne_e2.symm]

/-- Membership proofs built the way `insertPreservesClosed` in
`Blocklace.lean` already does it — chaining `Finmap.mem_insert`
(Finmap-level membership) and converting to `.keys`-level via
`Finmap.mem_keys` at the end — rather than going from "lookup succeeds"
to membership via an anonymous constructor. `a ∈ s` for a `Finmap`
unfolds through a `Quot.lift` internally (it's backed by a `Multiset`),
so it is not an inductive `Exists` that `⟨witness, proof⟩` syntax can
target; that mismatch, not the earlier `sorry` cascade, was the second
real bug in this section. -/
theorem g_mem : g.id ∈ demoB.keys :=
  Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inl rfl))

theorem e2_mem : e2.id ∈ demoB.keys :=
  Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inr
    (Finmap.mem_insert.mpr (Or.inl rfl))))

theorem e1_mem : e1.id ∈ demoB.keys :=
  Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inr
    (Finmap.mem_insert.mpr (Or.inr (Finmap.mem_insert.mpr (Or.inl rfl))))))

/-- `simp` (not `simp only`) deliberately, so the default simp set
supplies whatever this Mathlib version's empty-`Finmap`-membership lemma
is actually named, rather than this file having to guess and hard-code
it. -/
theorem demoB_keys_cases {b : BlockId} (hb : b ∈ demoB.keys) :
    b = g.id ∨ b = e1.id ∨ b = e2.id := by
  simp [demoB, Finmap.mem_keys, Finmap.mem_insert] at hb
  tauto

theorem creatorOf_g : creatorOf demoB g.id = some honestId := by
  simp only [creatorOf, lookup_g]; rfl

theorem creatorOf_e1 : creatorOf demoB e1.id = some cheaterId := by
  simp only [creatorOf, lookup_e1]; rfl

theorem creatorOf_e2 : creatorOf demoB e2.id = some cheaterId := by
  simp only [creatorOf, lookup_e2]; rfl

/-- No block in `demoB` has a predecessor: every one of `g`, `e1`, `e2`
was built with `predecessors := ∅`, and those are the only three blocks
`demoB` contains. Case-splits on `x` directly via `eq_or_ne` rather than
trying to derive `x ∈ demoB.keys` from `hlookup` through a membership
bridge lemma — same reason `g_mem` above avoids the anonymous
constructor — since that sidesteps needing to know the exact name of
whichever "lookup succeeds → key present" lemma this Mathlib version
ships, which this section doesn't otherwise need. The three
`Finmap.lookup_insert_of_ne _ hx*` arguments mirror
`insertPreservesClosed`'s `lookup_insert_of_ne B hid` call in
`Blocklace.lean` exactly: the inequality goes lookup-key ≠ insert-key,
with no `.symm`. -/
theorem demoB_directPred_false (x y : BlockId) : ¬ DirectPred demoB x y := by
  rintro ⟨blk, hlookup, hp⟩
  rcases eq_or_ne x g.id with rfl | hxg
  · rw [lookup_g] at hlookup; cases hlookup; simp [g, contentG] at hp
  rcases eq_or_ne x e1.id with rfl | hxe1
  · rw [lookup_e1] at hlookup; cases hlookup; simp [e1, contentE1] at hp
  rcases eq_or_ne x e2.id with rfl | hxe2
  · rw [lookup_e2] at hlookup; cases hlookup; simp [e2, contentE2] at hp
  · have hnone : demoB.lookup x = none := by
      simp [demoB, Finmap.lookup_insert_of_ne _ hxg,
        Finmap.lookup_insert_of_ne _ hxe1, Finmap.lookup_insert_of_ne _ hxe2]
    rw [hnone] at hlookup
    exact absurd hlookup (by simp)

/-- With no predecessor edges at all, `Observes demoB` only ever relates
a block to itself. -/
theorem demoB_observes_iff (x y : BlockId) : Observes demoB x y ↔ x = y := by
  constructor
  · intro h
    rcases Relation.ReflTransGen.cases_head h with hEq | ⟨c, hxc, -⟩
    · exact hEq
    · exact absurd hxc (demoB_directPred_false x c)
  · rintro rfl
    exact observes_refl demoB x

theorem not_observes_e1_e2 : ¬ Observes demoB e1.id e2.id := by
  rw [demoB_observes_iff]; exact e1_ne_e2

theorem not_observes_e2_e1 : ¬ Observes demoB e2.id e1.id := by
  rw [demoB_observes_iff]; exact e1_ne_e2.symm

/-- `e1` and `e2` are an explicit, concrete equivocation. -/
theorem e1_e2_equivocate : Equivocation demoB e1.id e2.id :=
  ⟨e1_mem, e2_mem, creatorOf_e1.trans creatorOf_e2.symm, e1_ne_e2,
    not_observes_e1_e2, not_observes_e2_e1⟩

/-- `HonestIn` evaluates false for the cheater. -/
theorem cheater_not_honest : ¬ HonestIn demoB cheaterId := fun h =>
  h ⟨e1.id, e2.id, e1_e2_equivocate, creatorOf_e1⟩

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
    rcases demoB_keys_cases hb1mem with rfl | rfl | rfl
    · rfl
    · rw [creatorOf_e1] at hb1
      exact absurd (Option.some.inj hb1.symm) honestId_ne_cheaterId
    · rw [creatorOf_e2] at hb1
      exact absurd (Option.some.inj hb1.symm) honestId_ne_cheaterId
  have hb2g : b₂ = g.id := by
    rw [hb1g, creatorOf_g] at hcreator
    rcases demoB_keys_cases hb2mem with rfl | rfl | rfl
    · rfl
    · rw [creatorOf_e1] at hcreator
      exact absurd (Option.some.inj hcreator) honestId_ne_cheaterId
    · rw [creatorOf_e2] at hcreator
      exact absurd (Option.some.inj hcreator) honestId_ne_cheaterId
  exact hne (hb1g.trans hb2g.symm)

/-- Neither `honestId` nor `cheaterId` can be vouched for on both sides
of the cheat at once: any node that acknowledges the equivocation
(observes both `e1` and `e2`) cannot cleanly vouch for either. -/
theorem demo_exclusion (c : BlockId) (hack : Acknowledges demoB c e1.id e2.id) :
    ¬ VouchesFor demoB c e1.id ∧ ¬ VouchesFor demoB c e2.id :=
  equivocation_exclusion demoB e1.id e2.id c e1_e2_equivocate hack

/-- The same demo, through `equivocation_not_approved` instead: a
stand-in `Approves` (here, literally `VouchesFor` itself, so the
implication hypotheses are trivial `id`s) shows the shape Issue 04 would
actually plug its own, real `Approves` and implication proofs into. -/
theorem demo_exclusion_via_approves (c : BlockId)
    (hack : Acknowledges demoB c e1.id e2.id) :
    ¬ VouchesFor demoB c e1.id ∧ ¬ VouchesFor demoB c e2.id :=
  equivocation_not_approved (Approves := VouchesFor) demoB e1.id e2.id c e1_e2_equivocate hack id
    id

end WorkedExample

end CordialMiners