# KR2 — Equivocation and Exclusion

Module: `LeanVerification/Equivocation.lean`

## Derived DAG depth

Rust defines a block's round as its DAG depth: initial blocks have depth zero,
and every other block has one plus the maximum depth of its predecessors. Lean
computes the same quantity by well-founded recursion:

```lean
def blockDepthWF
    (B : Blocklace)
    (hwf : WellFounded (fun a b => DirectPred B b a)) :
    BlockId → Nat

def blockDepth (B : Blocklace) (hV : ValidBlocklace B) (b : BlockId) : Nat :=
  blockDepthWF B (directPred_wf_of_valid B hV) b
```

`ValidBlocklace` supplies the proved well-founded predecessor relation. No
round projection, typeclass, or unproved “round decreases along observation”
axiom is introduced.

## Forks, equivocation, and honesty

The Rust implementation uses two related but different conflict notions:

- `equivocation_blocks_at_round` detects same-creator forks at one depth.
- `approves` rejects every observed incomparable same-creator block, even when
  the two blocks have different depths.

The Lean model therefore keeps both predicates explicit:

```lean
def Fork (B : Blocklace) (b₁ b₂ : BlockId) : Prop :=
  b₁ ∈ B.keys ∧ b₂ ∈ B.keys ∧
  creatorOf B b₁ = creatorOf B b₂ ∧ b₁ ≠ b₂ ∧
  ¬ Observes B b₁ b₂ ∧ ¬ Observes B b₂ b₁

def Equivocation
    (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ : BlockId) : Prop :=
  Fork B b₁ b₂ ∧ blockDepth B hV b₁ = blockDepth B hV b₂
```

`Fork` is the round-independent conflict relation used by approval and the
all-pairs chain invariant. `Equivocation` is the narrower same-depth relation
reported by Rust's equivocation detector and requested by Issue #186.

```lean
def Equivocator (B : Blocklace) (hV : ValidBlocklace B) (v : NodeId) : Prop :=
  ∃ b₁ b₂, Equivocation B hV b₁ b₂ ∧ creatorOf B b₁ = some v

def HonestIn (B : Blocklace) (v : NodeId) : Prop :=
  ¬ ∃ b₁ b₂, Fork B b₁ b₂ ∧ creatorOf B b₁ = some v
```

`Equivocator` matches the same-depth detector. `HonestIn` is deliberately the
stronger chain-honesty predicate: a validator is honest only when none of its
blocks form a structural fork, including a cross-depth fork. Consequently:

```lean
theorem honest_chain_linearity (B : Blocklace) (v : NodeId)
    (h : HonestIn B v) :
    IsChain (Observes B) (blocksBy B v)

theorem honest_not_equivocator
    (B : Blocklace) (hV : ValidBlocklace B) (v : NodeId)
    (h : HonestIn B v) :
    ¬ Equivocator B hV v
```

The converse is intentionally not claimed: absence from the same-depth Rust
detector alone does not rule out incomparable blocks at different depths.

## Acknowledgement before and after insertion

Rust validates a candidate before inserting it. Its
`acknowledges_equivocation` function receives the full candidate block and
reconstructs a view from the candidate's declared predecessors. Lean models
that stage directly:

```lean
def CandidateObserves (B : Blocklace) (candidate : Block) (b : BlockId) : Prop :=
  ∃ p ∈ candidate.content.predecessors, Observes B p b

def CandidateAcknowledges
    (B : Blocklace) (candidate : Block) (b₁ b₂ : BlockId) : Prop :=
  CandidateObserves B candidate b₁ ∧ CandidateObserves B candidate b₂

def CandidateHides
    (B : Blocklace) (candidate : Block) (b₁ b₂ : BlockId) : Prop :=
  ¬ CandidateAcknowledges B candidate b₁ b₂
```

After insertion, ordinary observation from the candidate's identifier is the
appropriate relation for approval and descendant monotonicity:

```lean
def Acknowledges (B : Blocklace) (c b₁ b₂ : BlockId) : Prop :=
  Observes B c b₁ ∧ Observes B c b₂

def Hides (B : Blocklace) (c b₁ b₂ : BlockId) : Prop :=
  ¬ Acknowledges B c b₁ b₂
```

`candidateAcknowledges_of_inserted` proves the bridge: when a candidate is
stored under its own ID, everything reconstructed from its declared
predecessors becomes ordinarily observable from that ID.

`acknowledgement_monotone` then proves that an inserted descendant observing an
acknowledging block also acknowledges both branches.

## Exclusion

```lean
def VouchesFor (B : Blocklace) (c b : BlockId) : Prop :=
  Observes B c b ∧
    ∀ b', creatorOf B b' = creatorOf B b → b' ≠ b → Observes B c b' →
      ¬ (¬ Observes B b b' ∧ ¬ Observes B b' b)
```

`VouchesFor` is the equivocation-relevant part of Rust `approves`: an approver
must observe the target and must not also observe an incomparable block by the
same creator.

```lean
theorem equivocation_exclusion
    (B : Blocklace) (hV : ValidBlocklace B) (b₁ b₂ c : BlockId)
    (heq : Equivocation B hV b₁ b₂)
    (hack : Acknowledges B c b₁ b₂) :
    ¬ VouchesFor B c b₁ ∧ ¬ VouchesFor B c b₂
```

If `c` observes both branches of a same-depth equivocation, each branch is an
observed incomparable sibling of the other. Therefore `c` can vouch for
neither. In the issue's wording: no single approving block can vouch for both
sides of the same lie.

`equivocation_not_approved` packages this result for Issue #187's eventual
concrete `Approves` relation. It requires the two explicit bridge hypotheses
from that relation to `VouchesFor`; no equivalence is assumed here.

## Concrete example

The worked example constructs four actual blocks:

- `g`, the honest validator's only block;
- `e1` and `e2`, two empty-predecessor blocks by the cheater at depth zero;
- `c`, a third validator's block with `{e1.id, e2.id}` as predecessors.

The blocklace is built through four `ValidBlocklace.insert` derivations.
The file proves that `e1` and `e2` form a same-depth `Equivocation`, that the
cheater is not `HonestIn`, and that the honest validator is `HonestIn`.

Before `c` is inserted, `candidate_c_acknowledges_before_insert` proves the
same acknowledgement Rust computes from the candidate's predecessor list.
After insertion, `c_acknowledges` uses the bridge theorem to produce an actual,
satisfiable `Acknowledges` witness. `demo_exclusion` then applies the exclusion
theorem without an assumed or impossible acknowledgement hypothesis.

## Lean ↔ Rust mapping

| Lean | Rust |
|---|---|
| `blockDepth` | `consensus::round::depth` |
| `creatorOf` | `BlockIdentity.creator` via block lookup |
| `Fork` | the incomparable same-creator conflict checked inside `consensus::approval::approves` |
| `Equivocation` | one pair returned by `equivocation_blocks_at_round` |
| `Equivocator` | a creator with a nonempty entry in `all_equivocations` |
| `HonestIn`, `honest_chain_linearity` | `Blocklace::satisfies_chain_axiom` |
| `CandidateObserves` | `cordiality::observed_block_ids` for a not-yet-inserted candidate |
| `CandidateAcknowledges` | `cordiality::acknowledges_equivocation` |
| `CandidateHides` | `cordiality::hidden_equivocations` reporting a missing branch |
| `Acknowledges`, `Hides` | the corresponding post-insertion observation predicates |
| `VouchesFor` | the equivocation-relevant condition in `consensus::approval::approves` |
| `equivocation_exclusion` | exclusion resulting when one approver observes both same-depth branches |
| `equivocation_not_approved` | the bridge consumed by the future concrete Lean `Approves` relation |
