# Approval Mechanics in Cordial Miners

This document describes the implementation of the "Approval" predicate, which is the foundational consensus primitive in the Cordial Miners protocol.

## Definition of Approval

According to Definition 18 of the Cordial Miners paper, a block $b$ **approves** a candidate block $b'$ if:
1. $b'$ is in the observed history of $b$ ($b' \in obs(b)$).
2. $b$ does not observe any equivocation of $node(b')$ at $round(b')$.

This ensures that a validator only acknowledges a block if it is aware of its existence and has no evidence that its creator was dishonest in that round.

## Proof-of-Stake Weighting

In our implementation, we extend the binary approval relation to a weighted **Threshold Approval**. This allows the protocol to reason about the total stake supporting a candidate block.

### Weighted Logic

The weighted approval sum for a candidate $C$ as seen by an approver $A$ is:
$$ W(A, C) = \sum_{v \in S} weight(v) $$
Where $S$ is the set of unique validators $v$ that have produced at least one block $x \in obs(A)$ such that $x$ approves $C$.

### Per-Validator Weighting

To prevent a single validator from inflating their influence, we ensure that each validator's stake is counted **at most once** per approval check, even if they have multiple blocks in the causal history that observe the candidate.

## Implementation

The logic is implemented in `crates/cordial-miners-core/src/consensus/approval.rs`:

- **`approves_binary`**: The base predicate implementing Definition 18.
- **`approves_weighted`**: The stake-weighted version used for higher-level consensus tasks like ratification.
- **`ApprovalThreshold`**: A struct defining the required support (e.g., 2/3 for supermajority).

### Integration with Consensus Predicates

The implementation reuses the core `blocklace` and `cordiality` predicates:
- `observed_block_ids`: Reconstructs the visible DAG of a block.
- `depth`: Determines the round of each block.

## Tests

The module includes unit tests covering:
- Correct observation detection.
- Rejection of approval upon detecting creator equivocations.
- Prevention of validator stake double-counting.
