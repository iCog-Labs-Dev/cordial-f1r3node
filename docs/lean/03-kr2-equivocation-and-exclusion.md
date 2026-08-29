# KR2 — Equivocation and Exclusion

Module: `LeanVerification/Equivocation.lean`

## `Equivocation`, `Equivocator`, `HonestIn`

```lean
def Equivocation (B : Blocklace) (b₁ b₂ : BlockId) : Prop :=
  b₁ ∈ B.keys ∧ b₂ ∈ B.keys ∧
  creatorOf B b₁ = creatorOf B b₂ ∧ b₁ ≠ b₂ ∧
  ¬ Observes B b₁ b₂ ∧ ¬ Observes B b₂ b₁
```

**Plain English:** `b₁` and `b₂` are an equivocation in `B` when both are
actually present, both were made by the same creator, they're different
blocks, and neither one is in the other's causal history — `b₁` doesn't
observe `b₂`, and `b₂` doesn't observe `b₁`. They sit side by side in the
DAG, silently contradicting each other.

```lean
def Equivocator (B : Blocklace) (v : NodeId) : Prop :=
  ∃ b₁ b₂, Equivocation B b₁ b₂ ∧ creatorOf B b₁ = some v
```

**Plain English:** validator `v` is an equivocator in `B` if some pair of
`v`'s own blocks in `B` form an equivocation.

```lean
def HonestIn (B : Blocklace) (v : NodeId) : Prop :=
  ¬ Equivocator B v
```

**Plain English:** validator `v` is honest in `B` simply when `v` is not
an equivocator in `B` — every pair of `v`'s blocks avoids the
`Equivocation` condition above.

Two supporting definitions used throughout the rest of the file:

```lean
def creatorOf (B : Blocklace) (b : BlockId) : Option NodeId :=
  (B.lookup b).map Block.creator

def blocksBy (B : Blocklace) (v : NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧ creatorOf B b = some v}
```

`creatorOf` looks up who made a block (`none` if the block isn't present
in `B`); `blocksBy B v` is the set of blocks `v` has actually produced in
`B`, used to state honest chain linearity below.

## Honest chain linearity — and why it matters

```lean
theorem honest_chain_linearity (B : Blocklace) (v : NodeId) (h : HonestIn B v) :
    IsChain (Observes B) (blocksBy B v)
```

**Plain English:** if `v` is honest in `B`, then `v`'s blocks form a
**chain** under `Observes B` — for any two of `v`'s blocks, one observes
the other. There is no case where an honest validator has two blocks
sitting side by side, unrelated to each other.

This matters because almost everything downstream — "the chain of
validator `v`", "`v`'s latest block", "the sequence of `v`'s blocks in
order" — silently assumes that concept is well-defined. It only is
because of this theorem. Without it, "the chain of `v`" would be
ambiguous the moment `v` produced two incomparable blocks; this theorem
is what guarantees that ambiguity can only ever arise from equivocation,
never from an honest validator's ordinary behavior.

## The exclusion property

Two more definitions are needed to state it:

```lean
def Acknowledges (B : Blocklace) (c b₁ b₂ : BlockId) : Prop :=
  Observes B c b₁ ∧ Observes B c b₂

def VouchesFor (B : Blocklace) (c b : BlockId) : Prop :=
  Observes B c b ∧
    ∀ b', creatorOf B b' = creatorOf B b → b' ≠ b → Observes B c b' →
      ¬ (¬ Observes B b b' ∧ ¬ Observes B b' b)
```

`Acknowledges B c b₁ b₂` says `c`'s causal history includes both `b₁` and
`b₂` — `c` has "seen" the whole equivocation. `VouchesFor B c b` says `c`
observes `b` and doesn't *also* observe some other, incomparable block by
`b`'s creator — this is the equivocation-relevant slice of the paper's
full approval relation.

```lean
theorem equivocation_exclusion (B : Blocklace) (b₁ b₂ c : BlockId)
    (heq : Equivocation B b₁ b₂) (hack : Acknowledges B c b₁ b₂) :
    ¬ VouchesFor B c b₁ ∧ ¬ VouchesFor B c b₂
```

**Plain language:** if `b₁` and `b₂` equivocate, and some block `c` has
acknowledged that equivocation (seen both `b₁` and `b₂` in its causal
history), then `c` cleanly vouches for *neither* branch.

Say it out loud the way the issue asks: **no single approving block can
vouch for both sides of the same lie.** More precisely, it can't even
vouch for *one* side once it's seen both — the instant a block observes
both equivocating branches, `VouchesFor`'s own "no incomparable sibling
observed" condition fails for each branch in turn, by definition. That's
the whole proof: it's not a deep combinatorial fact, it's the direct
consequence of what `VouchesFor` and `Equivocation` each require, put
together.

**This is the lemma Issue 04's Agreement proof imports** — but not
`equivocation_exclusion` directly. Since Issue 04 owns the real `Approves`
relation, which this file doesn't define, the actual import-ready form is
one level more general:

```lean
theorem equivocation_not_approved
    (B : Blocklace) (b₁ b₂ c : BlockId)
    {Approves : Blocklace → BlockId → BlockId → Prop}
    (heq : Equivocation B b₁ b₂) (hack : Acknowledges B c b₁ b₂)
    (hApproveImpliesVouch₁ : Approves B c b₁ → VouchesFor B c b₁)
    (hApproveImpliesVouch₂ : Approves B c b₂ → VouchesFor B c b₂) :
    ¬ Approves B c b₁ ∧ ¬ Approves B c b₂
```

`Approves` is left fully arbitrary. Issue 04 supplies its own `Approves`
and proves the two "approving implies vouching" side conditions once —
which should be close to immediate, since `Approves` is expected to be at
least as strict as `VouchesFor` — and gets the exclusion result for its
own relation directly, with zero changes to this file.

## Mapping table: Lean ↔ `consensus/cordiality.rs`

| Lean | Rust (`consensus/cordiality.rs`) |
|---|---|
| `creatorOf` | `BlockIdentity.creator`, read via `Block.identity.creator` throughout this file |
| `Equivocation` | `equivocation_blocks_at_round` / `creator_blocks_at_round`, generalized from "same round" to "incomparable under `Observes`" |
| `Equivocator` | `all_equivocations` reporting a nonempty `blocks` list for a `creator` |
| `HonestIn` | the absence of an entry for a creator across `all_equivocations`'s output |
| `honest_chain_linearity` | not directly present in this file — the property it formalizes is *assumed*, not checked, everywhere `creator_blocks_at_round`/`equivocation_blocks_at_round` treat "a validator's blocks" as if ordered; see `blocklace.rs`'s `satisfies_chain_axiom` for the Rust-side version of the same invariant |
| `Acknowledges` | `acknowledges_equivocation` |
| `Hides` (`¬ Acknowledges`) | `hidden_equivocations` / `HiddenEquivocation` reporting a nonempty `hidden` list — observing one branch but not the other still counts as hiding in both |
| `VouchesFor` | it's the equivocation-relevant slice of `approves` in `consensus/approval.rs`. `cordiality.rs` calls `approves` from inside `ratifies` (`"find all blocks in ratifier's closure that approve target"`), which is the point where this file's exclusion logic actually takes effect on the Rust side |
| `equivocation_exclusion` / `equivocation_not_approved` | the structural consequence of `ratifies`/`super_ratifies` filtering through `approves`: an equivocating pair can never both end up in the `approving` set `ratifies` builds, for any block that has already observed both branches |
