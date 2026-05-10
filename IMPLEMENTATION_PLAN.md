# Implementation Plan: τ (Tau) Total Ordering Function

**Task:** Implement `pub fn tau(...)` in `cordial-miners-core/src/finality.rs`  
**Paper reference:** Cordial Miners (arXiv:2205.09174) — Definition 5.1, Algorithm 2  
**Goal:** Convert the partial, multi-branching DAG (blocklace) into a strictly monotonic, append-only 1D list of block IDs using finalized wave leaders as anchors.

---

## Context & Background

The Cordial Miners protocol produces consensus through a DAG called the **blocklace**. Every correct node holds a local copy of this DAG and needs to derive the same totally ordered list of transactions from it. That derivation is the τ function.

The key properties τ must satisfy:

- **Determinism** — two nodes with identical blocklaces produce identical output vectors.
- **Monotonicity** — once a block appears in the output at position `i`, it stays at position `i` forever. Appending new blocks to the blocklace can only extend or reorder the un-finalized suffix; the finalized prefix is immutable.
- **Completeness** — every non-equivocating block that is causally reachable from a finalized leader eventually appears in the output.

---

## Codebase Orientation

### Where to work

The task lives entirely in one file:

```
crates/cordial-miners-core/src/finality.rs   ← currently just `//todo`
```

This file is already declared as `pub mod finality;` in `src/lib.rs`, so it is wired into the crate. No `Cargo.toml` changes are needed.

### Key existing types and functions

| Symbol | Location | What it does |
|--------|----------|--------------|
| `Blocklace` | `src/blocklace.rs` | The DAG. Holds `HashMap<BlockIdentity, BlockContent>` |
| `BlockIdentity` | `src/types/identity_id.rs` | `{ content_hash: [u8;32], creator: NodeId, signature: Vec<u8> }` — implements `Ord` |
| `BlockContent` | `src/types/content_id.rs` | `{ payload: Vec<u8>, predecessors: HashSet<BlockIdentity> }` |
| `NodeId` | `src/types/node_id.rs` | `(Vec<u8>)` — implements `Ord` |
| `Blocklace::ancestors_inclusive(id)` | `src/blocklace.rs` | Returns `HashSet<Block>` — all ancestors of `id` including itself |
| `Blocklace::precedes(a, b)` | `src/blocklace.rs` | True if `a` is in `b`'s ancestry (`a < b`) |
| `Blocklace::find_equivacators()` | `src/blocklace.rs` | Returns `HashSet<NodeId>` of Byzantine nodes |
| `Blocklace::tip_of(node)` | `src/blocklace.rs` | Most recent block of a given validator |
| `check_finality(blocklace, id, bonds)` | `src/consensus/finality.rs` | Returns `FinalityStatus::Finalized \| Pending \| Unknown` |
| `BlockProvider`, `ValidatorSet` traits | `src/cordiality.rs` | Generic abstractions (already defined, not yet used) |

### Module structure note

There are **two** finality-related files:

- `src/consensus/finality.rs` — the existing supermajority finality checker (`check_finality`, `find_last_finalized`, `can_be_finalized`). **Do not modify this.**
- `src/finality.rs` — the target file for this task. Currently empty (`//todo`).

---

## Algorithm

### High-level pseudocode

```
τ(blocklace, bonds):
  leaders = finalized_leader_chain(blocklace, bonds)   // oldest → newest
  if leaders is empty → return []

  output        = []
  already_output = {}

  for each leader in leaders:
      approved  = approved_causal_history(blocklace, leader)
      new_blocks = approved \ already_output
      sorted    = xsort(new_blocks, blocklace)
      output.extend(sorted)
      already_output.extend(sorted)

  return output
```

### The three building blocks

#### 1. `approves(blocklace, b, target) -> bool`

From Definition A.5 of the paper:

> Block `b` approves block `target` if `b` observes `target` AND `b` does not observe any block that equivocates with `target`.

In code terms:

```
b approves target iff:
  (1) target ∈ ancestors_inclusive(b)          // b observes target
  (2) ∀ block c ∈ ancestors_inclusive(b):
        if node(c) == node(target) and c ≠ target
        then precedes(c, target) or precedes(target, c)
        // no incomparable sibling of target is in b's causal history
```

This is the equivocation-aware filter. A block that has an equivocating sibling visible to `b` is excluded from `b`'s approved set, keeping Byzantine content out of the output.

#### 2. `approved_causal_history(blocklace, leader_id) -> HashSet<BlockIdentity>`

Walk `ancestors_inclusive(leader_id)`. For each block `a` in that set, keep it only if `leader` approves `a`. Return the filtered set.

This is the set of blocks that will be sorted and appended to the output for this leader anchor.

#### 3. `xsort(ids: HashSet<BlockIdentity>, blocklace) -> Vec<BlockIdentity>`

A deterministic topological sort of a set of block IDs.

**Algorithm:** Kahn's algorithm (iterative BFS topo sort) with a `BTreeSet` as the "ready" queue instead of a plain `VecDeque`.

- A block is "ready" when all of its predecessors (that are also in the input set) have already been emitted.
- Using `BTreeSet` as the ready queue means that among all currently-ready blocks, the one with the lexicographically smallest key is always emitted first.
- The sort key is `(creator, content_hash)` — both `NodeId` and `[u8; 32]` implement `Ord`, giving a strict total order with no ties.

**Why this is deterministic:** The `BTreeSet` imposes a canonical ordering on all incomparable blocks (blocks with no ancestry relationship between them). Given the same input set and the same blocklace, the output is always identical regardless of HashMap iteration order or insertion order.

**Future upgrade:** When wave and round metadata are added to the protocol, the sort key will be upgraded to `(wave, round, creator, content_hash)` as the paper specifies. The xsort interface does not need to change — only the comparison key.

---

## Implementation Steps

### Step 1 — `approves`

```rust
fn approves(blocklace: &Blocklace, b: &BlockIdentity, target: &BlockIdentity) -> bool
```

- Call `blocklace.ancestors_inclusive(b)` to get `b`'s causal history.
- Check that `target` is in that set (observation check).
- For every block `c` in the causal history where `c.creator == target.creator` and `c != target`, verify that `c` and `target` are comparable under `precedes` (i.e., one is an ancestor of the other). If any incomparable sibling exists, return `false`.

### Step 2 — `approved_causal_history`

```rust
fn approved_causal_history(
    blocklace: &Blocklace,
    leader_id: &BlockIdentity,
) -> HashSet<BlockIdentity>
```

- Get `ancestors_inclusive(leader_id)`.
- Filter: keep block `a` only if `approves(blocklace, leader_id, &a.identity)` is true.
- Return the set of `BlockIdentity` values.

### Step 3 — `xsort`

```rust
fn xsort(
    ids: HashSet<BlockIdentity>,
    blocklace: &Blocklace,
) -> Vec<BlockIdentity>
```

- Build an in-degree map: for each `id` in `ids`, count how many of its predecessors are also in `ids`.
- Seed a `BTreeSet<BlockIdentity>` with all IDs that have in-degree 0.
- Loop: pop the smallest element from the `BTreeSet`, emit it, decrement the in-degree of its successors (within `ids`), add any that reach 0 to the `BTreeSet`.
- Return the emitted sequence.

Note: "successors within ids" requires knowing which blocks in `ids` have the current block as a predecessor. Build a reverse-edge map upfront for efficiency.

### Step 4 — `finalized_leader_chain`

```rust
fn finalized_leader_chain(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Vec<BlockIdentity>
```

- Collect all `BlockIdentity` values from `blocklace.dom()` where `check_finality(blocklace, id, bonds)` returns `FinalityStatus::Finalized`.
- Topologically sort them using `xsort` (ancestors first).
- Return the sorted sequence.

This gives the ordered chain of finalized anchors that τ iterates over, from the oldest finalized block to the most recent.

### Step 5 — `pub fn tau`

```rust
pub fn tau(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Vec<BlockIdentity>
```

Wire steps 1–4 together following the pseudocode above. This is the public entry point.

---

## Files to Create / Modify

| File | Action | Notes |
|------|--------|-------|
| `crates/cordial-miners-core/src/finality.rs` | **Implement** | All five functions live here |
| `crates/cordial-miners-core/tests/test_tau.rs` | **Create** | Four tests (see below) |
| `crates/cordial-miners-core/tests/mod.rs` | **Edit** | Add `mod test_tau;` |
| `docs/cordial-miners/11-tau-ordering.md` | **Create** | Documentation (see below) |

---

## Tests

All tests go in `tests/test_tau.rs`. They follow the same helper pattern used in `test_finality.rs` and `test_consensus_simulation.rs` — a `MockVerifier` that always passes, and `node()` / `make_id()` / `genesis()` / `child()` / `insert()` / `bonds()` helpers.

### Test 1 — Determinism

Build the same blocklace twice, independently, in two separate `Blocklace` instances. Insert the same blocks in the same order. Call `tau()` on both. Assert the output `Vec<BlockIdentity>` is identical.

```
Scenario: 3 validators, linear cordial chain, all blocks finalized.
Assert: tau(&bl1, &bonds) == tau(&bl2, &bonds)
```

### Test 2 — Monotonicity (immutable finalized prefix)

```
Phase 1: Build a blocklace with 3 validators, enough blocks for some to be finalized.
         Call tau() → record output as `prefix`.

Phase 2: Add more blocks to the same blocklace (extending the DAG).
         Call tau() again → record as `extended`.

Assert: extended.starts_with(&prefix)
        // The finalized prefix is unchanged; new blocks only extend the suffix.
```

### Test 3 — Empty blocklace

```
Assert: tau(&Blocklace::new(), &bonds) == vec![]
```

### Test 4 — Equivocator exclusion

```
Scenario: 3 validators. v3 equivocates (two incomparable genesis blocks).
          v1 and v2 build a finalized chain.

Assert: tau() output contains no blocks created by v3.
```

---

## Documentation

`docs/cordial-miners/11-tau-ordering.md` should cover:

1. **What τ is** — the total ordering function, its role in the protocol, paper reference.
2. **Approval vs observation** — why we need the equivocation-aware filter and what it excludes.
3. **xsort tiebreak** — the `(creator, content_hash)` fallback, why it is deterministic and total, and where `(wave, round, creator, content_hash)` fits when waves are added.
4. **Why the finalized prefix is immutable** — the three-point argument:
   - Finality is monotonic (a finalized block stays finalized).
   - The approved causal history of a finalized leader is fixed (no new blocks can be inserted into its past).
   - xsort is deterministic (same input → same output).
5. **Relationship to the paper's wave model** — the current implementation uses the supermajority finality heuristic from `consensus/finality.rs` as a stand-in for wave-based leader finality. The τ correctness properties hold under either model. When Gaps 1–4 from `CONSENSUS_GAP_ANALYSIS.md` are implemented, `finalized_leader_chain` will be updated to use wave leaders.

---

## Why the Finalized Prefix Is Immutable (Proof Sketch)

Let `L₁, L₂, ..., Lₙ` be the finalized leader chain at time `t`. The output of τ at time `t` is:

```
τ(t) = xsort(ACH(L₁)) ++ xsort(ACH(L₂) \ ACH(L₁)) ++ ... ++ xsort(ACH(Lₙ) \ ∪ACH(Lᵢ, i<n))
```

where `ACH(L)` is the approved causal history of leader `L`.

At time `t' > t`, new blocks may arrive. Three things can happen:

1. **New blocks are added to the DAG** — they can only extend the future, not the past. They cannot be inserted into `ACH(Lᵢ)` for any already-finalized `Lᵢ` because the blocklace closure axiom forbids retroactive insertion.

2. **New leaders become finalized** — they are appended to the end of the leader chain. They do not change `L₁..Lₙ`.

3. **xsort output changes** — impossible for a fixed input set. `xsort` is a pure function of its inputs.

Therefore `τ(t')[0..len(τ(t))] == τ(t)`. The prefix is immutable. ∎

---

## Relationship to the Paper's Wave Model

The paper's τ uses **wave-based leader finality**: a block is a "final leader" only if it is the leader block of a wave AND it is super-ratified within that wave (two layers of supermajority approval). This requires:

- Round/depth computation (Gap 1)
- Block approval (Gap 2)
- Ratification and super-ratification (Gap 3)
- Wave structure and leader election (Gap 4)

None of these are implemented yet. This implementation uses the existing `check_finality()` supermajority heuristic as a stand-in. The τ function's correctness properties (determinism, monotonicity) are independent of which finality model is used — the only requirement is that finality itself is monotonic, which holds for both models.

When Gaps 1–4 are implemented, the change to τ is localized to `finalized_leader_chain`: replace the `check_finality` scan with a wave-leader scan. Everything else (approval, xsort, the main loop) stays the same.
The implementation is done. Here's a summary of what was created:

**`crates/cordial-miners-core/src/finality.rs`** — the main implementation with 5 functions:
- `approves()` — equivocation-aware filter (Definition A.5 from the paper)
- `approved_causal_history()` — all blocks a leader approves
- `xsort()` — deterministic topological sort using Kahn's algorithm + `BTreeSet` for canonical tiebreaking on `(creator, content_hash)`
- `finalized_leader_chain()` — collects all finalized blocks in topological order
- `pub fn tau()` — the public entry point that wires everything together

**`crates/cordial-miners-core/tests/test_tau.rs`** — 11 tests covering all 4 acceptance criteria:
- Determinism, monotonicity, empty blocklace, equivocator exclusion, plus unit tests for `approves`, `xsort`, and `approved_causal_history`

**`docs/cordial-miners/11-tau-ordering.md`** — full documentation covering the algorithm, the xsort tiebreak rationale, the immutability proof, and the relationship to the paper's wave model.

The diagnostics tool confirmed no compile errors. The only thing pending is the actual test run, which is blocked because your machine is downloading a new Rust nightly toolchain update in the background. Once that finishes, run:

```bash
cargo +nightly test -p cordial-miners-core
```