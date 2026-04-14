# Cordial Miners vs CBC Casper: Integration Roadmap

This document evaluates the current blocklace implementation against the CBC Casper consensus running in f1r3node, identifies concrete gaps, and provides an actionable roadmap for making Cordial Miners a production-ready alternative.

Reference codebase: `f1r3node` at branch `rust/dev` (v0.4.11).

---

## 1. Protocol Comparison

### 1.1 Structural Philosophy

**CBC Casper** uses a message-driven justification model. Each block carries explicit justifications -- a map of "validator X's latest block is Y." Safety comes from finding a clique of validators that mutually agree through the GHOST fork choice, and finality is determined by a clique oracle that computes fault tolerance.

**Cordial Miners** uses the blocklace itself as the consensus object. The DAG structure encodes what each validator saw through predecessor pointers. The chain axiom enforces honest behavior structurally, and consensus emerges from the blocklace properties rather than from an explicit justification protocol.

### 1.2 Core Mechanism Comparison

| Aspect | CBC Casper (f1r3node) | Cordial Miners (blocklace) |
|--------|----------------------|---------------------------|
| **Data structure** | `BlockMessage` with separate `Header` (parents), `Body` (deploys/state), `justifications` (fork-choice map) | `Block` with `BlockIdentity` + `BlockContent` (payload + predecessors) |
| **Parent model** | Explicit `parents_hash_list` (bounded by `max_number_of_parents`) + separate `justifications` list | Single `predecessors: HashSet<BlockIdentity>` encoding both parentage and what the creator saw |
| **Fork choice** | LMD-GHOST via `Estimator`: score blocks by validator weight, walk children from LCA | Not yet implemented; paper defines fork choice via the cordial condition and blocklace tips |
| **Finality** | Clique Oracle: find max-weight clique of agreeing validators, compare against fault tolerance threshold | Not yet implemented; paper defines finality via supermajority observable from blocklace structure |
| **Equivocation** | Multi-step: `EquivocationDetector` + `InvalidBlock::AdmissibleEquivocation` / `IgnorableEquivocation` / `NeglectedEquivocation` | Structural: chain axiom violation detected by `satisfies_chain_axiom()` and `find_equivocators()` |
| **State model** | Pre/post state hashes over RSpace tuplespace, bonds map, block number | Generic `payload: Vec<u8>` with no state transition model |
| **Cryptography** | Blake2b hashing, Secp256k1 signatures (primary), Ed25519 (secondary) | SHA-256 hashing, Ed25519 signatures |
| **Storage** | LMDB-backed `KeyValueBlockStore` with indexed DAG (`height_map`, `child_map`, `main_parent_map`) | In-memory `HashMap<BlockIdentity, BlockContent>` |
| **Network** | gRPC + TLS 1.3, `TransportLayer` trait, `CasperMessage` protocol | Raw TCP + bincode, `Peer`/`Node` with `Message` enum |

### 1.3 Safety and Liveness

Both protocols achieve BFT safety under < 1/3 Byzantine validators with mathematical proofs. The practical differences:

**Finality latency**: CBC Casper's clique oracle is computationally expensive (O(n^2) in validators, bounded by `MAX_CLIQUE_CANDIDATES = 128`). The f1r3node implementation uses cooperative yielding, bounded caches, and `COOPERATIVE_YIELD_TIMESLICE_MS` to avoid blocking. Cordial Miners can potentially achieve faster finality because supermajority is observable directly from the blocklace structure without clique enumeration.

**Liveness under adversity**: CBC Casper's GHOST fork choice degrades gracefully when validators have divergent views because weight-based scoring still produces a usable fork choice with partial information. Cordial Miners depends on the "cordial" condition (validators reference all known tips). In high-latency or adversarial networks, this condition may not hold consistently, and the protocol's behavior under partial cordiality needs to be well-defined.

**Equivocation cost**: In CBC Casper, equivocation handling is spread across three classification levels and a separate detection module. In Cordial Miners, equivocation is a structural property of the DAG (chain axiom violation), making detection simpler and the correctness argument cleaner.

---

## 2. Current Implementation Status

### 2.1 What Exists

| Component | Status | Notes |
|-----------|--------|-------|
| Block structure (`Block`, `BlockIdentity`, `BlockContent`) | Complete | Faithful to paper Definition 2.2 |
| Blocklace data structure | Complete | Map-view accessors, insertion with closure axiom |
| Predecessor/ancestor traversal | Complete | `predecessors()`, `ancestors()`, `ancestors_inclusive()`, `precedes()` |
| Chain axiom enforcement | Complete | `satisfies_chain_axiom()`, `find_equivocators()` |
| Per-node tip tracking | Complete | `tip_of(node)` returns most recent block per validator |
| Cryptography (hash, sign, verify) | Complete | SHA-256 + Ed25519 via `sha2` and `ed25519-dalek` |
| P2P networking | Complete | TCP with handshake, block broadcast, sync protocol |
| Block propagation | Complete | `BroadcastBlock`, `RequestBlock`, `SyncRequest`/`SyncResponse` |
| Test suite | Complete | 60 tests covering blocks, blocklace, crypto, networking |

### 2.2 What Is Missing

| Component | Priority | Required For |
|-----------|----------|-------------|
| Global fork choice | Critical | Replacing `Estimator` |
| Finality detector | Critical | Replacing `Finalizer` + `CliqueOracle` |
| `Casper` trait implementation | Critical | Plugging into f1r3node |
| Typed payload (deploys, state transitions) | Critical | Smart contract execution |
| Persistent storage | High | Running beyond in-memory prototype |
| Block validation pipeline | High | Replacing `BlockProcessor` validation |
| Deploy pool / selection | High | Block creation with transactions |
| `CasperSnapshot` equivalent | High | State management across the system |
| Indexed DAG (child map, height map) | Medium | Performance at scale |
| Cryptographic alignment (Blake2b, Secp256k1) | Medium | Wire compatibility with f1r3node |
| TLS and gRPC network layer | Low | Can bridge via `TransportLayer` trait |

---

## 3. Gap Analysis and Required Work

### 3.1 Fork Choice (Critical)

**Current state**: `tip_of(node)` returns the latest block per validator. No global fork choice across validators.

**What CBC Casper does** (f1r3node `casper/src/rust/estimator.rs`):
1. Collect each validator's latest message hash
2. Filter out invalid latest messages
3. Compute Lowest Common Ancestor (LCA) across all validators
4. Build a score map: for each validator, walk their supporting chain and add their stake weight
5. Rank fork choices by score, walking children from LCA
6. Filter out deep parents beyond `max_parent_depth`
7. Return `ForkChoice { tips, lca }`

**What Cordial Miners needs**:

```
fn fork_choice(&self, bonds: &HashMap<NodeId, u64>) -> ForkChoice
```

The paper defines the cordial miners fork choice through the blocklace structure. Implement:
1. Collect tips across all validators: extend `tip_of()` to return all validator tips
2. Define "cordial" block: a block whose predecessors include all tips the creator knew about
3. Weight tips by validator stake from the bonds map
4. Compute LCA using existing `ancestors()` (but needs optimization, see 3.6)
5. Return ordered tips and LCA

**Estimated scope**: New module `src/consensus/fork_choice.rs`, approximately 200-400 lines.

### 3.2 Finality Detector (Critical)

**Current state**: No finality detection. Equivocation detection exists (`find_equivocators()`).

**What CBC Casper does** (f1r3node `casper/src/rust/finality/finalizer.rs` + `casper/src/rust/safety/clique_oracle.rs`):
1. Finalizer scans blocks between current LFB and tips
2. For each candidate, check if >50% of total stake agrees on it (`cannot_be_orphaned`)
3. For blocks passing the pre-filter, run the Clique Oracle to compute exact fault tolerance
4. First block exceeding the `fault_tolerance_threshold` becomes the new LFB

**What Cordial Miners needs**:

```
fn check_finality(&self, block_id: &BlockIdentity, bonds: &HashMap<NodeId, u64>) -> FinalityStatus
```

The paper defines finality through supermajority agreement visible in the blocklace. Implement:
1. For a candidate block `b`, find all validators whose tips have `b` in their ancestry (using `precedes()`)
2. Sum the stake of agreeing validators
3. If agreeing stake > 2/3 of total stake, the block is finalized
4. Track `last_finalized_block` and advance it monotonically
5. Handle equivocator stake: exclude `find_equivocators()` from the weight calculation

**Estimated scope**: New module `src/consensus/finality.rs`, approximately 150-300 lines.

### 3.3 Casper Trait Implementation (Critical)

**Current state**: No interface compatible with f1r3node.

The `Casper` trait (f1r3node `casper/src/rust/casper.rs:81-137`) is the single integration point. Every other system component (engine, proposer, block processor, API) calls through this trait. A Cordial Miners implementation must provide:

```rust
impl Casper for CordialMinersCasper {
    // Return a snapshot of current consensus state
    async fn get_snapshot(&self) -> Result<CasperSnapshot, CasperError>;

    // Check if a block hash is known
    fn contains(&self, hash: &BlockHash) -> bool;
    fn dag_contains(&self, hash: &BlockHash) -> bool;
    fn buffer_contains(&self, hash: &BlockHash) -> bool;

    // Accept a user deploy into the pending pool
    fn deploy(&self, deploy: Signed<DeployData>) -> Result<Either<DeployError, DeployId>, CasperError>;

    // Fork choice: return the best tips
    async fn estimator(&self, dag: &mut KeyValueDagRepresentation) -> Result<Vec<BlockHash>, CasperError>;

    // Block validation
    async fn validate(&self, block: &BlockMessage, snapshot: &mut CasperSnapshot)
        -> Result<Either<BlockError, ValidBlock>, CasperError>;

    // Post-validation handlers
    async fn handle_valid_block(&self, block: &BlockMessage) -> Result<KeyValueDagRepresentation, CasperError>;
    fn handle_invalid_block(&self, block: &BlockMessage, status: &InvalidBlock, dag: &KeyValueDagRepresentation)
        -> Result<KeyValueDagRepresentation, CasperError>;
}
```

The extended `MultiParentCasper` trait adds:
- `last_finalized_block()` -- requires finality detector (3.2)
- `block_dag()` -- requires persistent DAG (3.6)
- `block_store()` -- requires storage layer (3.6)
- `runtime_manager()` -- requires RSpace integration (3.4)
- `has_pending_deploys_in_storage()` -- requires deploy pool (3.5)

**Strategy**: Two approaches:

**(A) Adapter pattern**: Keep the blocklace as a separate library. Build a `CordialMinersCasper` struct that wraps a `Blocklace` and implements the `Casper` trait by translating between `Block`/`BlockIdentity` and `BlockMessage`/`BlockHash`.

**(B) Native integration**: Rewrite the consensus internals to use blocklace types directly, replacing `KeyValueDagRepresentation` with `Blocklace`. This is cleaner but requires deeper changes across the codebase.

**Recommended**: Start with (A). It minimizes risk and lets you validate the protocol without touching the rest of f1r3node. Migrate to (B) once the protocol is proven.

**Estimated scope**: New crate or module, approximately 500-800 lines for the adapter.

### 3.4 Typed Payload and State Transitions (Critical)

**Current state**: `BlockContent.payload` is `Vec<u8>` with no structure.

**What CBC Casper uses** (f1r3node `models/src/rust/casper/protocol/casper_message.rs`):

```
Body {
    state: F1r3flyState {
        pre_state_hash,    // Tuplespace hash before deploys
        post_state_hash,   // Tuplespace hash after deploys
        bonds: Vec<Bond>,  // Validator stakes
        block_number,
    },
    deploys: Vec<ProcessedDeploy>,         // Executed user deploys with costs
    rejected_deploys: Vec<RejectedDeploy>, // Failed signature checks
    system_deploys: Vec<ProcessedSystemDeploy>, // Slash, close block
}
```

Each `ProcessedDeploy` contains:
- `Signed<DeployData>` -- the Rholang code, phlo price/limit, timestamps
- `PCost` -- actual gas consumed
- `Vec<Event>` -- execution trace (Produce/Consume/Comm events)
- `is_failed` -- whether execution errored

**What Cordial Miners needs**:

Option 1: Define a `CordialBlockPayload` struct that mirrors the CBC Casper `Body` and serialize it into `payload: Vec<u8>`. This preserves your generic block structure while carrying the required data.

```rust
struct CordialBlockPayload {
    pre_state_hash: [u8; 32],
    post_state_hash: [u8; 32],
    bonds: Vec<(NodeId, u64)>,
    block_number: u64,
    deploys: Vec<ProcessedDeploy>,
    system_deploys: Vec<ProcessedSystemDeploy>,
}
```

Option 2: Make `BlockContent` generic over the payload type:

```rust
struct BlockContent<P> {
    payload: P,
    predecessors: HashSet<BlockIdentity>,
}
```

This keeps the blocklace protocol-agnostic while allowing typed payloads for f1r3node integration.

**Estimated scope**: Type definitions ~100 lines, serialization ~100 lines, integration with RSpace runtime ~300 lines.

### 3.5 Deploy Pool and Selection (High)

**Current state**: No deploy handling.

**What CBC Casper does**:
- `KeyValueDeployStorage` stores pending deploys
- `BlockCreator::prepare_user_deploys()` filters by: not expired, not future, not already in ancestor blocks, within phlo price minimum
- Deploys are capped by `max_user_deploys_per_block`
- Bounded by `deploy_lifespan` (blocks) and optional `expiration_timestamp`

**What Cordial Miners needs**:
- A deploy pool that accepts `Signed<DeployData>` (or your equivalent)
- Selection logic that checks a deploy is not already in the blocklace ancestry of the proposed block's predecessors
- Deduplication using deploy signatures against the `deploys_in_scope` window

**Estimated scope**: New module `src/deploy_pool.rs`, approximately 200-300 lines.

### 3.6 Persistent Storage and Indexed DAG (High)

**Current state**: In-memory `HashMap<BlockIdentity, BlockContent>`.

**Problem**: The current `ancestors()` traversal is O(|B|) worst case because it does a full DFS. The `precedes(a, b)` check calls `ancestors(b)` and scans the result. For a chain of 1M blocks, this is unusable.

**What CBC Casper uses** (`block_storage/src/rust/dag/block_dag_key_value_storage.rs`):

```
KeyValueDagRepresentation {
    dag_set: imbl::HashSet,          // All known block hashes
    latest_messages_map: imbl::HashMap, // Validator -> latest block hash
    child_map: imbl::HashMap,        // Parent -> children (reverse index)
    height_map: imbl::OrdMap,        // Block number -> block hashes
    block_number_map: imbl::HashMap, // Block hash -> block number
    main_parent_map: imbl::HashMap,  // Block hash -> main parent
    self_justification_map: imbl::HashMap, // Block hash -> self-justification
    invalid_blocks_set: imbl::HashSet,
    last_finalized_block_hash,
    finalized_blocks_set: imbl::HashSet,
    block_metadata_index,            // LMDB-backed metadata store
    deploy_index,                    // LMDB-backed deploy index
}
```

**What Cordial Miners needs**:

1. **Child index**: `HashMap<BlockIdentity, HashSet<BlockIdentity>>` -- reverse of predecessors. Enables efficient "who points to this block?" queries needed for finality detection.

2. **Height/depth index**: `BTreeMap<u64, HashSet<BlockIdentity>>` -- blocks indexed by depth. Enables bounded ancestor queries and garbage collection.

3. **Latest message index**: `HashMap<NodeId, BlockIdentity>` -- each validator's current tip. Maintained incrementally on insert instead of scanning `blocks_by()`.

4. **Persistent backend**: Replace `HashMap` with LMDB or similar. The `Blocklace` struct should abstract over a storage trait:

```rust
trait BlocklaceStore {
    fn get(&self, id: &BlockIdentity) -> Option<BlockContent>;
    fn put(&mut self, id: BlockIdentity, content: BlockContent) -> Result<(), Error>;
    fn contains(&self, id: &BlockIdentity) -> bool;
    fn iter(&self) -> impl Iterator<Item = (BlockIdentity, BlockContent)>;
}
```

5. **Pruning**: Blocks below the last finalized block that are no longer needed for ancestor queries can be archived or removed. CBC Casper's `mergeable_channels_gc_depth_buffer` serves a similar purpose.

**Estimated scope**: Storage trait + LMDB impl ~400 lines, indexes ~300 lines, pruning ~200 lines.

### 3.7 Block Validation Pipeline (High)

**Current state**: Only the closure axiom is enforced on insert.

**What CBC Casper validates** (f1r3node `casper/src/rust/block_status.rs`, 23+ checks):

The following checks are grouped by what they protect against:

**Format and authenticity**:
- `InvalidFormat` -- required fields present, correct types
- `InvalidSignature` -- block signature verifies against sender's public key
- `InvalidSender` -- sender is a known validator
- `InvalidBlockHash` -- hash matches the block content
- `DeployNotSigned` -- all deploys carry valid signatures

**DAG consistency**:
- `InvalidParents` -- parent blocks exist and are valid
- `InvalidFollows` -- block follows from its claimed justifications
- `InvalidBlockNumber` -- block number is consistent with parents
- `InvalidSequenceNumber` -- sender's sequence number increments correctly
- `JustificationRegression` -- justifications don't go backward
- `InvalidVersion` -- protocol version matches
- `InvalidTimestamp` -- timestamp is reasonable
- `InvalidShardId` -- block belongs to the correct shard

**Equivocation**:
- `AdmissibleEquivocation` -- equivocation detected through justifications
- `IgnorableEquivocation` -- equivocation that doesn't affect consensus
- `NeglectedEquivocation` -- block creator failed to report known equivocation
- `NeglectedInvalidBlock` -- block creator failed to report known invalid block

**Deploy validity**:
- `InvalidRepeatDeploy` -- deploy already included in ancestry
- `ContainsExpiredDeploy` -- deploy past its block lifespan
- `ContainsTimeExpiredDeploy` -- deploy past its timestamp expiration
- `ContainsFutureDeploy` -- deploy not yet valid
- `LowDeployCost` -- deploy phlo price below minimum

**State transition**:
- `InvalidTransaction` -- executing deploys doesn't produce the claimed post-state hash
- `InvalidBondsCache` -- bonds in the block don't match computed bonds
- `InvalidRejectedDeploy` -- rejected deploy list doesn't match what actually failed

**What Cordial Miners needs**:

Not all checks apply. The blocklace structure replaces some (parents/justifications are unified as predecessors, equivocation is detected by chain axiom). But these are still needed:

1. **Signature verification** -- block identity signature is valid
2. **Closure check** -- all predecessors exist (already implemented)
3. **Chain axiom check** -- creator hasn't equivocated (already implemented for detection, not for rejection)
4. **Sender is a bonded validator** -- creator has stake
5. **Deploy validity** -- deploys are signed, not expired, not duplicated
6. **State transition** -- executing deploys produces the claimed post-state hash
7. **Cordial condition** -- if the protocol requires it, verify the block references all tips the creator should have seen

**Estimated scope**: New module `src/validation.rs`, approximately 300-500 lines.

### 3.8 Cryptographic Alignment (Medium)

**Current state**: SHA-256 + Ed25519.

**f1r3node uses**: Blake2b-256 for block hashing, Secp256k1 for primary signatures (validator identities), Ed25519 as secondary.

**Options**:
1. **Abstract over crypto**: Define traits for `Hasher` and `Signer`/`Verifier`. Let the blocklace library be crypto-agnostic. Provide implementations for both SHA-256/Ed25519 (standalone) and Blake2b/Secp256k1 (f1r3node integration).
2. **Switch to f1r3node's crypto**: Use `crypto::rust::signatures` from f1r3node directly. Simpler but couples the library.

**Recommended**: Option 1. The blocklace should remain a general-purpose library.

**Estimated scope**: Trait definitions ~50 lines, Blake2b/Secp256k1 impl ~100 lines.

---

## 4. Advantages Over CBC Casper

These are the concrete benefits that justify the integration effort:

### 4.1 Reduced Complexity

CBC Casper in f1r3node has accumulated significant accidental complexity:
- `CasperBufferKeyValueStorage` with stale TTL pruning, overflow pruning, dependency loop detection
- Missing dependency quarantine with attempt counters and cooldown timers
- Synchrony constraint checker with recovery stall windows and bypass counters
- Cooperative yielding in the clique oracle to avoid blocking the event loop
- 12-field `CasperSnapshot` threaded through every operation

Much of this exists because the justification model creates indirection that the blocklace eliminates. The predecessor set directly encodes what the creator saw, so there is no separate "latest messages" map to maintain, no justification regression to check, and no clique enumeration for finality.

### 4.2 Faster Finality Path

The clique oracle (`casper/src/rust/safety/clique_oracle.rs`) is the bottleneck for finality in CBC Casper. It requires:
1. For each candidate block, find validators that agree on it
2. Among agreeing validators, find the maximum-weight clique (NP-hard in general, bounded by `MAX_CLIQUE_CANDIDATES`)
3. Compare clique weight against `fault_tolerance_threshold`

Cordial Miners finality is a simple stake summation: count the stake of validators whose tips have the candidate in their ancestry. This is O(n * ancestor_depth) with indexed storage, compared to the clique oracle's worst case.

### 4.3 Cleaner Equivocation Model

CBC Casper classifies equivocation into three categories (`Admissible`, `Ignorable`, `Neglected`) with different handling paths. The Cordial Miners chain axiom gives a binary answer: a node either satisfies the chain axiom or it doesn't. Equivocators are identified structurally and their stake is excluded from consensus calculations.

### 4.4 Unified Parent/Justification Model

CBC Casper separates `parents_hash_list` (for state merging) from `justifications` (for fork-choice). This separation creates subtle bugs around justification freshness, regression, and the interplay between the two. The blocklace's single `predecessors` set serves both purposes, eliminating an entire class of consistency issues.

---

## 5. Risks and Mitigations

### 5.1 Performance at Scale

**Risk**: `ancestors()` is O(|B|) DFS. `precedes(a, b)` is O(|B|). `satisfies_chain_axiom()` is O(k^2 * |B|) for k blocks by one node. These are fine for testing but will not scale.

**Mitigation**: Implement indexed storage (Section 3.6). With a height index and child map, ancestor queries can be bounded. Cache finality results so they don't need recomputation.

### 5.2 Predecessor Set Growth

**Risk**: In CBC Casper, `parents_hash_list` is bounded by `max_number_of_parents` and `justifications` is bounded by validator count. In the blocklace, `predecessors` can grow with the number of tips a validator sees. With many concurrent validators, this grows block size.

**Mitigation**: Bound the predecessor set. The paper's cordial condition requires referencing all known tips, but in practice you can reference all *validator tips* (bounded by validator count) plus selected non-tip predecessors. Define a `max_predecessors` configuration parameter.

### 5.3 Cordial Condition Under Adversity

**Risk**: The protocol's liveness depends on miners being "cordial" (referencing all known tips). In high-latency or partitioned networks, validators may not see all tips, producing non-cordial blocks. CBC Casper's GHOST fork choice degrades more gracefully with partial information.

**Mitigation**: Define explicit fallback behavior for non-cordial rounds. The fork choice should still produce a valid result when blocks are not fully cordial, even if finality is delayed. Test with simulated network partitions.

### 5.4 State Transition Compatibility

**Risk**: The f1r3node execution layer (RSpace tuplespace, Rholang interpreter, phlogiston accounting) is tightly integrated with CBC Casper's `Body` structure. Bridging this requires careful mapping.

**Mitigation**: Use the adapter pattern (Section 3.3, option A). The execution layer doesn't care which consensus chose the block -- it just needs pre-state hash, deploys, and produces post-state hash. Keep the interface clean.

---

## 6. Implementation Roadmap

### Phase 1: Core Consensus (Standalone)

Goal: A working Cordial Miners consensus that can be tested independently.

| Task | Depends On | Estimated Effort |
|------|-----------|-----------------|
| 1.1 Global fork choice (`fork_choice.rs`) | -- | Medium |
| 1.2 Finality detector (`finality.rs`) | 1.1 | Medium |
| 1.3 Indexed storage (child map, height map, latest messages) | -- | Medium |
| 1.4 Block validation pipeline (`validation.rs`) | 1.1 | Medium |
| 1.5 Consensus test suite (multi-validator simulations) | 1.1, 1.2 | Medium |
| 1.6 Property-based tests for safety invariants | 1.2 | Small |

### Phase 2: Execution Layer Bridge

Goal: Cordial Miners can execute Rholang deploys and produce state transitions.

| Task | Depends On | Estimated Effort |
|------|-----------|-----------------|
| 2.1 Typed payload (`CordialBlockPayload`) | Phase 1 | Small |
| 2.2 Deploy pool and selection | 2.1 | Medium |
| 2.3 RSpace runtime integration (pre/post state hashes) | 2.1 | Large |
| 2.4 System deploy support (slash, close block) | 2.3 | Small |

### Phase 3: f1r3node Integration

Goal: Cordial Miners can run as an alternative consensus in f1r3node.

| Task | Depends On | Estimated Effort |
|------|-----------|-----------------|
| 3.1 `Casper` trait implementation (adapter) | Phase 1, Phase 2 | Large |
| 3.2 `MultiParentCasper` trait implementation | 3.1 | Medium |
| 3.3 `CasperSnapshot` construction from blocklace state | 3.1 | Medium |
| 3.4 Cryptographic alignment (Blake2b, Secp256k1 option) | -- | Small |
| 3.5 Block format translation (`Block` <-> `BlockMessage`) | 3.1 | Medium |
| 3.6 Configuration (`CasperShardConf` equivalent) | 3.1 | Small |

### Phase 4: Production Hardening

Goal: Production-ready alternative to CBC Casper.

| Task | Depends On | Estimated Effort |
|------|-----------|-----------------|
| 4.1 Persistent LMDB storage | Phase 1 | Medium |
| 4.2 DAG pruning and garbage collection | 4.1 | Medium |
| 4.3 Network integration (bridge to `TransportLayer` or native) | Phase 3 | Large |
| 4.4 Performance benchmarks (finality latency, throughput) | Phase 3 | Medium |
| 4.5 Adversarial testing (partitions, equivocators, non-cordial rounds) | Phase 3 | Large |
| 4.6 Consensus switching mechanism (runtime swap between CBC and Cordial) | Phase 3 | Large |

---

## 7. File Structure Proposal

```
blocklace/
  src/
    lib.rs
    block.rs                    -- (existing)
    blocklace.rs                -- (existing, extend with indexes)
    crypto.rs                   -- (existing, extract trait)
    types/                      -- (existing)
    consensus/
      mod.rs
      fork_choice.rs            -- Global fork choice (Phase 1.1)
      finality.rs               -- Finality detector (Phase 1.2)
      cordial_condition.rs      -- Cordial block verification
    validation/
      mod.rs
      block_validator.rs        -- Block validation pipeline (Phase 1.4)
      deploy_validator.rs       -- Deploy-level checks
    storage/
      mod.rs
      traits.rs                 -- BlocklaceStore trait
      memory.rs                 -- In-memory backend (current)
      lmdb.rs                   -- LMDB persistent backend (Phase 4.1)
      indexes.rs                -- Child map, height map, latest messages (Phase 1.3)
    execution/
      mod.rs
      payload.rs                -- CordialBlockPayload type (Phase 2.1)
      deploy_pool.rs            -- Deploy pool and selection (Phase 2.2)
    bridge/
      mod.rs
      casper_trait_impl.rs      -- Casper trait adapter (Phase 3.1)
      block_translation.rs      -- Block <-> BlockMessage (Phase 3.5)
      snapshot.rs               -- CasperSnapshot construction (Phase 3.3)
    network/                    -- (existing)
  tests/                        -- (existing, extend per phase)
  docs/
    implementation.md           -- (existing)
    cordial-miners-vs-cbc-casper.md -- (this document)
```

---

## 8. Key f1r3node Integration Files

These are the files in f1r3node that the Cordial Miners implementation must interface with. Read these when implementing the bridge layer:

| File | Contains | Relevance |
|------|----------|-----------|
| `casper/src/rust/casper.rs` | `Casper` trait, `MultiParentCasper` trait, `CasperSnapshot`, `CasperShardConf` | Primary integration interface |
| `casper/src/rust/estimator.rs` | `Estimator`, `ForkChoice`, LMD-GHOST | Replace with cordial fork choice |
| `casper/src/rust/finality/finalizer.rs` | `Finalizer`, `cannot_be_orphaned()` | Replace with blocklace finality |
| `casper/src/rust/safety/clique_oracle.rs` | `CliqueOracle`, fault tolerance computation | Eliminated by blocklace finality |
| `casper/src/rust/block_status.rs` | `BlockStatus`, `InvalidBlock` enum (23+ variants) | Subset needed for validation |
| `casper/src/rust/blocks/block_processor.rs` | `BlockProcessor`, incoming block pipeline | Calls `Casper` trait methods |
| `casper/src/rust/blocks/proposer/block_creator.rs` | `BlockCreator`, deploy selection, block assembly | Needs deploy pool integration |
| `casper/src/rust/blocks/proposer/proposer.rs` | `Proposer`, constraint checks, validation traits | Calls `Casper` trait methods |
| `casper/src/rust/multi_parent_casper_impl.rs` | `MultiParentCasperImpl` | Reference implementation to mirror |
| `casper/src/rust/engine/running.rs` | `Running` engine, message dispatch | Top-level consumer of `Casper` trait |
| `models/src/rust/casper/protocol/casper_message.rs` | `BlockMessage`, `Body`, `Header`, `DeployData`, all wire types | Block format translation |
| `models/src/rust/block_metadata.rs` | `BlockMetadata` | Lightweight block representation for DAG |
| `block_storage/src/rust/dag/block_dag_key_value_storage.rs` | `KeyValueDagRepresentation` | DAG storage interface |
