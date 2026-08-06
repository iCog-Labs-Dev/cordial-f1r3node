# Proof-of-Reputation Data Structures

This document records the first paper-aligned data model for `cordial-por`.
It is a guide for the next implementation issue; it should not introduce
reputation algorithms or Cordial Miners consensus behavior by itself.

The current pipeline is intentionally narrow:

```text
rating transactions
  -> validated round batch
  -> rating matrix
  -> normalized rating matrix
```

This stage validates `RatingRecord` instances and assembles a single-round
`RatingBatch`. The module `src/ratings.rs` owns that validation and
batch-ordering logic. The new `src/matrix.rs` module now owns deterministic
construction of a `RatingMatrix` from a validated `RatingBatch`. The module
`src/normalization.rs` owns paper-guided fixed-point normalization of matrix
values grouped by recipient.

Rating matrix construction and normalization are still data preparation only.
They do not compute Liquid Rank, update reputation values, or materialize a
dense matrix. Reputation values must not be updated in this stage.

## Paper Reference

Primary reference:

- Oladotun Aluko and Anton Kolonin, "Proof-of-Reputation: An Alternative
  Consensus Mechanism for Blockchain Systems", IJNSA, 2021.

Relevant sections:

- Section 4.1, "Consensus Mechanism"
- Section 4.1.1, "Consensus Group"
- Section 4.1.2, "Leader Selection"
- Section 4.1.3, "Block Publication"
- Section 4.2, "Reputation System"

The strict paper-first flow remains:

```text
rating transactions
  -> validated round batch
  -> rating matrix S
  -> normalized rating matrix S'
  -> previous reputation vector R
  -> liquid-rank reputation contribution P
  -> next reputation vector
  -> reputation list
  -> reputation block
```

The current implementation is in scope through normalized rating matrix
construction. Liquid Rank and reputation updates remain future work.

## File-Level Plan

### `src/types.rs`

Own the paper vocabulary. Define only deterministic data types here.

Planned types:

- `ReputationRound`
- `ReputationWeight`
- `RatingScore`
- `RatingRecord`
- `RatingBatch`
- `ReputationEntry`
- `ReputationList`
- `RatingMatrix`
- `NormalizedRatingEntry`
- `NormalizedRatingMatrix`
- `ReputationVector`
- `ReputationBlockHeader`
- `ReputationBlock`
- `ConsensusGroup`
- `ConsensusGroupMember`
- `LeaderSelection`

Rules:

- Use `cordial_miners_core::NodeId` for node/public-key identity.
- Use fixed-point integer fields for ratings and reputation values.
- Do not use `f32` or `f64` in consensus-relevant data.
- Keep entries ordered or orderable by `NodeId` for deterministic hashing and
  audit replay.
- For `RatingMatrix`, the canonical deterministic ordering is by `(recipient, rater)`,
  not insertion order.
- RatingMatrix is the canonical, deterministic representation of the paper's ratings matrix. It is intentionally stored as an ordered list of rating entries at this stage; dense matrix construction for numerical operations is deferred to the reputation/Liquid Rank implementation.
- The ordered list is deliberately kept as `(recipient, rater)` so it matches the paper's `S = [s_ij]` convention: rows index recipients and columns index raters, which is the layout later used by the liquid-rank update `P <- S · r`. The output preserves the batch round and the deterministic matrix ordering.
- A duplicate means the triple `(round, rater, recipient)`, not recipient-only duplication.
- Normalized rating values use fixed-point integers with `PorConfig::scale`; do
  not use `f32` or `f64` for consensus-relevant normalized values.
- Normalization is grouped by recipient because the paper defines the set of
  ratings received by recipient `i` as `{s_i1, ..., s_in}`.

### `src/config.rs`

Own protocol parameters, not runtime state.

Planned fields:

- fixed-point `scale`
- `initial_reputation`
- liquid-rank `alpha`
- rating bounds
- consensus group quota, paper default: reputation sum greater than 50 percent
  of total network reputation
- block publication quorum, paper default: greater than two-thirds of selected
  group reputation

### `src/state.rs`

Own the local reputation state container.

Planned state:

- current reputation round
- latest `ReputationList` or reputation map
- pending `RatingRecord`s for the next round
- latest accepted `ReputationBlock`

This file should not implement liquid-rank math. It should expose state access
and later delegate transitions to a dedicated calculation module.

### `src/weights.rs`

Own conversion from reputation state to Cordial Miners weighted-path inputs.

Planned role:

- export `HashMap<NodeId, u64>`
- keep the boundary explicit: `cordial-por` computes weights,
  `cordial-miners-core` consumes weights

This file should not implement ratification, finality, or tau ordering.

### Current and Future Files

The current implementation includes `src/ratings.rs`, which is responsible for
validation + deterministic round batching, `src/matrix.rs`, which builds the
canonical rating matrix, and `src/normalization.rs`, which applies the
paper-guided modified normalization formula.

Future work remains:

- `src/liquid_rank.rs`: liquid-rank calculation
- `src/block.rs`: reputation block construction helpers
- `src/audit.rs`: replay and transition verification
- `src/committee.rs`: consensus group selection
- `src/leader.rs`: leader selection from the consensus group

`EquivocationPenalty` and `InactivityPenalty` remain intentionally as Cordial
integration extensions and are not part of the first reputation calculation
step. Liquid Rank, reputation updates, and all later consensus-selection logic
remain future work.

## Paper-Aligned Structures

### Node Identity

Paper concept:

```text
Each node i is identified by public key pk_i.
```

Implementation target:

```text
src/types.rs
```

Use:

```text
cordial_miners_core::NodeId
```

### Rating Transaction

Paper concept:

```text
At the end of an interaction, a rater gives a recipient a rating in [0, 1].
The rating transaction is signed and broadcast.
```

Implementation target:

```text
src/types.rs
```

Planned shape:

```text
RatingRecord {
    round,
    rater,
    recipient,
    score,
    signed_payload_or_signature,
    interaction_ref,
}
```

### Rating Matrix

Paper concept:

```text
Ratings form matrix S = [s_ij].
```

Implementation target:

```text
src/types.rs
```

This may be a derived/internal structure rather than persisted chain data.

### Normalized Rating Matrix

Paper concept:

```text
Values s_i are normalized before the ratings matrix S = [s_ij] is used with the
previous-round rater reputation vector R.
```

Implementation target:

```text
src/types.rs
src/normalization.rs
```

The implementation uses the paper's modified normalization formula to avoid
null values. In fixed-point form, the paper's `+1` is represented by
`PorConfig::scale`:

```text
normalized = (((score - min) + scale) * scale) / ((max - min) + scale)
```

Normalization is performed per recipient row and preserves the canonical
`(recipient, rater)` ordering produced by `build_rating_matrix`.

### Reputation Vector

Paper concept:

```text
Previous rater reputations are blended with normalized ratings.
```

Implementation target:

```text
src/types.rs
```

Planned shape:

```text
ReputationVector {
    round,
    values: NodeId -> ReputationWeight,
}
```

### Reputation List

Paper concept:

```text
ReputationList_i contains all network nodes and their reputation values for
the latest round.
```

Implementation target:

```text
src/types.rs
```

Planned shape:

```text
ReputationList {
    round,
    entries,
}
```

### Reputation Block

Paper concept:

```text
ReputationBlock_k = (Header_k, ReputationList_k)
```

Implementation target:

```text
src/types.rs
```

Planned shape:

```text
ReputationBlockHeader {
    round,
    previous_reputation_hash,
    ratings_hash,
    reputation_root,
}

ReputationBlock {
    header,
    reputation_list,
}
```

### Consensus Group

Paper concept:

```text
G_k is selected from highest-reputation nodes whose collective reputation
exceeds 50 percent of total network reputation.
```

Implementation target:

```text
src/types.rs
src/committee.rs
```

`src/types.rs` should define the data shape. `src/committee.rs` should later
implement selection.

### Leader Selection

Paper concept:

```text
Leader L_k is randomly selected from G_k.
```

Implementation target:

```text
src/types.rs
src/leader.rs
```

`src/types.rs` should define the selected leader record. `src/leader.rs`
should later implement deterministic leader selection policy.

## Explicit Non-Goals

Do not include these in the current normalization stage:

- Liquid-rank calculation implementation
- Reputation clamping implementation
- Committee selection implementation
- Leader selection implementation
- Cordial Miners approval, ratification, finality, or tau ordering
- Cordial-specific penalty or slashing behavior implementation

Cordial-specific penalty behavior should come after the paper-guided reputation
calculation path is implemented.
