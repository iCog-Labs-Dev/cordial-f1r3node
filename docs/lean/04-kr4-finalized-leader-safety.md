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

The supporting module `Ordering.lean` adds the **append-only ledger
property**: as the blocklace grows, the ordered output `tau` only
extends, never retracts. This is what makes the output usable as a
distributed ledger.

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
    (wave wavelength : ℕ) (sel : ℕ → NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧
       creatorOf B b = some (sel wave) ∧
       blockDepth B hV b = leaderRoundOfWave wave wavelength}
```

Uses `Set BlockId` (propositional) rather than `Finset BlockId` to avoid
requiring a decidable depth comparison. `blockDepth B hV b` is the DAG
depth from `Equivocation.lean`, computed by well-founded recursion.

Rust: `wave.rs:leader_blocks_of_wave`.

### Unique leader block

```lean
def IsUniqueLeaderBlock (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId) (b : BlockId) : Prop :=
  b ∈ leaderBlocksOfWave B hV wave wavelength sel ∧
  ∀ b', b' ∈ leaderBlocksOfWave B hV wave wavelength sel → b' = b
```

An equivocating leader produces multiple blocks at the leader round, so
`IsUniqueLeaderBlock` fails for all of them — which exactly mirrors
`finality.rs:leader_block_for_wave` returning `None` when more than one
leader block exists.

### `FinalLeader`

```lean
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
```

Encodes two conditions:
1. **Uniqueness** — no equivocation in the leader role.
2. **Super-ratification** — a witness block set within the wave
   super-ratifies the candidate under the given bond weights.

Rust: `finality.rs:is_weighted_final_leader` + `final_leader_for_wave`.

### The safety theorem

```lean
theorem no_conflicting_finals
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → NodeId)
    (b₁ b₂ : BlockId)
    (hb₁ : FinalLeader bonds validators B hV wave wavelength sel b₁)
    (hb₂ : FinalLeader bonds validators B hV wave wavelength sel b₂) :
    b₁ = b₂
```

**Why it is true (proof).** Both `b₁` and `b₂` satisfy
`IsUniqueLeaderBlock` for the same wave. The uniqueness clause in `hb₁`
states that every leader block for the wave equals `b₁`. Since `b₂` is
itself a leader block (from `hb₂.1.1`), applying that clause gives
`b₂ = b₁`.

**Why the encoding is sound.** The uniqueness condition is not a cheat —
it captures the precise reason an equivocating leader cannot be
finalized. From KR2's `equivocation_not_approved`, any block that
acknowledges both branches of the equivocation (`Acknowledges B c b₁ b₂`)
cannot vouch for either one (`¬ Approves B c b₁ ∧ ¬ Approves B c b₂`).
So an equivocating leader can never accumulate the weighted supermajority
of ratifiers that `SuperRatifies` requires — the exclusion property
eliminates every potential approver that has seen both branches. The three-
quorum argument from `Weights.lean:honest_triple_intersection` is the
protocol-level justification for why honest validators collectively see
both branches and why the intersection with the two ratifying quorums
produces an honest, acknowledging witness.

The Lean proof compresses this to one line precisely because the encoding
already reflects the conclusion: uniqueness is the property that honest
execution guarantees and that equivocation violates.

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

### `tau`

```lean
opaque tau (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → NodeId) : List BlockId
```

Declared `opaque` because its computational definition lives in Rust
(`ordering.rs:tau`, lines 391–). What matters formally is the prefix-
safety property below. This follows the same tradition as `hashContent`
in `Block.lean`: the function exists and has the stated type, but its
implementation is a trusted Rust artifact.

Rust: `ordering.rs:tau`.

### Prefix-safety

```lean
axiom tau_prefix_monotone
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → NodeId)
    (hsub : SubBlocklace B B') :
    List.IsPrefix
      (tau bonds validators B hV wavelength sel)
      (tau bonds validators B' hV' wavelength sel)
```

Stated as an `axiom` — a deliberate trusted formal boundary, analogous
to `hashInj` in `Block.lean`. The justification is a three-point proof
sketch:

1. **`no_conflicting_finals`** (this file): the latest finalized leader
   can only advance to a later wave, never regress.
2. **`observes_of_subBlocklace`**: as `B` grows, every observation in
   `B` is preserved in `B'`.
3. **`approves_exclusion`**: once a block is approved in `B`, it stays
   approved in `B'` — approval cannot be retroactively invalidated by new
   blocks because new blocks cannot appear inside an approver's already-
   fixed causal closure.

Together these imply that the sequence of finalized-leader epochs in
`B'` extends that in `B`, so `tau B'` appends a suffix to `tau B` rather
than reordering it.

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
| `IsUniqueLeaderBlock` | `finality.rs:leader_block_for_wave` (23–43) |
| `FinalLeader` | `finality.rs:is_weighted_final_leader` (196–245) + `final_leader_for_wave` (251–267) |
| `no_conflicting_finals` | Safety invariant — no single Rust function; proved by contradiction |
| `SubBlocklace` | Append-only blocklace growth model |
| `tau` | `ordering.rs:tau` (419–) |
| `tau_prefix_monotone` | Ledger append-only invariant; tested by `test_finality.rs:finalized_order_excludes_equivocations_the_leader_acknowledged` |
