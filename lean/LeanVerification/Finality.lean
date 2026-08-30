/-
Finalized-leader safety: waves, leader selection, and the core safety
theorem that two distinct blocks cannot both be finalized for the same wave.

## Wave model

Rounds are partitioned into contiguous chunks of size `wavelength > 0`.
The first round of each wave is the **leader round**; the leader for wave `w`
is given by `sel w : Option NodeId` (mirrors Rust's `Fn(u64) → Option<NodeId>`;
`none` means no leader is elected for that wave).

## Finalized leader

`FinalLeader bonds validators B hV wave wavelength sel b` encodes the
Rust `final_leader_for_wave` pipeline:

- `b` is **present** in `B` with the correct creator and leader-round depth.
- A **witness** block set within the wave super-ratifies `b` under the
  given bond weights.

Uniqueness is NOT part of the definition — it is a *consequence* proved
by the safety theorem using the quorum intersection argument.

## Safety theorem

`no_conflicting_finals` states that two blocks satisfying `FinalLeader`
for the same wave are equal. The proof:

1. b₁ ≠ b₂ are both leader blocks → same creator, same depth → same-depth
   incomparable (by `Equivocation.same_depth_incomparable`) → Fork → Equivocation.
2. Each has a super-ratification witness giving a 2/3 ratifier set.
3. By `finset_honest_triple_intersection` (three-quorum argument), some
   honest validator v* ratified BOTH b₁ and b₂.
4. v* has two ratifying blocks; by its honesty they are comparable.
5. The heavier ratifier observes both sets of approvers for b₁ and b₂.
6. A second quorum intersection gives an honest approver a* of both.
7. a* Approves b₁ and b₂, so Acknowledges both branches.
8. By `approves_exclusion` (KR2), ¬ Approves a* b₁ — contradiction.

Owned by Issue 04 (KR4 — Finalized Leader Safety).
Rust: `consensus/wave.rs`, `consensus/finality.rs`.
-/
import LeanVerification.Approval

namespace CordialMiners

/-! ### Wave arithmetic -/

/-- The wave number containing a given round (zero-indexed, chunk size `wavelength`).
Mirrors `wave.rs:wave_of_round`. -/
def waveOfRound (round wavelength : ℕ) : ℕ := round / wavelength

/-- The first (leader) round of a wave: `wave * wavelength`.
Mirrors `wave.rs:first_round_of_wave` = `wave.rs:leader_round_of_wave`. -/
def leaderRoundOfWave (wave wavelength : ℕ) : ℕ := wave * wavelength

/-- The last round of a wave: `wave * wavelength + wavelength - 1`.
Mirrors `wave.rs:last_round_of_wave`. Requires `0 < wavelength` to
avoid underflow in ℕ arithmetic (identical to Rust's check). -/
def lastRoundOfWave (wave wavelength : ℕ) : ℕ := wave * wavelength + wavelength - 1

/-- `waveOfRound` and `leaderRoundOfWave` are inverses when `wavelength > 0`. -/
lemma waveOfRound_leaderRound (wave wavelength : ℕ) (hwl : 0 < wavelength) :
    waveOfRound (leaderRoundOfWave wave wavelength) wavelength = wave :=
  Nat.mul_div_cancel wave hwl

/-- The leader round is always within the wave's extent (requires `0 < wavelength`). -/
lemma leaderRound_le_lastRound (wave wavelength : ℕ) (hwl : 0 < wavelength) :
    leaderRoundOfWave wave wavelength ≤ lastRoundOfWave wave wavelength := by
  unfold leaderRoundOfWave lastRoundOfWave; omega

/-! ### Leader blocks -/

/-- The set of blocks that qualify as leader blocks for `wave`:
present in `B`, created by `sel wave` (if the wave has a leader),
and at the leader round's DAG depth.

`sel : ℕ → Option NodeId` mirrors Rust's `Fn(u64) → Option<NodeId>`.
When `sel wave = none`, the set is empty because `creatorOf B b` is always
`some v` for any present block `b`. -/
def leaderBlocksOfWave (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧
       creatorOf B b = sel wave ∧
       blockDepth B hV b = leaderRoundOfWave wave wavelength}

/-! ### Leader-blocks form an equivocation when distinct -/

/-- Any two distinct leader blocks for the same wave form an Equivocation.
This is the key structural fact that lets the quorum-intersection argument
apply: the two finalized candidates are not just "different" in some
abstract sense but specifically conflict in the sense of KR2. -/
lemma leaderBlocks_equivocation (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId)
    (b₁ b₂ : BlockId)
    (hb₁ : b₁ ∈ leaderBlocksOfWave B hV wave wavelength sel)
    (hb₂ : b₂ ∈ leaderBlocksOfWave B hV wave wavelength sel)
    (hne : b₁ ≠ b₂) : Equivocation B hV b₁ b₂ := by
  obtain ⟨hmem₁, hcreator₁, hdepth₁⟩ := hb₁
  obtain ⟨hmem₂, hcreator₂, hdepth₂⟩ := hb₂
  have hdepthEq : blockDepth B hV b₁ = blockDepth B hV b₂ := hdepth₁.trans hdepth₂.symm
  obtain ⟨hnobs₁₂, hnobs₂₁⟩ := same_depth_incomparable B hV b₁ b₂ hne hdepthEq
  exact ⟨⟨hmem₁, hmem₂, hcreator₁.trans hcreator₂.symm, hne, hnobs₁₂, hnobs₂₁⟩, hdepthEq⟩

/-! ### Finalized leader -/

/-- `b` is a finalized leader for `wave` in `B`:
1. `b` is a leader block for the wave (present, correct creator, leader depth).
2. There exists a witness block set within the wave whose blocks
   super-ratify `b` under the given bond weights.

Uniqueness is NOT encoded here — it is a consequence of protocol safety
proved by `no_conflicting_finals` using the quorum intersection argument.

Mirrors `finality.rs:is_weighted_final_leader` + `final_leader_for_wave`. -/
def FinalLeader (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId) (b : BlockId) : Prop :=
  b ∈ leaderBlocksOfWave B hV wave wavelength sel ∧
  ∃ witness : Finset BlockId,
    (∀ s ∈ witness,
      s ∈ B.keys ∧
      leaderRoundOfWave wave wavelength ≤ blockDepth B hV s ∧
      blockDepth B hV s ≤ lastRoundOfWave wave wavelength) ∧
    SuperRatifies bonds validators B witness b

/-! ### Safety theorem -/

/-- **Finalized-leader safety.**  Two blocks that are both `FinalLeader`
for the same wave must be equal.

**Proof uses** (in order): `leaderBlocks_equivocation` (KR2 equivocation),
`finset_honest_triple_intersection` (quorum intersection at the ratifier
level), `honest_chain_linearity` (honest validator's blocks are comparable),
`finset_honest_triple_intersection` again (quorum intersection at the
approver level), and `approves_exclusion` (KR2 exclusion property). -/
theorem no_conflicting_finals
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (_hwl : 0 < wavelength)
    (sel : ℕ → Option NodeId)
    -- Honest kernel: validators with positive bond who don't equivocate.
    (honestNodes : Finset NodeId)
    (hH_sub : honestNodes ⊆ validators)
    (hH_maj : StrictTwoThirdsMaj bonds validators honestNodes)
    (hHonest : ∀ v ∈ honestNodes, HonestIn B v)
    (b₁ b₂ : BlockId)
    (hb₁ : FinalLeader bonds validators B hV wave wavelength sel b₁)
    (hb₂ : FinalLeader bonds validators B hV wave wavelength sel b₂) :
    b₁ = b₂ := by
  -- Obtain leader-block membership and super-ratification witnesses.
  obtain ⟨hb₁_lead, W₁, _, hsr₁⟩ := hb₁
  obtain ⟨hb₂_lead, W₂, _, hsr₂⟩ := hb₂
  -- Prove by contradiction: assume b₁ ≠ b₂.
  by_contra hne
  -- The two leader blocks form an equivocation (same creator, same depth,
  -- both present, distinct → incomparable by same_depth_incomparable).
  have heqv : Equivocation B hV b₁ b₂ :=
    leaderBlocks_equivocation B hV wave wavelength sel b₁ b₂ hb₁_lead hb₂_lead hne
  -- Extract the two ratifier sets R₁ and R₂.
  obtain ⟨R₁, hR₁_sub, hR₁_wit, hR₁_maj⟩ := hsr₁
  obtain ⟨R₂, hR₂_sub, hR₂_wit, hR₂_maj⟩ := hsr₂
  -- First quorum intersection: find an honest ratifier v* ∈ R₁ ∩ R₂ ∩ H.
  obtain ⟨v_rat, hv_rat_mem⟩ :=
    finset_honest_triple_intersection bonds validators honestNodes R₁ R₂
      hH_sub hR₁_sub hR₂_sub hH_maj hR₁_maj hR₂_maj
  have hv_rat_R₁ : v_rat ∈ R₁ := (Finset.mem_inter.mp (Finset.mem_inter.mp hv_rat_mem).1).1
  have hv_rat_R₂ : v_rat ∈ R₂ := (Finset.mem_inter.mp (Finset.mem_inter.mp hv_rat_mem).1).2
  have hv_rat_H  : v_rat ∈ honestNodes := (Finset.mem_inter.mp hv_rat_mem).2
  -- v* ratified b₁ via some block r₁ in W₁.
  obtain ⟨r₁, hr₁_W₁, hr₁_creator, hrat₁⟩ := hR₁_wit v_rat hv_rat_R₁
  -- v* ratified b₂ via some block r₂ in W₂.
  obtain ⟨r₂, hr₂_W₂, hr₂_creator, hrat₂⟩ := hR₂_wit v_rat hv_rat_R₂
  -- Extract approver sets S₁ (approvers of b₁ seen by r₁) and S₂ (for b₂ by r₂).
  obtain ⟨S₁, hS₁_sub, hS₁_wit, hS₁_maj⟩ := hrat₁
  obtain ⟨S₂, hS₂_sub, hS₂_wit, hS₂_maj⟩ := hrat₂
  -- S₁ and S₂ both hold 2/3 supermajorities over validators, so a second
  -- quorum intersection gives an honest approver a* ∈ S₁ ∩ S₂ ∩ H.
  obtain ⟨a_app, ha_app_mem⟩ :=
    finset_honest_triple_intersection bonds validators honestNodes S₁ S₂
      hH_sub hS₁_sub hS₂_sub hH_maj hS₁_maj hS₂_maj
  have ha_app_S₁ : a_app ∈ S₁ := (Finset.mem_inter.mp (Finset.mem_inter.mp ha_app_mem).1).1
  have ha_app_S₂ : a_app ∈ S₂ := (Finset.mem_inter.mp (Finset.mem_inter.mp ha_app_mem).1).2
  have ha_app_H  : a_app ∈ honestNodes := (Finset.mem_inter.mp ha_app_mem).2
  -- From S₁: there's a₁ by a_app that observes b₁ and approves b₁.
  obtain ⟨a₁, ha₁_obs, ha₁_creator, ha₁_app⟩ := hS₁_wit a_app ha_app_S₁
  -- From S₂: there's a₂ by a_app that observes b₂ and approves b₂.
  obtain ⟨a₂, ha₂_obs, ha₂_creator, ha₂_app⟩ := hS₂_wit a_app ha_app_S₂
  -- a_app is honest; all its blocks form a chain under Observes.
  have ha_app_honest : HonestIn B a_app := hHonest a_app ha_app_H
  have hchain := honest_chain_linearity B a_app ha_app_honest
  -- Approves = VouchesFor, so .1 gives the observation.
  have hobs_a₁_b₁ : Observes B a₁ b₁ := ha₁_app.1
  have hobs_a₂_b₂ : Observes B a₂ b₂ := ha₂_app.1
  -- Prove a₁ ∈ B.keys from creatorOf B a₁ = some a_app.
  -- Use cases on B.lookup first (before any simp) to avoid Lean's semireducible
  -- transparency issue with Blocklace when using rw [Finmap.mem_keys].
  have ha₁_mem : a₁ ∈ B.keys := by
    cases hlookup₁ : B.lookup a₁ with
    | none =>
      have : creatorOf B a₁ = none := by simp [creatorOf, hlookup₁]
      simp [this] at ha₁_creator
    | some blk => exact Finmap.mem_keys.mpr (Finmap.mem_iff.mpr ⟨blk, hlookup₁⟩)
  have ha₂_mem : a₂ ∈ B.keys := by
    cases hlookup₂ : B.lookup a₂ with
    | none =>
      have : creatorOf B a₂ = none := by simp [creatorOf, hlookup₂]
      simp [this] at ha₂_creator
    | some blk => exact Finmap.mem_keys.mpr (Finmap.mem_iff.mpr ⟨blk, hlookup₂⟩)
  -- Case split: a₁ = a₂, or comparable via honest chain linearity.
  rcases eq_or_ne a₁ a₂ with rfl | hane
  · -- Same block sees both b₁ and b₂ → acknowledges the equivocation.
    exact (approves_exclusion B hV b₁ b₂ a₁ heqv ⟨hobs_a₁_b₁, hobs_a₂_b₂⟩).1 ha₁_app
  · -- a₁ ≠ a₂: by honesty of a_app they are comparable.
    have ha₁_in : a₁ ∈ blocksBy B a_app := ⟨ha₁_mem, ha₁_creator⟩
    have ha₂_in : a₂ ∈ blocksBy B a_app := ⟨ha₂_mem, ha₂_creator⟩
    rcases hchain ha₁_in ha₂_in hane with h12 | h21
    · -- a₁ observes a₂ → a₁ acknowledges both branches.
      exact (approves_exclusion B hV b₁ b₂ a₁ heqv
        ⟨hobs_a₁_b₁, observes_trans B h12 hobs_a₂_b₂⟩).1 ha₁_app
    · -- a₂ observes a₁ → a₂ acknowledges both branches.
      exact (approves_exclusion B hV b₁ b₂ a₂ heqv
        ⟨observes_trans B h21 hobs_a₁_b₁, hobs_a₂_b₂⟩).2 ha₂_app

end CordialMiners
