# 18 — Pruning and Cache Invalidation Policy

This document explains what state can be safely removed from the in-memory
blocklace after finalized ordered output is exported, what must be kept to
validate future blocks, and when the `OrderingCache` is invalidated — including
the equivocation-specific case where evidence recording does not trigger a
cache flush.

---

## 1. What Can Be Pruned After Finalized Ordered Output Export

Once a leader block has become final and the corresponding tau or weighted-tau
prefix has been materialized, the consensus engine can treat that leader as a
**checkpoint boundary** — a new in-memory genesis. All block contents that sit
strictly below that boundary are candidates for removal.

### Pruneable after a checkpoint advances

| Content | Can be pruned? | Notes |
|---|---|---|
| Block payload and predecessor sets of finalized history | ✅ Yes | The payload bytes and `HashSet<BlockIdentity>` of predecessors live in the blocklace `HashMap`. These are the dominant memory consumers and are safe to drop once the tau prefix is stored at the checkpoint. |
| Block identity entries for finalized history | ✅ Yes (with exceptions below) | Identities below the checkpoint boundary are removed during GC, provided they are not _protected_. |
| The checkpoint block itself | ❌ Retained permanently | The checkpoint block acts as the new in-memory genesis. It must remain so that `observe(descendant)` can walk down to it and stop. |
| Blocks still directly referenced by live non-finalized blocks | ❌ Retained (`protected_candidate_closure`) | If a retained (post-checkpoint) block has a direct predecessor edge pointing to a candidate, that candidate and its candidate ancestors are protected for this GC pass. This preserves the closure invariant. |
| The stored tau / weighted-tau ordering prefix | ❌ Retained at checkpoint | The `checkpoint_order_prefix` and `checkpoint_weighted_order_prefix` vectors are stored on the `Blocklace` struct itself (not in the block HashMap). They are cheap lists of `BlockIdentity` values needed to replay the ordering correctly after GC. |

The GC trigger functions are:

- `checkpoint_after_finality` — advances to the latest paper-native final
  leader and stores the unweighted tau prefix.
- `checkpoint_after_weighted_finality` — advances to the latest stake-weighted
  final leader and stores the weighted-tau prefix.
- `prune_below_checkpoint` — advances to an explicitly supplied checkpoint.

All three reject invalid moves: unknown checkpoints, backwards checkpoints
(depth regression), and disconnected checkpoints are returned as `PruneError`
variants rather than applied silently.

---

## 2. Structural Closure Retained for Future Block Validation

Pruning removes block _contents_ but must preserve the structural invariants
that let the engine validate and insert future blocks correctly.

### What is preserved

- **The checkpoint as the traversal boundary.** `observe(checkpoint)` returns
  only the checkpoint itself and stops — it does not descend further.
  `observe(descendant)` walks down to the checkpoint and halts there, without
  reading the now-absent blocks below it.

- **Logical round depth.** The checkpoint stores its original DAG depth (the
  depth it had before any GC). Descendants of the checkpoint continue from
  this depth. Wave and finality calculations remain monotonic: a round-6
  checkpoint is still round 6 after GC, so a new block at round 7 has the
  correct distance from it.

- **Closure invariant for live blocks.** Any retained block that still points
  at a pruned ancestor (because that ancestor was directly referenced by the
  retained block) is protected by `protected_candidate_closure`. The GC pass
  iterates all non-checkpoint retained blocks, finds predecessor edges pointing
  into the candidate set, and protects those candidates recursively. After GC,
  `blocklace.is_closed()` must return `true`.

### What incoming blocks see after GC

A new block whose predecessors include a pruned block will fail the closure
check and be **buffered** in `LiveBlocklaceMirror::pending` rather than
rejected. This is the normal behaviour for missing predecessors during height
bootstrap. The block will be applied from the buffer once its predecessor
becomes available (which, in the pruned case, will not happen unless the
predecessor is re-fetched or the block's predecessor list includes the
checkpoint directly).

---

## 3. When `OrderingCache` Is Invalidated

`OrderingCache` uses `(blocklace.generation(), dom().len())` as its freshness
key. The generation counter is a monotonically increasing `u64` on `Blocklace`
that advances on every structural mutation.

### Structural mutations that advance the generation

| Operation | Generation advances? |
|---|---|
| `blocklace.insert(block, verifier)` (successful) | ✅ Yes — `commit_validated` increments the counter |
| `blocklace.forget_block(id)` (block removed by GC) | ✅ Yes — `forget_block` increments if something was removed |
| `blocklace.set_checkpoint_state(...)` (checkpoint installed) | ✅ Yes — always incremented |
| A prune that removes 0 blocks (checkpoint already set to same point) | ⚠️ At most +1 — checkpoint reinstall increments; no block removal means no removal increments |
| `tau`, `weighted_tau`, `tau_with_cache`, `weighted_tau_with_cache` | ❌ No — these are read-only queries |
| `record_equivocation` / `record_rejected_equivocation` | ❌ No — evidence pool is a separate data structure (see §4) |

### How `sync_cache_generation` works

Every `*_with_cache` function calls `sync_cache_generation` at entry. If the
stored `(generation, size)` pair does not match the current blocklace, **all
six inner hash maps are cleared**:

- `approved_blocks_by_leader`
- `sorted_approved_by_leader`
- `previous_final_by_leader`
- `weighted_previous_final_by_leader`
- `tau_output_by_latest_leader`
- `weighted_tau_output_by_latest_leader`

The next call recomputes from scratch. The result is inserted back into the
cache under the fresh generation key so subsequent calls in the same generation
are served from the cache.

---

## 4. Equivocation Evidence and the Cache

The interaction between equivocation detection and the `OrderingCache` is
subtle enough to deserve an explicit section.

### Why equivocation evidence does not flush the cache

When a block fails the chain-axiom check (it would create equivocation for its
creator), it is **rejected and never inserted into the blocklace**. The caller
then calls `record_rejected_equivocation` to persist the proof in an
`InMemoryEvidencePool`.

`InMemoryEvidencePool` is a completely separate `BTreeMap`-based structure. It
has no connection to `Blocklace`. Calling `record_equivocation` on the pool:

- does **not** modify `blocklace.blocks`
- does **not** call `commit_validated`, `forget_block`, or
  `set_checkpoint_state`
- does **not** advance `blocklace.generation()`

Therefore the `OrderingCache` remains valid after evidence recording. The
cached entries computed against the pre-detection blocklace are still correct
because the blocklace itself has not changed.

This is safe because **equivocation exclusion is structural**: the equivocating
block was never inserted, so it is not in the domain of the blocklace, cannot
appear in any leader's approved set, and cannot be emitted by `tau` or
`weighted_tau`. The cache does not need to be invalidated to produce the
correct answer.

### What does flush the cache after equivocation detection

The next **structural mutation** to the blocklace — typically the next valid
block insertion — will advance the generation and cause the cache to flush on
the next `*_with_cache` call. The post-flush result will then agree with a
fresh uncached computation.

### Sequence diagram

```
1. honest blocks arrive         → generation advances on each insert
2. equivocating block arrives   → validate_block returns Equivocation error
                                  blocklace NOT mutated, generation stable
3. record_rejected_equivocation → evidence pool updated, blocklace NOT mutated
4. tau_with_cache               → cache hit (generation unchanged), correct result
5. next honest block inserted   → generation advances
6. tau_with_cache               → cache miss (generation changed), recomputes
```

---

## 5. Evidence Pool and the GC Boundary

The `InMemoryEvidencePool` is independent of the blocklace in two ways:

1. **Not subject to GC.** Checkpoint pruning operates only on `blocklace.blocks`.
   It does not touch the evidence pool. Equivocation proof recorded for a
   validator remains in the pool even after the equivocating block's epoch has
   been pruned from the blocklace.

2. **Not persisted automatically.** The pool is in-memory. If the node restarts,
   the pool is empty. Evidence must be re-detected from live traffic, or the
   node must persist the pool separately (out of scope for Phase 3). This is
   documented explicitly so that slashing implementations are not surprised by
   an empty pool after restart.

### Evidence pool size over time

The pool grows monotonically as evidence is recorded and is never automatically
trimmed. For long-running nodes, the pool may hold evidence for validators that
have been slashed and are no longer bonded. A future maintenance task
(`prune_evidence_for_expelled_validators`) is anticipated but not yet
implemented.

---

## 6. Test Coverage

The following test files cover the properties described in this document:

| File | Property |
|---|---|
| [`test_checkpoint_pruning.rs`](../../crates/cordial-miners-core/tests/test_checkpoint_pruning.rs) | Tau / weighted-tau prefix preserved after GC; memory bounded; observation stops at boundary |
| [`prop_pruning.rs`](../../crates/cordial-miners-core/tests/prop_pruning.rs) | Property-based: tau prefix stable for all generated DAG shapes |
| [`test_ordering_cache_generation.rs`](../../crates/cordial-miners-core/tests/test_ordering_cache_generation.rs) | Cache flushes on generation change; stable when nothing mutates; equal-size churn still invalidates |
| [`test_equivocation_cache_invalidation.rs`](../../crates/cordial-miners-core/tests/test_equivocation_cache_invalidation.rs) | Evidence recording does not advance generation; equivocating blocks absent from tau output; post-detection valid insert flushes cache; GC does not corrupt prefix or pool |
| [`test_evidence_after_partition.rs`](../../crates/cordial-miners-core/tests/test_evidence_after_partition.rs) | Evidence captured at rejection; not reconstructable afterwards; does not poison honest block ordering |

---

## 7. Summary

| Question | Answer |
|---|---|
| When can old block contents be dropped? | After `checkpoint_after_finality` or `checkpoint_after_weighted_finality` advances the checkpoint |
| What is kept at the checkpoint? | The checkpoint block itself plus the tau/weighted-tau ordering prefix |
| What keeps the closure invariant? | `protected_candidate_closure` retains candidates still referenced by live blocks |
| When does `OrderingCache` flush? | On any structural mutation (`insert`, `forget_block`, `set_checkpoint_state`) detected via `blocklace.generation()` |
| Does evidence recording flush the cache? | No — the pool is separate; the blocklace generation does not advance |
| Does equivocation exclusion need cache invalidation? | No — exclusion is structural (block never inserted); cache correctness is unaffected |
| Does GC affect the evidence pool? | No — the pool is a separate data structure not subject to blocklace GC |
