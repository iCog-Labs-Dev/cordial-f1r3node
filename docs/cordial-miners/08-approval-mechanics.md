# Approval Mechanics: Proof-of-Stake Extension

**Document**: 08-approval-mechanics.md  
**Module**: `crates/cordial-miners-core/src/finality.rs`  
**Paper Reference**: Definition 18 (Approves), Cordial Miners: Fast and Efficient Consensus for Every Eventuality (arXiv:2205.09174)

## Overview

Approval is the fundamental building block of the Cordial Miners consensus protocol. A block **approves** a candidate block if it observes the candidate in its causal history and does not observe any conflicting equivocations from the candidate's creator. This document extends the paper's original definition to support Proof-of-Stake (PoS) consensus by introducing weighted approval.

---

## Definition 18: Block Approves (Original Paper)

From the Cordial Miners paper:

> Block `b` **approves** candidate block `b'` if:
> 1. `b` observes `b'` (i.e., `b'` is in `b`'s causal history, inclusive)
> 2. `b` does NOT observe any equivocating block of `b'`

An **equivocating block** of `b'` is any other block `b''` created by the same node as `b'` (i.e., `node(b') = node(b'')`) that is not comparable under the precedence relation (violating the chain axiom).

### Key Distinction: Observation vs. Approval

- **Observation** (`≽`): Transitive. If `b` observes `b'` and `b'` observes `b''`, then `b` observes `b''`.
- **Approval**: NOT transitive. Even if `b` approves `b'` and `b'` approves `b''`, `b` may NOT approve `b''` if `b` also observes an equivocating sibling of `b''`.

---

## Proof-of-Stake Extension: Weighted Approval

The protocol is extended to Proof-of-Stake by introducing a **ValidatorSet** that assigns stake weights to validators, and an **ApprovalThreshold** that defines the minimum fraction of total stake required for consensus.

### ApprovalThreshold

Represents a rational threshold `numerator / denominator`:

```rust
pub struct ApprovalThreshold {
    pub numerator: u128,
    pub denominator: u128,
}
```

**Examples**:
- `2/3` majority: `ApprovalThreshold { numerator: 2, denominator: 3 }`
- `1/2` majority: `ApprovalThreshold { numerator: 1, denominator: 2 }`
- Unanimous: `ApprovalThreshold { numerator: 1, denominator: 1 }`

### Weighted Approval Condition

A candidate block `b'` is **weighted-approved** by an observer block `b` if:
1. `b` approves `b'` according to the basic approval rules (Definition 18)
2. The sum of weights of all blocks in `b`'s causal history that approve `b'` exceeds the ApprovalThreshold relative to total validator stake

**Formally**:
```
weighted_approve(b, b', threshold, validators) =
    approves(b, b') ∧
    Σ{weight(node(bi)) | bi ∈ observe(b), approves(bi, b')} / total_weight > numerator / denominator
```

### Integer-Only Arithmetic (No Floating Point)

To avoid floating-point precision issues, threshold checks use integer cross-multiplication:

$$\text{support\_weight} \times \text{threshold.denominator} > \text{total\_weight} \times \text{threshold.numerator}$$

**Example**: Checking if `100/150 > 2/3`
- Cross multiply: `100 * 3 = 300` vs `150 * 2 = 300`
- `300 > 300` is `false`, so the threshold is NOT exceeded

---

## Implementation Details

### Unified Approval: `approve()`

The `approve()` function consolidates both basic and weighted approval into a single function that:
1. Performs the basic approval check from Definition 18
2. Computes weighted stake sums from the observer's causal history
3. Validates against the ApprovalThreshold

**Location**: [`finality.rs` lines 155-214](finality.rs#L155-L214)

```rust
pub fn approve<VS>(
    blocklace: &Blocklace,
    b: &BlockIdentity,
    b_prime: &BlockIdentity,
    threshold: ApprovalThreshold,
    validators: &VS,
) -> bool
where
    VS: for<'a> ValidatorSet<&'a NodeId, Weight = u128>,
```

### Basic Approval Check: `approves()`

**Location**: [`finality.rs` lines 115-153](finality.rs#L115-L153)

```rust
pub fn approves(blocklace: &Blocklace, b: &BlockIdentity, b_prime: &BlockIdentity) -> bool
```

This is the foundation function used by `approve()` and performs the basic equivocation check:

**Algorithm**:
1. Check if `b` observes `b'` using the blocklace's causal history
2. Get all blocks created by the creator of `b'`
3. For each other block by the same creator:
   - If `b` observes both blocks and neither precedes the other (equivocation):
     - Return `false` (b does not approve b_prime due to equivocation)
4. Return `true` (b observes b_prime and no equivocations)

**Complexity**: O(n + m) where n = size of b's causal history, m = number of blocks by b_prime's creator

### Weighted Approval: `approve()`

**Location**: [`finality.rs` lines 155-214](finality.rs#L155-L214)

```rust
pub fn approve<VS>(
    blocklace: &Blocklace,
    b: &BlockIdentity,
    b_prime: &BlockIdentity,
    threshold: ApprovalThreshold,
    validators: &VS,
) -> bool
where
    VS: for<'a> ValidatorSet<&'a NodeId, Weight = u128>,
```

### Weighted Approval Algorithm (within `approve()`)

**Algorithm**:
1. Check if `b` itself approves `b_prime` using `approves()` (basic approval check fails → return false)
2. Iterate through all blocks in `b`'s causal history:
   - For each block that approves `b_prime`, add its creator's weight to `support_weight`
3. Retrieve `total_weight` from the ValidatorSet
4. Use cross-multiplication to check: `support_weight * threshold.denominator > total_weight * threshold.numerator`

**Complexity**: O(n + m × k) where:
- n = size of b's causal history
- m = number of blocks that approve b_prime
- k = cost of weight lookup in ValidatorSet (typically O(1) or O(log N))

---

## Design Rationale

### Why Weighted Approval?

1. **Byzantine Fault Tolerance**: In Proof-of-Stake, voting power is distributed unequally. A protocol that treats all validators equally would allow malicious actors with small stake to block consensus.

2. **Supermajority Safety**: Requiring 2/3 or more of total stake ensures that no single validator or coalition with < 1/3 stake can produce conflicting chains.

3. **Economic Security**: Stake is at risk; validators that equivocate or double-sign can lose their stake. Weighted consensus aligns incentives with honest participation.

### Why Integer Cross-Multiplication?

Floating-point arithmetic introduces precision errors that could lead to:
- Different nodes reaching different consensus decisions
- Non-deterministic behavior across different architectures/implementations

Integer arithmetic ensures **exact, deterministic computation** on all systems.

### Why Not Transitive Approval?

Non-transitivity is intentional and mathematically necessary:
- **Scenario**: Validator X equivocates with blocks X1 and X2 (both in blocklace)
  - If b observes X1, b cannot approve any block in X1's chain
  - Even if X1 "approved" some future block Y, b should not approve Y
  - This prevents equivocators from indirectly approving through their ancestry

---

## ValidatorSet Trait

The generic implementation relies on the `ValidatorSet` trait defined in [cordiality.rs](cordiality.rs):

```rust
pub trait ValidatorSet<V> {
    type Weight;

    fn weight_of(&self, validator: &V) -> Option<Self::Weight>;
    fn total_weight(&self) -> Self::Weight;
}
```

**For Approval**: We use `ValidatorSet<&NodeId, Weight = u128>`

This allows different implementations:
- **Fixed stake**: All validators have equal weight
- **Dynamic stake**: Validators can increase/decrease stake over time
- **Delegated stake**: Stake can be delegated from one validator to another

---

## Test Coverage

Three acceptance criteria from the specification:

### Test 1: Candidate Not Observed

```rust
#[test]
fn test_approve_returns_false_if_candidate_not_observed()
```

**Setup**: Blocklace with:
- Genesis block (validator 1)
- Block by validator 2 observing genesis
- Isolated candidate block by validator 3 (not reachable from block_v2)

**Expected**: `approves(block_v2, candidate)` returns `false`

### Test 2: Stake Threshold

```rust
#[test]
fn test_weighted_approve_returns_true_above_threshold()
```

**Setup**: Blocklace with:
- Candidate block and multiple approval blocks
- Validators A (weight 2), B (weight 1), C (weight 2) → total 5
- 2/3 threshold requires > 3.33 support weight

**Expected**: With A and C approving (weight 4), `approve()` returns `true`

```rust
#[test]
fn test_weighted_approve_returns_false_below_threshold()
```

**Expected**: With only A approving (weight 2), `approve()` returns `false`

### Test 3: Equivocation Blocks Approval

```rust
#[test]
fn test_approve_returns_false_with_equivocation()
```

**Setup**: Two equivocating blocks (incomparable under precedence) by validator 2

**Expected**: Neither equivocating block is approved by observers of both

```rust
#[test]
fn test_weighted_approve_returns_false_with_equivocation_despite_weight()
```

**Expected**: High stake cannot overcome equivocation rejection — `approve()` returns `false` even with sufficient weight

---

## Integration with Consensus

### Role in Protocol

Approval is used in the following consensus steps:

1. **Ratification** (Definition A.9): A block ratifies a target if its closure includes a supermajority of blocks that approve the target
2. **Super-ratification** (Definition A.9): A set of blocks super-ratifies a target if they include a supermajority that ratify the target
3. **Leader Finality** (Algorithm 4): A leader block is final if super-ratified within its wave

### Future Extensions

- **Conditional Approval**: Extend to support conditional approval (e.g., "approve if X1, reject if X2")
- **Weighted Equivocation Penalties**: Reduce validator weight upon detection of equivocation
- **Dynamic Threshold Adjustment**: Adapt threshold based on network synchrony assumptions

---

## References

- Cordial Miners Paper: [arXiv:2205.09174](https://arxiv.org/abs/2205.09174)
  - Definition 18 (Approves)
  - Definition A.9 (Ratification, Super-ratification)
  - Definition A.12 (Cordial condition)
- Proof-of-Stake Consensus: [Casper FFG](https://arxiv.org/abs/1710.09437), [Casper CBC](https://github.com/cbc-casper/cbc-casper-paper)
- Related Work: Hotstuff ([arXiv:1803.05069](https://arxiv.org/abs/1803.05069)), Tendermint ([arXiv:1807.04938](https://arxiv.org/abs/1807.04938))
