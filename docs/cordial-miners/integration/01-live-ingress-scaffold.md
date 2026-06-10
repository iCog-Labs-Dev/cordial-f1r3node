# 01. Live Ingress Scaffold

## Summary

This note documents the first implementation step in the live `f1r3node`
integration track: introducing a dedicated `live_ingress` module inside
`crates/cordial-f1r3node-adapter`.

This step is intentionally structural. It does not attach to a running node
yet. Its purpose is to create a clear adapter-side home for future runtime
interception work.

## Why This Step Exists

Most of the adapter-side integration primitives already exist in
`crates/cordial-f1r3node-adapter`, including:

- block/message translation
- protobuf ingestion validation
- Casper-facing adapter work
- snapshot support
- crypto alignment
- proposer, repository, and runtime bridge support

What was missing was a narrow runtime-facing entry point for live interception.

Without that boundary, future work would risk being spread across unrelated
modules. The scaffold added here prevents that by making the live integration
path explicit before attaching real traffic.

## Files

### Implementation

- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`
- `crates/cordial-f1r3node-adapter/src/lib.rs`

### Tests

- `crates/cordial-f1r3node-adapter/tests/test_live_ingress.rs`

## What Was Added

The new `live_ingress` module contains:

- `LiveIngressPhase`
  - a small runtime phase enum describing the current integration state
- `LiveIngress<A>`
  - a minimal wrapper around an adapter-side runtime component
  - intentionally generic so later steps can plug in richer state

The first supported phases are:

- `Defined`
  - module exists, but no live transport is attached
- `Traced`
  - host ingress seam has been identified and documented
- `Connected`
  - live message flow is attached to the adapter boundary

## Responsibility Boundary

This module is the adapter-side home for future work that will:

- receive live block-bearing messages from `f1r3node`
- translate them through existing adapter logic
- feed translated blocks into a local Cordial blocklace mirror
- expose enough state for snapshot, finality, and ordering work

## What This Step Does Not Do

This scaffold does **not** yet:

- attach to a real gRPC or transport server
- ingest live `BlockMessage` traffic
- maintain a stateful blocklace mirror
- run snapshot, finality, or tau ordering
- compare adapter state with HTTP-visible node state

Those are deferred to the next integration notes and implementation issues.

## Verification

The scaffold is covered by a dedicated adapter test file:

- `cargo test -p cordial-f1r3node-adapter --test test_live_ingress`

The tests verify:

- new live ingress starts in the `Defined` phase
- phase progression does not disturb the wrapped adapter value

## Outcome

This step completes the first live-integration sub-issue:

> create the adapter-side home for live interception work

The result is modest by design, but important. Future commits can now build on
an explicit runtime boundary instead of introducing live interception behavior
indirectly through unrelated adapter modules.
