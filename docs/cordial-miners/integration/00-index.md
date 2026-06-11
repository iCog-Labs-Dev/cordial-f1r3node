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

## Scope Of This Track

The integration notes in this folder focus on:

- how `f1r3node` traffic is traced and understood
- how live ingress is attached from this repository
- how intercepted messages are translated into Cordial Miners state
- how live state is compared, validated, and eventually ordered

## Planned Follow-up Topics

Future notes in this folder are expected to cover:

- snapshot, finality, and tau ordering on live mirrored state
- HTTP-based comparison harnesses
- deploy ingress tracing and later interception candidates
