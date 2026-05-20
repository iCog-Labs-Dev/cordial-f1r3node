# Tau Ordering

## Paper Reference

This module implements the ordering layer that follows final leader discovery
in the Cordial Miners protocol.

Primary reference:

- `arXiv:2205.09174`

The paper-native story is:

1. determine final leaders
2. recursively walk backward through the leader chain
3. emit the blocks approved by each leader in deterministic order
4. never emit the same block twice

That is the role of `tau`.

## Implemented Module

Implementation lives in:

- `crates/cordial-miners-core/src/consensus/ordering.rs`

Tests live in:

- `crates/cordial-miners-core/tests/test_ordering.rs`

The module currently provides both:

- paper-native unweighted ordering
- stake-weighted ordering for PoS / f1r3node-style integration
- first-pass caching via `OrderingCache`

## Helper Functions

### `approved_blocks_for_leader(...)`

Returns the set of blocks approved by a given leader block using the existing
`approves(...)` predicate.

This is the membership function for a leader’s output fragment: before ordering
blocks, we first determine which blocks belong to that leader’s contribution.

### `xsort(...)`

Returns a deterministic topological order of a supplied block set.

Properties:

- predecessor edges inside the selected subset are respected
- predecessors outside the subset are ignored
- ties are broken by the natural ordering of `BlockIdentity`

This gives every node the same local order for the same selected block set.

### `previous_final_leader(...)`

Given a current final leader, walks backward through earlier waves and returns
the newest earlier final leader that the current leader ratifies.

This is the recursion edge for the paper-native `tau`.

### `weighted_previous_final_leader(...)`

Weighted counterpart of `previous_final_leader(...)`.

It uses:

- weighted final leader discovery
- weighted ratification

instead of the paper-native creator-count predicates.

### `OrderingCache`

`OrderingCache` is a first-pass memoization layer for repeated ordering calls.

It currently caches:

- approved block sets per leader
- deterministic sorted fragments per leader
- previous-final-leader traversal results keyed by leader and ordering parameters
- weighted previous-final-leader traversal results keyed by leader, wavelength, and bonded stake map
- full `tau` output prefixes keyed by the current latest final leader
- full `weighted_tau` output prefixes keyed by the current latest weighted final leader

Cache entries are invalidated automatically when the blocklace size changes.
When reusing a cache across calls, the caller must pass a stable
`leader_selection_id` describing the leader-selection policy in use.
Cache keys include that identifier so results from different selector
policies are not reused accidentally.

## `tau(...)`

### Paper-native path

`tau(...)` is implemented as:

1. find the latest final leader with `latest_final_leader(...)`
2. recursively walk to `previous_final_leader(...)`
3. for each leader, collect `approved_blocks_for_leader(...)`
4. filter out blocks already emitted by earlier recursion
5. emit the remaining blocks using `xsort(...)`

This preserves monotonicity:

- earlier output remains a prefix of later output
- blocks are not emitted twice

### Weighted path

`weighted_tau(...)` mirrors the same structure:

1. find the latest weighted final leader with `latest_weighted_final_leader(...)`
2. recursively walk to `weighted_previous_final_leader(...)`
3. collect approved blocks
4. suppress duplicates
5. emit deterministically with `xsort(...)`

This keeps the ordering structure aligned between:

- paper-native consensus
- stake-weighted integration

### Cached entrypoints

The module also provides:

- `tau_with_cache(...)`
- `weighted_tau_with_cache(...)`

These reuse an `OrderingCache` across repeated ordering calls while preserving
the same output as the uncached paths. Callers are responsible for providing a
stable `leader_selection_id` for repeated uses of the same selector policy.

## Current Behavior

The implemented ordering layer now supports:

- empty output when no final leader exists
- deterministic fragment ordering for a single final leader
- recursive growth across multiple final leaders
- duplicate suppression across recursive segments
- divergence between unweighted and weighted output when finality differs
- cached and uncached ordering equivalence
- cache invalidation when the blocklace grows
- repeated cached calls can reuse full output prefixes when the latest final leader is unchanged

## Current Limitations

The current implementation is intentionally direct and readable.

Things not yet optimized:

- no adapter-facing ordered output plumbing yet

Those can be added later without changing the high-level semantics.

## Test Coverage

Current tests cover:

- `approved_blocks_for_leader(...)`
- `xsort(...)`
- `previous_final_leader(...)`
- `weighted_previous_final_leader(...)`
- `tau(...)`
- `weighted_tau(...)`
- `tau_with_cache(...)`
- `weighted_tau_with_cache(...)`

Important tested properties include:

- deterministic order
- predecessor-respecting order
- monotonic growth
- no duplicate output
- weighted/unweighted divergence where expected
- cached/uncached equivalence
- cache invalidation on blocklace growth

## Relationship to Other Modules

| Module | Role |
|---|---|
| `consensus/approval.rs` | base approval predicate |
| `consensus/cordiality.rs` | ratification and weighted ratification |
| `consensus/finality.rs` | final leader and weighted final leader discovery |
| `consensus/ordering.rs` | deterministic output ordering (`tau`) |

## Next Likely Follow-up

Useful follow-up work after this implementation:

- expose ordered finalized output to the adapter layer
- add memoization / caching for leader-chain traversal
- document weighted ordering behavior in adapter-facing docs
