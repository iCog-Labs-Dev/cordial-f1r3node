# Production Roadmap: Cordial Miners

**Date:** Tue Jul 28 2026  
**Goal:** Move from a live integrated prototype to a production-ready Cordial Miners path that applications can safely build on.

This roadmap is intentionally realistic. It assumes the recent f1r3node integration PRs are merged, including live block mirroring, ordered output export, deploy ingress/proxy work, and four-node ordering convergence tooling.

## Current Baseline

Already working:

- Core blocklace, approval, ratification, super-ratification, finality, and tau ordering.
- Weighted and unweighted consensus paths.
- Live f1r3node block mirroring through gRPC.
- Height bootstrap and live mirror diagnostics.
- Ordered finalized output seam and JSON/file export.
- Shared ordered output reader with append-only prefix checks.
- Deploy ingress observation through external gRPC/HTTP proxy paths.
- Four-node f1r3node Docker cluster support.
- Four-node finalized tau ordering convergence verification.
- LMDB-backed adapter/repository persistence tests.

Still not production-ready:

- Ordered output is not yet consumed by f1r3node proposer/execution.
- The system is still observer/sidecar-first, not a CBC Casper replacement.
- Core `Blocklace` is still in-memory.
- Property-based safety testing and benchmarks are missing.
- Observability and operations are not yet mature.

## Phase 1: Safety and Invariant Hardening

This phase turns the consensus implementation from “tested by examples” into “checked against protocol invariants.”

### 1.1 Property-Based Consensus Tests

**Target files:**

- `crates/cordial-miners-core/tests/prop_tau.rs` (new)
- `crates/cordial-miners-core/tests/prop_finality.rs` (new)
- `crates/cordial-miners-core/tests/prop_equivocation.rs` (new)

**Work:**

- Add generated DAG tests for tau prefix preservation.
- Check deterministic `xsort` output.
- Verify closure/ancestor consistency.
- Verify finalized ordered output excludes equivocating branches.
- Verify weighted ratification thresholds are respected.

**Acceptance criteria:**

- Property tests run in CI with bounded case counts.
- A failing generated case prints enough DAG context to reproduce.

### 1.2 Adversarial Simulation Tests

**Target files:**

- `crates/cordial-miners-core/tests/test_adversarial_simulation.rs` (new)
- `crates/cordial-miners-core/src/simulation/`

**Work:**

- Simulate delayed blocks and reverse delivery.
- Simulate validator equivocation.
- Simulate temporary partitions and healing.
- Verify convergence after delivery resumes.

**Acceptance criteria:**

- Tests cover the fault bound expected by the ES configuration.
- Safety properties hold even when liveness is delayed.

### 1.3 Explicit Equivocator Excommunication Mode

**Target files:**

- `crates/cordial-miners-core/src/consensus/dissemination.rs`
- `crates/cordial-miners-core/src/consensus/mod.rs`
- `crates/cordial-f1r3node-adapter/src/proposer.rs`
- `crates/cordial-miners-core/tests/test_dissemination.rs`

**Work:**

- Added `PredecessorSelectionMode` enum with two variants:
  - `Compatibility` (default) — re-adds known equivocator branches not yet transitively
    visible through the current honest tip set. Preserves inter-node compatibility.
    Required for adapter / snapshot paths.
  - `Strict` — paper-native excommunication (Cordial Miners §6.1): a proposer never
    places a direct pointer to an equivocator branch. Relies solely on transitive
    acknowledgement through honest tips.
- All existing public functions (`select_predecessors`, `next_block_predecessors`,
  `build_block_candidate`, `select_predecessors_sorted`) delegate to `Compatibility`
  mode — no existing call-site changes required.
- Added new mode-aware entry-points:
  - `select_predecessors_with_mode` / `select_predecessors_strict`
  - `select_predecessors_sorted_with_mode`
  - `next_block_predecessors_with_mode`
  - `build_block_candidate_with_mode`
- Added `StrictTipSelector` in the adapter alongside the existing
  `DisseminationTipSelector` (compatibility). `CordialProposer<TS>` accepts either
  with no struct changes — callers swap the type parameter.
- Documented the three-way distinction in module-level doc comments:
  1. Finality / ordering exclusion (always active, in `finality.rs` / `ordering.rs`)
  2. Compatibility predecessor selection (current default)
  3. Strict predecessor-selection excommunication (paper-native, opt-in)

**Acceptance criteria:**

- Tests show finality/order exclusion remains unchanged (all pre-existing tests pass).
- Tests show strict predecessor selection avoids known equivocator branches
  (8 new tests, invariants S1–S7 in `test_dissemination.rs`).
- Compatibility mode remains the default for all adapter paths.
- Existing proposer and dissemination tests continue to pass (no API breakage).

## Phase 2: Live Integration Reliability

This phase makes the f1r3node sidecar path repeatable and measurable.

### 2.1 Four-Node Ordering Convergence Hardening

**Target files:**

- `docker/scripts/verify-four-node-cluster-ordering.sh`
- `docs/cordial-miners/integration/13-four-node-ordering-convergence.md`
- `crates/cordial-f1r3node-adapter/src/bin/live_mirror_check.rs`

**Work:**

- Keep common target height selection stable.
- Keep height-window ordering mode documented and tested.
- Make failure output concise and useful.
- Add recommended run commands for fresh clusters and long-running clusters.

**Acceptance criteria:**

- Four real f1r3node validators export the same finalized tau order.
- Divergence reports first mismatch, lengths, and node identity.

### 2.2 Ordered Output Consumer Boundary

**Target files:**

- `docs/cordial-miners/integration/14-ordered-output-consumer-boundary.md`
- `crates/cordial-f1r3node-adapter/src/ordered_output.rs`
- `crates/cordial-f1r3node-adapter/src/shared_ordered_output.rs`

**Work:**

- Define the first downstream consumer contract.
- Keep the contract sidecar-compatible.
- Decide whether the first consumer is a file reader, API reader, or proposer-facing adapter.

**Acceptance criteria:**

- A downstream component can consume ordered output without calling debug harness code.
- Append-only prefix behavior is preserved.

### 2.3 Deploy-to-Ordering Trace

**Target files:**

- `crates/cordial-f1r3node-adapter/src/live_deploy_ingress.rs`
- `crates/cordial-f1r3node-adapter/src/live_deploy_proxy.rs`
- `crates/cordial-f1r3node-adapter/src/live_grpc.rs`
- `docs/cordial-miners/integration/13-deploy-ingress-path.md`

**Work:**

- Correlate observed deploy signatures with later mirrored block inclusion.
- Report whether a deploy was observed, accepted by f1r3node, included in a block, and present in finalized ordered output.

**Acceptance criteria:**

- A demo can submit a valid deploy through the proxy and later show its inclusion path.
- This remains external-gRPC based and does not require f1r3node source changes.

## Phase 3: Performance and Recovery

This phase keeps the system fast and restart-safe as live chains grow.

### 3.1 Benchmark Suite

**Target files:**

- `crates/cordial-miners-core/benches/` (new)
- `crates/cordial-f1r3node-adapter/benches/` (optional new)

**Work:**

- Benchmark final leader lookup.
- Benchmark tau ordering and ordered fragment export.
- Benchmark height bootstrap over large live mirrored histories.
- Benchmark four-node convergence script runtime.

**Acceptance criteria:**

- Benchmarks produce stable local baselines.
- CI can run a lightweight benchmark smoke check.

### 3.2 Persistence and Restart Semantics

**Target files:**

- `crates/cordial-f1r3space-adapter/src/lmdb_store/`
- `crates/cordial-f1r3node-adapter/src/repository.rs`
- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`

**Work:**

- Clarify which state is persisted today: blocks, finalized cursor, ordered output, checkpoints.
- Add recovery tests that replay persisted state into live ingress.
- Decide whether core `Blocklace` needs a native `BlocklaceStore` trait now or later.

**Acceptance criteria:**

- Restart tests demonstrate persisted blocks and finalized cursor survive process restart.
- Documentation clearly distinguishes adapter-level LMDB persistence from core in-memory blocklace.

### 3.3 Pruning and Cache Invalidation Policy

**Target files:**

- `crates/cordial-miners-core/src/consensus/pruning.rs`
- `crates/cordial-miners-core/src/consensus/ordering.rs`
- `docs/cordial-miners/integration/`

**Work:**

- Document what can be pruned after finalized ordered output is exported.
- Keep enough structural closure for validating future blocks.
- Define when ordering/finality caches are invalidated after equivocation evidence.

**Acceptance criteria:**

- Pruning tests preserve append-only ordered output.
- Cache invalidation behavior is explicit and tested.

## Phase 4: Application-Facing Prototype

This phase proves that the consensus output is useful to applications.

### 4.1 Ordered Output API

**Target files:**

- `crates/cordial-f1r3node-adapter/src/ordered_output.rs`
- `crates/cordial-f1r3node-adapter/src/shared_ordered_output.rs`
- New API module or binary if needed

**Work:**

- Expose latest finalized ordered output through a stable local interface.
- Keep file export for simple demos.
- Consider an HTTP/gRPC read endpoint after the file/trait boundary is stable.

**Acceptance criteria:**

- A small application can read finalized ordered output without knowing consensus internals.

### 4.2 End-to-End Rholang Execution Test

**Target files:**

- `crates/cordial-f1r3space-adapter/tests/test_e2e_execution.rs` (new)

**Work:**

- Execute a simple Rholang deploy through the adapter runtime path.
- Verify state hash changes and deploy result is visible.

**Acceptance criteria:**

- The test proves Cordial-adjacent adapter logic can reach actual F1R3FLY execution machinery.

### 4.3 Demo Application Candidate

**Target docs:**

- `docs/cordial-miners/application-demo-plan.md` (new)

**Work:**

- Define one small demo application, preferably a micro-payment ledger or decentralized social feed.
- Keep the first demo read-oriented: submit deploys, observe inclusion, show finalized ordered output.

**Acceptance criteria:**

- The demo proves multi-user ordering semantics without requiring full production replacement mode.

## Phase 5: Operations and Deployment

This phase prepares for longer-running demos and testnet-style operation.

### 5.1 Metrics and Tracing

**Target files:**

- Adapter live mirror and deploy ingress modules
- Core finality/order modules

**Work:**

- Add structured spans for mirror bootstrap, pending blocks, finality lookup, ordering export, and deploy observation.
- Add counters/gauges for mirrored blocks, ordered output length, unresolved predecessors, and observed deploys.

**Acceptance criteria:**

- Long-running live mirror sessions can be debugged without reading raw logs manually.

### 5.2 Docker and CI Stability

**Target files:**

- `docker/README.md`
- `.github/workflows/`
- `Justfile`

**Work:**

- Keep Docker build paths independent of local folder names.
- Document `docker compose` and legacy `docker-compose` commands.
- Add lightweight CI checks for docs, tests, clippy, and possibly Docker smoke tests.

**Acceptance criteria:**

- A new contributor can run the four-node cluster from a clean checkout.

## Work We Should Not Rush

These are valid production goals, but should not block the next integration milestone:

- Replacing CBC Casper inside f1r3node.
- Full Cordial standalone P2P networking.
- TLS/mTLS and peer discovery.
- Kubernetes manifests.
- Public testnet operations.
- Lean formalization of the full protocol.

They matter, but they should come after the live observer path is stable, benchmarked, and application-visible.

## Definition of Production-Ready

We should call this production-ready only when all of the following are true:

- Core consensus invariants are property-tested.
- Adversarial simulations cover equivocation, delay, and partitions.
- Live four-node f1r3node ordering convergence is repeatable.
- Ordered output has a stable consumer interface.
- Deploys can be traced from ingress to inclusion to ordered output.
- Persistence/recovery semantics are documented and tested.
- Finality and ordering have benchmark baselines.
- Metrics/tracing make long-running behavior observable.
- The application layer can consume finalized ordered output.

## Near-Term Milestone

The next concrete milestone is:

**A live f1r3node sidecar demo where a deploy is submitted through the Cordial proxy, observed before proposal, included by f1r3node, mirrored into Cordial Miners, finalized, tau-ordered, and exported through the ordered output seam.**

That milestone is realistic, builds directly on what is already merged, and is the cleanest bridge from research prototype to functional node-facing system.
