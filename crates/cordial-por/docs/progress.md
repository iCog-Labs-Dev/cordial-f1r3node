# Proof-of-Reputation Progress

This document explains what has already been implemented in `cordial-por`,
what each stage means conceptually, and what remains in the paper-aligned PoR
pipeline.

## Current Pipeline

```text
RatingRecord
  -> RatingBatch
  -> RatingMatrix S
  -> NormalizedRatingMatrix S'
  -> Liquid-Rank contribution P
```

The current implementation can take round-scoped ratings, normalize them, and
compute the paper-guided Liquid-Rank contribution:

```text
P = S' * R_k
```

It does not yet compute the final next-round reputation vector.

## Accomplished

### 1. PoR Crate Scaffold

`cordial-por` now exists as the dedicated crate for Proof-of-Reputation data,
calculation, and weight export.

It owns PoR-specific logic. Cordial Miners consensus logic stays outside this
crate.

### 2. Initial PoR Data Model

The crate defines the core data structures needed for the PoR pipeline:

- `RatingRecord`
- `RatingBatch`
- `RatingMatrix`
- `NormalizedRatingMatrix`
- `ReputationVector`
- `ReputationList`
- `ReputationState`
- `ReputationBlock`
- penalty placeholders
- consensus group placeholders

These types give us the paper vocabulary in code.

## Concept: RatingRecord

`RatingRecord` means one rating transaction.

Example:

```text
A rates B with score 0.8 in round 7
```

Conceptual shape:

```text
RatingRecord {
  round: 7,
  rater: A,
  recipient: B,
  score: 0.8,
  signature: ...
}
```

It answers:

```text
Who rated whom, in which round, with what score?
```

## Concept: RatingBatch

`RatingBatch` means many rating records collected for the same reputation round.

Example:

```text
RatingBatch {
  round: 7,
  ratings: [
    A rates B,
    C rates B,
    A rates D
  ]
}
```

It validates and groups ratings before matrix construction.

This stage ensures:

- every rating belongs to the same round
- ratings are signed
- self-ratings are rejected
- scores are within configured bounds
- duplicate `(round, rater, recipient)` ratings are rejected
- ratings are ordered deterministically

Pipeline stage:

```text
RatingRecord -> RatingBatch
```

## Concept: RatingMatrix S

The rating matrix represents all ratings for a round.

Paper concept:

```text
S = [s_ij]
```

Where:

- `i` is the recipient node
- `j` is the rater node
- `s_ij` is the rating given by rater `j` to recipient `i`

Example:

```text
A rates X = 0.8
B rates X = 0.6
A rates Y = 0.4
```

Matrix interpretation:

```text
s_X,A = 0.8
s_X,B = 0.6
s_Y,A = 0.4
```

In code, we store this as a sparse ordered list, not a dense matrix.

Canonical order:

```text
(recipient, rater)
```

Pipeline stage:

```text
RatingBatch -> RatingMatrix S
```

## Concept: Normalization

Normalization turns raw ratings into stable, bounded inputs for reputation
calculation.

Why this is needed:

- raw ratings may have different ranges
- each recipient may receive different rating distributions
- the lowest rating should not become a useless zero
- Liquid Rank needs comparable fixed-point inputs

Paper modified normalization:

```text
s'_ij = ((s_ij - min_i) + 1) / ((max_i - min_i) + 1)
```

Fixed-point implementation:

```text
normalized_score =
    ((score - min_score) + scale) * scale
    / ((max_score - min_score) + scale)
```

Example with `scale = 100`:

```text
A rates X = 20
B rates X = 80

min = 20
max = 80
```

For `A -> X`:

```text
normalized = ((20 - 20) + 100) * 100 / ((80 - 20) + 100)
normalized = 10000 / 160
normalized = 62
```

For `B -> X`:

```text
normalized = ((80 - 20) + 100) * 100 / ((80 - 20) + 100)
normalized = 16000 / 160
normalized = 100
```

Pipeline stage:

```text
RatingMatrix S -> NormalizedRatingMatrix S'
```

## Concept: Previous Reputation Vector R_k

`R_k` is the reputation vector from the previous round.

Example:

```text
R_k = {
  A: 80,
  B: 20,
  C: 50
}
```

This matters because PoR does not treat every rater equally.

The rating from a high-reputation node should carry more influence than the
same rating from a low-reputation node.

## Concept: Liquid-Rank Contribution P

Liquid-Rank contribution computes the reputation signal created by the current
round's ratings.

Pipeline stage:

```text
NormalizedRatingMatrix S' + previous reputation vector R_k -> contribution vector P
```

Paper formula:

```text
P = S' * R_k
```

Expanded per recipient:

```text
P_i = sum_j(s'_ij * R_j)
```

Where:

- `P_i` is the contribution received by node `i`
- `s'_ij` is the normalized rating from rater `j` to recipient `i`
- `R_j` is the previous reputation of rater `j`

Important:

```text
P is not the final reputation yet.
```

It is only the current rating-based contribution.

### Example: Same Rating, Different Rater Reputation

Previous reputation:

```text
A = 80
B = 20
```

Normalized ratings:

```text
A rates X = 0.9
B rates X = 0.9
```

Contribution:

```text
P_X = (0.9 * 80) + (0.9 * 20)
P_X = 72 + 18
P_X = 90
```

Both raters gave the same score, but `A` contributes more because `A` has
higher reputation.

### Example: Lower Rating From Higher Reputation Rater

Previous reputation:

```text
A = 80
B = 20
```

Normalized ratings:

```text
A rates X = 0.5
B rates X = 1.0
```

Contribution:

```text
P_X = (0.5 * 80) + (1.0 * 20)
P_X = 40 + 20
P_X = 60
```

Even though `B` gave the maximum rating, `B` has lower reputation, so the
overall contribution is still strongly affected by `A`.

### Example: Multiple Recipients

Previous reputation:

```text
A = 80
B = 20
C = 50
```

Normalized ratings:

```text
A rates X = 0.9
B rates X = 0.9

A rates Y = 0.4
C rates Y = 1.0
```

Contribution for `X`:

```text
P_X = (0.9 * 80) + (0.9 * 20)
P_X = 72 + 18
P_X = 90
```

Contribution for `Y`:

```text
P_Y = (0.4 * 80) + (1.0 * 50)
P_Y = 32 + 50
P_Y = 82
```

Contribution vector:

```text
P = {
  X: 90,
  Y: 82
}
```

### Fixed-Point Form

The implementation avoids floating-point values.

If:

```text
scale = 1_000_000_000
```

Then:

```text
0.9 = 900_000_000
0.5 = 500_000_000
1.0 = 1_000_000_000
```

Fixed-point formula:

```text
P_i = sum_j(normalized_score_ij * reputation_j) / scale
```

Example:

```text
P_X = (900_000_000 * 80) / 1_000_000_000
P_X = 72
```

The implementation accumulates first using `u128`, then divides by `scale`.
This preserves more fixed-point precision and stays aligned with the paper's
real-number formula.

## What Needs To Be Done Next

### 1. Reputation Transition With Alpha Blend

The next paper-aligned step is to combine:

- previous reputation vector `R_k`
- Liquid-Rank contribution vector `P`
- configured alpha value `alpha`

Pipeline stage:

```text
P + R_k + alpha -> R_next
```

Paper-guided meaning:

```text
new reputation = recent contribution + previous reputation influence
```

Fixed-point formula:

```text
R_next_i =
    (alpha * P_i + (scale - alpha) * R_k_i) / scale
```

This implements the paper's temporal scoping principle:

```text
older reputation still matters, but recent behavior can change reputation over time
```

Example with `scale = 100`:

```text
alpha = 60
P_X = 90
R_k_X = 50
```

Then:

```text
R_next_X = (60 * 90 + (100 - 60) * 50) / 100
R_next_X = (5400 + 2000) / 100
R_next_X = 74
```

So node `X` moves from `50` toward the new contribution `90`, but does not jump
there immediately.

### 2. Sigmoid Clamp

After alpha blending, the paper clamps reputation to prevent sharp jumps.

Paper formula:

```text
R_clamped = R_next / sqrt(1 + R_next^2)
```

Purpose:

- bound reputation growth
- reduce sudden reputation jumps
- make reputation movement smoother across rounds

Example intuition:

```text
small values move mostly unchanged
very large values get compressed
```

Implementation note:

```text
This must be deterministic and fixed-point.
Avoid f32/f64 in consensus-relevant calculation.
```

### 3. Reputation State Update

After computing the final clamped vector, apply it to the state.

Pipeline stage:

```text
R_clamped -> ReputationState / ReputationList
```

This should:

- preserve deterministic `NodeId` ordering
- update each node's reputation
- advance the reputation round
- keep math helpers separate from state mutation

### 4. Reputation Block And Audit Path

The paper stores reputation values in a reputation chain or sidechain.

Paper structure:

```text
ReputationBlock = ReputationBlockHeader + ReputationList
```

Purpose:

- publish reputation updates
- let validators replay reputation calculation
- make reputation values auditable
- avoid depending on a central reputation authority

### 5. Consensus Group Selection

The paper selects the highest-reputation nodes into a consensus group.

Rule:

```text
sum(selected reputation) > 50% of total network reputation
```

Purpose:

- choose a dynamic group of high-reputation validators
- keep consensus group selection tied to reputation distribution

Example:

```text
Total reputation = 300
Threshold > 150

A = 90
B = 70
C = 50
D = 40
```

Select from highest reputation:

```text
A + B = 160
```

Since:

```text
160 > 150
```

The consensus group can be:

```text
G = { A, B }
```

### 6. Leader Selection

After selecting the consensus group, choose the round leader from that group.

Pipeline stage:

```text
ConsensusGroup -> LeaderSelection
```

This should remain paper-guided, but adapted carefully to Cordial Miners.

### 7. Cordial Miners Weighted Path Adapter

Finally, reputation values are exported as weights for Cordial Miners.

Pipeline stage:

```text
ReputationState -> HashMap<NodeId, ReputationWeight>
```

Current output shape:

```rust
HashMap<NodeId, u64>
```

This allows the existing Cordial Miners weighted path to consume PoR reputation
without moving consensus logic into `cordial-por`.

## Recommended Next Issue

### Title

Implement reputation transition with alpha blend

### Goal

Compute the next-round reputation vector from the Liquid-Rank contribution
vector and the previous reputation vector.

### Formula

```text
R_next_i =
    (alpha * P_i + (scale - alpha) * R_k_i) / scale
```

### Acceptance Criteria

- Accepts a Liquid-Rank contribution vector `P`.
- Accepts previous reputation vector `R_k`.
- Uses `PorConfig::liquid_rank_alpha`.
- Computes the next reputation vector deterministically.
- Uses fixed-point integer arithmetic.
- Rejects invalid alpha values.
- Does not mutate `ReputationState`.
- Does not build reputation blocks.
- Includes focused tests for successful blend, zero alpha, full alpha, missing previous reputation, and overflow.

## Later Issue

### Title

Implement deterministic sigmoid clamp for reputation transition

### Formula

```text
R_clamped = R_next / sqrt(1 + R_next^2)
```

This should be handled as a separate issue because deterministic fixed-point
square root needs careful implementation and testing.