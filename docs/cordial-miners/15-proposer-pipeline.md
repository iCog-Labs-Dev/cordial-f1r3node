# 15. Proposer Pipeline: Blocklace → Mempool → RSpace → Sign → Broadcast

**Implementation**: `crates/cordial-f1r3node-adapter/src/proposer.rs`  
**Module**: `cordial_f1r3node_adapter::proposer`  
**Tests**: `crates/cordial-f1r3node-adapter/tests/test_proposer.rs`  
**Purpose**: Outbound block creation — the mirror of the inbound [gRPC ingestion layer](./13-grpc-ingestion.md)

---

## What problem does this solve?

Receiving blocks is only half of a validator's job. A Cordial Miners node must also **create** blocks: pick parents from the DAG, include pending deploys, execute them against RSpace to obtain a post-state hash, sign the result, and broadcast it to peers.

The proposer module implements that outbound pipeline behind small, swappable traits so tests can mock RSpace and the network without pulling in f1r3node's full node binary.

---

## Architecture overview

```
Local Blocklace + bonds
        ↓
┌─────────────────────────────────────┐
│         CordialProposer             │
│                                     │
│  (a) TipSelector::select_tips       │  ← select_predecessors()
│  (b) DeployPool::select_for_block   │  ← mempool + ancestor dedup
│  (c) ExecutionEngine::execute       │  ← RuntimeManager (RSpace / mock)
│  (d) BlockSigner::sign_block        │  ← Blake2b hash + Secp256k1
│  (e) BlockBroadcaster::broadcast    │  ← P2P / test recorder
└─────────────────────────────────────┘
        ↓
  Signed Block (Block + CordialBlockPayload in payload bytes)
        ↓
  Peers / local blocklace (after ingest + verify)
```

Inbound vs outbound:

| Direction | Module | Input | Output |
|-----------|--------|-------|--------|
| Inbound | `grpc_ingest` | `BlockMessage` (wire) | `Block` in blocklace |
| Outbound | `proposer` | blocklace + deploy pool | signed `Block` on the network |

---

## Sequential pipeline (steps a–e)

### (a) Tip selection — `TipSelector`

**Default**: [`DisseminationTipSelector`](../../crates/cordial-f1r3node-adapter/src/proposer.rs) delegates to [`select_predecessors`](../../crates/cordial-miners-core/src/consensus/dissemination.rs).

Returns the set of **live honest validator tips** (excludes equivocators). This satisfies the Cordial Miners dissemination rule: a new block points at all visible honest tips.

**Genesis**: On an empty blocklace, tip selection returns an empty predecessor set and the proposer builds block number `0` with `pre_state_hash = []`.

**Failure**: If the blocklace is non-empty but no honest tips exist (`ProposeError::NoTips`), proposal aborts.

### (b) Mempool — `DeployPool`

The proposer does not own the pool; callers pass `&DeployPool` into [`CordialProposer::propose`](../../crates/cordial-f1r3node-adapter/src/proposer.rs).

1. [`compute_deploys_in_scope`](../../crates/cordial-miners-core/src/execution/deploy_pool.rs) walks predecessor ancestry to build a signature dedup set.
2. [`select_for_block`](../../crates/cordial-miners-core/src/execution/deploy_pool.rs) applies validity filters and the oldest-plus-newest cap.

### (c) Execution — `ExecutionEngine`

**Default**: [`RuntimeExecutionEngine<R>`](../../crates/cordial-f1r3node-adapter/src/proposer.rs) wraps any [`RuntimeManager`](../../crates/cordial-miners-core/src/execution/runtime.rs).

| Environment | Implementation |
|-------------|----------------|
| Unit tests | [`MockRuntime`](../../crates/cordial-miners-core/src/execution/runtime.rs) — deterministic post-state hashes |
| Production | [`F1r3RspaceRuntime`](../../crates/cordial-f1r3space-adapter/src/lib.rs) — delegates to f1r3node `compute_state` |

**Chain head derivation** (before execution):

From the selected predecessor tips, decode each `CordialBlockPayload` and pick the tip with the **highest** `state.block_number`. Use its `post_state_hash` as the next block's `pre_state_hash` and `block_number + 1` as the new block number.

> **Multi-parent caveat**: True multi-parent state merge is performed inside f1r3node's `RuntimeManager` when using real RSpace. This proposer uses the highest-numbered tip as the execution parent, which is sufficient for `MockRuntime` strict chaining and matches the current integration scope.

**System deploys**: By default the proposer includes `SystemDeployRequest::CloseBlock` (f1r3node block sealing). Disable with `CordialProposer::with_close_block(false)` in tests.

### (d) Signing — `BlockSigner`

**Default**: [`Secp256k1BlockSigner`](../../crates/cordial-f1r3node-adapter/src/proposer.rs)

1. Build `CordialBlockPayload` from `ExecutionResult` (pre/post state, deploys, bonds).
2. Serialize into `BlockContent.payload` via bincode.
3. `content_hash = hash_content(content)` (Blake2b-256, f1r3node-aligned).
4. Sign with Secp256k1 DER over the content hash.

Verification on receive uses [`F1r3flyCryptoAdapter`](./14-crypto-adapter.md) implementing [`CryptoVerifier`](../../crates/cordial-miners-core/src/crypto.rs).

### (e) Broadcast — `BlockBroadcaster`

**Production**: Wire to `network::Node::create_block` or f1r3node's gossip layer (future integration).

**Tests**: [`RecordingBroadcaster`](../../crates/cordial-f1r3node-adapter/src/proposer.rs) collects blocks in a `Vec` for assertions.

---

## Trait reference

| Trait | Responsibility | Default impl | Test double |
|-------|----------------|--------------|-------------|
| `TipSelector` | Parent IDs from blocklace | `DisseminationTipSelector` | Custom closure / fixed set |
| `ExecutionEngine` | Run deploys → `post_state_hash` | `RuntimeExecutionEngine<R>` | `MockRuntime` |
| `BlockSigner` | `BlockIdentity` from content | `Secp256k1BlockSigner` | Test key + real crypto |
| `BlockBroadcaster` | Publish signed block | `FnBroadcaster` / `RecordingBroadcaster` | In-memory `Vec<Block>` |

---

## Types

There is no separate `CordialBlock` struct. A proposed block is:

- [`Block`](../../crates/cordial-miners-core/src/block.rs) — consensus envelope (`BlockIdentity` + `BlockContent`)
- [`CordialBlockPayload`](../../crates/cordial-miners-core/src/execution/payload.rs) — execution body inside `BlockContent.payload`

---

## Usage sketch

```rust
use std::collections::HashMap;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::execution::{DeployPool, DeployPoolConfig, MockRuntime};
use cordial_miners_core::types::NodeId;
use cordial_f1r3node_adapter::proposer::{
    CordialProposer, DisseminationTipSelector, RecordingBroadcaster,
    RuntimeExecutionEngine, Secp256k1BlockSigner,
};

let mut proposer = CordialProposer::new(
    DisseminationTipSelector,
    RuntimeExecutionEngine::new(MockRuntime::new()),
    Secp256k1BlockSigner::new(private_key),
    RecordingBroadcaster::new(),
    NodeId(public_key),
    bonds,
    DeployPoolConfig::default(),
);

let block = proposer.propose(&blocklace, &deploy_pool)?;
```

---

## Tests (acceptance criteria)

| Test | What it proves |
|------|----------------|
| `proposer_selects_live_tips_from_blocklace` | `block.content.predecessors == select_predecessors(...)` |
| `proposer_packages_post_state_hash_from_execution` | Payload `post_state_hash` matches direct `MockRuntime::execute_block` |
| `proposed_block_passes_f1r3fly_crypto_verifier` | `F1r3flyCryptoAdapter::verify_block` OK + `blocklace.insert` accepts the block |

Run:

```bash
cargo +nightly-2025-06-15 test -p cordial-f1r3node-adapter proposer
```

Requires the workspace to resolve (sibling `f1r3node` checkout for adapter path dependencies).

---

## Related documentation

- [13 — gRPC Ingestion](./13-grpc-ingestion.md) — inbound path
- [14 — Crypto Adapter](./14-crypto-adapter.md) — signature verification after propose/ingest
- [Dissemination (`select_predecessors`)](../../crates/cordial-miners-core/src/consensus/dissemination.rs) — tip selection rules
- [INTEGRATION_NEXT_STEPS.md](../INTEGRATION_NEXT_STEPS.md) — wiring proposer into f1r3node's `BlockCreator` (future)

---

## Out of scope (this module)

- f1r3node `casper::Proposer` / `BlockCreator` wiring
- Real `F1r3RspaceRuntime` in proposer unit tests (see e2e task in integration next steps)
- Multi-parent RSpace merge policy beyond highest-`block_number` tip selection
