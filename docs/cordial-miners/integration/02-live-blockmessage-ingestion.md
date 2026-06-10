# 02. Live BlockMessage Ingestion

## Summary

This note documents the second integration increment: wiring the
`live_ingress` module into the existing `grpc_ingest` pipeline so it can accept
live protobuf `BlockMessage` values and route them through the already
implemented translation and structural validation path.

This step still stops short of a full live blocklace mirror. Its purpose is to
ensure that live ingress does not introduce a parallel or ad hoc decoding path.

## Why This Step Exists

The adapter crate already had a reusable protobuf-first ingestion layer in
`grpc_ingest.rs`, built around:

- `GrpcBlockMapper`
- `BlocklaceAdapter`

The new `live_ingress` scaffold from the previous step established a runtime
home for live interception work, but it did not yet accept real block-bearing
messages.

This increment closes that gap by making `live_ingress` reuse the existing
mapping path instead of inventing a second one.

## Files

### Implementation

- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`

### Tests

- `crates/cordial-f1r3node-adapter/tests/test_live_ingress.rs`

## What Was Added

The `live_ingress` module now:

- owns a `GrpcBlockMapper`
- accepts protobuf `BlockMessage` values
- translates them through the existing `grpc_ingest` logic
- hands the translated block to the adapter callback
- surfaces errors separately for:
  - mapping/validation failures
  - adapter-side rejection failures

## Error Model

This step introduces a small `LiveIngressError` boundary so the live ingress
path can distinguish:

- `Mapping`
  - failed to translate or structurally validate the incoming `BlockMessage`
- `Adapter`
  - the translated block was rejected by the adapter callback

This keeps future debugging cleaner once real traffic is attached.

## Runtime Behavior

On successful ingestion:

- `live_ingress` routes the message through `GrpcBlockMapper`
- the adapter callback receives the translated internal `Block`
- the ingress phase advances to `Connected`

On adapter rejection:

- the error is returned as `LiveIngressError::Adapter`
- the ingress phase does not advance

## What This Step Does Not Do

This increment does **not** yet:

- attach to a real transport or gRPC server
- buffer or reorder live messages
- maintain a persistent blocklace mirror
- compute snapshots or run finality
- perform tau ordering

Those remain separate follow-up increments.

## Verification

The dedicated live-ingress test file covers:

- initial phase behavior
- phase progression
- successful `BlockMessage` ingestion through `grpc_ingest`
- adapter rejection after successful mapping

Verification command:

- `cargo test -p cordial-f1r3node-adapter --test test_live_ingress`

## Outcome

This completes the second live-integration sub-issue:

> wire live `BlockMessage` ingestion into the existing `grpc_ingest` pipeline

The adapter now has a real ingestion boundary for block-bearing protobuf
messages, while still keeping state mirroring and live runtime attachment as
separate, modular steps.
