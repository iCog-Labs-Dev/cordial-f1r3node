# Approval Mechanics in Cordial Miners

## Overview

Approval is a fundamental predicate in the Cordial Miners protocol that determines whether a block has observed and validated a target block through its entire DAG view. It serves as the basis for consensus and finality decisions.

**Reference**: Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains" (arXiv:2205.09174)

## Definition

A block `approver` **approves** a `target` block if and only if:

1. **Observation**: The `approver` observes the `target` block
   - This means the `target` is in the transitive closure of the `approver`'s predecessor references
   - The `approver` has the `target` in its DAG view through a chain of predecessor links

2. **No Conflicting Equivocation**: The `approver` does NOT observe any equivocating sibling of the `target`
   - An equivocating sibling is another block created by the same validator at the same round/depth
   - If the `approver` can see multiple conflicting blocks from the same creator at the same round, it cannot approve any single one of them

## Implementation

### Function Signature

```rust
pub fn approves(
    blocklace: &Blocklace,
    approver: &BlockIdentity,
    target: &BlockIdentity,
) -> bool
```

### Algorithm

The approval check follows these steps:

1. **Retrieve the approver block** from the blocklace
   - If not found, return `false`

2. **Compute observed blocks** using the approver's predecessor closure
   - Transitively collects all blocks reachable from the approver's predecessors
   - Uses `observed_block_ids()` from the cordiality module

3. **Verify target is observed**
   - Check if the target is in the observed set
   - If not, return `false` (cannot approve what you don't see)

4. **Retrieve the target block** to determine its creator and round
   - If not found, return `false`

5. **Get the target's round** (depth) in the DAG
   - Compute using the longest path back to any genesis block

6. **Find all equivocating siblings** of the target at that round
   - Uses `equivocation_blocks_at_round()` from the cordiality module
   - Returns all blocks created by the same validator at the same round

7. **Check for conflicting observations**
   - For each equivocating sibling (excluding the target itself):
     - If the approver also observes that sibling, return `false`
   - If we reach the end without finding a conflicting sibling, return `true`

### Key Properties

- **Asymmetric**: If block A approves block B, block B may not approve block A
- **Transitive**:  If A approves B and B approves C, it does not necessarily follow that A approves C (due to equivocation checks)
- **Deterministic**: Given a fixed blocklace, the result is always the same
- **Non-blocking**: Approval can be computed without consensus or finality

## Examples

### Example 1: Simple Approval (No Equivocation)

```
Round 0:  [Block A by node 1]
          
Round 1:  [Block B by node 2, references A]

Round 2:  [Block C by node 3, references B]
```

**Query**: Does C approve A?

**Result**: `true`
- C observes A (through B)
- No equivocating siblings of A exist at round 0
- C approves A

### Example 2: Rejection Due to Observed Equivocation

```
Round 0:  [Block A₁ by node 1]  [Block A₂ by node 1] (equivocation)
          
Round 1:  [Block B by node 2, references both A₁ and A₂]
```

**Query**: Does B approve A₁?

**Result**: `false`
- B observes both A₁ and A₂ (both are equivocating siblings)
- Because B sees the equivocation, it cannot approve either branch
- B does not approve A₁

### Example 3: Approval Despite Equivocation Existing

```
Round 0:  [Block A₁ by node 1]  [Block A₂ by node 1] (equivocation, unknown to B)
          
Round 1:  [Block B by node 2, references only A₁]
```

**Query**: Does B approve A₁?

**Result**: `true`
- B observes A₁
- B does NOT observe A₂ (it only references A₁)
- Even though A₂ exists in the blocklace, B's lack of visibility into it means B can safely approve A₁
- B approves A₁

### Example 4: Transitive Observation

```
Round 0:  [Block A by node 1]

Round 1:  [Block B by node 2, references A]

Round 2:  [Block C by node 3, references B]

Round 3:  [Block D by node 4, references C]
```

**Query**: Does D approve A?

**Result**: `true`
- D observes C (direct predecessor)
- C observes B (direct predecessor)
- B observes A (direct predecessor)
- So D transitively observes A through C and B
- No equivocating siblings of A exist
- D approves A

## Relationship to Other Consensus Predicates

### Approval vs. Cordiality

- **Approval**: Focuses on whether a block observes a target and can validate it
- **Cordiality**: Broader check that ensures a block acknowledges ALL known equivocations and validator tips
- A block can be cordial without approving every other block (due to equivocation conflicts)

### Approval vs. Observed Blocks

- `observed_block_ids(block)` returns the full set of blocks the block can see
- `approves(approver, target)` is a binary predicate: does the approver support approval of the target?
- Every approved block is an observed block, but not every observed block is approved

### Approval and Finality

- Approval is a prerequisite for finality calculations
- Blocks must approve the target to participate in threshold calculations
- Finality emerges when sufficient stake approves the same target

## Querying Approvals

### Finding All Approving Blocks

```rust
pub fn approving_blocks(
    blocklace: &Blocklace,
    target: &BlockIdentity,
) -> HashSet<Block>
```

The `approving_blocks` function performs an exhaustive search across the entire blocklace to find all blocks that approve a given target.

**Algorithm**:
1. Iterate through all blocks in the blocklace using `blocklace.dom()`
2. For each block, call `approves(blocklace, &block.identity, target)`
3. Collect all blocks where `approves` returns `true`
4. Return the set of approving blocks

**Use Cases**:
- **Consensus calculation**: Determine how many blocks (or stake) approve a candidate block
- **Finality checks**: Build approval sets needed for finality predicates
- **Fork analysis**: Identify which blocks support different branches of the DAG
- **Validation debugging**: Understand why a particular block is or isn't approved

**Performance**:
- O(m) where m is the total number of blocks in the blocklace
- Each `approves` call is O(n) where n is the average predecessor closure size
- Overall O(m × n) in typical cases
- Suitable for analysis tools; not recommended for hot-path finality calculations

**Example**:

```rust
// Find all blocks that approve the target
let approvers = approving_blocks(&blocklace, &target.identity);

// Use the result to check consensus
if approvers.len() > 2 * blocklace.dom().len() / 3 {
    // Supermajority of blocks approve the target
}
```

## Implementation Notes

### Imports

The approval module reuses existing predicates to avoid reimplementation:

```rust
use std::collections::HashSet;
use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::cordiality::{equivocation_blocks_at_round, observed_block_ids};
use crate::consensus::round::depth;
use crate::types::BlockIdentity;
```

### Error Handling

The function returns `false` for:
- Approver block not in blocklace
- Target block not in blocklace
- Target not in approver's observed set
- Any equivocating sibling of the target is observed
- Unable to compute depth for the target

This conservative approach (fail-safe to `false`) ensures that invalid or impossible approval queries never incorrectly return `true`.

### Performance Considerations

- **Blocklace lookup**: O(1) for each block retrieval (hash map)
- **Observed computation**: O(n) where n is the size of the predecessor transitive closure
- **Equivocation check**: O(k) where k is the number of equivocating siblings (typically small)
- **Overall**: Efficient for typical DAG structures; quadratic worst case if many equivocations exist at same round

## Testing

The implementation is validated by 9 comprehensive test cases covering:

### Core `approves` Tests (7 cases)

1. **Approval without equivocation**: Standard case where blocks can safely approve
2. **Rejection with observed equivocation**: Blocks cannot approve when they see conflicting branches
3. **Approval despite existing equivocation**: Blocks unseen by the approver don't prevent approval
4. **Rejection when target not observed**: Blocks cannot approve what they don't see
5. **Edge case - approver not in blocklace**: Safe failure on invalid input
6. **Edge case - target not in blocklace**: Safe failure on invalid input
7. **Transitive approval**: Approval works through multiple hops in the DAG

### `approving_blocks` Tests (2 cases)

8. **Correct approval set**: Verifies that `approving_blocks` returns exactly the set of blocks that approve a target
   - Tests multiple approving blocks
   - Tests blocks that don't observe the target (excluded)
   - Tests blocks that observe equivocating siblings (excluded)

9. **Empty approval set**: Verifies that `approving_blocks` returns an empty set when no blocks observe the target
   - Tests with multiple blocks that have no predecessors
   - Validates that absence of observation prevents approval

## Future Extensions

The current implementation is intentionally minimal and does not include:

- **Stake weighting**: All blocks are treated equally; future versions may weight by validator stake
- **Ratification**: Multi-round approval thresholds (see Definition 19 in the paper)
- **Super-ratification**: High-confidence approval across waves (see Definition 20)
- **Time-based decay**: Approval confidence over time

These extensions are planned for future phases of the protocol but are out of scope for the current approval mechanics implementation.

## References

- **Cordial Miners Paper**: "Cordial Miners: Voluntary Participation in Blockchains" (arXiv:2205.09174)
  - Definition 18: Approval predicate
  - Definition 19: Ratification (threshold approval)
  - Definition 20: Super-ratification (cross-wave approval)

- **Source Code**:
  - Module: `src/consensus/approval.rs`
  - Tests: `tests/test_approval.rs`
  - Related: `src/consensus/cordiality.rs` (equivocation detection)
  - Related: `src/consensus/round.rs` (depth computation)
