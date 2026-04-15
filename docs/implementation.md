# Blocklace Implementation Documentation

This document describes the current state of the blocklace implementation, based on the formal definitions from the Cordial Miners paper (https://arxiv.org/abs/2205.09174).

## Overview

The blocklace is a DAG-based data structure used in Byzantine fault-tolerant distributed systems. Each node in the network creates cryptographically signed blocks that reference predecessor blocks, forming a directed acyclic graph (a "lace" of blocks). Consensus emerges from the blocklace structure itself rather than from an explicit justification protocol.

---

## Project Structure

```
src/
  lib.rs             -- Crate root; re-exports public types
  main.rs            -- Binary entry point (placeholder)
  block.rs           -- Block struct and free functions
  blocklace.rs       -- Blocklace struct (the core data structure)
  crypto.rs          -- SHA-256 hashing, ED25519 signing and verification
  consensus/
    mod.rs           -- Re-exports consensus types and functions
    fork_choice.rs   -- Global fork choice, validator tips, cordial condition
    finality.rs      -- Finality detector via supermajority agreement
    validation.rs    -- Block validation pipeline
  execution/
    mod.rs           -- Re-exports execution types
    payload.rs       -- CordialBlockPayload and all deploy/state types
    deploy_pool.rs   -- Deploy pool, selection, and ancestor dedup
  network/
    mod.rs           -- Re-exports Message, Peer, Node
    message.rs       -- Wire protocol message types
    peer.rs          -- TCP peer: bind, connect, handshake, send/recv
    node.rs          -- Node: ties Peer + Blocklace for block propagation
    NETWORK.md       -- Detailed networking documentation
  types/
    mod.rs           -- Re-exports NodeId, BlockIdentity, BlockContent
    node_id.rs       -- NodeId type
    identity_id.rs   -- BlockIdentity type
    content_id.rs    -- BlockContent type
tests/
  mod.rs                       -- Shared test helpers
  test_block.rs                -- Unit tests for Block (9 tests)
  test_blocklace.rs            -- Unit tests for Blocklace (7 tests)
  test_hash.rs                 -- SHA-256 content hashing tests (5 tests)
  test_sign.rs                 -- ED25519 signing/verification tests (6 tests)
  test_message.rs              -- Handshake/keepalive message tests (6 tests)
  test_message_propagation.rs  -- Block propagation message tests (7 tests)
  test_peer.rs                 -- TCP peer tests (7 tests)
  test_node.rs                 -- Node message handling tests (13 tests)
  test_fork_choice.rs          -- Fork choice and cordial condition tests (13 tests)
  test_finality.rs             -- Finality detector tests (13 tests)
  test_validation.rs           -- Block validation pipeline tests (18 tests)
  test_consensus_simulation.rs -- Multi-validator simulation tests (10 tests)
  test_payload.rs              -- Typed payload serialization tests (10 tests)
  test_deploy_pool.rs          -- Deploy pool and selection tests (18 tests)
```

---

## Core Types

### `NodeId` (src/types/node_id.rs)

Represents a node's identity in the network (in practice, a public key).

```rust
pub struct NodeId(pub Vec<u8>);
```

Derives: `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Serialize`, `Deserialize`

### `BlockIdentity` (src/types/identity_id.rs)

The cryptographic identity of a block: `i = signedhash((v, P), k_p)`.

| Field          | Type       | Description                                |
|----------------|------------|--------------------------------------------|
| `content_hash` | `[u8; 32]` | SHA-256 hash of the serialized BlockContent |
| `creator`      | `NodeId`   | The node that signed this block            |
| `signature`    | `Vec<u8>`  | `sign(content_hash, creator_private_key)`  |

### `BlockContent` (src/types/content_id.rs)

The block content `C = (v, P)` -- a payload and a set of predecessor identities.

| Field          | Type                    | Description                              |
|----------------|-------------------------|------------------------------------------|
| `payload`      | `Vec<u8>`               | Arbitrary value `v` (operations, txns)   |
| `predecessors` | `HashSet<BlockIdentity>`| Set `P` of predecessor block identities  |

A block is **initial (genesis)** when `predecessors` is empty.

### `Block` (src/block.rs)

A complete block combining identity and content. All core types derive `Serialize`/`Deserialize` for wire transport.

| Method              | Description                                         |
|---------------------|-----------------------------------------------------|
| `is_initial()`      | True if the block has no predecessors (genesis)     |
| `node()`            | Returns the creator: `node(b) = p`                 |
| `id()`              | Returns the block's identity: `id(b) = i`          |
| `is_pointed_from()` | True if `other` lists `self` as a predecessor       |

---

## Blocklace (src/blocklace.rs)

The central data structure -- a set of blocks stored as `HashMap<BlockIdentity, BlockContent>`.

### Invariants

1. **CLOSED**: Every predecessor must exist in the blocklace. `P ⊂ dom(B)`
2. **CHAIN**: All blocks from a correct node are totally ordered under precedence.

### Map-View Accessors (Definition 2.3)

| Method      | Description                                       |
|-------------|---------------------------------------------------|
| `content()` | `B(b)` -- get content by identity                 |
| `get()`     | `B[b]` -- get full block by identity              |
| `get_set()` | `B[P]` -- get all blocks matching a set of ids    |
| `dom()`     | `dom(B)` -- set of all known block identities     |

### Pointed Relation (Definition 2.2)

| Method                  | Description                                         |
|-------------------------|-----------------------------------------------------|
| `predecessors()`        | `<-b` -- direct predecessors of block `b`           |
| `ancestors()`           | `<b` -- transitive closure (all ancestors, not `b`) |
| `ancestors_inclusive()`  | `<=b` -- ancestors of `b` including `b` itself     |
| `precedes()`            | `a < b` -- true if `a` is in `b`'s ancestry        |
| `preceedes_or_equals()` | `a <= b` -- precedes or equal                       |

### Chain Axiom and Byzantine Detection

| Method                      | Description                                              |
|-----------------------------|----------------------------------------------------------|
| `blocks_by()`               | All blocks created by node `p`                           |
| `satisfies_chain_axiom()`   | Check CHAIN axiom for a specific node                    |
| `satisfies_chain_axiom_all()` | Check CHAIN axiom for every node                       |
| `find_equivacators()`       | Returns nodes violating CHAIN (Byzantine equivocators)   |
| `tip_of()`                  | The most recent block of node `p` (chain tip)            |

---

## Cryptography (src/crypto.rs)

| Crate            | Version | Purpose                        |
|------------------|---------|--------------------------------|
| `sha2`           | 0.10    | SHA-256 content hashing        |
| `ed25519-dalek`  | 2       | ED25519 signing & verification |
| `rand`           | 0.8     | Key generation (tests)         |

| Function         | Description                                                       |
|------------------|-------------------------------------------------------------------|
| `hash_content()` | Deterministic SHA-256 hash: length-prefixed payload + sorted predecessors |
| `sign()`         | ED25519 signature (64 bytes) over a content hash                  |
| `verify()`       | Verify an ED25519 signature against a public key                  |

---

## Consensus (src/consensus/)

The consensus module implements the Cordial Miners protocol from the paper, providing fork choice, finality detection, and block validation.

### Fork Choice (fork_choice.rs)

Replaces CBC Casper's LMD-GHOST estimator. The fork choice is derived directly from the blocklace structure -- no separate justification model needed.

| Function                  | Description                                                    |
|---------------------------|----------------------------------------------------------------|
| `fork_choice()`           | Compute weighted fork choice: collect tips, exclude equivocators, compute LCA, score by stake, rank tips |
| `collect_validator_tips()`| Collect each bonded validator's latest block (replaces `latest_message_hashes`) |
| `is_cordial()`            | Check if a block references all known validator tips (paper's cordial condition) |

**`ForkChoice` struct**: `{ tips: Vec<BlockIdentity>, lca: BlockIdentity, scores: HashMap<BlockIdentity, u64> }`

**Note**: `fork_choice()` is primarily for f1r3node compatibility. The paper's protocol does not define a traditional fork choice -- validators reference ALL tips (cordial condition) rather than picking one fork. `is_cordial()` and `collect_validator_tips()` are the paper-native functions.

### Finality Detector (finality.rs)

Replaces CBC Casper's clique oracle with simple supermajority stake summation. A block is finalized when > 2/3 of honest stake has it in their ancestry.

| Function               | Description                                                     |
|------------------------|-----------------------------------------------------------------|
| `check_finality()`     | Check if a block has > 2/3 honest stake support. Returns `FinalityStatus` |
| `find_last_finalized()`| Find the highest finalized block in the blocklace               |
| `can_be_finalized()`   | Check if a block can still reach finality or is orphaned        |

**`FinalityStatus` enum**: `Finalized { supporting_stake, total_honest_stake }` | `Pending { ... }` | `Unknown`

**Key properties**:
- Equivocator stake is excluded from total honest stake
- Uses integer arithmetic (`supporting * 3 > total * 2`) to avoid floating point
- `can_be_finalized()` accounts for undecided validators (no tip yet)

### Block Validation (validation.rs)

Block validation pipeline that checks blocks before insertion. Replaces CBC Casper's 23+ validation checks with the subset required by Cordial Miners.

| Function            | Description                                                       |
|---------------------|-------------------------------------------------------------------|
| `validate_block()`  | Run all enabled checks, collect all errors (no short-circuit)     |
| `validated_insert()` | Validate then insert on success                                  |

**`InvalidBlock` variants**:

| Variant                | Description                                          |
|------------------------|------------------------------------------------------|
| `InvalidContentHash`   | `content_hash` does not match `hash(content)`        |
| `InvalidSignature`     | ED25519 signature verification failed                |
| `UnknownSender`        | Creator is not a bonded validator                    |
| `MissingPredecessors`  | Closure axiom violation                              |
| `Equivocation`         | Chain axiom violation (creates fork in creator's chain) |
| `NotCordial`           | Block doesn't reference all known validator tips     |

**`ValidationConfig`**: Toggleable checks. `Default` enables hash, signature, sender, closure, and chain axiom. `strict()` also enables cordial condition. Individual checks can be disabled (e.g., skip crypto for self-created blocks).

---

## Execution Layer (src/execution/)

Typed block payloads for carrying deploy execution data within the blocklace's generic `payload: Vec<u8>`.

### CordialBlockPayload (payload.rs)

Serialized into `BlockContent.payload` via bincode, keeping the blocklace protocol-agnostic while providing structured data for f1r3node integration.

| Type | f1r3node equivalent | Description |
|------|---------------------|-------------|
| `CordialBlockPayload` | `Body` | Top-level: state + deploys + rejected + system deploys |
| `BlockState` | `F1r3flyState` | Pre/post state hashes, bonds, block number |
| `Bond` | `Bond` | Validator identity + stake amount |
| `Deploy` | `DeployData` | Code, phlo price/limit, timestamps, shard id |
| `SignedDeploy` | `Signed<DeployData>` | Deploy + deployer public key + signature |
| `ProcessedDeploy` | `ProcessedDeploy` | Executed deploy with cost and failure flag |
| `RejectedDeploy` | `RejectedDeploy` | Rejected deploy with reason enum |
| `ProcessedSystemDeploy` | `ProcessedSystemDeploy` | Slash equivocator / CloseBlock |

**Helpers**:
- `CordialBlockPayload::genesis(bonds)` -- create empty genesis payload
- `CordialBlockPayload::to_bytes()` / `from_bytes()` -- bincode serialization
- `CordialBlockPayload::bonds_map()` -- extract `HashMap<NodeId, u64>` for consensus functions

### Deploy Pool (deploy_pool.rs)

Manages pending user deploys and selects them for inclusion in new blocks. Adapts f1r3node's `KeyValueDeployStorage` + `BlockCreator::prepare_user_deploys()` to the blocklace.

**Note**: The Cordial Miners paper does not define a deploy pool -- this is an engineering layer above the consensus protocol.

| Type | Description |
|------|-------------|
| `DeployPool` | Signature-keyed storage for pending deploys |
| `DeployPoolConfig` | `max_user_deploys_per_block`, `deploy_lifespan`, `min_phlo_price` |
| `PoolError` | `Duplicate`, `InsufficientPhloPrice`, `InvalidSignature` |
| `SelectedDeploys` | Result: `deploys: Vec<SignedDeploy>` + `cap_hit: bool` |

**DeployPool API**:

| Method | Description |
|--------|-------------|
| `add()` | Add a deploy; dedup by signature, validate phlo price |
| `remove()` | Remove by signature |
| `len()` / `is_empty()` / `iter()` | Standard collection accessors |
| `prune_expired()` | Remove block-expired deploys; returns removed signatures |
| `select_for_block()` | Apply filters, cap, return selected deploys |

**Selection filters (applied in order)**:
1. Not future: `valid_after_block_number <= current_block`
2. Not block-expired: within `deploy_lifespan` window (handles u64 underflow)
3. Not duplicated in ancestry: not in `deploys_in_scope` set

**Capping strategy** (`oldest-plus-newest`): when more valid deploys exist than `max_user_deploys_per_block`, select `(cap - 1)` oldest + 1 newest. Prevents head-of-line blocking under stress. Special case `cap == 1` selects just the newest.

**Ancestor dedup**:

`compute_deploys_in_scope(blocklace, predecessors, current_block, lifespan)` walks the blocklace ancestry of the given predecessors, deserializing each block's `CordialBlockPayload` to extract deploy signatures. Blocks outside the lifespan window are skipped. Returns a `HashSet<Vec<u8>>` of signatures used to filter pending deploys against duplicates.

---

## Networking (src/network/)

P2P communication and block propagation over async TCP using tokio, serde, and bincode.

- **Message** -- Wire protocol with handshake, keepalive, block propagation, and sync
- **Peer** -- TCP peer with bind/connect, Hello/HelloAck handshake, and connection tracking
- **Node** -- Integrates `Peer` + `Blocklace`: creates/broadcasts blocks, handles incoming messages, requests missing predecessors

For detailed API reference, wire protocol diagrams, and flow charts, see [src/network/NETWORK.md](../src/network/NETWORK.md).

---

## Test Coverage (142 tests)

| Test File | Count | What it covers |
|-----------|-------|----------------|
| `test_block.rs` | 9 | Block struct, equality, hashing, is_initial, is_pointed_from |
| `test_blocklace.rs` | 7 | Insert, closure axiom, map-view accessors |
| `test_hash.rs` | 5 | SHA-256 determinism, uniqueness, ordering independence |
| `test_sign.rs` | 6 | ED25519 sign/verify roundtrip, tamper detection |
| `test_message.rs` | 6 | Handshake/keepalive message serialization |
| `test_message_propagation.rs` | 7 | Block propagation message serialization |
| `test_peer.rs` | 7 | TCP peer binding, handshake, multi-client |
| `test_node.rs` | 13 | Node message handling, block broadcast |
| `test_fork_choice.rs` | 13 | Fork choice, LCA, scoring, validator tips, cordial condition |
| `test_finality.rs` | 13 | Supermajority finality, equivocator exclusion, orphan detection |
| `test_validation.rs` | 18 | Content hash, signature, sender, closure, equivocation, cordial |
| `test_consensus_simulation.rs` | 10 | Multi-validator end-to-end scenarios |
| `test_payload.rs` | 10 | Typed payload serialization, block integration, bonds map |
| `test_deploy_pool.rs` | 18 | Add/remove, selection filters, capping, prune, ancestor dedup |

---

## Roadmap Status

Based on the roadmap in [cordial-miners-vs-cbc-casper.md](cordial-miners-vs-cbc-casper.md):

### Phase 1: Core Consensus (Standalone)

| Task | Status |
|------|--------|
| 1.1 Global fork choice | Complete |
| 1.2 Finality detector | Complete |
| 1.3 Indexed storage (child map, height map) | Not started (optimization) |
| 1.4 Block validation pipeline | Complete |
| 1.5 Consensus test suite (multi-validator simulations) | Complete |
| 1.6 Property-based tests for safety invariants | Not started (hardening) |

### Phase 2: Execution Layer Bridge (branch: `phase2/execution-layer-bridge`)

| Task | Status |
|------|--------|
| 2.1 Typed payload (`CordialBlockPayload`) | Complete |
| 2.2 Deploy pool and selection | Complete |
| 2.3 RSpace runtime integration | Not started |
| 2.4 System deploy support | Not started |

## What Is Not Yet Implemented

**Optimizations** (needed for production scale):
- Indexed storage: child map, height map, latest messages index for O(1) lookups
- Persistent storage (LMDB or similar) to replace in-memory HashMap

**Protocol gaps**:
- Pending-block retry: blocks rejected for missing predecessors are dropped; no re-attempt after predecessors arrive
- Transitive sync: sync discovers missing block ids but doesn't recursively fetch their predecessors

**f1r3node integration** (Phase 2-3):
- RSpace runtime integration (pre/post state hashes)
- System deploy support (slash, close block)
- `Casper` trait adapter for f1r3node
- Cryptographic alignment (Blake2b, Secp256k1 option)
- Time-based deploy expiration (`Option<expiration_timestamp>` on `Deploy`)
