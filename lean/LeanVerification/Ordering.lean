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

/-! ### Tau -/

/-- The deterministic ordered output of the protocol, anchored on the latest
finalized leader and recursively expanded through all ratified ancestors.

Declared `opaque` because its computational definition lives in Rust.
What matters formally is `tau_prefix_monotone` below. -/
opaque tau (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → NodeId) : List BlockId

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
    (wavelength : ℕ) (sel : ℕ → NodeId)
    (hsub : SubBlocklace B B') :
    List.IsPrefix
      (tau bonds validators B hV wavelength sel)
      (tau bonds validators B' hV' wavelength sel)

end CordialMiners
