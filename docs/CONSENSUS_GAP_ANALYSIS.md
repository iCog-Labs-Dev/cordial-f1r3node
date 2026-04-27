# Consensus Gap Analysis: Paper vs Implementation

**Paper**: Cordial Miners: Fast and Efficient Consensus for Every Eventuality (arXiv:2205.09174)
**Codebase**: `crates/cordial-miners-core/src/consensus/`

---

## Executive Summary

The paper defines **three algorithmic components** of consensus:

1. **Dissemination** — how blocks propagate between miners
2. **Equivocation Exclusion** — how equivocating blocks are excluded from output
3. **Ordering** — the τ function that produces a total order from the DAG

The current `consensus/` folder has **fork_choice.rs**, **finality.rs**, and **validation.rs**. These cover basic building blocks (equivocator detection, cordial condition, validating blocks), but are **missing the core protocol logic** from the paper. The current finality model uses a simplified "supermajority in ancestry" heuristic rather than the paper's wave-based leader finality.

---

## What EXISTS and is correct

| Component | File | Paper Reference | Status |
|-----------|------|----------------|--------|
| Blocklace structure | `blocklace.rs` | Def. A.2 | ✅ Complete |
| Block observation (≽) | `blocklace.rs` `precedes()` | Def. A.3 | ✅ Complete |
| Closure [b] | `blocklace.rs` `ancestors_inclusive()` | Def. A.6 | ✅ Complete |
| Equivocation detection | `blocklace.rs` `find_equivacators()` | Def. A.4 | ✅ Complete |
| Cordial condition check | `fork_choice.rs` `is_cordial()` | Def. A.12 | ✅ Complete |
| Validator tips | `fork_choice.rs` `collect_validator_tips()` | — | ✅ Complete |
| Chain axiom enforcement | `validation.rs` | — | ✅ Complete |
| Basic block validation | `validation.rs` | — | ✅ Complete |

## What is MISSING (7 gaps)

### Gap 1: Round/Depth computation (Critical)

**Paper reference**: Def. A.7 — "The depth of a block b is the length of the longest path emanating from b."

The paper's entire protocol revolves around **rounds** (depth-based layers of the blocklace). Rounds determine when a miner can create a new block, which wave a block belongs to, and when finality can be checked.

**Current state**: No concept of rounds/depth exists anywhere in the codebase.

**What's needed**: `fn depth(blocklace, block_id) -> u64` — computes the longest path from a block to any initial block. Plus `fn blocks_at_depth(blocklace, d) -> HashSet<Block>` and `fn depth_prefix(blocklace, d) -> Blocklace` (B(d)).

---

### Gap 2: Block Approval (Critical)

**Paper reference**: Def. A.5 — "Block b approves b' if b observes b' and does NOT observe any equivocating block of b'."

This is a fundamental distinction the paper makes between **observing** and **approving**. Observation is transitive (if b observes b', and b' observes b'', then b observes b''). But **approval is NOT transitive** — even if b approves b' and b' approves b'', b may NOT approve b'' (if b also observes an equivocating sibling of b'').

**Current state**: The codebase only has `precedes()` (observation). There is no approval concept. `check_finality()` counts validators whose tips have a block in their **ancestry** (observation), not those that **approve** it.

**What's needed**: `fn approves(blocklace, b, b') -> bool` — b observes b' AND b does not observe any block that forms an equivocation with b'.

---

### Gap 3: Ratification and Super-ratification (Critical)

**Paper reference**: Def. A.9

- **Ratification**: b ratifies b' if the closure [b] includes a **supermajority** of blocks that **approve** b'
- **Super-ratification**: a set B super-ratifies b' if B includes a **supermajority** of blocks that **ratify** b'

These are the building blocks of leader finality. The current finality check (`check_finality`) uses a simpler model: "does >2/3 of honest stake have this block in their ancestry?" The paper's model is more nuanced — it requires **approval** (not just observation), and it requires **two layers** of supermajority (ratification + super-ratification).

**What's needed**: `fn ratifies(blocklace, b, target, n, f) -> bool` and `fn super_ratifies(blocklace, block_set, target, n, f) -> bool`

---

### Gap 4: Wave structure and Leader election (Critical)

**Paper reference**: Def. A.10, A.11, Alg. 4

Rounds are grouped into **waves** of fixed wavelength:
- **Asynchronous protocol**: wavelength = 5, leader elected retrospectively via random coin
- **Eventually Synchronous (ES) protocol**: wavelength = 3, leader elected prospectively (e.g., round-robin)

Each wave's first round may contain a **leader block** — a block by the elected leader. A leader block is **final** if it is **super-ratified** within its wave (by the blocks up to round r + w - 1).

**Current state**: No concept of waves, wavelength, or leader election exists.

**What's needed**:
- `WaveConfig { wavelength, leader_fn }` — configurable for async vs ES
- `fn wave_of(round) -> u64` — which wave a round belongs to
- `fn leader_of_wave(wave, miners) -> NodeId` — deterministic leader election
- `fn leader_block(blocklace, wave, leader) -> Option<Block>` — the leader's block in the wave's first round
- `fn is_final_leader(blocklace, leader_block, wave_config, n, f) -> bool` — checks super-ratification within the wave

---

### Gap 5: The τ ordering function (Critical — **the core missing piece**)

**Paper reference**: Def. 5.1, Alg. 2

τ is THE output function of the protocol. It converts the partially-ordered blocklace into a totally-ordered sequence of blocks. Every miner runs τ locally on their blocklace to produce the same deterministic output.

**Algorithm**:
1. Find the **last final leader** block in the blocklace
2. If none, output empty sequence
3. Otherwise, recursively: for each final leader b, find the previous leader block that b ratifies, recurse on it, then append `xsort(b, B)` — a topological sort of all blocks **approved** by b that haven't been output yet
4. Equivocating blocks are excluded during xsort (only approved blocks are included)

**Key property**: τ is **monotonic** — once blocks are output, they stay in the output forever, in the same order. This is what provides finality.

**Current state**: `find_last_finalized()` exists but produces only a single block identity, not the ordered sequence. There is no `xsort`, no recursive leader chaining, no monotonic output sequence.

**What's needed**: Full implementation of Alg. 2 as `fn tau(blocklace, wave_config, n, f) -> Vec<Block>`

---

### Gap 6: Cordial Dissemination protocol (High)

**Paper reference**: Alg. 3, Section 6.1

The paper's dissemination is NOT just "broadcast block to all." It's a protocol where:
1. Blocks are **buffered** until all predecessors are present (no dangling pointers)
2. After inserting a block, call τ to check for new finalized blocks
3. When a round becomes **cordial** (supermajority), create a new block for the next round
4. When sending to peer q, include **all blocks q might not know** — inferred from q's last block (what q's block observes tells us what q knows; what it doesn't observe tells us what q is missing)

The current `network::Node` has basic `BroadcastBlock` and `RequestBlock` messages, but:
- Doesn't track what each peer knows (no per-peer "last received block")
- Doesn't implement cordial dissemination (sending blocks the peer is missing)
- Doesn't wait for supermajority before creating a new block
- Drops blocks with missing predecessors instead of buffering them

---

### Gap 7: Equivocator excommunication (Medium)

**Paper reference**: Section 3 — "After detecting an equivocation, correct miners ignore the Byzantine miner by not including direct pointers to their blocks."

Once a miner detects equivocation, they should stop including the equivocator's blocks as predecessors in new blocks. The current code detects equivocators and excludes their stake from finality calculations, but there's no mechanism to prevent a block creator from referencing an equivocator's blocks as predecessors.

---

## Relationship between existing finality and paper's finality

The current `check_finality()` uses a simplified model:
- Finalized = >2/3 of honest stake has the block in their ancestry

The paper's model is:
- A block is in the final output if it passes through τ, which requires:
  1. A leader block in the first round of a wave
  2. The leader block is super-ratified (two layers of supermajority approval)
  3. The leader block serves as an anchor for xsort, which outputs all approved non-equivocating blocks

The current model is a **reasonable approximation** but differs in important ways:
- It uses observation instead of approval (ignoring equivocation-aware filtering)
- It doesn't use the wave/leader structure
- It doesn't produce an ordered output sequence

---

## Implementation Priority

| Priority | Component | Effort |
|----------|-----------|--------|
| 1 | Round/depth computation (`rounds.rs`) | Small |
| 2 | Block approval (`approval.rs`) | Small |
| 3 | Ratification & super-ratification (in `approval.rs`) | Medium |
| 4 | Wave structure & leader election (`waves.rs`) | Medium |
| 5 | τ ordering function (`ordering.rs`) | Large |
| 6 | Cordial dissemination updates (network) | Large |
| 7 | Equivocator excommunication | Small |

Components 1-5 belong in `consensus/`. Component 6 updates `network/`. Component 7 is a policy layer for block creation.
