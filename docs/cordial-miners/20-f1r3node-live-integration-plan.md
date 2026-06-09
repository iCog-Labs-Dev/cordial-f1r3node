# 20. F1r3node Live Integration Plan

**Purpose**: turn the completed Cordial Miners consensus core into a live f1r3node-backed prototype without replacing the node all at once.

## Current Status

We have now verified two important things:

1. `f1r3node` runs locally and is actively proposing/finalizing blocks.
2. The node already exposes enough API surface for us to begin integration in a staged way.

Confirmed live endpoints:

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

This gives us a practical starting point: we do not need to guess how the node behaves from paper-level design alone anymore.

## What We Confirmed In The Code Path

The HTTP layer is intentionally thin.

At the web boundary:

- `shared_handlers.rs` holds the route handlers and shared `AppState`
- `AppState` contains `web_api: Arc<dyn WebApi + Send + Sync>`

That means the HTTP handlers are not the real integration seam. They are only entry points.

For the endpoints we care about:

- `deploy_handler` calls `app_state.web_api.deploy(request).await`
- `get_blocks_handler` calls `app_state.web_api.get_blocks(1).await`
- `status_handler` calls `app_state.web_api.status().await`

Inside `web_api.rs`, `WebApiImpl` then hands these operations to `BlockAPI`:

- `deploy()` -> `BlockAPI::deploy(...)`
- `last_finalized_block()` -> `BlockAPI::last_finalized_block(...)`
- `get_block()` -> `BlockAPI::get_block(...)`
- `get_blocks()` -> `BlockAPI::get_blocks(...)`

## Why This Matters

This tells us there are two distinct integration modes:

### 1. Observation / Mirroring Mode

We observe finalized and recent blocks from f1r3node and feed them into a local Cordial Miners blocklace.

Good properties:

- low risk
- no change to f1r3node proposal flow
- easy to validate against real node output
- ideal for first live prototype

Limits:

- Cordial Miners does not influence transaction intake yet
- this proves compatibility and convergence plumbing, not full replacement

### 2. Interception / Ingress Mode

We hook before or around deploy submission and proposal so transactions are routed into the Cordial path earlier.

Good properties:

- matches the original “ghost DAG sidecar” vision more closely
- lets us reason about local ordering before native Casper final output

Limits:

- more invasive
- touches deploy lifecycle, proposer timing, and runtime expectations
- higher risk of breaking node behavior

## Recommended Integration Strategy

We should start with **observation-first integration**, then move inward.

That sequence is the safest and gives us proof at each layer:

1. mirror live blocks into local blocklace
2. reconstruct local Cordial ordering from observed node output
3. compare Cordial ordering with node-visible finalized history
4. only then move toward deploy interception

This keeps our work measurable and avoids jumping straight into the riskiest seam.

## Proposed Work Breakdown

### Issue 1: Live Block Observation Adapter

**Goal**: build a small adapter that consumes live f1r3node block data and inserts translated blocks into the local blocklace.

**Primary inputs**:

- `/api/blocks`
- `/api/last-finalized-block`

**Implementation area**:

- `crates/cordial-f1r3node-adapter/src/`
- likely a new module such as `live_observer.rs` or `http_observer.rs`

**Acceptance criteria**:

- fetches recent blocks from a running local node
- translates them into Cordial-compatible internal blocks
- inserts them into a local blocklace without violating closure rules
- can print or expose the mirrored DAG state for debugging

### Issue 2: Snapshot Reconstruction From Live Node Output

**Goal**: reconstruct a Cordial snapshot from observed node blocks so we can run our local predicates and ordering on real data.

**Implementation area**:

- `crates/cordial-f1r3node-adapter/src/snapshot.rs`
- `crates/cordial-miners-core/src/consensus/`

**Acceptance criteria**:

- derive local blocklace state from live observed blocks
- compute round/depth/wave metadata consistently
- run finality and tau ordering over the mirrored state

### Issue 3: Live Ordering Comparison

**Goal**: compare the local Cordial ordered output against what the node exposes as recent/finalized blocks.

**Implementation area**:

- adapter integration tests
- possibly a dedicated demo or verifier under `docs/cordial-miners/`

**Acceptance criteria**:

- one command or test can connect to the running node
- produces a readable comparison between:
  - observed f1r3node block history
  - locally reconstructed Cordial ordering
- mismatches are surfaced clearly

### Issue 4: Deploy Ingress Trace

**Goal**: trace exactly how a submitted deploy moves from `/deploy` through `WebApiImpl` into `BlockAPI::deploy`, proposal, and inclusion.

**Implementation area**:

- documentation first
- then adapter crate or integration harness

**Acceptance criteria**:

- written call-path document from HTTP submit to block inclusion
- identifies the narrowest safe seam for ingress-side Cordial interception
- names concrete files and functions to modify

### Issue 5: Early Interception Prototype

**Goal**: route submitted deploys into a local Cordial staging path before or alongside native proposal.

**Implementation area**:

- depends on the results of Issue 4

**Acceptance criteria**:

- deploys can be observed and queued through the Cordial side path
- no regression to normal node execution path
- prototype remains explicitly non-production

## Concrete Technical Reading Order

Before touching interception, the next files to fully trace are:

1. `shared_handlers.rs`
2. `web_api.rs`
3. `BlockAPI::deploy(...)`
4. `BlockAPI::get_blocks(...)`
5. `BlockAPI::last_finalized_block(...)`
6. proposer / finalizer path that turns accepted deploys into blocks

This is the right order because it follows the actual control flow we already verified.

## Suggested Immediate Next Step

The next implementation step should be:

**Build Issue 1 first: a live block observation adapter backed by the existing HTTP API.**

Why this first:

- it uses confirmed working endpoints
- it exercises real f1r3node output
- it lets us reuse our completed block translation, validation, finality, and tau ordering work
- it creates the shortest path to a real demo

## Summary

The good news is that integration is no longer vague.

We now know:

- the node is running
- the API is reachable
- the handlers are thin
- `WebApiImpl` is the real application boundary
- `BlockAPI` is the next operational seam

So our path is clear: **mirror first, compare second, intercept third**.
