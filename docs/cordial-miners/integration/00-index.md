# Cordial Miners Integration Index

This folder tracks the incremental integration work that connects the
implemented Cordial Miners consensus logic in this repository to a running
`f1r3node` instance.

The purpose of this index is to keep the integration track modular, readable,
and easy to extend as new implementation notes are added.

## Documents

1. [01-live-ingress-scaffold.md](./01-live-ingress-scaffold.md)
   - Introduces the first adapter-side runtime scaffold for live interception
   - Establishes the `live_ingress` module as the home for future runtime wiring
2. [02-live-blockmessage-ingestion.md](./02-live-blockmessage-ingestion.md)
   - Connects `live_ingress` to the existing `grpc_ingest` pipeline
   - Documents the first live `BlockMessage` adapter-side acceptance path
3. [03-live-blocklace-mirror.md](./03-live-blocklace-mirror.md)
   - Adds a stateful local blocklace mirror behind `live_ingress`
   - Documents buffering and release of out-of-order block traffic
4. [04-live-snapshot-finality-ordering.md](./04-live-snapshot-finality-ordering.md)
   - Projects the mirrored live blocklace into snapshot, finality, and tau output
   - Documents the first consensus-facing read path over intercepted block traffic
5. [05-live-grpc-block-source.md](./05-live-grpc-block-source.md)
   - Attaches the adapter to a running `f1r3node` over the node's public gRPC block APIs
   - Documents trusted live block mirroring from node-facing gRPC responses
6. [06-http-observer-comparison.md](./06-http-observer-comparison.md)
   - Adds an HTTP observer over `/api/blocks` and `/api/last-finalized-block`
   - Documents mirror-vs-node comparison and mismatch reporting
7. [07-deploy-ingress-trace.md](./07-deploy-ingress-trace.md)
   - Traces deploy flow from external API ingress into proposal scheduling
   - Identifies the first safe pre-proposal Cordial interception seam
8. [08-live-mirror-check-harness.md](./08-live-mirror-check-harness.md)
   - Documents the live mirror diagnostic binary and its runtime modes
   - Explains parameters, output phases, and baseline-vs-drift interpretation
9. [09-live-deploy-observer.md](./09-live-deploy-observer.md)
   - Adds the first deploy-side pre-proposal observer seam
   - Documents staged deploy metadata and unchanged adapter admission behavior
10. [10-f1r3node-grpc-deploy-wiring.md](./10-f1r3node-grpc-deploy-wiring.md)
   - Documents host-side gRPC deploy wiring into the Cordial observer seam
   - Records the first live node hook before native BlockAPI admission
11. [11-external-grpc-deploy-proxy.md](./11-external-grpc-deploy-proxy.md)
   - Adds a no-node-changes external gRPC proxy for deploy observation
   - Preserves `f1r3node` method names while observing and forwarding `doDeploy`
12. [12-ordered-output-reintegration.md](./12-ordered-output-reintegration.md)
    - Defines the first clean seam for reconnecting Cordial output back to node-facing behavior
    - Recommends exporting finalized ordered output before attempting proposer-side control
13. [13-ordered-output-export.md](./13-ordered-output-export.md)
    - Documents the stable `OrderedFinalizedOutput` export type and its fields
    - Explains how the adapter produces, exposes, and intends the output to be consumed
14. [14-ordered-output-consumer-boundary.md](./14-ordered-output-consumer-boundary.md)
    - Defines the first node-facing consumer boundary for exported finalized ordered output
    - Describes the `ReadOrderedOutput` trait and adapter-side shared container
15. [15-shared-ordered-output-reader.md](./15-shared-ordered-output-reader.md)
    - Documents the implemented in-process shared ordered output reader
    - Covers prefix-preserving updates, staleness checks, and read-only consumption
16. [16-ordered-output-file-export.md](./16-ordered-output-file-export.md)
    - Documents the JSON file export seam for finalized ordered output
    - Shows how sidecar tooling can read tau output without touching `f1r3node`
17. [13-deploy-ingress-path.md](./13-deploy-ingress-path.md) · [13-four-node-ordering-convergence.md](./13-four-node-ordering-convergence.md)
    - Deploy-to-ordering trace: correlates observed deploy signatures with block inclusion and finalized output
    - Four-node convergence: verifies that real f1r3node validators export the same finalized tau order
18. [18-pruning-and-cache-invalidation-policy.md](./18-pruning-and-cache-invalidation-policy.md)
    - Documents what can be pruned after finalized ordered output is exported
    - Explains structural closure preservation and `OrderingCache` invalidation triggers
    - Covers the equivocation-evidence / cache interaction: why evidence recording does not flush the cache
19. [17-persistence-and-restart-semantics.md](./17-persistence-and-restart-semantics.md)
    - Wires `RSpaceBlocklaceRepository` into `LiveIngress` via `with_persistent_store`, `ingest_and_persist`, and `persist_finalized_cursor`
    - Documents exactly what survives a restart and what is recomputed, plus the startup lifecycle callers should follow

## Scope Of This Track

The integration notes in this folder focus on:

- how `f1r3node` traffic is traced and understood
- how live ingress is attached from this repository
- how intercepted messages are translated into Cordial Miners state
- how live state is compared, validated, and eventually ordered

## Planned Follow-up Topics

Future notes in this folder are expected to cover:

- transport wiring for ordered output (gRPC / IPC serving of `OrderedFinalizedOutput`)
- push / notification delivery for ordered output consumers
- proposer-facing integration only after the consumer boundary is validated
