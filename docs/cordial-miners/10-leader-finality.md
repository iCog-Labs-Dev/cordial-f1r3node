# 10 — Wave Leader Selection and Leader Finality

## Paper Reference

Sections 3 and 5, Definition 24 of the Cordial Miners paper
(arXiv:2205.09174).

## Wave Structure

The blocklace rounds are partitioned into fixed-length waves of size
`wavelength`. Each wave has exactly one elected leader validator. Any
block by that leader in the first round of the wave is a **leader block**.

## Leader Block Selection

`leader_block_for_wave` finds the leader block for a wave by:

1. Asking `leader_selection(wave)` for the elected validator
2. Finding the first round of the wave (the leader round)
3. Collecting all blocks by that validator in that round
4. Returning the single block with the lowest `content_hash` byte value

### Deterministic Tie-Breaking

If the elected leader equivocated (produced multiple blocks in the leader
round), there is more than one candidate leader block. To ensure all
validators agree on the same leader block without extra communication,
tie-breaking is done deterministically by selecting the block with the
lexicographically lowest `content_hash`.

This means:
- Same inputs always produce the same output
- No randomness or network communication needed
- All correct validators independently arrive at the same choice

## Leader Finality

`is_final_leader` checks whether a leader block has achieved finality
within its wave.

### Paper Definition

Per Definition 24 of arXiv:2205.09174:

> "A leader block b of round r is final in B if it is
> super-ratified in B(r + w − 1)"

Where `B(r + w - 1)` is the depth prefix of the entire blocklace up
to the last round of the wave.

### Implementation

The check proceeds as:

1. Verify the candidate is the deterministically selected leader block
   for its wave — if not, return false immediately
2. Find the wave boundaries using `wave_of_round` and
   `last_round_of_wave`
3. Collect witness blocks within the bounded round range
4. Call `super_ratifies(witness_blocks, candidate)` — if true, the
   block is a Final Leader

### Performance Note

Definition 24 technically requires checking `B(r + w - 1)` — the
entire depth prefix of the blocklace up to the last round of the wave.
This would include all blocks from round 0 up to that point.

However, the implementation only collects blocks from
`candidate_round` up to `last_round` of the wave, not from round 0.
This is mathematically equivalent because:

- A block created **before** `candidate_round` cannot have a hash
  pointer to the candidate block — hash pointers only go backward
  in time
- A block that cannot observe the candidate cannot approve it
- A block that cannot approve the candidate cannot ratify it
- A block that cannot ratify the candidate cannot contribute to
  super-ratification

Therefore, scanning blocks before `candidate_round` adds zero to the
super-ratification result. Excluding them is mathematically equivalent
to the full `B(r + w - 1)` check.

This optimization keeps finality checking O(wave size) rather than
O(total chain history), which is critical for a live consensus engine
processing thousands of blocks.

## Why Finality Is Locked In

Once a leader block is super-ratified:
- A supermajority of blocks ratify it
- Each ratifying block has a supermajority of approving blocks in
  its closure
- By the properties of supermajority intersection, no conflicting
  block can also achieve super-ratification
- The tau ordering function uses final leaders as anchors — once
  a block is a final leader its position in the total order is
  permanent and will never be undone by a subsequent call to tau

## Relationship to tau

Final leaders are the anchors of the tau ordering function described
in Section 5 of the paper. Each final leader triggers a topological
sort of the blocks it observes that have not yet been ordered,
producing the next fragment of the total output sequence. The
monotonicity of tau guarantees that once a fragment is output it
is never retracted.

## Relationship to Other Modules

| Module | Role |
|---|---|
| `consensus/wave.rs` | Wave boundaries and leader round computation |
| `consensus/round.rs` | Depth computation and `blocks_at_depth` |
| `consensus/approval.rs` | `approves` — the base approval predicate |
| `consensus/cordiality.rs` | `ratifies`, `super_ratifies`, `is_supermajority` |
| `consensus/finality.rs` | `leader_block_for_wave`, `is_final_leader` (this module) |