# Blocklace Implementation Documentation

This document describes the current state of the blocklace implementation, based on the formal definitions from the Cordial Miners paper (https://arxiv.org/abs/2205.09174).

## Overview

The blocklace is a DAG-based data structure used in Byzantine fault-tolerant distributed systems. Each node in the network creates cryptographically signed blocks that reference predecessor blocks, forming a directed acyclic graph (a "lace" of blocks). Consensus emerges from the blocklace structure itself rather than from an explicit justification protocol.

---

## Project Structure

The repo is a Cargo workspace with three crates, layered from bottom to top:

- **`crates/blocklace/`** — the standalone consensus library. No f1r3node dependencies. SHA-256 + ED25519.
- **`crates/blocklace-f1r3node/`** — f1r3node integration adapter: Casper trait mirror, block translation, snapshot construction, crypto bridge. Also no f1r3node dependencies — uses mirror types so the translation layer builds standalone.
- **`crates/blocklace-f1r3rspace/`** — real RSpace-backed `RuntimeManager` adapter. Depends on f1r3node's `casper`, `models`, `rholang`, `rspace_plus_plus`, `crypto`, `shared` path dependencies. Requires `protoc` on PATH to build.

```
Cargo.toml               -- Workspace manifest with [workspace.dependencies]
.cargo/
  config.toml            -- RUST_MIN_STACK=8MB + target-feature=+aes,+sse2
                            (mirrors f1r3node; required by Rholang interpreter + gxhash)
crates/
  blocklace/             -- Core library
    Cargo.toml
    src/
      lib.rs             -- Crate root; re-exports public types
      main.rs            -- Binary entry point (placeholder)
      block.rs           -- Block struct and free functions
      blocklace.rs       -- Blocklace struct (the core data structure)
      crypto.rs          -- SHA-256 hashing, ED25519 signing and verification
      consensus/
        mod.rs
        fork_choice.rs   -- Global fork choice, validator tips, cordial condition
        finality.rs      -- Finality detector via supermajority agreement
        validation.rs    -- Block validation pipeline
      execution/
        mod.rs
        payload.rs       -- CordialBlockPayload and all deploy/state types
        deploy_pool.rs   -- Deploy pool, selection, and ancestor dedup
        runtime.rs       -- RuntimeManager trait + MockRuntime implementation
      network/
        mod.rs
        message.rs       -- Wire protocol message types
        peer.rs          -- TCP peer: bind, connect, handshake, send/recv
        node.rs          -- Node: ties Peer + Blocklace for block propagation
        NETWORK.md       -- Detailed networking documentation
      types/
        mod.rs
        node_id.rs
        identity_id.rs
        content_id.rs
    tests/
      (15 integration test files — see Test Coverage below)

  blocklace-f1r3node/    -- f1r3node integration adapter (Phase 3)
    Cargo.toml
    src/
      lib.rs             -- Crate root
      block_translation.rs -- Phase 3.5: Block <-> BlockMessage mirrors
      snapshot.rs        -- Phase 3.3: CasperSnapshot construction
      shard_conf.rs      -- Phase 3.6: CasperShardConf + FinalizerConf mirror
      crypto_bridge.rs   -- Phase 3.4: Blake2b, Secp256k1, ED25519 + block hash
      casper_adapter.rs  -- Phase 3.1/3.2: CordialCasper + CordialMultiParentCasper
      rspace_runtime.rs  -- Deprecated placeholder (moved to blocklace-f1r3rspace)
    tests/
      test_block_translation.rs  -- 13 tests
      test_snapshot.rs           -- 16 tests
      test_shard_conf.rs         -- 8 tests
      test_crypto_bridge.rs      -- 22 tests
      test_casper_adapter.rs     -- 21 tests

  blocklace-f1r3rspace/  -- Real RSpace-backed RuntimeManager adapter
    Cargo.toml           -- Path deps: f1r3node casper, models, rholang,
                             rspace_plus_plus, crypto, shared
    src/
      lib.rs             -- F1r3RspaceRuntime<'a> + translation helpers
    tests/
      test_translation.rs -- 13 tests (pure translation; no RSpace setup)
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

### Runtime (runtime.rs)

Abstract interface between consensus and execution. Defines what the blocklace needs from an execution engine without embedding f1r3node's RSpace/Rholang dependencies. Real RSpace integration is planned as a separate crate in Phase 3; this module ships the trait plus a deterministic mock.

**Note**: Not a paper concept. This abstraction keeps the core crate protocol-agnostic and execution-engine-agnostic.

| Type | Description |
|------|-------------|
| `RuntimeManager` | Trait: `execute_block()` and `validate_post_state()` |
| `MockRuntime` | Deterministic in-memory implementation for tests |
| `ExecutionRequest` | `{ pre_state_hash, deploys, system_deploys, bonds, block_number }` |
| `ExecutionResult` | `{ post_state_hash, processed_deploys, rejected_deploys, system_deploys, new_bonds }` |
| `SystemDeployRequest` | `Slash { validator }` \| `CloseBlock` |
| `RuntimeError` | `UnknownPreState` \| `InternalError(String)` |

**RuntimeManager trait**:

| Method | Description |
|--------|-------------|
| `execute_block()` | Consume an `ExecutionRequest`, produce an `ExecutionResult`. Must be deterministic: same input → same output |
| `validate_post_state()` | Default impl: re-run `execute_block` and compare against declared post-state. Replaces f1r3node's `InvalidTransaction` check |

**MockRuntime semantics**:
- **Cost**: `min(deploy.term.len(), phlo_limit)` -- 1 phlo per byte as a stand-in for phlogiston accounting
- **Failure**: `is_failed = true` when natural cost exceeds `phlo_limit`
- **Rejection**: deploys with empty signatures are rejected with `RejectReason::InvalidSignature`
- **Slash**: removes the validator's bond entry; `succeeded = true` iff the bond existed
- **CloseBlock**: always succeeds
- **Post-state hash**: SHA-256 over `(pre_state, block_number, processed_deploys, system_deploys, sorted bonds)` — deterministic and bond-order-independent
- **Modes**: `new()` chains pre/post states strictly (rejects unknown pre-states); `permissive()` accepts any pre-state for tests that don't care about chaining

---

## Networking (src/network/)

P2P communication and block propagation over async TCP using tokio, serde, and bincode.

- **Message** -- Wire protocol with handshake, keepalive, block propagation, and sync
- **Peer** -- TCP peer with bind/connect, Hello/HelloAck handshake, and connection tracking
- **Node** -- Integrates `Peer` + `Blocklace`: creates/broadcasts blocks, handles incoming messages, requests missing predecessors

For detailed API reference, wire protocol diagrams, and flow charts, see [src/network/NETWORK.md](../crates/blocklace/src/network/NETWORK.md).

---

## f1r3node Adapter (crates/blocklace-f1r3node/)

Phase 3 integration layer that exposes the Cordial Miners consensus through the interface f1r3node expects. The core `blocklace` crate stays free of f1r3node's RSpace, Rholang, and gRPC dependencies — all integration-specific code lives in this separate crate.

**Current model** (pre-`models`/`casper` path dependencies): every f1r3node type (`BlockMessage`, `Header`, `Body`, `F1r3flyState`, `Bond`, `DeployData`, `CasperShardConf`, etc.) is *mirrored* locally with the same field names, types, and defaults. The adapter operates on these mirrors. When the f1r3node path deps are uncommented in `crates/blocklace-f1r3node/Cargo.toml`, the mirrors get swapped for `use models::...` / `use casper::...` imports — method bodies don't change.

### Phase 3.5 — Block translation (block_translation.rs)

| Function | Direction | Notes |
|----------|-----------|-------|
| `block_to_message(&Block, shard_id) -> BlockMessage` | blocklace → f1r3node | Packs predecessors into both `parents_hash_list` and `justifications` (validator → latest block hash). Decodes `BlockContent.payload` as `CordialBlockPayload` and repacks it into f1r3node's `Body`. Output deterministic via sorted hashes. |
| `message_to_block(&BlockMessage) -> Block` | f1r3node → blocklace | Takes the union of `parents_hash_list` and `justifications.latest_block_hash` as the predecessor set. Looks up each predecessor's creator from the justifications map, falling back to `sender`. Recomputes `content_hash` so the blocklace stays internally consistent. |

`TranslationError` covers `PayloadDecodeFailed`, `NumericOverflow` (u64 ↔ i64 boundary), and `InvalidPredecessorHash` (wrong-length hash on the wire).

### Phase 3.3 — CasperSnapshot construction (snapshot.rs)

`build_snapshot(&Blocklace, &bonds, shard_conf, shard_id) -> Result<CasperSnapshot, SnapshotError>` assembles f1r3node's `CasperSnapshot` from live blocklace state. Reuses every Phase 1/2 primitive: `collect_validator_tips`, `fork_choice`, `find_last_finalized`, `compute_deploys_in_scope`, `block_to_message`.

| Snapshot field | Source |
|----------------|--------|
| `dag.dag_set`, `block_number_map`, `height_map` | `Blocklace::dom()` + each block's `CordialBlockPayload.state.block_number` |
| `dag.child_map` | Inverted predecessor relation |
| `dag.latest_messages_map` | `collect_validator_tips` (excludes equivocators) |
| `dag.last_finalized_block_hash`, `finalized_blocks_set` | `find_last_finalized` + `ancestors_inclusive` |
| `tips`, `lca` | `fork_choice` |
| `parents` | Each tip translated via `block_to_message` |
| `justifications` | `(validator, tip_hash)` per validator tip |
| `deploys_in_scope` | Ancestry walk within `deploy_lifespan` window |
| `max_block_num`, `max_seq_nums` | Derived from payloads / block counts |
| `on_chain_state.bonds_map`, `active_validators` | From bonds argument; `active_validators` excludes equivocators |

`DagRepresentation` is a simplified mirror using plain `HashMap`/`HashSet`/`BTreeMap` instead of f1r3node's concurrent `DashSet` and LMDB-backed indices — the snapshot is constructed once per call and read many times, not concurrently mutated.

### Phase 3.6 — CasperShardConf (shard_conf.rs)

Full mirror of f1r3node's 25-field `CasperShardConf` plus nested `FinalizerConf` (work-budget / step-timeout durations, defaults 8s/1s). Matches `CasperShardConf::new()` field-for-field.

Key helpers:

- `CasperShardConf::from_cordial(&DeployPoolConfig, shard_name)` — imports `max_user_deploys_per_block`, `deploy_lifespan`, `min_phlo_price` from the Cordial-native config; seeds `fault_tolerance_threshold = 0.333` to match the 2/3 supermajority our finality detector uses; saturates `u64`/`usize` values into `i64`/`u32` rather than panicking.
- `to_snapshot_conf()` — projects the fuller struct into the minimal form `build_snapshot` needs. `max_parent_depth == 0` maps to `None`.

### Phase 3.4 — Crypto bridge (crypto_bridge.rs)

Pluggable hashing / signing / verification traits with f1r3node-compatible implementations.

| Trait | Implementations |
|-------|-----------------|
| `Hasher` | `Sha256Hasher` (blocklace native), `Blake2b256Hasher` (Blake2b with U32 digest, matches f1r3node's `Blake2b256::hash`) |
| `Signer` / `Verifier` | `Ed25519` (blocklace native), `Secp256k1` (k256 ECDSA, f1r3node's primary validator algo) |

`SigAlgorithm` enum carries the wire identifier strings (`"ed25519"`, `"secp256k1"`) f1r3node puts in `BlockMessage.sig_algorithm`. `CryptoError` covers wrong-length keys/signatures and bad curve points without panicking.

`compute_block_hash(&BlockMessage) -> [u8; 32]` produces an f1r3node-style Blake2b hash over a deterministic length-prefixed layout of header + body + sender + sig_algorithm + seq_num + shard_id + extra_bytes. Crucially, **the sender is in the hash input**, which fixes the content-hash collision that `snapshot.rs` would otherwise have (two blocks with identical `BlockContent` but different creators collapse to one entry in snapshot indices). Not byte-for-byte equal to f1r3node's `hash_block()` — that function prost-encodes the header and body, which requires the `models` path dependency. Logical equivalence is sufficient for snapshot correctness.

### Phase 3.1 & 3.2 — Casper trait adapter (casper_adapter.rs)

Two local mirror traits with method signatures identical to f1r3node's — swapping imports is mechanical when the `casper` path dep is enabled.

- **`CordialCasper`**: `get_snapshot`, `contains`, `dag_contains`, `buffer_contains`, `get_approved_block`, `deploy`, `estimator`, `get_version`, `validate`, `validate_self_created`, `handle_valid_block`, `handle_invalid_block`, `get_dependency_free_from_buffer`, `get_all_from_buffer`
- **`CordialMultiParentCasper`**: `last_finalized_block`, `normalized_initial_fault`, `has_pending_deploys_in_storage`

`CordialCasperAdapter` implements both. It owns:
- `tokio::sync::Mutex<Blocklace>` — consensus state
- `tokio::sync::Mutex<DeployPool>` — pending deploys
- `tokio::sync::Mutex<HashMap<BlockHash, BlockMessage>>` — pending-block buffer (blocks waiting on missing predecessors)
- `tokio::sync::Mutex<HashMap<BlockHash, Validator>>` — invalid-block registry
- `bonds`, `shard_conf`, `shard_id`, `approved_block`, `validation_config`

Method mapping:

| f1r3node method | Adapter body |
|-----------------|--------------|
| `get_snapshot` | `build_snapshot` over current state |
| `contains` / `dag_contains` | `Blocklace::dom()` lookup by content_hash (via `try_lock`) |
| `deploy` | Translate to core type, push to `DeployPool`; `PoolError` maps to `DeployError::PoolRejected` / `SignatureVerificationFailed` |
| `estimator` | `fork_choice` tips, content_hashes out |
| `validate` | `message_to_block` + core `validate_block`. `MissingPredecessors` → `BlockError::MissingBlocks`, other core errors map via `map_core_error()` |
| `validate_self_created` | Same, with `check_content_hash` and `check_signature` disabled |
| `handle_valid_block` | Insert into blocklace, then drain buffer of entries whose preds now exist |
| `handle_invalid_block` | Record in `invalid_blocks`, do NOT insert |
| `last_finalized_block` | `find_last_finalized` + `block_to_message` |
| `normalized_initial_fault` | `equivocator_stake / total_stake` |

`RuntimeManager`, `block_store`, and `get_history_exporter` from f1r3node's trait are intentionally omitted — those belong to the deferred RSpace adapter crate.

---

## Real RSpace Runtime (crates/blocklace-f1r3rspace/)

Phase 3 extension. The previous adapter crate (`blocklace-f1r3node`) stays free of f1r3node's real crates by using mirror types. When you want blocks to actually execute Rholang against a real RSpace tuplespace, you add `blocklace-f1r3rspace` to your build.

**This is the only crate in the workspace that pulls in f1r3node's heavy dependencies.** The core `blocklace` and `blocklace-f1r3node` crates stay lightweight regardless.

### Build requirements

Because `blocklace-f1r3rspace` path-depends on f1r3node's `casper`, `models`, `rholang`, `rspace_plus_plus`, `crypto`, and `shared`, the workspace now inherits f1r3node's build requirements:

- **`protoc` on PATH** — `models/build.rs` invokes `tonic_prost_build` to compile `.proto` files.
- **`.cargo/config.toml` at the workspace root** mirrors f1r3node's:
  - `RUST_MIN_STACK = "8388608"` — Rholang's deep async recursion overflows the default 2 MB stack in debug test builds.
  - `rustflags = ["-C", "target-cpu=native"]` and `target-feature=+aes,+sse2` — gxhash (transitive f1r3node dep) requires AES+SSE2 CPU intrinsics.
  - `[future-incompat-report] frequency = "never"` — silences the HOCON warning f1r3node has no workaround for.
- **f1r3node checked out at `../../../f1r3node`** relative to `crates/blocklace-f1r3rspace/Cargo.toml` — the path deps expect it at that location.

Clean build ≈ 5 minutes the first time (compiles f1r3node's full Rholang interpreter and RSpace). Incremental builds are fast.

### Design: A-lite — caller supplies the RuntimeManager

Constructing a real RSpace requires LMDB storage, Rholang interpreter setup, history repository initialization, and bond/genesis bootstrapping — f1r3node's node-binary-sized setup. This crate **does not duplicate that**. Instead it wraps a `&mut f1r3node::RuntimeManager` supplied by the caller (f1r3node's node binary, or a dedicated integration harness), and handles the translation from our `ExecutionRequest` / `ExecutionResult` to f1r3node's types.

```rust
use blocklace::execution::{ExecutionRequest, RuntimeManager as _};
use blocklace_f1r3rspace::F1r3RspaceRuntime;

let mut f1r3_rt: casper::rust::util::rholang::runtime_manager::RuntimeManager =
    /* caller constructs */;
let mut adapter = F1r3RspaceRuntime::new(&mut f1r3_rt);
let result = adapter.execute_block(request)?;
```

### Translation surface

Five public helper functions, each unit-tested:

| Helper | Direction | Notes |
|--------|-----------|-------|
| `signed_deploy_to_f1r3node` | `SignedDeploy` → `Signed<DeployData>` | Constructs `Signed` directly via public fields. Hardcodes `sig_algorithm = Secp256k1` — see caveat below |
| `system_deploy_to_f1r3node` | `SystemDeployRequest` → `SystemDeployEnum` | Slash + CloseBlock mapped to f1r3node's struct variants. `initial_rand` derived from pre-state hash |
| `build_block_data` | `ExecutionRequest` → `BlockData` | Sender picked from first bond; zero-pubkey fallback if bonds is empty. `u64 → i64` overflows surface as `RuntimeError` |
| `processed_deploy_from_f1r3node` | `ProcessedDeploy` (f1r3node) → `ProcessedDeploy` (core) | Cost + is_failed + term + signature preserved |
| `system_deploy_from_f1r3node` | `ProcessedSystemDeploy` (f1r3node) → core equivalent | Succeeded variants mapped to `Slash` or `CloseBlock`; `Failed` surfaces as `CloseBlock { succeeded: false }` (documented info loss) |

### execute_block flow

1. Translate `ExecutionRequest` → f1r3node inputs (terms, system_deploys, block_data, invalid_blocks)
2. Call `f1r3_rt.compute_state(...)` — async, blocked on the current Tokio handle
3. Translate f1r3node outputs (`StateHash`, `Vec<ProcessedDeploy>`, `Vec<ProcessedSystemDeploy>`) back into `ExecutionResult`

### Known caveats

- **Signature algorithm mismatch.** f1r3node's `SignaturesAlgFactory` explicitly disables ed25519, registering only `secp256k1` and `secp256k1-eth`. The adapter hardcodes `Secp256k1` as the algorithm for every translated deploy. Ed25519-signed deploys (our core-crate default) will round-trip by shape but will not verify on f1r3node's side. Adapter callers need to feed secp256k1-signed deploys for verification to pass downstream.
- **`new_bonds` is unchanged.** `execute_block` returns the request's bonds verbatim. Bonds in f1r3node are addressable by post-state hash and read via `RuntimeManager::compute_bonds(post_hash)`. Adding that call is follow-up work.
- **Slash uses validator bytes as the `invalid_block_hash` placeholder.** Real slashes need the actual invalid-block hash; our `SystemDeployRequest::Slash` only carries a validator NodeId. Callers needing tighter semantics should construct a richer request type.
- **No end-to-end tests** in this crate. Unit tests cover pure translation; exercising `execute_block` against a real RuntimeManager requires LMDB + Rholang bootstrap. Belongs in a Phase 4 integration harness.

---

## Test Coverage (252 tests)

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
| `test_runtime.rs` | 17 | Runtime trait, MockRuntime determinism, cost, slash, state chaining |

### `crates/blocklace-f1r3node/tests/` — 80 tests

| Test File | Count | What it covers |
|-----------|-------|----------------|
| `test_block_translation.rs` | 13 | Block ↔ BlockMessage roundtrip, parents/justifications union, numeric overflow, deterministic ordering |
| `test_snapshot.rs` | 16 | dag_set / height_map / child_map / latest_messages, finality, tips, parents, justifications, deploys_in_scope, equivocator exclusion |
| `test_shard_conf.rs` | 8 | Full f1r3node defaults parity, from_cordial import, saturating casts, to_snapshot_conf projection |
| `test_crypto_bridge.rs` | 22 | SHA-256 and Blake2b-256 known vectors, ED25519 and Secp256k1 roundtrip + tamper detection, compute_block_hash determinism and sender-differentiation |
| `test_casper_adapter.rs` | 21 | Adapter construction, contains / dag_contains / buffer, deploy acceptance + rejection, estimator, get_snapshot, validate (accept / missing / invalid sender), handle_valid / handle_invalid, last_finalized, normalized_initial_fault |

### `crates/blocklace-f1r3rspace/tests/` — 13 tests

| Test File | Count | What it covers |
|-----------|-------|----------------|
| `test_translation.rs` | 13 | signed_deploy_to_f1r3node (signature preserved, non-UTF-8 term handling), system_deploy_to_f1r3node (Slash + Close), build_block_data (sender from bonds, overflow error), processed_deploy round-trip, system_deploy_from_f1r3node (Succeeded / Failed / Empty variants). No `execute_block` tests here — see caveats above |

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

### Phase 2: Execution Layer Bridge

| Task | Status |
|------|--------|
| 2.1 Typed payload (`CordialBlockPayload`) | Complete |
| 2.2 Deploy pool and selection | Complete |
| 2.3 RSpace runtime integration | Complete (trait + `MockRuntime`; real RSpace adapter deferred to a separate crate) |
| 2.4 System deploy support | Covered by 2.3 mock (Slash / CloseBlock); real impl with RSpace adapter |

### Phase 3: f1r3node Integration (branch: `phase3/f1r3node-integration`, merged)

| Task | Status |
|------|--------|
| 3.1 `Casper` trait adapter | Complete |
| 3.2 `MultiParentCasper` trait adapter | Complete |
| 3.3 `CasperSnapshot` construction | Complete |
| 3.4 Cryptographic alignment (Blake2b, Secp256k1) | Complete |
| 3.5 Block format translation (`Block` ↔ `BlockMessage`) | Complete |
| 3.6 `CasperShardConf` equivalent | Complete |

### Phase 3 extension: RSpace runtime (branch: `phase3-extension/rspace-adapter`)

| Task | Status |
|------|--------|
| Third workspace crate scaffolding + f1r3node path deps | Complete |
| `F1r3RspaceRuntime` adapter translating `RuntimeManager` calls | Complete (compiles; unit tests on translation only) |
| End-to-end Rholang execution test (needs LMDB + Rholang bootstrap) | Not started — Phase 4 integration harness |

## What Is Not Yet Implemented

**Optimizations** (needed for production scale):
- Indexed storage: child map, height map, latest messages index for O(1) lookups
- Persistent storage (LMDB or similar) to replace in-memory HashMap

**Protocol gaps**:
- Pending-block retry in the network layer: blocks rejected for missing predecessors are dropped; no automatic re-attempt after predecessors arrive (the adapter's buffer does retry; the `network::Node` layer still drops)
- Transitive sync: sync discovers missing block ids but doesn't recursively fetch their predecessors

**f1r3node integration — deferred follow-ups**:
- Enabling the `models` / `casper` / `block_storage` path dependencies in `blocklace-f1r3node/Cargo.toml`; at that point the local mirror types get swapped for `use models::...` / `use casper::...` imports. (Note: the `blocklace-f1r3rspace` crate already path-depends on these for the RSpace adapter; enabling them in `blocklace-f1r3node` too is consolidation work.)
- Byte-for-byte block-hash parity via prost-encoded header/body (our `compute_block_hash` is logically equivalent but not byte-for-byte compatible with f1r3node's `hash_block`)
- RSpace-coupled `MultiParentCasper` methods (`runtime_manager`, `block_store`, `get_history_exporter`) — live with `blocklace-f1r3rspace`
- Time-based deploy expiration (`Option<expiration_timestamp>` on `Deploy`)

**RSpace adapter follow-ups** (in `blocklace-f1r3rspace`):
- `F1r3RspaceRuntime::execute_block` should call `RuntimeManager::compute_bonds(post_hash)` and return updated bonds rather than echoing the caller's input
- Richer `SystemDeployRequest::Slash` carrying the invalid-block hash, not just the validator id
- Secp256k1 test-key setup so signed deploys can be end-to-end verified through f1r3node's signature path
- Integration test harness that brings up a real `RuntimeManager` (LMDB + Rholang bootstrap) and exercises `execute_block` end-to-end
