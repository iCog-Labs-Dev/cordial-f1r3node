# 11 — τ (Tau) Total Ordering

**Paper reference:** Cordial Miners (arXiv:2205.09174) — Definition 5.1, Algorithm 2  
**Implementation:** `crates/cordial-miners-core/src/finality.rs`  
**Tests:** `crates/cordial-miners-core/tests/test_tau.rs`

---

## What τ is

The blocklace is a partially-ordered DAG. Every correct node holds a local copy and needs to derive the same totally-ordered list of transactions from it. That derivation is the τ function.

τ takes a blocklace and a bonds map and returns a `Vec<BlockIdentity>` — a deterministic, append-only sequence of all blocks that have been causally anchored to a finalized leader. Every correct node running τ on the same blocklace produces the exact same vector.

---

## Key properties

| Property | Meaning |
|----------|---------|
| **Determinism** | Identical blocklaces → identical output vectors, regardless of insertion order or HashMap iteration order. |
| **Monotonicity** | The finalized prefix never changes. Adding new blocks can only extend or reorder the un-finalized suffix. |
| **Equivocator exclusion** | Blocks from Byzantine validators are never emitted. |

---

## Algorithm

```
τ(blocklace, bonds):
  leaders = finalized_leader_chain(blocklace, bonds)   // oldest → newest
  if leaders is empty → return []

  output         = []
  already_output = {}

  for each leader in leaders:
      approved   = approved_causal_history(blocklace, leader)
      new_blocks = approved \ already_output
      sorted     = xsort(new_blocks, blocklace)
      output.extend(sorted)
      already_output.extend(sorted)

  return output
```

---

## Building blocks

### Approval (Definition A.5)

**Observation** (`precedes_or_equals`) is transitive: if `b` observes `a` and `a` observes `x`, then `b` observes `x`.

**Approval is not transitive.** Block `b` approves block `target` iff:

1. `b` observes `target` — `target ∈ ancestors_inclusive(b)`.
2. `b` does **not** observe any block that equivocates with `target` — there is no block `c ≠ target` in `ancestors_inclusive(b)` with `node(c) == node(target)` such that `c` and `target` are incomparable under `≺`.

The second condition is the equivocation-aware filter. Even if `b` observes `target`, if `b` also observes an incomparable sibling of `target` (a Byzantine fork), `b` does not approve `target`. This prevents Byzantine content from entering the output.

### Approved causal history

`approved_causal_history(leader)` = all blocks in `ancestors_inclusive(leader)` that the leader approves.

This is the set of blocks that will be sorted and appended to the τ output for this leader anchor. Equivocating blocks are excluded at this step.

### xsort — deterministic topological sort

`xsort` sorts a set of block identities into a total order that respects the DAG's partial order.

**Algorithm:** Kahn's algorithm (iterative BFS) with a `BTreeSet` as the ready queue.

- A block is "ready" when all of its predecessors within the input set have already been emitted.
- The `BTreeSet` ensures that among all currently-ready blocks, the one with the lexicographically smallest key is always emitted first.

**Sort key (tiebreak for incomparable blocks):**

`BlockIdentity` derives `Ord` from its struct field declaration order, giving:

```
(content_hash: [u8; 32], creator: NodeId, signature: Vec<u8>)
```

All three fields implement `Ord`, giving a strict total order with no ties. In practice, `content_hash` alone is sufficient to distinguish blocks (it is a cryptographic hash of the block content), so the `creator` and `signature` fields serve only as a theoretical fallback. For any two blocks with no ancestry relationship between them, the one whose `content_hash` sorts earlier lexicographically is always emitted first.

**Why this is correct:** The tiebreak is a pure function of the block's identity — it does not depend on when the block was received, what order it was inserted into the blocklace, or any local state. Any two nodes with the same set of blocks will produce the same xsort output.

**Future upgrade:** The paper specifies the tiebreak as `(wave, round, creator, id)`. When wave and round metadata are added to the protocol (Gaps 1–4 in `CONSENSUS_GAP_ANALYSIS.md`), the sort key will be upgraded accordingly. The xsort interface does not need to change — only the comparison key inside it.

### Finalized leader chain

`finalized_leader_chain(blocklace, bonds)` collects all blocks where `check_finality()` returns `Finalized`, then topologically sorts them using `xsort` (ancestors first).

This gives the ordered sequence of finalized anchors that τ iterates over, from the oldest finalized block to the most recent.

---

## Why the finalized prefix is immutable

Let `L₁, L₂, ..., Lₙ` be the finalized leader chain at time `t`. The τ output at time `t` is:

```
τ(t) = xsort(ACH(L₁))
     ++ xsort(ACH(L₂) \ ACH(L₁))
     ++ ...
     ++ xsort(ACH(Lₙ) \ ∪ ACH(Lᵢ, i < n))
```

where `ACH(L)` is the approved causal history of leader `L`.

At time `t' > t`, new blocks may arrive. Three things can happen:

**1. New blocks are added to the DAG.**  
They can only extend the future, not the past. The blocklace closure axiom (`P ⊂ dom(B)`) forbids retroactive insertion — a new block can only reference blocks already in the blocklace. Therefore no new block can be inserted into `ACH(Lᵢ)` for any already-finalized `Lᵢ`.

**2. New leaders become finalized.**  
They are appended to the end of the leader chain as `Lₙ₊₁, Lₙ₊₂, ...`. They do not change `L₁..Lₙ` or their approved causal histories.

**3. xsort output changes.**  
Impossible for a fixed input set. `xsort` is a pure function of its inputs — same set of block identities and same blocklace structure → same output vector.

Therefore `τ(t')[0..len(τ(t))] == τ(t)`. The prefix is immutable. ∎

---

## Relationship to the paper's wave model

The paper's τ uses **wave-based leader finality**: a block is a "final leader" only if it is the leader block of a wave AND it is super-ratified within that wave (two layers of supermajority approval). This requires:

- Round/depth computation (Gap 1)
- Block approval — `approves()` (Gap 2, **now implemented in `finality.rs`**)
- Ratification and super-ratification (Gap 3)
- Wave structure and leader election (Gap 4)

The current implementation uses the existing `check_finality()` supermajority heuristic from `consensus/finality.rs` as a stand-in for wave-based leader finality. The τ correctness properties (determinism, monotonicity, equivocator exclusion) hold under either model — the only requirement is that finality itself is monotonic, which holds for both.

When Gaps 1–4 are implemented, the change to τ is localized to `finalized_leader_chain`: replace the `check_finality` scan with a wave-leader scan. Everything else — `approves`, `approved_causal_history`, `xsort`, and the main `tau` loop — stays the same.

---

## Test coverage

| Test | What it checks |
|------|---------------|
| `tau_empty_blocklace_returns_empty` | τ on an empty blocklace returns `[]` |
| `tau_is_deterministic` | Two independently-built identical blocklaces produce the same τ output |
| `tau_finalized_prefix_is_immutable` | Adding blocks only extends the suffix; the finalized prefix is unchanged |
| `tau_excludes_equivocator_blocks` | Byzantine validator's blocks never appear in τ output |
| `tau_single_validator_linear_chain` | Single validator: output is in topological order |
| `approves_honest_block_in_ancestry` | `approves()` returns true for honest ancestors |
| `approves_rejects_when_equivocating_sibling_observed` | `approves()` returns false when an equivocating sibling is visible |
| `xsort_respects_topological_order` | Ancestors always appear before descendants |
| `xsort_is_deterministic_for_incomparable_blocks` | Same input → same output for incomparable blocks |
| `approved_causal_history_excludes_equivocating_blocks` | Equivocating blocks are filtered out of ACH |
| `approved_causal_history_includes_honest_ancestors` | Honest ancestors are included in ACH |
