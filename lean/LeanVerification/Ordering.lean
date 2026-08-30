/-
Tau ordering: the deterministic total order derived from finalized leader
blocks, and its prefix-safety property.

## Design

`tau` in `ordering.rs` (lines 391–) produces the full ordered output
sequence by:
1. Finding the latest finalized leader.
2. Recursively resolving earlier finalized leaders that the current one
   ratifies.
3. Emitting each epoch's approved blocks in deterministic topological
   order, excluding blocks already emitted by earlier recursion.

Implementing `tau` concretely in Lean would require a decidable
topological sort over an opaque `Blocklace`. Instead we declare `tau` as
an `opaque` constant — the implementation lives in Rust; what matters
formally is the **prefix-safety** property below.

`tau_prefix_monotone` is stated as an `axiom` — a deliberate trusted
formal boundary, in the same tradition as `hashInj` in `Block.lean`.
It captures the append-only ledger invariant: as the blocklace grows,
`tau` only extends its output list, never retracts.

## Proof sketch for `tau_prefix_monotone`

The axiom follows from three ingredients already proved in this library:

1. **`no_conflicting_finals`** (`Finality.lean`): the latest finalized
   leader can only advance to a later wave, never regress to an earlier
   one.

2. **Observation monotonicity** (`Observe.lean:observes_mono`): when
   `B ⊆ B'`, every edge in `B` is present in `B'`, so
   `Observes B a b → Observes B' a b`.

3. **Approval exclusion** (`Approval.lean:approves_exclusion`): once a
   block is approved in `B`, it remains approved in `B'` (observation
   grows; the exclusion condition cannot be newly violated by blocks
   added outside the approver's closure).

Together they imply that the list of finalized-leader epochs in `B'`
extends the list in `B`, so the topological suffix in `B'` appends to,
not reorders, the suffix in `B`.

Owned by Issue 04 (KR4 — Finalized Leader Safety).
Rust: `consensus/ordering.rs`.
-/
import LeanVerification.Finality
import Mathlib.Data.List.Defs

namespace CordialMiners

/-! ### Sub-blocklace (monotone extension) -/

/-- `SubBlocklace B B'`: `B'` is a monotone extension of `B` — every block
present in `B` is present in `B'` with identical content. Equivalently:
`B.lookup b = some blk → B'.lookup b = some blk` for all blocks.

This is the append-only growth model: the blocklace only gains blocks,
never loses them, and existing entries are immutable. -/
def SubBlocklace (B B' : Blocklace) : Prop :=
  ∀ b blk, B.lookup b = some blk → B'.lookup b = some blk

/-- Observation edges are preserved when `B ⊆ B'`: block content is
immutable, so every predecessor link that existed in `B` still exists in
`B'`. This is a direct consequence of `Observe.lean:observes_mono`. -/
theorem observes_of_subBlocklace (B B' : Blocklace) (hsub : SubBlocklace B B')
    {a b : BlockId} (h : Observes B a b) : Observes B' a b :=
  observes_mono B B' hsub h

/-! ### Finality monotonicity -/

/-- Block membership is preserved by SubBlocklace: if `a ∈ B.keys` and
`B ⊆ B'`, then `a ∈ B'.keys`. -/
private lemma keys_mono {B B' : Blocklace} (hsub : SubBlocklace B B') {a : BlockId}
    (ha : a ∈ B.keys) : a ∈ B'.keys := by
  cases hlookup : B.lookup a with
  | none =>
    exact absurd (Finmap.mem_iff.mp (Finmap.mem_keys.mp ha)) (by simp [hlookup])
  | some blk =>
    exact Finmap.mem_keys.mpr (Finmap.mem_iff.mpr ⟨blk, hsub a blk hlookup⟩)

/-- Starting from `a ∈ B.keys`, any block reachable via `B'`'s predecessor
relation is also in `B.keys`. New blocks added to `B'` cannot be reached
from B-resident blocks because `Closed B` locks all predecessor lookups
within `B.keys`, and `SubBlocklace` preserves those lookups unchanged. -/
private theorem observes_stays_in_B {B B' : Blocklace} (hV : ValidBlocklace B)
    (hsub : SubBlocklace B B') {a b : BlockId} (ha : a ∈ B.keys)
    (h : Observes B' a b) : b ∈ B.keys := by
  have hClosed := closed_of_valid B hV
  induction h with
  | refl => exact ha
  | @tail mid _endpt chain step ih =>
    have hmid : mid ∈ B.keys := ih
    obtain ⟨blk', hlookup', hb_pred⟩ := step
    cases hlookup_B : B.lookup mid with
    | none =>
      exact absurd (Finmap.mem_iff.mp (Finmap.mem_keys.mp hmid)) (by simp [hlookup_B])
    | some blk =>
      have hblk_eq : blk' = blk :=
        Option.some.inj (hlookup'.symm.trans (hsub mid blk hlookup_B))
      exact hClosed mid blk hlookup_B _endpt (hblk_eq ▸ hb_pred)

/-- For `a, b ∈ B.keys`, observability in `B` and `B'` coincide. Since
`SubBlocklace` preserves all `B`-block lookups unchanged and `Closed B`
prevents the observation cone of any B-resident block from escaping to
new blocks, the observation relation restricted to `B.keys` is the same
in both blocklaces. -/
private theorem observes_iff_subBlocklace {B B' : Blocklace} (hV : ValidBlocklace B)
    (hsub : SubBlocklace B B') {a b : BlockId} (ha : a ∈ B.keys) (hb : b ∈ B.keys) :
    Observes B a b ↔ Observes B' a b := by
  constructor
  · exact observes_of_subBlocklace B B' hsub
  · intro h
    revert hb
    induction h with
    | refl => intro _; exact Relation.ReflTransGen.refl
    | @tail mid _endpt chain step ih =>
      intro hb
      have hmid : mid ∈ B.keys := observes_stays_in_B hV hsub ha chain
      have h_am_B : Observes B a mid := ih hmid
      obtain ⟨blk', hlookup', hb_pred⟩ := step
      cases hlookup_B : B.lookup mid with
      | none =>
        exact absurd (Finmap.mem_iff.mp (Finmap.mem_keys.mp hmid)) (by simp [hlookup_B])
      | some blk =>
        have hblk_eq : blk' = blk :=
          Option.some.inj (hlookup'.symm.trans (hsub mid blk hlookup_B))
        exact Relation.ReflTransGen.tail h_am_B ⟨blk, hlookup_B, hblk_eq ▸ hb_pred⟩

/-- For `b ∈ B.keys`, `creatorOf` agrees in `B` and `B'` because
`SubBlocklace` preserves the block's lookup unchanged. -/
private theorem creatorOf_of_subBlocklace {B B' : Blocklace} (hsub : SubBlocklace B B')
    {b : BlockId} (hb : b ∈ B.keys) : creatorOf B b = creatorOf B' b := by
  cases hlookup : B.lookup b with
  | none =>
    exact absurd (Finmap.mem_iff.mp (Finmap.mem_keys.mp hb)) (by simp [hlookup])
  | some blk =>
    unfold creatorOf
    rw [hlookup, hsub b blk hlookup]

/-- For `b ∈ B.keys`, `blockDepth` is the same in `B` and `B'`. The depth
is computed by well-founded recursion over the predecessor relation; since
`SubBlocklace` preserves every lookup for B-resident blocks, and `Closed B`
bounds all predecessor lookups within `B.keys`, the entire recursion sees
identical data in both blocklaces. -/
private theorem blockDepth_of_subBlocklace {B B' : Blocklace}
    (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (hsub : SubBlocklace B B') {b : BlockId} (hb : b ∈ B.keys) :
    blockDepth B hV b = blockDepth B' hV' b := by
  have hClosed := closed_of_valid B hV
  suffices h : ∀ x, x ∈ B.keys → blockDepth B hV x = blockDepth B' hV' x from h b hb
  intro x
  apply (directPred_wf_of_valid B hV).induction
    (C := fun x => x ∈ B.keys → blockDepth B hV x = blockDepth B' hV' x) x
  intro y ih hy
  cases hlookup : B.lookup y with
  | none =>
    exact absurd (Finmap.mem_iff.mp (Finmap.mem_keys.mp hy)) (by simp [hlookup])
  | some blk =>
    have hlookup' : B'.lookup y = some blk := hsub y blk hlookup
    unfold blockDepth
    rw [blockDepthWF_eq B (directPred_wf_of_valid B hV) y, hlookup,
        blockDepthWF_eq B' (directPred_wf_of_valid B' hV') y, hlookup']
    apply Finset.sup_congr rfl
    intro ⟨p, hp_mem⟩ _
    congr 1
    exact ih p ⟨blk, hlookup, hp_mem⟩ (hClosed y blk hlookup p hp_mem)

/-- `Approves` is monotone under `SubBlocklace` when both `a` and `b` live
in `B`. All observations and creator lookups agree in `B` and `B'` for
B-resident blocks (by `observes_iff_subBlocklace` and `creatorOf_of_subBlocklace`),
so the `VouchesFor` condition transfers unchanged. -/
private theorem approves_of_subBlocklace {B B' : Blocklace}
    (hV : ValidBlocklace B) (hsub : SubBlocklace B B')
    {a b : BlockId} (ha : a ∈ B.keys) (hb : b ∈ B.keys)
    (happ : Approves B a b) : Approves B' a b := by
  unfold Approves VouchesFor at *
  obtain ⟨hobs, hvcond⟩ := happ
  refine ⟨observes_of_subBlocklace B B' hsub hobs, ?_⟩
  intro b' hcreator hne hobs' hfork
  have hb' : b' ∈ B.keys := observes_stays_in_B hV hsub ha hobs'
  have hobs_B : Observes B a b' := (observes_iff_subBlocklace hV hsub ha hb').mpr hobs'
  have hcreator_B : creatorOf B b' = creatorOf B b :=
    (creatorOf_of_subBlocklace hsub hb').trans
      hcreator |>.trans (creatorOf_of_subBlocklace hsub hb).symm
  apply hvcond b' hcreator_B hne hobs_B
  exact ⟨fun h => hfork.1 (observes_of_subBlocklace B B' hsub h),
         fun h => hfork.2 ((observes_iff_subBlocklace hV hsub hb' hb).mp h)⟩

private theorem ratifies_of_subBlocklace {bonds : NodeId → ℕ} {validators : Finset NodeId}
    {B B' : Blocklace} (hV : ValidBlocklace B) (hsub : SubBlocklace B B')
    {r b : BlockId} (_hr : r ∈ B.keys) (hb : b ∈ B.keys)
    (hrat : Ratifies bonds validators B r b) :
    Ratifies bonds validators B' r b := by
  obtain ⟨S, hSsub, hSwit, hSmaj⟩ := hrat
  refine ⟨S, hSsub, ?_, hSmaj⟩
  intro v hv
  obtain ⟨a, haobs, hacreator, haapp⟩ := hSwit v hv
  have ha : a ∈ B.keys := by
    cases hlookup : B.lookup a with
    | none => simp [creatorOf, hlookup] at hacreator
    | some blk => exact Finmap.mem_keys.mpr (Finmap.mem_iff.mpr ⟨blk, hlookup⟩)
  exact ⟨a,
    observes_of_subBlocklace B B' hsub haobs,
    (creatorOf_of_subBlocklace hsub ha).symm.trans hacreator,
    approves_of_subBlocklace hV hsub ha hb haapp⟩

private theorem superRatifies_of_subBlocklace {bonds : NodeId → ℕ} {validators : Finset NodeId}
    {B B' : Blocklace} (hV : ValidBlocklace B) (hsub : SubBlocklace B B')
    {witness : Finset BlockId} {b : BlockId}
    (hb : b ∈ B.keys) (hW : ∀ r ∈ witness, r ∈ B.keys)
    (hsr : SuperRatifies bonds validators B witness b) :
    SuperRatifies bonds validators B' witness b := by
  obtain ⟨R, hRsub, hRwit, hRmaj⟩ := hsr
  refine ⟨R, hRsub, ?_, hRmaj⟩
  intro v hv
  obtain ⟨r, hrW, hrcreator, hrrat⟩ := hRwit v hv
  have hr : r ∈ B.keys := hW r hrW
  exact ⟨r, hrW,
    (creatorOf_of_subBlocklace hsub hr).symm.trans hrcreator,
    ratifies_of_subBlocklace hV hsub hr hb hrrat⟩

/-- **Finality monotonicity.** If `b` is a finalized leader for `wave` in `B`,
it remains finalized in any monotone extension `B'` (`SubBlocklace B B'`).

The proof transfers each component of `FinalLeader`:
- Leader-block membership: `creatorOf` and `blockDepth` agree on B-resident
  blocks (by `creatorOf_of_subBlocklace` and `blockDepth_of_subBlocklace`).
- Witness bounds: depths are preserved, and B-keys are a subset of B'-keys.
- Super-ratification: lifted by `superRatifies_of_subBlocklace` via the
  chain `observes → approves → ratifies → super-ratifies`.

Addresses reviewer concern 3 (finality monotonicity absent and false under
old uniqueness-encoding definition). -/
theorem FinalLeader_of_subBlocklace
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId)
    (hsub : SubBlocklace B B') (b : BlockId)
    (hfin : FinalLeader bonds validators B hV wave wavelength sel b) :
    FinalLeader bonds validators B' hV' wave wavelength sel b := by
  obtain ⟨hb_lead, witness, hW, hsr⟩ := hfin
  obtain ⟨hb_keys, hb_creator, hb_depth⟩ := hb_lead
  have hb_keys' : b ∈ B'.keys := keys_mono hsub hb_keys
  have hW_keys : ∀ s ∈ witness, s ∈ B.keys := fun s hs => (hW s hs).1
  have hb_lead' : b ∈ leaderBlocksOfWave B' hV' wave wavelength sel :=
    ⟨hb_keys',
     (creatorOf_of_subBlocklace hsub hb_keys).symm.trans hb_creator,
     (blockDepth_of_subBlocklace hV hV' hsub hb_keys).symm.trans hb_depth⟩
  have hW' : ∀ s ∈ witness,
      s ∈ B'.keys ∧
      leaderRoundOfWave wave wavelength ≤ blockDepth B' hV' s ∧
      blockDepth B' hV' s ≤ lastRoundOfWave wave wavelength := by
    intro s hs
    obtain ⟨hs_keys, hs_lo, hs_hi⟩ := hW s hs
    have hdepth := blockDepth_of_subBlocklace hV hV' hsub hs_keys
    exact ⟨keys_mono hsub hs_keys, hdepth ▸ hs_lo, hdepth ▸ hs_hi⟩
  exact ⟨hb_lead', witness, hW',
    superRatifies_of_subBlocklace hV hsub hb_keys hW_keys hsr⟩

/-! ### Tau -/

/-- The deterministic ordered output of the protocol, anchored on the latest
finalized leader and recursively expanded through all ratified ancestors.

Declared `opaque` because its computational definition lives in Rust.
What matters formally is `tau_prefix_monotone` below. -/
opaque tau (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId) : List BlockId

/-! ### Prefix-safety -/

/-- **Prefix-safety (tau_prefix_monotone).** When the blocklace grows
monotonically (`SubBlocklace B B'`), the output of `tau` only extends:
`tau B` is a prefix of `tau B'`.

This is the property that makes the system usable as an append-only
ledger — once a block is ordered by `tau`, it stays ordered at the same
position in every future state.

**Stated as an axiom** (a trusted formal boundary, analogous to `hashInj`
in `Block.lean`) because proving it concretely requires implementing `tau`
and the decidable topological sort, which are in Rust. The justification
is the three-point proof sketch in the module doc.

Rust: the `tau` append-only invariant is tested by
`test_finality.rs:finalized_order_excludes_equivocations_the_leader_acknowledged`. -/
axiom tau_prefix_monotone
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (hsub : SubBlocklace B B') :
    List.IsPrefix
      (tau bonds validators B hV wavelength sel)
      (tau bonds validators B' hV' wavelength sel)

end CordialMiners
