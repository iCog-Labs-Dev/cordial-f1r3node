# Live Deploy Observer Seam

This note documents the first deploy-side implementation step after the deploy
ingress tracing work.

The goal of this increment is intentionally narrow: observe typed deploys before
proposal, stage their metadata in adapter-side runtime state, and keep existing
Casper admission behavior unchanged.

## What Was Added

- `crates/cordial-f1r3node-adapter/src/live_deploy_ingress.rs`
- `crates/cordial-f1r3node-adapter/tests/test_live_deploy_ingress.rs`

## Responsibility

The new deploy observer module provides:

- a typed deploy ingress source (`http` or `grpc`)
- a staged deploy record with key metadata
- adapter-side state for observed deploys
- an observation-plus-admission helper that records deploy metadata and then
  passes the deploy into the existing adapter `deploy(...)` path

This means the deploy observer can sit in front of native Casper admission
without changing what happens after admission.

## Stored Metadata

For each observed deploy, the module records:

- signature
- deployer public key
- signature algorithm
- shard id
- term length
- phlo price
- phlo limit
- `valid_after_block_number`
- optional expiration timestamp
- observed ingress source(s)
- observation count

## Why This Shape

This is the safest first increment because:

- it does not modify proposer scheduling
- it does not replace `BlockAPI::deploy(...)`
- it does not require deeper node-side queue hooks
- it gives the adapter a pre-proposal deploy view immediately

## What It Enables Next

With this seam in place, the next deploy-side step can focus on:

- connecting observed deploys to later proposer/block inclusion
- comparing pending observed deploys to deploys that appear in finalized blocks
- deciding whether the adapter should remain observational or become
  proposer-aware

## Verification

The current tests cover:

- gRPC deploy observation
- HTTP + gRPC source merging for the same deploy
- observation followed by successful adapter admission
- rejected deploys remaining visible in observer state for debugging
