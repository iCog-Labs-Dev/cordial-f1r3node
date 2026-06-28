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

## Scope Of This Track

The integration notes in this folder focus on:

- how `f1r3node` traffic is traced and understood
- how live ingress is attached from this repository
- how intercepted messages are translated into Cordial Miners state
- how live state is compared, validated, and eventually ordered

## Planned Follow-up Topics

Future notes in this folder are expected to cover:

- actual deploy-side interception work at the documented pre-proposal seam
- proposer-facing deploy correlation after observation
