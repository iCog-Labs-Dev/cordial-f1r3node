# KR2 — Equivocation and Exclusion

Lean module: `LeanVerification/Equivocation.lean`
Depends on: `LeanVerification.Observe` (signature of `Observes` only, per the
issue's decoupling note).

## Why this exists

Cordial Miners' entire safety argument rests on one fact: a validator that
equivocates — creates two blocks for the same round that don't observe each
other — can never get *both* of those blocks vouched for by an honest,
acknowledging node. This module defines equivocation precisely and proves
that fact as a standalone, reusable lemma.

## Block identity: `CreatorRound`

```lean
class CreatorRound (Validator : Type*) where
  creatorOf : Blocklace → BlockId → Option Validator
  roundOf   : Blocklace → BlockId → Option ℕ
  round_lt_of_observes :
    ∀ B a b, Observes B a b → a ≠ b →
      ∀ ra rb, roundOf B a = some ra → roundOf B b = some rb → rb < ra
  round_defined_of_creator_defined :
    ∀ B b v, creatorOf B b = some v → ∃ r, roundOf B b = some r
```

`Observe.lean` only gives us the DAG reachability relation `Observes`; it
doesn't expose "who created this block" or "what round is this block at."
Rather than block this issue on Issue 02 shipping that projection, we
axiomatize the two properties of it we actually need: rounds strictly
decrease along `Observes`, and every block with a known creator has a known
round. This is the "soft dependency" the issue description allows for —
once Issue 02/05 land a real, computable `roundOf`, it slots in as a
`CreatorRound` instance and every theorem below applies unchanged.

Rust: `BlockIdentity` (creator), `round::depth` — see `types/identity_id.rs`.

## Core definitions

```lean
def creatorBlocksAtRound (B) (v : Validator) (r : ℕ) : Set BlockId :=
  { b | creatorOf B b = some v ∧ roundOf B b = some r }

def Equivocation (B) (v) (r) (b1 b2 : BlockId) : Prop :=
  b1 ∈ creatorBlocksAtRound B v r ∧ b2 ∈ creatorBlocksAtRound B v r ∧ b1 ≠ b2

def EquivocatesAt (B) (v) (r) : Prop := (creatorBlocksAtRound B v r).Nontrivial

def Equivocator (B) (v) : Prop := ∃ r, EquivocatesAt B v r

def HonestIn (B) (v) : Prop := ¬ Equivocator B v
```

In plain English: `v` **equivocates at round `r`** if it has two or more
distinct blocks at that round. `v` **is an equivocator** if this happens at
any round, ever. `v` **is honest** in `B` exactly when it never does.
`Equivocation B v r b1 b2` witnesses one specific instance of the cheat.

Rust: `Equivocation` struct (`cordiality.rs:25–30`), `creator_blocks_at_round`
(`:47–56`), `equivocation_blocks_at_round`/`all_equivocations` (`:60–111`).

## Honest chain linearity

```lean
def ExtendsOwnChain (B) (v) : Prop :=
  ∀ b b', creatorOf B b = some v → creatorOf B b' = some v →
    ∀ r r', roundOf B b = some r → roundOf B b' = some r' →
      r' < r → Observes B b b'

theorem honest_chain_linear (B) (v) (hHonest : HonestIn B v)
    (hExtends : ExtendsOwnChain B v) :
    IsChain (Observes B) { b | creatorOf B b = some v }
```

**Why it's true:** take any two of `v`'s blocks `b1 ≠ b2`. Each has a round
(by `round_defined_of_creator_defined`). If the rounds differ, `ExtendsOwnChain`
directly gives observation in the right direction (the later block observes
the earlier one). If the rounds are *equal*, `b1` and `b2` are two distinct
blocks in the same `creatorBlocksAtRound B v r` — i.e. exactly the definition
of equivocation — which contradicts `hHonest`. So the "same round" case is
vacuous, and every pair of `v`'s blocks is comparable: `v`'s history is a
chain, not a tree.

`ExtendsOwnChain` is stated as an explicit hypothesis rather than derived,
mirroring `satisfies_chain_axiom` in `blocklace.rs:252–279`, which the Rust
side likewise treats as an invariant to be checked, not something proved
from more primitive facts.

**`HonestIn` vs `ExtendsOwnChain` — two distinct predicates.**
`HonestIn B v` rules out only same-round duplication: `v` never has two
distinct blocks at the *same* round. `ExtendsOwnChain B v` is a strictly
stronger property: every later block observes every earlier block, across
*all* round pairs. A validator could satisfy `HonestIn` (one block per
round) yet violate `ExtendsOwnChain` by producing blocks at different rounds
that are incomparable — i.e. neither observes the other. Such a validator
also fails Rust's `satisfies_chain_axiom`, which checks all block pairs
regardless of round. `honest_chain_linear` therefore requires *both*
`HonestIn` and `ExtendsOwnChain` to conclude the full chain property;
`HonestIn` alone is not enough. The Rust side conflates these into one
`satisfies_chain_axiom` check; the Lean side makes the decomposition
explicit.

## Acknowledgement and its monotonicity

```lean
def Acknowledges (B) (w : BlockId) (v) (r) : Prop :=
  ∀ b ∈ creatorBlocksAtRound B v r, Observes B w b

def Hides (B) (w) (v) (r) : Prop := ¬ Acknowledges B w v r

theorem acknowledges_mono (h : Acknowledges B w v r) (hobs : Observes B w' w) :
    Acknowledges B w' v r
```

`w` **acknowledges** `v`'s round `r` when it has observed *every* block `v`
produced at that round — including, if `v` cheated, both branches of the
equivocation. `w` **hides** the round otherwise. Monotonicity says
acknowledgement can only spread forward through the DAG: if `w'` observes
`w` and `w` has already seen all of round `r`, `w'` has too. Evidence of an
equivocation, once observed, can't be un-observed downstream.

Rust: `acknowledges_equivocation` (`cordiality.rs:130–145`),
`hidden_equivocations` (`:147–177`).

## The exclusion property

```lean
def VouchesFor (V) [CreatorRound V] (B) (w b : BlockId) : Prop :=
  Observes B w b ∧
    ∀ b', creatorOf B b' = creatorOf B b → b' ≠ b → Observes B w b' →
      ¬ (¬ Observes B b b' ∧ ¬ Observes B b' b)

theorem acknowledges_no_vouch_for_equivocation
    (heq : EquivocatesAt B v r) (hb : b ∈ creatorBlocksAtRound B v r)
    (hack : Acknowledges B w v r) :
    ¬ VouchesFor Validator B w b

theorem equivocation_not_approved
    (heq : EquivocatesAt B v r) (hb : b ∈ creatorBlocksAtRound B v r)
    (hack : Acknowledges B w v r)
    (hApproveImpliesVouch : Approves B w b → VouchesFor Validator B w b) :
    ¬ Approves B w b
```

`w` **cleanly vouches for** `b` when it observes `b`, and observes no other,
incomparable, same-creator competitor to `b`. **Why the theorem is true:**
if `v` equivocated at round `r`, then for any block `b` at that round there's
always another block `s` at the same round, distinct from `b`, that `v` also
created. Same-round distinct blocks are provably incomparable
(`creatorBlocksAtRound_isAntichain`, since `round_lt_of_observes` would force
a strict round decrease between them, which is impossible at equal rounds).
If `w` acknowledges the round, it has observed `s` too — which is exactly
the incomparable, same-creator competitor `VouchesFor` rules out. So `w`
cannot cleanly vouch for `b`.

`equivocation_not_approved` restates this for an arbitrary `Approves`
relation: this is the exact hypothesis shape Issue 04's Agreement proof can
import — supply your concrete finality/approval relation, prove it implies
`VouchesFor`, and the exclusion falls out with no changes to this file.

**The one-sentence version:** *no single approving block can vouch for both
sides of the same lie.*

Rust: `acknowledges_equivocation` / the approval-exclusion check downstream
of it in `cordiality.rs`.

## Rust correspondence

| Lean | Rust (`consensus/cordiality.rs` unless noted) |
|---|---|
| `CreatorRound.creatorOf` | `BlockIdentity` creator field |
| `CreatorRound.roundOf` | `round::depth` |
| `creatorBlocksAtRound` | `creator_blocks_at_round` (47–56) |
| `Equivocation`, `EquivocatesAt` | `Equivocation` struct (25–30), `equivocation_blocks_at_round` (60–71) |
| `Equivocator`, `HonestIn` | `all_equivocations` (74–111) |
| `ExtendsOwnChain`, `honest_chain_linear` | `satisfies_chain_axiom` (`blocklace.rs:252–279`) |
| `Acknowledges`, `Hides` | `acknowledges_equivocation` (130–145), `hidden_equivocations` (147–177) |
| `VouchesFor`, `acknowledges_no_vouch_for_equivocation`, `equivocation_vouch_exclusion`, `equivocation_not_approved` | approval-exclusion logic downstream of `hidden_equivocations` |

