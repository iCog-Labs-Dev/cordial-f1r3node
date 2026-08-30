# KR4 — Finalized Leader Safety

Lean modules: `LeanVerification/Approval.lean`, `LeanVerification/Finality.lean`,
`LeanVerification/Ordering.lean`.
Depends on: KR1 (`Observe.lean`), KR2 (`Equivocation.lean`), KR3 (`Weights.lean`, `WCert.lean`).

## Why this exists

`no_conflicting_finals` is the theorem everything else in this initiative
exists to support: **two distinct blocks cannot both be finalized for the
same wave.** Until this is proved, the protocol has no formal safety
guarantee — only Rust tests covering the scenarios that were thought of.
This module provides the general proof, over all possible blocklace
histories regardless of adversarial choice.

The supporting module `Ordering.lean` adds two guarantees:
- **Append-only ledger**: as the blocklace grows, the ordered output `tau`
  only extends, never retracts.
- **Finality monotonicity**: once a block is finalized in `B`, it stays
  finalized in any superset `B'`.

---

## Approval (`Approval.lean`)

### `Approves`

```lean
def Approves (B : Blocklace) (approver target : BlockId) : Prop :=
  VouchesFor B approver target
```

Definitionally equal to `VouchesFor` from `Equivocation.lean`. The Rust
`approves` function (`consensus/approval.rs:29–104`, Definition 18 of
arXiv:2205.09174) checks exactly the same thing: the approver observes
the target and has no observed incomparable same-creator competitor.
Naming it separately lets `Finality.lean` and `Ordering.lean` refer to
the approval concept without depending on KR2's internals.

### Bond-weighted supermajority

```lean
abbrev bondOf (bonds : NodeId → ℕ) (S : Finset NodeId) : ℕ :=
  ∑ v ∈ S, bonds v

def StrictTwoThirdsMaj (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (S : Finset NodeId) : Prop :=
  3 * bondOf bonds S > 2 * bondOf bonds validators
```

`NodeId = Nat` is infinite, so `Fintype NodeId` does not hold. We carry
an explicit finite `validators : Finset NodeId` universe and express the
2/3 threshold via cross-multiplication over `Finset.sum`. This is
arithmetically identical to `OpenTheta w 2 3` from `Weights.lean` on a
`Fintype` type.

Rust: `cordiality.rs:strict_two_thirds`, `is_weighted_supermajority`.

### `Ratifies` and `SuperRatifies`

```lean
def Ratifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (r b : BlockId) : Prop :=
  ∃ S : Finset NodeId,
    S ⊆ validators ∧
    (∀ v ∈ S, ∃ a, Observes B r a ∧ creatorOf B a = some v ∧ Approves B a b) ∧
    StrictTwoThirdsMaj bonds validators S

def SuperRatifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (witness : Finset BlockId) (b : BlockId) : Prop :=
  ∃ R : Finset NodeId,
    R ⊆ validators ∧
    (∀ v ∈ R, ∃ r ∈ witness, creatorOf B r = some v ∧ Ratifies bonds validators B r b) ∧
    StrictTwoThirdsMaj bonds validators R
```

Both are stated propositionally (using `∃`) to avoid requiring a
decidable `Observes` check, which would need a finite graph traversal
over the opaque `Blocklace`.

Rust:
- `Ratifies` → `cordiality.rs:weighted_ratifies` (252–303)
- `SuperRatifies` → `cordiality.rs:weighted_super_ratifies` (311–340)

### Bridge lemmas

```lean
theorem approves_implies_vouchesFor : Approves B approver target → VouchesFor B approver target
theorem approves_exclusion          : Equivocation B hV b₁ b₂ → Acknowledges B approver b₁ b₂ →
                                      ¬ Approves B approver b₁ ∧ ¬ Approves B approver b₂
theorem ratifies_mono               : Observes B d r → Ratifies ... B r b → Ratifies ... B d b
theorem superRatifies_mono          : witness ⊆ witness' → SuperRatifies ... witness b →
                                      SuperRatifies ... witness' b
```

`approves_exclusion` is the key bridge: an acknowledging block cannot
approve either branch of an equivocation. This is a direct application of
`Equivocation.lean:equivocation_not_approved` to the concrete `Approves`.

---

## Finalized Leader Safety (`Finality.lean`)

### Wave arithmetic

```lean
def waveOfRound      (round wavelength : ℕ) : ℕ := round / wavelength
def leaderRoundOfWave (wave wavelength : ℕ) : ℕ := wave * wavelength
def lastRoundOfWave   (wave wavelength : ℕ) : ℕ := wave * wavelength + wavelength - 1
```

Direct translation of `wave.rs`. Uses pure `ℕ`-division; call sites
hypothesize `0 < wavelength` where needed.

Rust: `wave.rs:wave_of_round`, `first_round_of_wave`, `last_round_of_wave`.

### Leader blocks

```lean
def leaderBlocksOfWave (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧
       creatorOf B b = sel wave ∧
       blockDepth B hV b = leaderRoundOfWave wave wavelength}
```

Uses `Set BlockId` (propositional) rather than `Finset BlockId` to avoid
requiring a decidable depth comparison. `blockDepth B hV b` is the DAG
depth from `Equivocation.lean`, computed by well-founded recursion.

`sel : ℕ → Option NodeId` mirrors Rust's `Fn(u64) → Option<NodeId>`:
`sel wave = none` means no leader was elected for that wave (e.g. not
enough blocks have arrived yet), so the leader-block set is empty.

Rust: `wave.rs:leader_blocks_of_wave`.

### `FinalLeader`

```lean
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
```

Encodes two conditions:
1. **Leader-block membership** — `b` is present, has the correct creator
   (`creatorOf B b = sel wave`), and sits at the leader depth.
2. **Super-ratification** — a witness block set within the wave
   super-ratifies `b` under the given bond weights.

Uniqueness is **not** an axiom here — it is a theorem (`no_conflicting_finals`)
proved by the quorum-intersection argument from KR3.

Rust: `finality.rs:is_weighted_final_leader` + `final_leader_for_wave`.

### `leaderBlocks_equivocation`

```lean
lemma leaderBlocks_equivocation (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId)
    (b₁ b₂ : BlockId)
    (hb₁ : b₁ ∈ leaderBlocksOfWave B hV wave wavelength sel)
    (hb₂ : b₂ ∈ leaderBlocksOfWave B hV wave wavelength sel)
    (hne : b₁ ≠ b₂) : Equivocation B hV b₁ b₂
```

Any two distinct leader blocks for the same wave form a KR2-`Equivocation`:
they share creator (`sel wave`) and depth (`leaderRoundOfWave`), so
`same_depth_incomparable` gives incomparability, which is exactly the
equivocation condition. This lemma is the bridge from the leader-block
structure to the quorum-intersection argument.

### The safety theorem

```lean
theorem no_conflicting_finals
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (_hwl : 0 < wavelength)
    (sel : ℕ → Option NodeId)
    (honestNodes : Finset NodeId)
    (hH_sub : honestNodes ⊆ validators)
    (hH_maj : StrictTwoThirdsMaj bonds validators honestNodes)
    (hHonest : ∀ v ∈ honestNodes, HonestIn B v)
    (b₁ b₂ : BlockId)
    (hb₁ : FinalLeader bonds validators B hV wave wavelength sel b₁)
    (hb₂ : FinalLeader bonds validators B hV wave wavelength sel b₂) :
    b₁ = b₂
```

**Proof sketch (by contradiction, assume `b₁ ≠ b₂`)**:

1. `leaderBlocks_equivocation` gives `Equivocation B hV b₁ b₂`.
2. Extract ratifier sets `R₁` (from `hb₁`) and `R₂` (from `hb₂`).
3. `finset_honest_triple_intersection` yields an honest ratifier
   `v* ∈ R₁ ∩ R₂ ∩ honestNodes`.
4. `v*` ratified `b₁` via `r₁` and `b₂` via `r₂`.
5. Extract approver sets `S₁`, `S₂`; `finset_honest_triple_intersection`
   again gives an honest approver `u* ∈ S₁ ∩ S₂ ∩ honestNodes`.
6. `u*` approved both `b₁` and `b₂`; `honest_chain_linearity` gives
   comparability of `u*`'s blocks, so `u*` has an approver block `a`
   that observes both ratifier blocks and therefore both finalized blocks.
7. `approves_exclusion` contradicts `u*` approving either branch of the
   `b₁`/`b₂` equivocation.

**Why the encoding is sound.** The quorum-intersection argument at both
the ratifier and approver levels uses the honest-kernel lemmas from KR3
(`Weights.lean:honest_triple_intersection`). The equivocation exclusion
(`approves_exclusion` from KR2) provides the final contradiction.

---

## Tau Ordering (`Ordering.lean`)

### Sub-blocklace

```lean
def SubBlocklace (B B' : Blocklace) : Prop :=
  ∀ b blk, B.lookup b = some blk → B'.lookup b = some blk
```

`B'` is an append-only extension of `B`: every block in `B` is present
in `B'` with identical content (block content is immutable). From this,
`observes_of_subBlocklace` follows: observation can only grow, it never
shrinks.

### Finality monotonicity

```lean
theorem FinalLeader_of_subBlocklace
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId)
    (hsub : SubBlocklace B B') (b : BlockId)
    (hfin : FinalLeader bonds validators B hV wave wavelength sel b) :
    FinalLeader bonds validators B' hV' wave wavelength sel b
```

If `b` is a finalized leader in `B`, it remains finalized in any monotone
extension `B'`. The proof transfers each component of `FinalLeader`:

| Component | Lemma used |
|---|---|
| `b ∈ B'.keys` | `keys_mono` |
| `creatorOf B' b = sel wave` | `creatorOf_of_subBlocklace` |
| `blockDepth B' hV' b = leaderRoundOfWave ...` | `blockDepth_of_subBlocklace` |
| Witness depths preserved | `blockDepth_of_subBlocklace` |
| `SuperRatifies ... B' witness b` | `superRatifies_of_subBlocklace` |

**`blockDepth_of_subBlocklace`** is the most involved: well-founded
induction over `directPred_wf_of_valid B hV`, using `Finset.sup_congr`
at each step to equate the `blockDepthWF` recursive equation in `B` and
`B'` (same lookup for B-resident blocks, `Closed B` bounds all
predecessor lookups within `B.keys`).

**`observes_stays_in_B`** is the key containment lemma: starting from
`a ∈ B.keys`, any block reachable via `B'`'s predecessor relation stays
in `B.keys`. New blocks added to `B'` cannot be reached from B-resident
blocks because `Closed B` locks all predecessor lookups within `B.keys`,
and `SubBlocklace` preserves those lookups unchanged.

**`observes_iff_subBlocklace`** follows: for `a, b ∈ B.keys`,
`Observes B a b ↔ Observes B' a b`. This allows lifting the
`VouchesFor` fork-freedom condition (`Approves`) from `B` to `B'` and
back, making `approves_of_subBlocklace` provable.

Addresses reviewer concern 3 on PR #229: finality monotonicity was absent
from the original formalization and is false under the old
uniqueness-encoding definition.

### `tau`

```lean
opaque tau (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId) : List BlockId
```

Declared `opaque` because its computational definition lives in Rust
(`ordering.rs:tau`). What matters formally is the prefix-safety property
below. `sel : ℕ → Option NodeId` matches Rust's `Fn(u64) → Option<NodeId>`.

Rust: `ordering.rs:tau`.

### Prefix-safety

```lean
axiom tau_prefix_monotone
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (hsub : SubBlocklace B B') :
    List.IsPrefix
      (tau bonds validators B hV wavelength sel)
      (tau bonds validators B' hV' wavelength sel)
```

Stated as an `axiom` — a deliberate trusted formal boundary. The
justification is a three-point proof sketch:

1. **`FinalLeader_of_subBlocklace`** (proved): any finalized leader in
   `B` is still finalized in `B'`, so the sequence of finalized-leader
   epochs can only advance forward.
2. **`no_conflicting_finals`**: the finalized leader for each wave is
   unique, so there is no ambiguity in what `tau` appends.
3. **`observes_of_subBlocklace`**: as `B` grows, every observation is
   preserved, so `tau B'` appends a suffix to `tau B` rather than
   reordering it.

---

## Rust Correspondence

| Lean | Rust location |
|---|---|
| `Approves` | `consensus/approval.rs:approves` (29–104) |
| `bondOf`, `StrictTwoThirdsMaj` | `cordiality.rs:strict_two_thirds`, `is_weighted_supermajority` |
| `Ratifies` | `cordiality.rs:weighted_ratifies` (252–303) |
| `SuperRatifies` | `cordiality.rs:weighted_super_ratifies` (311–340) |
| `waveOfRound` | `wave.rs:wave_of_round` (26–32) |
| `leaderRoundOfWave` | `wave.rs:first_round_of_wave` / `leader_round_of_wave` (35–69) |
| `lastRoundOfWave` | `wave.rs:last_round_of_wave` (44–47) |
| `leaderBlocksOfWave` | `wave.rs:leader_blocks_of_wave` (79–100) |
| `FinalLeader` | `finality.rs:is_weighted_final_leader` (196–245) + `final_leader_for_wave` (251–267) |
| `leaderBlocks_equivocation` | Structural precondition for KR2 exclusion |
| `no_conflicting_finals` | Safety invariant — no single Rust function; proved by contradiction |
| `SubBlocklace` | Append-only blocklace growth model |
| `FinalLeader_of_subBlocklace` | Finality monotonicity — once final, always final |
| `tau` | `ordering.rs:tau` (419–) |
| `tau_prefix_monotone` | Ledger append-only invariant; tested by `test_finality.rs:finalized_order_excludes_equivocations_the_leader_acknowledged` |
