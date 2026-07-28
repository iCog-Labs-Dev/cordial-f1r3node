# Production-Grade Consensus Protocol Evaluation: Cordial Miners

**Date:** Tue Jul 28 2026  
**Purpose:** Internal production-readiness artifact for tracking what is already working, what is still prototype-grade, and what must be verified before calling the system production-ready.

## Overall Status

The project is no longer only a paper implementation. The core Cordial Miners logic exists, the f1r3node adapter can mirror live node blocks, finalized tau ordering can be exported, deploy ingress can be observed before proposal, and a real four-node f1r3node cluster can be used to verify that mirrored validators produce the same finalized Cordial tau order.

That said, the system should still be described as a **functional consensus prototype with live-node integration**, not yet a production validator implementation. The remaining work is mostly hardening, persistence semantics, operational tooling, application-facing APIs, and deeper f1r3node reintegration.

Practical readiness estimate:

- **Core consensus logic:** strong prototype / near complete
- **Live f1r3node observer integration:** functional and testable
- **Production node readiness:** not yet complete
- **Application-facing readiness:** emerging, but needs a stable API and execution loop

## 1. Protocol Completeness

The main paper-level consensus mechanisms have been implemented and tested in the core crate.

| Component | Current Status | Notes |
|-----------|----------------|-------|
| Blocklace DAG structure | Complete | Multi-parent blocklace container exists. |
| Observation / precedes relation | Complete | Used by approval, ratification, and ordering. |
| Closure / ancestry logic | Complete | Supports finality and ordering computations. |
| Equivocation detection | Complete | Equivocations are detected and excluded from finality/order output. |
| Round/depth computation | Complete | Optimized and reused in live finality paths. |
| Approval | Complete | Includes memoized paths. |
| Ratification and super-ratification | Complete | Includes weighted and unweighted variants. |
| Wave/finality logic | Complete for ES mode | Eventual synchrony path uses wavelength 3. Async coin work remains future work. |
| Tau ordering | Complete | Used by simulations, live mirror checks, and ordered output export. |
| Ordered output export | Complete | `ordered_output.rs`, shared reader, file export, and inspection tooling exist. |
| Cordial dissemination | Partial | Predecessor selection exists; full peer-knowledge dissemination is not productionized. |
| Excommunicating equivocators in predecessor selection | Needs refinement | Finality/order exclusion exists; predecessor selection should make the excommunication mode explicit. |

Assessment: the protocol core is in good shape for continued integration work, but it needs property-based testing and adversarial validation before we should call it production-grade.

## 2. f1r3node Integration Status

The adapter work has advanced beyond static translation.

What is working:

- Live block mirroring from a running f1r3node through gRPC.
- Height bootstrap for reconstructing the node-visible block view.
- Local Cordial finality and tau ordering over mirrored f1r3node blocks.
- Ordered finalized output export through a stable adapter seam.
- CLI inspection and JSON export of ordered output.
- HTTP comparison/debugging surface against f1r3node-visible state.
- Four-node Docker cluster support for real f1r3node validator mirroring.
- Four-node finalized tau ordering convergence verification.
- Deploy ingress observation before proposal through external gRPC/HTTP proxy paths, without modifying upstream f1r3node.

What is still missing:

- The ordered output is not yet consumed by f1r3node’s proposer or execution path.
- Cordial Miners does not yet replace CBC Casper inside the node.
- Deploys are observed and proxied, but not reordered back into proposer input.
- End-to-end Rholang execution from Cordial-ordered output still needs a dedicated test.
- Hash parity and deeper internal node substitution are still future integration work.

Assessment: the sidecar/observer path is real and useful. Replacement-mode integration remains a later milestone.

## 3. Storage and Persistence

The previous automated statement that there is “no LMDB” is outdated.

What exists:

- `cordial-f1r3space-adapter` contains an LMDB-backed repository layer.
- Adapter repository tests verify LMDB round trips, reopen/recovery behavior, finalized cursor persistence, and corrupted-data handling.
- Checkpoint pruning exists and is tested in the core path.

What remains:

- Core `Blocklace` is still an in-memory structure.
- LMDB persistence is available through the adapter/repository layer, not as a native pluggable core `BlocklaceStore`.
- Crash/restart testing should be expanded from repository-level recovery into live mirror recovery and ordered-output recovery.
- Long-running pruning/checkpoint policy needs clearer production rules.

Assessment: persistence exists, but production persistence semantics are not finished. The right wording is **adapter-level LMDB persistence exists; core-native persistent blocklace storage is still a future hardening task**.

## 4. Testing Status

The test suite is broad and actively growing.

What is covered:

- Unit tests across the consensus modules.
- Weighted and unweighted finality/order paths.
- Dissemination and simulation tests.
- Four-node simulated convergence.
- Four-node real f1r3node cluster connectivity and ordering convergence.
- Live gRPC mirror tests.
- Ordered output export and shared reader tests.
- Deploy ingress and deploy proxy tests.
- LMDB repository tests.

Highest-value missing tests:

- Property-based tests for tau prefix preservation, closure, deterministic ordering, and equivocation exclusion.
- Adversarial tests for partitions, delayed delivery, equivocation injection, and bounded Byzantine faults.
- Benchmarks for finality lookup, tau ordering, height bootstrap, and four-node convergence.
- End-to-end Rholang execution test driven by Cordial-observed deploys or ordered output.

Assessment: current testing is good for a prototype and integration track. Production confidence requires invariant testing and benchmarks.

## 5. Networking and Dissemination

The core network layer should not yet be treated as production infrastructure.

What exists:

- Basic peer/node/message abstractions.
- Block broadcast/request/sync-style mechanics.
- Pending block buffering exists at the consensus/dissemination layer.

What remains:

- Full paper-native cordial dissemination with peer-knowledge inference.
- Buffer/retry wiring in the runtime network path.
- Rate limiting and message-size bounds.
- TLS or authenticated transport.
- Peer discovery and peer scoring.
- Clear decision on whether production networking is owned by f1r3node transport, Cordial’s own network, or a bridge.

Assessment: for near-term f1r3node integration, the external gRPC sidecar path is the pragmatic route. Core network productionization is important, but it should not block live-node observer work.

## 6. Operations and Observability

This is still early.

What exists:

- CLI tools for live mirror checks, ordered output export, deploy proxying, and four-node verification.
- Docker-based local cluster support.
- Documentation for integration and demos.

What remains:

- Structured tracing across consensus, adapter, and live mirror paths.
- Metrics for mirrored blocks, pending blocks, finalized anchors, ordered output length, latency, and errors.
- Health/readiness endpoints for the adapter services.
- Config files or stable environment-variable configuration.
- CI jobs for property tests, integration tests, and Docker cluster smoke tests.

Assessment: operational tooling exists for development and demos, but not for production operations.

## 7. Production Priorities

The next production-readiness work should be ordered like this:

| Priority | Work Item | Why It Matters |
|----------|-----------|----------------|
| P0 | Property-based tests for consensus invariants | Proves the implementation keeps safety properties under many generated DAGs. |
| P0 | Four-node live ordering convergence hardening | Keeps the real f1r3node integration measurable and repeatable. |
| P0 | Ordered output consumer boundary | Defines where finalized tau output goes next. |
| P1 | Live deploy-to-ordering trace | Connects deploy observability to block inclusion and ordered output. |
| P1 | Benchmarks for finality and ordering | Prevents hidden performance regressions. |
| P1 | Core/adaptor persistence semantics | Clarifies what survives restart and where. |
| P2 | Metrics and tracing | Makes the system debuggable during long runs. |
| P2 | End-to-end Rholang execution test | Demonstrates application-level usefulness. |
| P3 | Core network productionization | Needed for standalone Cordial networking, but less urgent than f1r3node sidecar integration. |

## Summary

The project has crossed an important line: Cordial Miners is not just correct in isolation anymore. It can observe live f1r3node blocks, compute Cordial finality and tau order, export ordered output, observe deploy ingress, and verify ordering convergence across a real four-node f1r3node cluster.

The honest next label is **live integrated prototype**, not **production-ready consensus node**. The shortest path to production is to harden what already works: property tests, benchmarks, ordered-output consumption, persistence/recovery semantics, and application-facing execution tests.
