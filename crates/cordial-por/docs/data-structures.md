# Proof-of-Reputation Data Structures

This document records the paper-aligned data model and implemented reputation
calculation pipeline for `cordial-por`. It does not introduce Cordial Miners
consensus behavior.

The current pipeline is intentionally narrow:

```text
rating transactions
  -> validated round batch
  -> rating matrix
  -> normalized rating matrix
  -> liquid-rank contribution vector
  -> alpha-blended next reputation vector
  -> clamped reputation vector
  -> committed reputation block
  -> audited reputation state snapshot
```

This stage validates `RatingRecord` instances and assembles a single-round
`RatingBatch`. The module `src/ratings.rs` owns that validation and
batch-ordering logic. The new `src/matrix.rs` module now owns deterministic
construction of a `RatingMatrix` from a validated `RatingBatch`. The module
`src/normalization.rs` owns paper-guided fixed-point normalization of matrix
values grouped by recipient. The module `src/liquid_rank.rs` owns the
paper-guided `P = S * R` contribution calculation from a normalized matrix and
previous reputation vector.

The module `src/transition.rs` blends that contribution with the previous
reputation using `PorConfig::liquid_rank_alpha` and the fixed-point scale. It
returns a new deterministic vector and does not mutate `ReputationState`.
The module `src/clamp.rs` applies the paper-guided sigmoid clamp using
deterministic fixed-point integer arithmetic.
The module `src/state.rs` can apply the finalized vector as the current
`ReputationState` snapshot after validating canonical `NodeId` ordering. State
application takes ownership of the finalized vector so entries can be moved into
the snapshot without cloning.

Rating matrix construction and normalization are still data preparation only.
The liquid-rank, transition, and clamp stages are pure calculation stages. They
do not mutate reputation state or materialize a dense matrix. State application
is explicit and happens only through `ReputationState::apply_reputation_vector`.

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
  -> alpha-blended next reputation vector
  -> clamped reputation vector
  -> reputation list
  -> reputation state snapshot
  -> reputation block
```

The current implementation covers this complete local calculation, commitment,
audit, state-application, and durable snapshot path. Reputation block
publication and historical storage remain future work.

## File-Level Plan

### `src/types.rs`

Own the paper vocabulary. Define only deterministic data types here.

Planned types:

- `ReputationRound`
- `ReputationWeight`
- `ReputationCommitment`
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
- `ReputationVector` values are expected in canonical `NodeId` order so the
  liquid-rank contribution step can use deterministic binary-search lookups
  without allocating an index.
- For `RatingMatrix`, the canonical deterministic ordering is by `(recipient, rater)`,
  not insertion order.
- RatingMatrix is the canonical, deterministic representation of the paper's ratings matrix. It is intentionally stored as an ordered list of rating entries; the current contribution calculation consumes that sparse ordered form directly.
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
and delegate calculations to dedicated modules. It applies finalized reputation
vectors only after the calculation pipeline has already produced them.

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
paper-guided modified normalization formula. The implementation now also
includes `src/liquid_rank.rs`, which computes the `P = S * R` contribution
vector without mutating reputation state.

The `src/transition.rs` module computes the next vector with:

```text
R_next_i = (alpha * P_i + (scale - alpha) * R_k_i) / scale
```

It requires both vectors to be in canonical `NodeId` order and requires the
contribution round to immediately follow the previous reputation round. The
calculation rejects invalid scale or alpha values and uses checked `u128`
fixed-point intermediates before converting each result back to
`ReputationWeight`. The output covers the union of the two node sets, in
canonical order, and takes its round from the contribution vector.

Rounds are sparse in practice: a node that receives no ratings produces no
Liquid-Rank contribution entry. Nodes present on only one side are resolved
through `PorConfig::missing_entry_policy` rather than rejecting the round:

| Policy | Node with no contribution | Node with no previous reputation |
|--------|---------------------------|----------------------------------|
| `Reject` | `MissingContributionEntry` | `MissingPreviousReputation` |
| `CarryForward` (default) | `P_i := R_k_i`, and the pipeline clamp copies `R_k_i` so the finalized reputation is unchanged | seeded from `initial_reputation` |
| `Neutral` | `P_i := initial_reputation` | seeded from `initial_reputation` |

Naming the fallback is the point: the earlier strict rejection existed to stop
reputation being carried forward *silently* when ratings were incomplete, and a
configured policy keeps that property while letting sparse rounds proceed. The
policy lives in `PorConfig` because it is part of the replayed input for
`verify_reputation_transition` — two validators must not be able to disagree
about a reputation block because they resolved a missing entry differently.

Absence of ratings is not evidence of inactivity: a node can be online and
simply not interacted with. Punishing genuine inactivity belongs to the
`InactivityPenalty` stage, which carries the missed-round count.

The sigmoid clamp is not idempotent. Clamping an already-finalized CarryForward
value would shrink it every sparse round — at production defaults
(`scale = 1_000_000_000`, `R = 200_000_000`) that is a drop to `196_116_135`,
about 1.94% per round, which is an implicit inactivity penalty. The pipeline
therefore clamps with `clamp_reputation_transition`, which copies the previous
finalized reputation for CarryForward entries rather than trusting the blended
value, and still clamps rated nodes, newly seeded nodes, and every entry under
`Reject` or `Neutral`.

The `src/clamp.rs` module applies the paper sigmoid-style clamp with:

```text
R_clamped = R / sqrt(1 + R^2)
```

Fixed-point form:

```text
clamp_fixed = round((r * scale) / sqrt(scale^2 + r^2))
```

It uses deterministic integer arithmetic only, rejects zero scale, reports
checked arithmetic overflow as `PorError::ClampOverflow`, preserves vector
round and ordering, and does not mutate `ReputationState`.
`clamp_reputation_vector` clamps every entry. The audit replay and any other
full-pipeline caller use `clamp_reputation_transition`, which takes the same
previous and contribution vectors as the blend and restores CarryForward
entries from previous reputation so a hand-built blend cannot preserve an
arbitrary unclamped value.

The `src/state.rs` module applies a finalized vector with:

```text
R_clamped -> ReputationState / ReputationList
```

`ReputationState::apply_reputation_vector` consumes the finalized vector,
validates canonical `NodeId` ordering, rejects duplicate or unsorted entries,
updates the current round, and replaces the stored `ReputationList` by moving
the vector contents. It does not perform rating validation, matrix construction,
normalization, Liquid Rank, alpha blending, or clamping.

The `src/block.rs` module assembles a reputation block with:

```text
ReputationBlockContext + RatingBatch + ReputationList + PorConfig
  -> ReputationBlock
```

`build_reputation_block` accepts protocol inputs rather than caller-supplied
hashes. It derives the source round from the finalized wave, commits the
configuration, signed rating batch, and reputation list, and links the block to
the canonical hash of the immediately preceding block when one exists. A
previous block must belong to the same shard and immediately preceding round.

`validate_reputation_block` checks the v1 format version, non-empty bounded
shard identifier, finalized-wave-to-round relation, header/list round match,
canonical `NodeId` ordering, and the recomputed reputation-list commitment.
Structural validation cannot prove external facts such as which shard or wave
the caller expected; `verify_reputation_transition` checks those against its
`ReputationBlockContext`. Neither operation mutates `ReputationState` or
publishes a block.

## Canonical Reputation Commitments

All v1 commitments use Blake2b-256. Integers are unsigned big-endian, collection
counts and byte lengths are `u64`, optional values use a one-byte `0`/`1`
discriminant, and Boolean values use `0`/`1`. Domain separators are included
verbatim as the first bytes of their preimages.

```text
config_commitment = H(
    "cordial-por:config-commitment:v1"
    || scale_u64
    || initial_reputation_u64
    || liquid_rank_alpha_u64
    || minimum_rating_u64
    || maximum_rating_u64
    || missing_entry_policy_u8
)

rating_batch_commitment = H(
    "cordial-por:rating-batch-commitment:v1"
    || round_u64
    || rating_count_u64
    || each(
        canonical_rating_payload_len_u64
        || canonical_rating_payload
        || signature_len_u64
        || signature
    )
)

reputation_list_commitment = H(
    "cordial-por:reputation-list-commitment:v1"
    || round_u64
    || entry_count_u64
    || each(node_id_len_u64 || node_id || reputation_u64 || is_excluded_u8)
)

reputation_block_hash = H(
    "cordial-por:reputation-block-commitment:v1"
    || version_u16
    || shard_id_len_u64 || shard_id
    || source_finalized_wave_u64
    || round_u64
    || previous_hash_presence_u8 || [previous_hash_32]
    || config_hash_32
    || ratings_hash_32
    || reputation_root_32
)
```

Ratings are first validated and sorted by `(recipient, rater)`, so the batch
commitment is independent of arrival order. It commits the exact signatures as
well as the canonical signed payloads. Reputation entries must already be in
strict `NodeId` order; the list commitment includes `is_excluded`, making
exclusion part of the auditable state. Golden vectors in
`tests/commitments.rs` lock the v1 formats against accidental changes.

## Durable Reputation State Snapshot

`src/snapshot.rs` encodes the complete finalized state required to resume after
a restart: the current round and reputation list, the permanent ejection
registry, and the latest audited reputation block. Pending ratings are not
finalized state; encoding rejects a state containing them instead of silently
dropping them.

The outer v1 envelope is:

```text
"cordial-por-state"             17 bytes
version                          u16 big-endian (= 1)
payload_length                   u64 big-endian
payload                          payload_length bytes
checksum                         Blake2b-256
```

The checksum preimage is:

```text
"cordial-por:state-snapshot:v1"
|| "cordial-por-state"
|| version_u16
|| payload_length_u64
|| payload
```

The payload uses the same unsigned big-endian integers, `u64` lengths/counts,
and one-byte `0`/`1` discriminants as the commitment formats:

```text
current_round
reputation_list
excluded_key_count || each(node_id_length || node_id)
latest_block_presence || [latest_reputation_block]
```

A reputation list contains its round, entry count, and each node identifier,
reputation value, and exclusion flag. A stored block contains the complete v1
header and its reputation list, not merely its hash.

Decode is bounded to 64 MiB, one million entries, 4 KiB per node identifier,
and the existing 256-byte shard identifier limit. Restore checks the checksum,
rejects trailing or truncated data, validates canonical ordering and the latest
block, requires state/list/latest-block rounds to agree, and verifies that every
exclusion flag exactly matches a zero-weight key in the permanent registry.
`tests/snapshot.rs` locks the v1 format with a golden hash.

The adapter writes these bytes to
`<data_dir>/por/reputation-state.bin`. It syncs a temporary file, atomically
renames it, and syncs the directory, so a failed replacement cannot destroy the
last committed state. Runtime startup/application wiring remains intentionally
outside this persistence slice.

The `src/audit.rs` module replays the whole pipeline so that any member can
audit a proposed reputation block:

```text
ratings + previous reputation + config -> expected ReputationList
```

`replay_reputation_transition` runs batching, matrix construction,
normalization, Liquid Rank, alpha blending, and `clamp_reputation_transition`
for one round, so a shuffled rating set yields the same list.
`verify_reputation_transition` applies `validate_reputation_block` first, then
checks the expected shard, source finalized wave, previous-block hash,
configuration commitment, and signed-rating commitment before comparing the
replayed list against `ReputationBlock.reputation_list`. Node-set, value, and
exclusion differences are reported separately. Replay is read-only: it does
not mutate `ReputationState`, publish blocks, or perform networking.

Future work remains:

- `src/committee.rs`: consensus group selection
- `src/leader.rs`: leader selection from the consensus group

`EquivocationPenalty` and `InactivityPenalty` remain intentionally as Cordial
integration extensions and are not part of the first reputation calculation
step. Reputation block publication and later consensus-selection logic remain
future work.

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
    version,
    shard_id,
    source_finalized_wave,
    round,
    previous_reputation_hash,
    config_hash,
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
- Committee selection implementation
- Leader selection implementation
- Cordial Miners approval, ratification, finality, or tau ordering
- Cordial-specific penalty or slashing behavior implementation

Cordial-specific penalty behavior should come after the paper-guided reputation
calculation path is implemented.
