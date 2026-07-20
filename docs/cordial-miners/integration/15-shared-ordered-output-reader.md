# Shared Ordered Output Reader

This note documents the first implemented consumer boundary for exported
`OrderedFinalizedOutput`.

It follows the design in
[14-ordered-output-consumer-boundary.md](./14-ordered-output-consumer-boundary.md)
and keeps the integration sidecar-first: no upstream `f1r3node` code is
modified, and no proposer behavior is changed.

## Purpose

The adapter can produce finalized ordered output through
`LiveIngress::latest_finalized_ordered_output(...)`. The shared reader gives
downstream adapter-side code a stable way to hold and read the latest output
without recomputing ordering or touching the blocklace.

## Implementation

The implementation lives in:

```text
crates/cordial-f1r3node-adapter/src/shared_ordered_output.rs
```

It defines:

- `ReadOrderedOutput`
  - read-only trait for consumers
  - exposes `latest()`, `anchor_hash()`, and `is_stale(...)`
- `SharedOrderedOutput`
  - adapter-side container for the latest `OrderedFinalizedOutput`
  - supports prefix-preserving updates
  - supports clearing for lifecycle resets and tests
- `SharedOrderedOutputError`
  - currently reports prefix violations

## Consumer Contract

Updates must preserve the previous ordered prefix. A new output is accepted
only if it is identical to the previous output or appends new ordered blocks.
The container rejects reordered, truncated, or replaced prefixes.

This matches the reintegration rule we want downstream consumers to rely on:
once a finalized ordered prefix is visible, later outputs must not rewrite it.

## Tests

Coverage lives in:

```text
crates/cordial-f1r3node-adapter/tests/test_shared_ordered_output.rs
```

The tests cover:

- empty reader state
- latest output reads
- anchor hash lookup
- staleness checks
- prefix-preserving append updates
- identical updates
- rejection of reordered output
- rejection of truncated output
- clearing the reader

## What This Does Not Do

This implementation is an in-process consumer boundary only. It does not yet:

- serve ordered output over HTTP or gRPC
- push notifications to consumers
- modify upstream `f1r3node`
- affect proposer behavior
- reorder transaction execution

Those are separate follow-up steps after this reader boundary is stable.

## Next Step

The next issue should decide how external tools should read the shared output:

- local file export
- lightweight HTTP endpoint
- lightweight gRPC endpoint
- or a sidecar application reader

That decision should happen before any proposer-facing integration.
