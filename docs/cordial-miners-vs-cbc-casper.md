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
| **Fork choice** | LMD-GHOST via `Estimator`: score blocks by validator weight, walk children from LCA | **Implemented**: `fork_choice()` with stake-weighted scoring + `is_cordial()` for paper-native cordial condition |
| **Finality** | Clique Oracle: find max-weight clique of agreeing validators, compare against fault tolerance threshold | **Implemented**: `check_finality()` via supermajority (> 2/3 honest stake) observable from blocklace ancestry |
| **Equivocation** | Multi-step: `EquivocationDetector` + `InvalidBlock::AdmissibleEquivocation` / `IgnorableEquivocation` / `NeglectedEquivocation` | **Implemented**: structural detection via `find_equivocators()` + rejection via `validate_block()` (`Equivocation` variant) |
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
| Block structure (`Block`, `BlockIdentity`, `BlockContent`) | Complete | Faithful to paper Definition 2.2, with serde serialization |
| Blocklace data structure | Complete | Map-view accessors, insertion with closure axiom |
| Predecessor/ancestor traversal | Complete | `predecessors()`, `ancestors()`, `ancestors_inclusive()`, `precedes()` |
| Chain axiom enforcement | Complete | `satisfies_chain_axiom()`, `find_equivocators()` |
| Per-node tip tracking | Complete | `tip_of(node)` returns most recent block per validator |
| Cryptography (hash, sign, verify) | Complete | SHA-256 + Ed25519 via `sha2` and `ed25519-dalek` |
| P2P networking | Complete | TCP with handshake, block broadcast, sync protocol |
| Block propagation | Complete | `BroadcastBlock`, `RequestBlock`, `SyncRequest`/`SyncResponse` |
| **Global fork choice** | **Complete** | `fork_choice()`, `collect_validator_tips()`, `is_cordial()` — replaces LMD-GHOST |
| **Finality detector** | **Complete** | `check_finality()`, `find_last_finalized()`, `can_be_finalized()` — replaces clique oracle |
| **Block validation pipeline** | **Complete** | `validate_block()`, `validated_insert()` — content hash, signature, sender, closure, chain axiom, cordial condition |
| **Multi-validator consensus tests** | **Complete** | 10 end-to-end simulation scenarios |
| **Typed block payload** | **Complete** (Phase 2.1) | `CordialBlockPayload` maps to f1r3node's `Body` with `BlockState`, `Bond`, `Deploy`, `SignedDeploy`, `ProcessedDeploy`, `RejectedDeploy`, `ProcessedSystemDeploy` |
| **Deploy pool & selection** | **Complete** (Phase 2.2) | `DeployPool`, `select_for_block()` with filters + oldest-plus-newest capping, `compute_deploys_in_scope()` for ancestor dedup |
| **Runtime abstraction** | **Complete** (Phase 2.3) | `RuntimeManager` trait + `MockRuntime` deterministic stand-in. Real RSpace adapter deferred to Phase 3 as a separate crate to keep the core library free of RSpace/Rholang deps |
| Test suite | Complete | **159 tests** covering blocks, blocklace, crypto, networking, consensus, execution |

### 2.2 What Is Missing

| Component | Priority | Required For |
|-----------|----------|-------------|
| ~~Global fork choice~~ | ~~Critical~~ | ~~Replacing `Estimator`~~ **DONE** |
| ~~Finality detector~~ | ~~Critical~~ | ~~Replacing `Finalizer` + `CliqueOracle`~~ **DONE** |
| ~~Block validation pipeline~~ | ~~High~~ | ~~Replacing `BlockProcessor` validation~~ **DONE** |
| ~~Typed payload (deploys, state transitions)~~ | ~~Critical~~ | ~~Smart contract execution~~ **DONE (types; execution pending)** |
| ~~Deploy pool / selection~~ | ~~High~~ | ~~Block creation with transactions~~ **DONE** |
| ~~RSpace runtime integration~~ | ~~High~~ | ~~Execute deploys, produce post-state hash~~ **DONE (trait + mock; real RSpace adapter in Phase 3)** |
| `Casper` trait implementation | Critical | Plugging into f1r3node |
| Real RSpace adapter crate | High | Execute Rholang against actual tuplespace |
| Persistent storage | High | Running beyond in-memory prototype |
| `CasperSnapshot` equivalent | High | State management across the system |
| Indexed DAG (child map, height map) | Medium | Performance at scale |
| Cryptographic alignment (Blake2b, Secp256k1) | Medium | Wire compatibility with f1r3node |
| TLS and gRPC network layer | Low | Can bridge via `TransportLayer` trait |

---

## 3. Gap Analysis and Required Work

### 3.1 Fork Choice (Critical) -- COMPLETE

**Implemented in**: `src/consensus/fork_choice.rs` (239 lines)

**What was built**:
- `fork_choice(blocklace, bonds)` -- collects validator tips, excludes equivocators, computes LCA via ancestor set intersection, builds stake-weighted score map, ranks tips by descending score
- `collect_validator_tips(blocklace, bonds)` -- replaces CBC Casper's `latest_message_hashes()` map
- `is_cordial(block, known_tips)` -- paper-native cordial condition check (no CBC Casper equivalent)
- `ForkChoice { tips, lca, scores }` -- maps to f1r3node's `ForkChoice { tips, lca }`

**Note**: `fork_choice()` is primarily for f1r3node compatibility. The paper doesn't define a traditional fork choice rule -- validators reference ALL tips (cordial condition) rather than picking one fork. The paper-native functions are `is_cordial()` and `collect_validator_tips()`.

**Tests**: 13 tests in `test_fork_choice.rs` covering single/multi-validator, diverging forks, equivocator exclusion, and cordial condition.

### 3.2 Finality Detector (Critical) -- COMPLETE

**Implemented in**: `src/consensus/finality.rs` (200 lines)

**What was built**:
- `check_finality(blocklace, block_id, bonds)` -- checks if > 2/3 of honest stake has the block in their ancestry. Returns `FinalityStatus::Finalized`, `Pending`, or `Unknown`
- `find_last_finalized(blocklace, bonds)` -- scans all blocks, returns the highest finalized one
- `can_be_finalized(blocklace, block_id, bonds)` -- orphan detection, analogous to CBC Casper's `cannot_be_orphaned` pre-filter

**Key properties**:
- Equivocator stake excluded from total honest stake (cleaner than CBC's three-tier classification)
- Integer arithmetic (`supporting * 3 > total * 2`) avoids floating point precision issues
- `can_be_finalized()` accounts for undecided validators (no tip yet)

**Replaces**: CBC Casper's `Finalizer` + `CliqueOracle` (NP-hard clique enumeration) with simple O(n) stake summation.

**Tests**: 13 tests in `test_finality.rs` covering supermajority threshold, exact 2/3 boundary, equivocator exclusion, orphan detection, and monotonic finality advancement.

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

**What Cordial Miners needs** -- **COMPLETE**:

**Implemented in**: `src/consensus/validation.rs` (249 lines)

The blocklace structure replaces many of CBC Casper's 23+ checks. The validation pipeline implements the subset that applies:

1. **Content hash verification** -- `InvalidContentHash` (`hash(content) != content_hash`)
2. **Signature verification** -- `InvalidSignature` (ED25519 verify fails)
3. **Sender is bonded** -- `UnknownSender` (creator not in bonds map)
4. **Closure check** -- `MissingPredecessors` (predecessors not in blocklace)
5. **Chain axiom enforcement** -- `Equivocation` (rejects blocks that would violate CHAIN, not just detects)
6. **Cordial condition** -- `NotCordial` (block doesn't reference all known tips)

Checks eliminated by blocklace structure: justification regression, separate parent/justification consistency, three-tier equivocation classification, block number/sequence checks.

`ValidationConfig` allows toggling checks (e.g., skip crypto for self-created blocks, disable cordial for non-strict mode). `validated_insert()` combines validation and insertion.

**Tests**: 18 tests in `test_validation.rs` + 10 end-to-end simulation tests in `test_consensus_simulation.rs`.

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

| Task | Depends On | Status |
|------|-----------|--------|
| 1.1 Global fork choice (`fork_choice.rs`) | -- | **COMPLETE** (239 lines, 13 tests) |
| 1.2 Finality detector (`finality.rs`) | 1.1 | **COMPLETE** (200 lines, 13 tests) |
| 1.3 Indexed storage (child map, height map, latest messages) | -- | Not started (optimization) |
| 1.4 Block validation pipeline (`validation.rs`) | 1.1 | **COMPLETE** (249 lines, 18 tests) |
| 1.5 Consensus test suite (multi-validator simulations) | 1.1, 1.2 | **COMPLETE** (10 scenarios) |
| 1.6 Property-based tests for safety invariants | 1.2 | Not started (hardening) |

### Phase 2: Execution Layer Bridge

Goal: Cordial Miners can execute Rholang deploys and produce state transitions.

Branch: `phase2/execution-layer-bridge`

| Task | Depends On | Status |
|------|-----------|--------|
| 2.1 Typed payload (`CordialBlockPayload`) | Phase 1 | **COMPLETE** (193 lines, 10 tests) |
| 2.2 Deploy pool and selection | 2.1 | **COMPLETE** (295 lines, 18 tests) |
| 2.3 RSpace runtime integration (pre/post state hashes) | 2.1 | **COMPLETE** — trait + `MockRuntime` (314 lines, 17 tests). Real RSpace adapter deferred to Phase 3 to keep core crate free of RSpace/Rholang deps |
| 2.4 System deploy support (slash, close block) | 2.3 | **COMPLETE** (covered by 2.3 mock; Slash / CloseBlock variants modeled). Real RSpace-backed impl comes with the Phase 3 adapter crate |

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

## 7. File Structure

Items marked with checkmarks exist; unmarked items are planned for future phases.

```
blocklace/
  src/
    lib.rs                       -- [x] Crate root
    block.rs                     -- [x] Block struct with serde
    blocklace.rs                 -- [x] Core data structure
    crypto.rs                    -- [x] SHA-256 + ED25519
    types/                       -- [x] NodeId, BlockIdentity, BlockContent
    consensus/
      mod.rs                     -- [x] Re-exports
      fork_choice.rs             -- [x] Phase 1.1: fork choice, LCA, cordial condition
      finality.rs                -- [x] Phase 1.2: supermajority finality detector
      validation.rs              -- [x] Phase 1.4: block validation pipeline
    network/
      mod.rs                     -- [x] Re-exports
      message.rs                 -- [x] Wire protocol (handshake + propagation + sync)
      peer.rs                    -- [x] TCP peer with async handshake
      node.rs                    -- [x] Node = Peer + Blocklace
      NETWORK.md                 -- [x] Networking documentation
    storage/                     -- [ ] Phase 1.3 / Phase 4
      traits.rs                  -- [ ] BlocklaceStore trait
      memory.rs                  -- [ ] In-memory backend (extract from blocklace.rs)
      lmdb.rs                    -- [ ] LMDB persistent backend
      indexes.rs                 -- [ ] Child map, height map, latest messages
    execution/                   -- [x] Phase 2
      mod.rs                     -- [x] Re-exports
      payload.rs                 -- [x] Phase 2.1: CordialBlockPayload + deploy types
      deploy_pool.rs             -- [x] Phase 2.2: Deploy pool, selection, ancestor dedup
      runtime.rs                 -- [x] Phase 2.3: RuntimeManager trait + MockRuntime
    bridge/                      -- [ ] Phase 3 (likely a separate workspace crate)
      casper_trait_impl.rs       -- [ ] Casper trait adapter
      block_translation.rs       -- [ ] Block <-> BlockMessage
      snapshot.rs                -- [ ] CasperSnapshot construction
      rspace_runtime.rs          -- [ ] Real RuntimeManager impl against f1r3node RSpace
  tests/
    test_block.rs                -- [x] 9 tests
    test_blocklace.rs            -- [x] 7 tests
    test_hash.rs                 -- [x] 5 tests
    test_sign.rs                 -- [x] 6 tests
    test_message.rs              -- [x] 6 tests
    test_message_propagation.rs  -- [x] 7 tests
    test_peer.rs                 -- [x] 7 tests
    test_node.rs                 -- [x] 13 tests
    test_fork_choice.rs          -- [x] 13 tests
    test_finality.rs             -- [x] 13 tests
    test_validation.rs           -- [x] 18 tests
    test_consensus_simulation.rs -- [x] 10 tests
    test_payload.rs              -- [x] 10 tests
    test_deploy_pool.rs          -- [x] 18 tests
    test_runtime.rs              -- [x] 17 tests (159 total)
  docs/
    implementation.md            -- [x] Implementation documentation
    cordial-miners-vs-cbc-casper.md -- [x] This document
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
