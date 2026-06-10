# 20. F1r3node Live Integration Plan

**Purpose**: turn the completed Cordial Miners consensus core into a live f1r3node-backed prototype, with **gRPC interception as the primary integration path** and HTTP as a supporting verification surface.

## Current Status

We have now verified several important things:

1. `f1r3node` runs locally and is actively proposing and finalizing blocks.
2. The node exposes live ports for protocol, gRPC, and HTTP access.
3. The HTTP API is reachable and useful for inspection.
4. The Cordial adapter side already has a gRPC ingestion layer we can build on.

Confirmed live HTTP endpoints:

- `GET /status`
- `GET /api/status`
- `GET /api/blocks`
- `GET /api/last-finalized-block`

Observed local ports:

- `40400` protocol server
- `40401` external gRPC
- `40402` internal gRPC
- `40403` HTTP API
- `40404` peer discovery
- `40405` admin / metrics

Existing Cordial-side foundation:

- `crates/cordial-f1r3node-adapter/src/grpc_ingest.rs`

This is important because it means gRPC interception is not a new idea from scratch. We already have the beginning of the translation path from protobuf `BlockMessage` values into internal consensus blocks.

## What Is Already Implemented On Our Side

Most of the **library-level integration work** is already present in
`crates/cordial-f1r3node-adapter`. The remaining gap is mostly about
**live wiring into a running f1r3node**, not about re-implementing the adapter concepts.

### Implemented adapter modules

- `block_translation.rs`
  - translates between internal Cordial blocks and f1r3node-style `BlockMessage`
- `grpc_ingest.rs`
  - protobuf-first ingestion, structural validation, and adapter boundary
- `casper_adapter.rs`
  - local mirror of Casper / MultiParentCasper-facing integration
- `snapshot.rs`
  - snapshot construction and finalized-block projection support
- `crypto_bridge.rs`
  - hash and signature alignment for the integration boundary
- `shard_conf.rs`
  - shard and configuration bridge
- `proposer.rs`
  - proposer-side integration support
- `repository.rs`
  - repository and persistence-side adapter support
- `runtime_bridge.rs`
  - runtime-facing bridge logic
- `rspace_runtime.rs`
  - RSpace-backed runtime integration support
- `slashing.rs`
  - slashing-related integration logic

### Implemented test coverage

The adapter crate already has dedicated tests covering most of these areas:

- `test_grpc_ingest.rs`
- `test_block_translation.rs`
- `test_casper_adapter.rs`
- `test_snapshot.rs`
- `test_proposer.rs`
- `test_repository.rs`
- `test_crypto_bridge.rs`
- `test_shard_conf.rs`
- `test_slashing.rs`
- `conformance.rs`

### What this means

From our side, the following are **already available at the adapter/library level**:

- block/message translation
- protobuf ingestion validation
- Casper-facing adapter abstractions
- snapshot-building support
- crypto alignment
- runtime bridge scaffolding
- proposer and repository bridge support
- slashing-related integration logic

So it would be inaccurate to say “integration has not started.” A large portion of the integration layer already exists as reusable code.

## What Is Still Needed For Live Interception

What is still missing is the **runtime attachment to real f1r3node traffic**.

### Still needed

1. **Real gRPC service trace**
   - identify the exact f1r3node gRPC services, methods, and Rust handlers that carry the traffic we want

2. **Live gRPC hook**
   - attach the running node’s message flow to our existing `grpc_ingest` path

3. **Stateful live mirror**
   - maintain a long-lived local blocklace fed by real intercepted traffic
   - handle out-of-order blocks, dependency buffering, and replay behavior in live mode

4. **Continuous snapshot/finality/ordering execution**
   - run our existing consensus logic incrementally as live blocks arrive

5. **Verification harness**
   - compare the gRPC-fed Cordial view with f1r3node’s HTTP-visible state

6. **Later deploy-side interception**
   - trace and possibly hook deploy ingress before or around native proposal

### Short version

The adapter crate is **largely implemented as a library**.  
The missing work is **live interception plumbing**.

That distinction is important:

- **already implemented**: adapter logic
- **still needed**: runtime hookup to real node traffic

## What We Confirmed In The HTTP Code Path

The HTTP layer is intentionally thin.

At the web boundary:

- `shared_handlers.rs` holds the route handlers and shared `AppState`
- `AppState` contains `web_api: Arc<dyn WebApi + Send + Sync>`

For the endpoints we inspected:

- `deploy_handler` calls `app_state.web_api.deploy(request).await`
- `get_blocks_handler` calls `app_state.web_api.get_blocks(1).await`
- `status_handler` calls `app_state.web_api.status().await`

Inside `web_api.rs`, `WebApiImpl` then hands these operations to `BlockAPI`:

- `deploy()` -> `BlockAPI::deploy(...)`
- `last_finalized_block()` -> `BlockAPI::last_finalized_block(...)`
- `get_block()` -> `BlockAPI::get_block(...)`
- `get_blocks()` -> `BlockAPI::get_blocks(...)`

This confirms that HTTP is very useful for observability and debugging, but it is not the most protocol-native place to intercept consensus traffic.

## Why gRPC Should Be The Primary Track

There are two realistic live integration modes:

### 1. gRPC Interception / Ingestion

We intercept or observe protobuf traffic at the gRPC boundary and feed it into the Cordial adapter path.

Good properties:

- closest to the real communication path of the node
- aligns with the existing `grpc_ingest` architecture
- works with protobuf `BlockMessage` values directly
- better fit for a sidecar or consensus-adjacent prototype

Limits:

- requires tracing the actual gRPC service and handler implementations in f1r3node
- more moving parts than simple HTTP polling
- may involve both external and internal gRPC paths

### 2. HTTP Observation / Mirroring

We observe blocks and finalized state through the HTTP API and reconstruct local Cordial state from that surface.

Good properties:

- low risk
- quick to verify locally
- useful as a comparison and debugging surface

Limits:

- less native than gRPC
- reconstructs state after the fact
- weaker fit for a true interception architecture

## Recommended Integration Strategy

We should treat **gRPC as the main implementation path** and use **HTTP as the comparison path**.

The recommended sequence is:

1. trace the real gRPC services and methods in f1r3node
2. connect live protobuf messages to our existing `grpc_ingest` pipeline
3. feed intercepted blocks into the local Cordial blocklace
4. use HTTP endpoints to compare the mirrored Cordial state against live node-visible state
5. only then move deeper toward deploy-side interception if needed

This gives us a protocol-native design without sacrificing observability.

## Proposed Work Breakdown

### Issue 1: gRPC Service Trace

**Goal**: identify the exact f1r3node gRPC services and methods that carry block or deploy traffic relevant to Cordial ingestion.

**Implementation area**:

- f1r3node protobuf service definitions
- gRPC server registration code
- Rust handler implementations for the relevant methods

**Acceptance criteria**:

- documented external and internal gRPC service map
- identified file paths for the live server implementation
- recommendation for the narrowest safe interception seam

### Issue 2: Live gRPC Ingestion Adapter

**Goal**: connect live f1r3node gRPC-delivered block messages into the Cordial `grpc_ingest` pipeline.

**Primary inputs**:

- protobuf `BlockMessage`
- existing `GrpcBlockMapper`
- adapter-side `BlocklaceAdapter`

**Implementation area**:

- `crates/cordial-f1r3node-adapter/src/grpc_ingest.rs`
- a new live integration module in `crates/cordial-f1r3node-adapter/src/`

**Acceptance criteria**:

- receives live protobuf block messages from a running node path
- translates them into Cordial-compatible internal blocks
- inserts them into a local blocklace without violating closure rules
- surfaces ingestion failures clearly

### Issue 3: Snapshot Reconstruction From gRPC-fed State

**Goal**: reconstruct a Cordial snapshot from intercepted block traffic so we can run local predicates and ordering on real node data.

**Implementation area**:

- `crates/cordial-f1r3node-adapter/src/snapshot.rs`
- `crates/cordial-miners-core/src/consensus/`

**Acceptance criteria**:

- derive local blocklace state from live intercepted blocks
- compute round, depth, and wave metadata consistently
- run finality and tau ordering over the mirrored state

### Issue 4: HTTP Comparison Harness

**Goal**: use the existing HTTP API as a verification surface for the gRPC-fed Cordial view.

**Primary inputs**:

- `/api/blocks`
- `/api/last-finalized-block`

**Implementation area**:

- `crates/cordial-f1r3node-adapter/src/`
- likely a verifier or observer module such as `http_observer.rs`

**Acceptance criteria**:

- one command or test can connect to the running node
- produces a readable comparison between:
  - gRPC-intercepted Cordial blocklace state
  - HTTP-visible node block history
- mismatches are surfaced clearly

### Issue 5: Deploy Ingress Trace

**Goal**: trace exactly how a submitted deploy moves from ingress through API or gRPC boundaries into proposal and inclusion.

**Implementation area**:

- documentation first
- then adapter crate or integration harness

**Acceptance criteria**:

- written call-path document from ingress to block inclusion
- identified narrowest safe seam for ingress-side Cordial interception
- named files and functions to modify

### Issue 6: Early Deploy Interception Prototype

**Goal**: route submitted deploys into a local Cordial staging path before or alongside native proposal.

**Implementation area**:

- depends on the results of Issue 5

**Acceptance criteria**:

- deploys can be observed and queued through the Cordial side path
- no regression to normal node execution path
- prototype remains explicitly non-production

## Concrete Technical Reading Order

Before implementation, the next files or areas to fully trace are:

1. f1r3node protobuf service definitions
2. gRPC server registration and routing
3. Rust handler implementations for the relevant gRPC methods
4. `crates/cordial-f1r3node-adapter/src/grpc_ingest.rs`
5. `shared_handlers.rs`
6. `web_api.rs`
7. `BlockAPI::deploy(...)`
8. proposer and finalizer path that turns accepted deploys into blocks

This is the right order because it follows the live traffic boundary first, then the supporting HTTP and proposal paths.

## Suggested Immediate Next Step

The next implementation step should be:

**Build Issue 1 first: trace and document the real gRPC service boundary we want to intercept.**

Why this first:

- it matches the preferred long-term architecture
- it lets us attach live traffic to the `grpc_ingest` work we already have
- it reduces the chance of building the wrong adapter first
- it still leaves HTTP available as a comparison surface

## Summary

The good news is that integration is no longer vague.

We now know:

- the node is running
- HTTP API is reachable for verification
- the HTTP handlers are thin
- `WebApiImpl` is the real HTTP application boundary
- gRPC should be treated as the primary live ingestion boundary
- `BlockAPI` remains an important downstream seam for deploy flow tracing

So our path is now: **trace gRPC first, ingest live blocks second, compare over HTTP third, then move toward deploy interception**.
