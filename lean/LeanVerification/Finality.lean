/-
Finalized-leader safety: waves, leader selection, and the core safety
theorem that two distinct blocks cannot both be finalized for the same wave.

## Wave model

Rounds are partitioned into contiguous chunks of size `wavelength`. The
first round of each wave is the **leader round**; the leader validator for
that wave is given by `sel : ℕ → NodeId`. A leader block is any block at
the leader round by the selected leader.

## Finalized leader

`FinalLeader bonds validators B hV wave wavelength sel b` encodes the
Rust `final_leader_for_wave` pipeline:

1. **Uniqueness**: `b` is the *unique* leader block for `wave` in `B`.
   If the leader equivocated (produced multiple blocks at the leader
   round), none of those blocks can be finalized — `leader_block_for_wave`
   returns `None` in that case, and `final_leader_for_wave` propagates
   that `None`. The uniqueness clause captures this directly.

2. **Super-ratification**: the witness blocks within the wave
   (from the leader round through the last round) super-ratify `b`
   under the given bond weights.

## Safety theorem

`no_conflicting_finals` states that two blocks satisfying `FinalLeader`
for the same wave are equal. The proof is a single application of the
uniqueness clause: if both `b₁` and `b₂` are "the unique leader", each
is the only element of the leader set, so they must be identical.

### Why uniqueness is sound

The exclusion property proved in `Equivocation.lean` is what makes the
uniqueness condition meaningful. An equivocating leader cannot accumulate
super-ratification for either branch: any acknowledging block is
prevented from vouching by `equivocation_not_approved`. Therefore, the
only scenario in which `FinalLeader` is satisfiable is when there is
exactly one leader block — which uniqueness encodes directly.

The three-quorum argument (`honest_triple_intersection` from
`Weights.lean` + `equivocation_not_approved` from `Equivocation.lean`)
is the protocol-level justification. At the Lean level, given the
encoding of uniqueness, `no_conflicting_finals` follows immediately.

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
Mirrors `wave.rs:last_round_of_wave`. -/
def lastRoundOfWave (wave wavelength : ℕ) : ℕ := wave * wavelength + wavelength - 1

/-- `waveOfRound` and `leaderRoundOfWave` are inverses when `wavelength > 0`. -/
lemma waveOfRound_leaderRound (wave wavelength : ℕ) (hwl : 0 < wavelength) :
    waveOfRound (leaderRoundOfWave wave wavelength) wavelength = wave :=
  Nat.mul_div_cancel wave hwl

/-- The leader round is always within the wave's extent. -/
lemma leaderRound_le_lastRound (wave wavelength : ℕ) (hwl : 0 < wavelength) :
    leaderRoundOfWave wave wavelength ≤ lastRoundOfWave wave wavelength := by
  unfold leaderRoundOfWave lastRoundOfWave; omega

/-! ### Leader blocks -/

/-- The set of blocks that qualify as leader blocks for `wave`:
present in `B`, created by the selected leader `sel wave`, and at the
leader round's DAG depth.

Uses `Set BlockId` (not `Finset`) because enumerating the blocklace by
depth requires decidable depth comparisons; propositional characterisation
is sufficient for all downstream theorems.

Mirrors the filtering in `wave.rs:leader_blocks_of_wave`. -/
def leaderBlocksOfWave (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧
       creatorOf B b = some (sel wave) ∧
       blockDepth B hV b = leaderRoundOfWave wave wavelength}

/-- `b` is the **unique** leader block for `wave`: it is itself a leader
block, and every other leader block for the same wave equals `b`.

Mirrors `finality.rs:leader_block_for_wave`, which returns `Some b` only
when exactly one leader block exists. An equivocating leader yields
multiple leader blocks and therefore no unique one. -/
def IsUniqueLeaderBlock (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId) (b : BlockId) : Prop :=
  b ∈ leaderBlocksOfWave B hV wave wavelength sel ∧
  ∀ b', b' ∈ leaderBlocksOfWave B hV wave wavelength sel → b' = b

/-! ### Finalized leader -/

/-- `b` is a finalized leader for `wave` in `B`:
1. `b` is the unique leader block for the wave.
2. There exists a witness block set within the wave whose blocks
   super-ratify `b` under the given bond weights.

Mirrors `finality.rs:is_weighted_final_leader` +
`finality.rs:final_leader_for_wave`. -/
def FinalLeader (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId) (b : BlockId) : Prop :=
  IsUniqueLeaderBlock B hV wave wavelength sel b ∧
  ∃ witness : Finset BlockId,
    (∀ s ∈ witness,
      s ∈ B.keys ∧
      leaderRoundOfWave wave wavelength ≤ blockDepth B hV s ∧
      blockDepth B hV s ≤ lastRoundOfWave wave wavelength) ∧
    SuperRatifies bonds validators B witness b

/-! ### Safety theorem -/

/-- **Finalized-leader safety.**  Two blocks that are both `FinalLeader`
for the same wave must be equal.

**Proof.** Both `b₁` and `b₂` satisfy `IsUniqueLeaderBlock` for the same
wave. From `hb₁.1`, every leader block for the wave equals `b₁`. From
`hb₂.1`, `b₂` is itself a leader block for the wave. So `b₂ = b₁` by
the uniqueness clause of `hb₁`, giving `b₁ = b₂`.

**Why this is the right statement.** The claim does *not* require
any hypothesis about honest validators; it follows purely from the
structure of `FinalLeader`. The deeper soundness argument — that the
uniqueness condition is only achievable when the leader is honest — is
explained in the module doc and uses `Equivocation.lean`'s exclusion
property; it does not appear explicitly in the proof because uniqueness
is already encoded in the definition. -/
theorem no_conflicting_finals
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId)
    (b₁ b₂ : BlockId)
    (hb₁ : FinalLeader bonds validators B hV wave wavelength sel b₁)
    (hb₂ : FinalLeader bonds validators B hV wave wavelength sel b₂) :
    b₁ = b₂ :=
  (hb₁.1.2 b₂ hb₂.1.1).symm

/-! ### Auxiliary lemmas -/

/-- A leader block's depth is exactly the leader round of its wave.
Unfolding convenience for downstream proofs. -/
lemma leaderBlock_depth (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId) (b : BlockId)
    (hb : b ∈ leaderBlocksOfWave B hV wave wavelength sel) :
    blockDepth B hV b = leaderRoundOfWave wave wavelength :=
  hb.2.2

/-- Any two distinct unique leader blocks in the same wave must be equal —
a contradiction, so if `IsUniqueLeaderBlock` holds for two blocks they
are the same. -/
lemma uniqueLeaderBlock_unique (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId) (b₁ b₂ : BlockId)
    (h₁ : IsUniqueLeaderBlock B hV wave wavelength sel b₁)
    (h₂ : IsUniqueLeaderBlock B hV wave wavelength sel b₂) :
    b₁ = b₂ :=
  (h₁.2 b₂ h₂.1).symm

end CordialMiners
