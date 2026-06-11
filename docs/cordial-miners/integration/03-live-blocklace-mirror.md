# 03. Live Blocklace Mirror

## Summary

This note documents the third live integration increment: adding a stateful
local blocklace mirror behind `live_ingress`.

With this step, live-ingested blocks are no longer only translated and passed
through an adapter callback. They are also accumulated into a long-lived local
`Blocklace` view inside the adapter crate.

## Why This Step Exists

The previous increment established a live protobuf ingestion boundary using the
existing `grpc_ingest` pipeline. That was enough to accept and validate
`BlockMessage` values, but it still left the adapter without persistent
consensus-facing runtime state.

This increment addresses that by introducing a local mirror that can:

- store accepted blocks
- buffer out-of-order arrivals whose predecessors are not yet available
- release buffered blocks once their dependencies are satisfied

This is the first step where live ingress begins to look like a real mirrored
consensus view rather than a stateless translation edge.

## Files

### Implementation

- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`

### Tests

- `crates/cordial-f1r3node-adapter/tests/test_live_ingress.rs`

## What Was Added

The live-ingress path now owns a `LiveBlocklaceMirror`, which contains:

- a local `Blocklace`
- a pending buffer for blocks with missing predecessors
- a verifier used for final blocklace insertion

The mirror supports three outcomes for incoming blocks:

- `Applied`
  - block inserted immediately into the local blocklace
- `Buffered`
  - block held until required predecessors arrive
- `Duplicate`
  - block already known in blocklace or pending state

## Runtime Behavior

The mirror currently follows this flow:

1. `live_ingress` maps and structurally validates a protobuf `BlockMessage`
2. the adapter callback accepts the translated internal block
3. the mirror checks whether all predecessors are already present
4. if yes:
   - the block is inserted into the local blocklace
   - pending blocks are scanned and dependency-free blocks are released
5. if no:
   - the block is kept in a pending buffer until predecessors arrive

## Why This Design Is Useful

This keeps the live interception path modular:

- structural translation remains in `grpc_ingest`
- runtime accumulation now lives in `live_ingress`
- finality and ordering remain deferred

It also matches the reality of live traffic: block-bearing messages may arrive
out of order, so the mirror needs buffering before higher-level consensus logic
can safely run.

## What This Step Does Not Do

This increment still does **not** yet:

- attach to a real running transport server
- run snapshot construction automatically
- execute finality checks
- execute tau ordering
- compare the local mirror with HTTP-visible node state

Those remain separate follow-up increments.

## Verification

The dedicated live-ingress test file now covers:

- successful ingestion into the local blocklace
- adapter rejection without mirror mutation
- buffering of out-of-order blocks
- release of buffered blocks after predecessor arrival
- duplicate handling inside the mirror

Verification command:

- `cargo test -p cordial-f1r3node-adapter --test test_live_ingress`

## Outcome

This completes the third live-integration sub-issue:

> add a stateful live blocklace mirror for intercepted block traffic

The adapter now has long-lived local state and basic dependency buffering,
which makes the next steps possible:

- snapshot reconstruction from live state
- finality and tau execution on the mirror
- comparison against node-visible state
