# Cordiality Predicate Implementation

This document describes the cordiality and equivocation predicates implemented in:

- `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `crates/cordial-miners-core/src/consensus/validation.rs`

The implementation is based on the Cordial Miners paper:

- Idit Keidar, Oded Naor, Ouri Poupko, Ehud Shapiro, *Cordial Miners: Fast and Efficient Consensus for Every Eventuality*
- arXiv:2205.09174v6
- https://arxiv.org/abs/2205.09174

## Scope

This module provides the DAG-facing predicates that sit between:

- structural helpers such as `round.rs` and `wave.rs`
- the block validation layer in `validation.rs`

It is intended to answer two questions:

1. Has a validator equivocated in a round?
2. Does a candidate block remain cordial, or does it hide a known equivocation?

## Paper Alignment

The paper separates three related ideas:

- dissemination through the blocklace
- equivocation exclusion
- ordering

Our current cordiality implementation belongs to the equivocation-exclusion side of the protocol.

The code follows the paper's general direction:

- blocks are reasoned about through the DAG they observe
- equivocations matter semantically, not just syntactically
- a block should not suppress relevant conflicting information already visible in the DAG

The implementation is also shaped by a practical limitation:

- validation does not know the creator's full private local view
- validation only knows the candidate block and the local blocklace

Because of that, the implementation uses a conservative interpretation of "known":

- a known equivocation is one already present in the local blocklace
- a candidate acknowledges it if the equivocation is visible through the closure of its predecessors

This is slightly stronger and more operational than the paper's abstract local-knowledge model, but it is usable during block validation.

## Implemented Predicates

### 1. `creator_blocks_at_round`

Returns all blocks by a specific creator at a specific depth/round.

This uses the depth logic from `round.rs` to group the DAG by round.

### 2. `equivocation_blocks_at_round`

Returns the set of same-round blocks by one creator when there are at least two.

This is the repository's explicit predicate for the user story:

> A validator equivocates if they create two different blocks in the exact same round.

### 3. `all_equivocations`

Scans the current blocklace and reports all same-round equivocations as:

- creator
- round
- conflicting block identities

### 4. `observed_block_ids`

Reconstructs the candidate block's visible DAG from its predecessor closure without inserting the block.

This is the core "what does this block acknowledge?" helper.

### 5. `acknowledges_equivocation`

Checks whether a candidate block's predecessor closure includes every branch of a same-round equivocation.

This is intentionally closure-based rather than predecessor-list-based. A block can acknowledge an equivocation indirectly through a predecessor that already carries both branches in its ancestry.

### 6. `hidden_equivocations`

Reports any locally known same-round equivocations that the candidate block fails to acknowledge.

This is the implementation of "must not hide known equivocations" in local validation terms.

### 7. `missing_known_tips`

Returns validator tips omitted by a candidate block.

This preserves the earlier cordiality check that a block should not omit the known tip frontier.

### 8. `is_cordial_block`

The current repo-level cordiality predicate is:

- no missing known tips
- no hidden known equivocations

In other words, a block is cordial only when it preserves both the visible tip frontier and visible equivocation evidence.

## Validation Integration

`validation.rs` now reuses the predicates from `consensus/cordiality.rs` instead of embedding the logic directly.

The strict cordiality path performs two checks:

1. `NotCordial { missing_tips }`
2. `HiddenEquivocation { creator, round, hidden }`

This keeps `validation.rs` as an enforcement layer and `cordiality.rs` as the source of truth for the math.

## Tests

The behavior is covered by:

- `crates/cordial-miners-core/tests/test_cordiality.rs`
- `crates/cordial-miners-core/tests/test_validation.rs`

Important test cases include:

- detecting same-round equivocation
- distinguishing same-round forks from same-creator blocks in different rounds
- acknowledging equivocation through predecessor closure
- rejecting blocks that hide a locally known equivocation
- preserving the earlier missing-tip cordiality checks

## Current Limitation

This implementation does **not** reconstruct the creator's full private local view from the paper.

That means:

- if a creator knew about an equivocation but omitted all evidence of it from the block's predecessor closure, validation cannot prove that from the DAG alone
- the current implementation therefore uses the local blocklace as the knowledge boundary

This is a deliberate engineering choice for now. It gives us a deterministic, testable predicate that can run inside block validation.

## Next Step

The next paper-aligned step is to build the approval and ratification predicates on top of this layer:

- `approves`
- `ratifies`
- `super-ratifies`

Those will feed into leader finality and the eventual `tau` ordering function described later in the paper.
