# HTTP Observer Comparison Harness

This note records the HTTP verification increment for the
`cordial-f1r3node-adapter`.

The goal of this step is to compare the Cordial live mirror against the
block and finality state exposed by a running `f1r3node` through its HTTP API.

## What Was Added

A new HTTP observer module was added in:

- `crates/cordial-f1r3node-adapter/src/http_observer.rs`

This module provides:

- a small HTTP client for `/api/blocks`
- a small HTTP client for `/api/last-finalized-block`
- comparison of HTTP-visible node state against the local Cordial live mirror
- explicit mismatch reporting for:
  - blocks missing from the mirror
  - blocks missing from the HTTP-visible node view
  - last-finalized-block disagreement

## Why This Matters

This makes live integration debuggable.

Without this comparison layer, the adapter can mirror live traffic and compute
its own snapshot/finality/order views, but there is no simple way to check
whether that mirror still agrees with what the running node itself exposes.

The HTTP observer turns the node API into a verification surface.

## Files

Implementation:

- `crates/cordial-f1r3node-adapter/src/http_observer.rs`

Tests:

- `crates/cordial-f1r3node-adapter/tests/test_http_observer.rs`

## Acceptance Shape

This increment is considered complete when:

- the harness can query `/api/blocks`
- the harness can query `/api/last-finalized-block`
- the live mirror can be compared against HTTP-visible node state
- mismatches are reported clearly for debugging
