# 12 - Checkpoint Pruning

## Goal

A production validator cannot keep every block from genesis in the in-memory
blocklace. Once a leader block is finalized and the corresponding tau prefix is
materialized, the finalized leader becomes a checkpoint boundary. The consensus
engine can then treat that checkpoint as the new in-memory genesis and remove
older DAG contents from the `HashMap`.

## Trigger

Checkpoint garbage collection is triggered after leader finality:

1. `latest_final_leader(...)` finds the newest finalized leader.
2. `tau(...)` materializes the deterministic finalized output prefix for that
   leader.
3. `prune_below_checkpoint(...)` advances the checkpoint and deletes old blocks
   below it.

The convenience API is `checkpoint_after_finality(...)`. It returns `None` when
there is no finalized leader or when the latest final leader is already the
current checkpoint.

## Boundary Semantics

The checkpoint is retained in memory. Its predecessors may be removed.

Traversal treats the checkpoint as a boundary:

- `observe(checkpoint)` returns the checkpoint itself and stops.
- `observe(descendant)` can walk down to the checkpoint but not below it.
- Logical depth does not reset. The checkpoint stores its original depth, so
  descendants continue at the same round numbers they had before pruning.

This keeps wave and finality calculations monotonic while preventing repeated
walks back to genesis.

## What Is Deleted

The pruning candidate set is the checkpoint's observed closure minus the
checkpoint itself. Those blocks are physically removed from the in-memory
`HashMap` when they are orphaned by the new checkpoint boundary.

If a retained non-checkpoint block still directly depends on one of those
candidates, that candidate and its candidate ancestors are protected for this
prune pass. This preserves the closure invariant for retained live blocks while
still allowing normal finalized history to be collected.

## Memory Boundary

After regular finalization, retained block contents are bounded by:

- the current checkpoint block
- blocks created after that checkpoint
- any protected side-history still directly referenced by retained live blocks
- the stored tau output identities needed to prove finalized ordering stability

The large payload and predecessor-set memory belongs to the block `HashMap`.
That memory plateaus under frequent checkpointing because old finalized block
contents are removed instead of remaining traversable forever.

## Safety Invariants

Checkpoint pruning must preserve:

- finalized tau output stability before and after GC
- checkpoint monotonicity, meaning the checkpoint cannot move backwards
- branch continuity, meaning a new checkpoint must observe the current one
- closure for retained non-checkpoint blocks
- deterministic observation, with no pruned block returned from `observe`

Pruning rejects unknown checkpoints, backwards checkpoints, and disconnected
checkpoints instead of guessing.
