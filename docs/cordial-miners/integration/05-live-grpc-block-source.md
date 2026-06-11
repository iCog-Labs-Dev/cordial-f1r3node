# Live gRPC Block Source

This note records the fifth integration increment for the
`cordial-f1r3node-adapter`.

The goal of this step is to connect the adapter to a running `f1r3node`
through a real gRPC surface and feed that live node state into the existing
`live_ingress` mirror.

## What Was Added

A new live gRPC block source was added in:

- `crates/cordial-f1r3node-adapter/src/live_grpc.rs`

This module introduces:

- a client for `f1r3node`'s `DeployService` gRPC endpoint,
- streaming of recent node blocks through `getBlocks`,
- decoding of live `LightBlockInfo` values into trusted Cordial blocks,
- ingestion of those trusted blocks into `live_ingress`.

## Important Boundary

This path uses the node's public gRPC block APIs, which expose block views
such as `LightBlockInfo`, not the raw internal transport `BlockMessage`
stream.

That means this step is a real running-node gRPC attachment, but it is not
yet the same as tapping directly into the internal Casper packet bus. The
adapter therefore treats these gRPC-derived blocks as trusted node views and
mirrors them into the blocklace accordingly.

## Why This Matters

This increment is the first one that lets the adapter consume block data from
an actual live `f1r3node` endpoint rather than only from tests or manually
constructed messages.

That gives us a concrete integration bridge:

- running node gRPC -> trusted live block reconstruction -> `live_ingress`
  mirror -> snapshot/finality/tau views

## Files

Implementation:

- `crates/cordial-f1r3node-adapter/src/live_grpc.rs`
- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`

Tests:

- `crates/cordial-f1r3node-adapter/tests/test_live_grpc.rs`

## Acceptance Shape

This increment is considered complete when:

- the adapter can connect to a live `f1r3node` gRPC block endpoint,
- recent blocks can be fetched from the node,
- fetched blocks can be reconstructed into trusted local mirror entries,
- the reconstructed blocks can be fed into `live_ingress`,
- the conversion path is covered by adapter-level tests.
