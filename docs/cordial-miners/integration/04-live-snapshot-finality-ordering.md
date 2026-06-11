# Live Snapshot, Finality, and Ordering

This note records the fourth live-integration increment for the
`cordial-f1r3node-adapter`.

The earlier increments established three things:

1. a dedicated live-ingress module,
2. translation of live `BlockMessage` values through `grpc_ingest`, and
3. a stateful local blocklace mirror that can buffer out-of-order arrivals.

This increment turns that mirrored state into something consensus-facing.
Once live traffic has been mirrored into a local blocklace, the adapter can
now build a `CasperSnapshot`, expose the latest finalized block hash, and
read the current weighted tau output from the same mirrored state.

## What Was Added

The `LiveIngress` adapter now carries the extra consensus-view inputs needed
to evaluate the mirrored blocklace:

- bonded validator weights,
- shard configuration, and
- shard identifier.

Using those inputs, `LiveIngress` now exposes:

- `snapshot()` to build a `CasperSnapshot`,
- `last_finalized_block_hash()` to report the latest finalized leader block,
- `ordered_finalized_blocks()` to report the current weighted tau output.

## Why This Matters

This is the first point where the live interception path stops being just a
message mirror and starts behaving like a read-only consensus view over live
`f1r3node` traffic.

That is important because the integration goal is not only to ingest blocks,
but to show that the intercepted stream can drive the same kinds of
finality- and ordering-facing outputs that the adapter already supports in
its standalone and test paths.

## Files

Implementation:

- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`

Tests:

- `crates/cordial-f1r3node-adapter/tests/test_live_ingress.rs`

## Acceptance Shape

This increment is considered complete when:

- live mirrored state can be projected into a `CasperSnapshot`,
- a finalized leader wave can be recognized from mirrored live traffic,
- weighted tau output can be read from that same mirrored state,
- the behavior is covered by adapter-level tests.
