# Integration Next Steps

A practical guide for contributors picking up the f1r3node integration work. Read this **after** [`implementation.md`](implementation.md) (which describes what exists) and [`cordial-miners-vs-cbc-casper.md`](cordial-miners-vs-cbc-casper.md) (which compares the two protocols).

Each task below has a clear scope, a concrete starting file, the rationale for *why* it matters, and a difficulty estimate. They're ordered roughly by "what unblocks the most downstream work first," but you can pick any of them — they're independent unless noted.

---

## How the integration stack is layered

If you're new to the repo, this is the mental model:

```
┌───────────────────────────────────────────────────────┐
│  blocklace-f1r3rspace                                 │  ← real Rholang execution
│  Wraps f1r3node's RuntimeManager.                     │     (delegates to f1r3node)
│  Path-depends on f1r3node's casper, models, rholang.  │
└───────────────────────────────────────────────────────┘
              ▲
              │ implements blocklace::execution::RuntimeManager
              │
┌───────────────────────────────────────────────────────┐
│  blocklace-f1r3node                                   │  ← Casper trait adapter
│  Mirror types of f1r3node's BlockMessage,             │     (no f1r3node dep)
│  CasperSnapshot, CasperShardConf. Translation +       │
│  CordialCasperAdapter.                                │
└───────────────────────────────────────────────────────┘
              ▲
              │ uses
              │
┌───────────────────────────────────────────────────────┐
│  blocklace                                            │  ← pure consensus core
│  Block, Blocklace, fork choice, finality, validation, │     (no f1r3node, no RSpace)
│  deploy pool, RuntimeManager trait + MockRuntime,     │
│  P2P networking. SHA-256 + ED25519.                   │
└───────────────────────────────────────────────────────┘
```

The split exists so consumers who only want consensus pay nothing for f1r3node integration, and consumers who want integration pay only for what they use. Keep this layering in mind when contributing — **do not let RSpace types leak into `blocklace`, and do not let f1r3node real crates leak into `blocklace-f1r3node`.**

---

## Task 1 — End-to-end Rholang execution test

**Status:** Not started. Highest-value next step.

**Why it matters.** Today our `blocklace-f1r3rspace` crate compiles against f1r3node's real types, but we have no test that actually runs a Rholang deploy through `F1r3RspaceRuntime::execute_block` and verifies the post-state hash changes. Until we do this, we don't *truly* know the integration works — we only know it type-checks.

**What to build.** A new test file `crates/blocklace-f1r3rspace/tests/test_e2e_execution.rs` that:

1. Builds a real `casper::rust::util::rholang::runtime_manager::RuntimeManager` with an in-memory or `tempdir`-backed LMDB store. f1r3node's own test helpers do this — search their codebase for `RuntimeManager::create_with_store` or `create_with_history` for the right factory.
2. Calls `runtime_manager.compute_genesis(...)` to initialize the tuplespace at the empty state hash.
3. Constructs a Cordial Miners `ExecutionRequest` with one trivial Rholang deploy (`"@0!(\"hello\")"` or similar).
4. Wraps the `RuntimeManager` in `F1r3RspaceRuntime::new(&mut rt)` and calls `execute_block`.
5. Asserts:
   - `post_state_hash != pre_state_hash`
   - `processed_deploys.len() == 1`
   - `processed_deploys[0].is_failed == false`
   - `processed_deploys[0].cost > 0`

**Difficulty:** Medium. The translation code is done; the work is figuring out f1r3node's RuntimeManager bootstrap. Expect to spend a few hours reading f1r3node test helpers (look in `casper/tests/` for examples).

**Files to read first:**
- f1r3node `casper/src/rust/util/rholang/runtime_manager.rs` — search for `create_with_store`, `create_with_space`, or `create_with_history`
- f1r3node `casper/tests/` — find a working test that constructs a RuntimeManager
- Our `crates/blocklace-f1r3rspace/src/lib.rs` — see what `F1r3RspaceRuntime` expects

**Acceptance criteria:** One green test that proves Rholang executes end-to-end through our adapter.

---

## Task 2 — Wire up `compute_bonds` so `new_bonds` reflects post-state

**Status:** Documented as a follow-up; placeholder in code.

**Why it matters.** `F1r3RspaceRuntime::execute_block` currently returns the caller's input bonds verbatim in `ExecutionResult.new_bonds`. That's wrong: bonds in f1r3node are *state-hash-addressable*, meaning they live inside the tuplespace and change when system deploys (slash, register validator) execute. After running deploys, the *correct* bonds may differ from the pre-deploy bonds.

**What to build.** In `crates/blocklace-f1r3rspace/src/lib.rs`, after the `compute_state` call returns the post-state hash, call `self.f1r3_rt.compute_bonds(&post_hash).await?` and translate the resulting `Vec<f1r3node::Bond>` into our `Vec<blocklace::execution::Bond>`. Replace the current line:

```rust
new_bonds: request.bonds.clone(), // unchanged; see module docs
```

with the translated result.

**Difficulty:** Small. The method exists on `RuntimeManager` (line 729 in `runtime_manager.rs`); only translation glue is needed. There's a `bond_to_blocklace` helper to write — straightforward `validator: NodeId(b.validator.to_vec()), stake: b.stake as u64`.

**Test it.** Once Task 1 lands, extend it: insert a `SystemDeployRequest::Slash` for a known validator and assert that validator no longer appears in `result.new_bonds`.

---

## Task 3 — Richer `SystemDeployRequest::Slash` carrying invalid-block hash

**Status:** API limitation, documented.

**Why it matters.** Our `SystemDeployRequest::Slash { validator: NodeId }` only names the validator being slashed. f1r3node's real `SlashDeploy` requires *both* the validator's public key *and* the hash of the block that's being slashed for (the equivocating block). The current adapter uses the validator bytes as a placeholder for the block hash, which is wrong but at least doesn't panic.

**What to build.** Extend `SystemDeployRequest::Slash` in `crates/blocklace/src/execution/runtime.rs`:

```rust
pub enum SystemDeployRequest {
    Slash {
        validator: NodeId,
        invalid_block_hash: Vec<u8>,  // NEW
    },
    CloseBlock,
}
```

Then update `F1r3RspaceRuntime::system_deploy_to_f1r3node` to pass the real `invalid_block_hash`. Update `MockRuntime` to carry the new field through (it doesn't *use* it, just preserves it). Update tests.

**Difficulty:** Small. Touches three files: the enum definition, the mock runtime, the adapter. Plus test fixture updates.

**Caveat.** This is a breaking change to a public type. Coordinate with anyone consuming `SystemDeployRequest` directly (right now: `MockRuntime` and `F1r3RspaceRuntime`).

---

## Task 4 — Secp256k1-signed deploy test fixtures

**Status:** Adapter compiles but ed25519-signed deploys won't verify on f1r3node's side.

**Why it matters.** f1r3node's `SignaturesAlgFactory` registers only `secp256k1` and `secp256k1-eth` for deploy signing. ED25519 is *explicitly disabled*. Our `blocklace` core defaults to ED25519 keys for everything. As a result, deploys constructed in our test code today round-trip *by shape* through `F1r3RspaceRuntime` but would fail signature verification if any downstream f1r3node code re-checks them.

**What to build.** A small helper in `crates/blocklace-f1r3rspace/tests/common/` (or a `mod.rs`):

1. `fn secp256k1_keypair() -> (PrivateKey, PublicKey)` — generates a real k256 key.
2. `fn signed_deploy_secp256k1(sk: &PrivateKey, term: &str, ...) -> SignedDeploy` — uses our `Secp256k1` signer (already in `blocklace-f1r3node::crypto_bridge`) to produce a real signature.
3. Use this helper in any e2e test that needs deploys to actually verify.

**Difficulty:** Small. The signer already exists; you're writing a 30-line helper.

**Acceptance criteria:** A deploy constructed through this helper passes f1r3node's `Signed::from_signed_data` verification (which is what `compute_state` calls internally for some code paths).

---

## Task 5 — Consolidate mirror types: enable `models` path dep in `blocklace-f1r3node`

**Status:** Currently kept separate to keep `blocklace-f1r3node` lightweight (mirror-types-only, no f1r3node real dependency).

**Why it matters.** Today `blocklace-f1r3node` defines its own `BlockMessage`, `Header`, `Body`, etc. as plain Rust structs that look like f1r3node's. `blocklace-f1r3rspace` already path-depends on f1r3node's real `models` crate. So we have two parallel type universes. Consolidating means deleting the mirror types and importing from `models::*` directly.

**Why it's not done yet.** It would force every consumer of `blocklace-f1r3node` to also pull in f1r3node's heavy dependency tree (LMDB, prost, gRPC) even if they don't want RSpace integration. That's a real cost.

**Trade-off.** Three honest options:

- **(a) Leave it as-is.** Two type universes, but `blocklace-f1r3node` stays cheap. Cost: consumers translating to/from real f1r3node types twice (once in our adapter, once in their code).
- **(b) Feature-gate.** Add `#[cfg(feature = "real-models")]` blocks in `blocklace-f1r3node` so users can opt into real types. Mirror types are the default. Cost: more `#[cfg]` clutter; both code paths must stay in sync.
- **(c) Move `blocklace-f1r3node` to depend on `models` unconditionally.** Simpler code, but forces all integration users into the heavy tree. Path of least resistance if no one's actually using `blocklace-f1r3node` standalone.

**Difficulty:** Medium for (b), Small for (c). Requires a design discussion with maintainers before doing the work.

**Recommendation.** Don't do this until someone actually needs it. The duplication cost is documentation only right now.

---

## Task 6 — Byte-for-byte block hash parity

**Status:** Our `compute_block_hash` is logically equivalent but not byte-identical to f1r3node's `hash_block`.

**Why it matters.** f1r3node's `hash_block` (in `casper/src/rust/util/proto_util.rs:391`) computes Blake2b-256 over the *protobuf-encoded* header and body bytes. Ours hashes a deterministic length-prefixed layout of the mirror struct fields. The result is the same conceptually (sender + content + shard all contribute) but the bytes differ. This means a block hash we compute in `blocklace-f1r3node::compute_block_hash` won't match what f1r3node's machinery would compute for the same logical block.

**Where it bites.** Anywhere downstream code expects to compare hashes between our adapter output and a hash f1r3node already computed. For now, no such code path exists in our integration — we always re-derive hashes on each side.

**What to build.** When/if we enable the `models` path dep in `blocklace-f1r3node` (Task 5), `compute_block_hash` can call `header.to_proto().encode_to_vec()` and `body.to_proto().encode_to_vec()` directly, matching f1r3node's layout exactly. Add a test that pins our output to f1r3node's `hash_block` for a known fixture.

**Difficulty:** Small once Task 5 is done; Medium otherwise (mirror prost serialization manually, brittle).

---

## Task 7 — Replace mirror types with real types in `blocklace-f1r3node`

**Status:** Same as Task 5, different angle.

**Why it matters.** When the team commits to depending on `models`, the `CasperSnapshot`, `BlockMessage`, etc. types defined in `blocklace-f1r3node/src/block_translation.rs` and `snapshot.rs` should be removed in favour of `use models::...`. The translation function bodies stay the same; only type imports change.

**Difficulty:** Mechanical but tedious. Requires running `cargo check -p blocklace-f1r3node` after each removal to find compile errors.

**Sequencing.** Do Task 5 first (the design decision), then this.

---

## Task 8 — RSpace-coupled `MultiParentCasper` methods

**Status:** Three methods intentionally omitted from `CordialCasperAdapter`: `runtime_manager()`, `block_store()`, `get_history_exporter()`.

**Why it matters.** f1r3node's `MultiParentCasper` trait requires these. Omitting them means our adapter doesn't fully implement the trait — anyone trying to substitute `CordialCasperAdapter` for `MultiParentCasperImpl` in real f1r3node code today would hit "missing method" errors.

**What to build.** A new struct in `blocklace-f1r3rspace` (not in `blocklace-f1r3node`) that wraps `CordialCasperAdapter` *and* a `RuntimeManager` reference and a `KeyValueBlockStore` reference, then implements the full `MultiParentCasper` trait by delegating: our consensus methods to the inner adapter, the storage/runtime methods to the wrapped f1r3node objects.

**Sketch:**

```rust
pub struct CordialMultiParentCasperFull {
    inner: CordialCasperAdapter,
    runtime: Arc<tokio::sync::Mutex<RuntimeManager>>,
    block_store: Arc<KeyValueBlockStore>,
    history_exporter: Arc<dyn RSpaceExporter>,
}

#[async_trait]
impl MultiParentCasper for CordialMultiParentCasperFull {
    // delegate methods 1:1
}
```

**Difficulty:** Medium. Glue code, no algorithmic content. Requires Task 1 first (knowing how to actually construct these things).

---

## Task 9 — Time-based deploy expiration

**Status:** `Option<expiration_timestamp>` is missing from our `Deploy` type.

**Why it matters.** f1r3node's `DeployData` carries `expiration_timestamp: Option<i64>` — deploys can specify a wall-clock-time deadline beyond which they're rejected. Our `Deploy` struct in `crates/blocklace/src/execution/payload.rs` only has `valid_after_block_number` (block-height window). Block-height expiration is the harder guarantee (deterministic across nodes), but timestamp expiration is what users actually request from RPC clients.

**What to build.**

1. Add `pub expiration_timestamp: Option<u64>` to `blocklace::execution::Deploy`.
2. Update `DeployPool::is_block_expired` (or add a sibling `is_time_expired`) to check it.
3. Wire `current_time_millis` through `select_for_block` and `prune_expired` (already accepted as a parameter, currently unused).
4. Update tests in `test_deploy_pool.rs` to cover timestamp expiration.

**Difficulty:** Small. The pool already accepts `current_time_millis` parameters that go nowhere — the plumbing exists, only the field and the comparison are missing.

---

## Task 10 — Phase 4: Production hardening

Beyond the integration work, the existing roadmap (in `cordial-miners-vs-cbc-casper.md` §6) names Phase 4: Production Hardening. The relevant items for integration consumers:

- **4.1 Persistent LMDB storage** — replace the in-memory `HashMap<BlockIdentity, BlockContent>` in `Blocklace` with an LMDB-backed store. f1r3node already does this for their DAG; we'd want a `BlocklaceStore` trait in `crates/blocklace/src/storage/` with both an in-memory and an LMDB implementation.
- **4.2 DAG pruning and garbage collection** — when blocks are finalized far enough back, prune them. Critical for long-running chains.
- **4.3 Network integration** — bridge our `network::Node` to f1r3node's `TransportLayer`, or replace it entirely with f1r3node's gRPC transport.
- **4.4 Performance benchmarks** — end-to-end finality latency on N-validator setups, throughput under deploy load.
- **4.5 Adversarial testing** — partitions, equivocators, non-cordial rounds.

Each of these is a 1-2 week chunk and a separate PR.

---

## Build environment quick reference

If `cargo build` fails on a fresh checkout:

| Symptom | Fix |
|---------|-----|
| `tonic_prost_build` errors about `protoc` not found | Install `protoc`: `sudo apt install protobuf-compiler` (Linux), `brew install protobuf` (Mac) |
| `gxhash` errors about AES/SSE2 intrinsics | Make sure `.cargo/config.toml` exists at the repo root with `target-feature=+aes,+sse2` |
| Stack overflow in Rholang tests | `RUST_MIN_STACK=8388608` is set via `.cargo/config.toml` `[env]` section. Verify it's there. |
| `models` path dep "file not found" | f1r3node must be checked out at `../../../f1r3node` relative to `crates/blocklace-f1r3rspace/Cargo.toml`. Adjust the path in that crate's Cargo.toml if your layout differs. |
| First build takes 5+ minutes | Expected. f1r3node's casper, rholang, and rspace crates are large. Incremental builds are fast. |

---

## Where to ask for help

- **Implementation questions** — open a GitHub Discussion with the `integration` label.
- **f1r3node API questions** — read the f1r3node source first; their codebase is well-commented. If still stuck, the file paths in their `casper/src/rust/util/rholang/` directory are the most relevant for runtime work.
- **Design decisions** — if a task above says "coordinate with maintainers," open an RFC issue rather than just merging the change.

---

## How to claim a task

1. Open a GitHub issue titled "Task N: <description>" and link this document.
2. Comment on the issue saying you're picking it up.
3. Branch off `master` with a name like `task1/e2e-rholang-test` or similar.
4. Open a PR referencing the issue when you're ready.
5. The integration work is in two layers — make sure your PR doesn't accidentally bleed RSpace types into `blocklace`, or f1r3node real types into `blocklace-f1r3node`. If you have to, ask first.

Good luck. Most of these tasks are well-scoped — none should take more than a few days for someone familiar with Rust async and the f1r3node codebase.
