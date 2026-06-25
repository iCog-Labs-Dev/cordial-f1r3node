# Deploy Ingress Trace And Interception Seam

This note records the deploy-side tracing step for the
`cordial-f1r3node-adapter` integration track.

The goal of this step is not to intercept deploys yet. The goal is to trace
how a deploy enters `f1r3node`, how it reaches proposal, and where the first
safe Cordial interception seam exists before block production.

## Reference Files

In `f1r3node`:

- `node/src/rust/api/deploy_grpc_service_v1.rs`
- `node/src/rust/api/web_api.rs`
- `casper/src/rust/api/block_api.rs`
- `node/src/rust/instances/proposer_instance.rs`

In this repository:

- `docs/cordial-miners/integration/07-deploy-ingress-trace.md`

## Traced Call Path

There are two external deploy ingress paths today:

1. gRPC deploy ingress
   - `DeployGrpcServiceV1Impl::do_deploy`
   - decodes `DeployDataProto`
   - converts it into `Signed<DeployData>`
   - calls `BlockAPI::deploy(...)`

2. HTTP deploy ingress
   - `WebApiImpl::deploy`
   - converts `DeployRequest` with `to_signed_deploy(...)`
   - calls `BlockAPI::deploy(...)`

Both ingress paths converge on the same shared deploy entry point:

- `casper::rust::api::block_api::BlockAPI::deploy`

## What Happens Inside `BlockAPI::deploy`

`BlockAPI::deploy(...)` is the real handoff from API ingress into Casper.

At this point the node:

- checks node read-only mode
- checks shard id compatibility
- checks forbidden signer keys
- checks minimum phlo price
- checks deploy expiration
- obtains the live Casper instance from `EngineCell`
- calls `casper.deploy(deploy_data)`

If deploy admission succeeds, `BlockAPI::deploy(...)` then triggers proposal
asynchronously through the configured `trigger_propose` function.

That trigger does not create the block inline. Instead it schedules proposal
work and returns deploy success to the caller without waiting for block
inclusion or finalization.

## How Proposal Is Reached

After `BlockAPI::deploy(...)` accepts the deploy:

- the node calls `trigger_propose(casper, true)`
- that eventually reaches the proposer runtime
- `node/src/rust/instances/proposer_instance.rs` processes queued propose
  requests
- the proposer produces a `BlockMessage` later, if proposal succeeds

So the flow is:

`API ingress -> Signed<DeployData> -> BlockAPI::deploy -> casper.deploy -> async trigger_propose -> proposer queue -> block creation`

## First Safe Cordial Interception Seam

The first safe pre-proposal seam is:

- after request decoding into `Signed<DeployData>`
- before calling `BlockAPI::deploy(...)`

This seam is the safest starting point because:

- both HTTP and gRPC deploy ingress already pass through it
- the deploy has already been parsed into a typed internal form
- proposal has not started yet
- we can observe, mirror, tag, or stage deploys without modifying proposer
  internals first
- we avoid coupling the first Cordial deploy integration to the deeper Casper
  queue and retry machinery

## Current Components We Can Reuse

This repository already contains most of the building blocks needed for the
first deploy-side increment. The main work left is wiring them at the right
pre-proposal seam.

Reusable pieces include:

- `crates/cordial-f1r3node-adapter/src/casper_adapter.rs`
  - already exposes `deploy(...)` and owns the adapter-side `DeployPool`
  - already models the deploy admission boundary in Cordial-native types
- `crates/cordial-miners-core/src/execution/deploy_pool.rs`
  - already provides pending deploy storage, deduplication, phlo-price checks,
    expiry pruning, and block-time selection logic
- `crates/cordial-f1r3node-adapter/src/proposer.rs`
  - already consumes `DeployPool` during proposal and block construction
  - gives us the downstream path deploys eventually need to reach
- `crates/cordial-f1r3node-adapter/src/block_translation.rs`
  - already translates signed deploys and processed deploys between Cordial and
    f1r3node-facing representations
- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`
  - already establishes the pattern for runtime-facing observation state in the
    adapter
  - this is a useful design reference even though it currently focuses on
    block-side live ingress

## Recommended Boundary For The First Increment

The first deploy-side increment should stay observational and non-breaking.

That means:

- decode the deploy into `Signed<DeployData>` exactly as the current ingress
  path already does
- copy or project the deploy into an adapter-facing observation type
- send that observed deploy into Cordial-side runtime state
- continue calling `BlockAPI::deploy(...)` unchanged

This keeps the existing node behavior intact while giving the adapter an early
view of deploy traffic before Casper admission and before proposal starts.

## Suggested Adapter Shape

The next implementation issue should likely introduce:

- a deploy-facing runtime module in `crates/cordial-f1r3node-adapter/src/`
  - for example `live_deploy_ingress.rs` or an extension of `live_ingress.rs`
- a small deploy observation type carrying:
  - ingress source (`http` or `grpc`)
  - shard id
  - deploy signature / deploy id
  - signer / deployer identity
  - phlo price
  - phlo limit
  - timestamps / validity window
- a lightweight adapter-side queue, buffer, or registry for pending observed
  deploys

The key point is that this first layer should observe and stage deploys, not
yet alter proposer scheduling or Casper admission.

## Why Not Intercept Deeper First

A deeper seam inside the proposer path would be more invasive.

By the time execution reaches `trigger_propose` and `ProposerInstance`, the
deploy has already been admitted into Casper, and the control flow is mixed
with scheduling, retries, and queue management. That is a good later
integration point for proposal-aware behavior, but it is not the best first
deploy-side seam.

## Suggested Next Implementation Issue

The next issue can be framed as:

`Add a pre-proposal deploy observer seam in the adapter before BlockAPI::deploy admission`

That issue should focus on:

- defining a deploy-facing adapter input type
- capturing deploy metadata at both HTTP and gRPC ingress
- routing the typed deploy view into adapter-side observation code
- reusing the existing deploy-pool / proposer-side components where possible
- keeping Casper admission behavior unchanged for the first increment

## Planning Outcome

From the current assessment, the recommended implementation sequence is:

1. add deploy-side observation at the pre-`BlockAPI::deploy(...)` seam
2. store or mirror pending deploy metadata in adapter runtime state
3. connect observed deploys to later proposer / block-inclusion visibility
4. only then consider deeper proposer-aware interception or replacement logic

## Acceptance Outcome

This tracing step satisfies the current acceptance goals:

- the ingress-to-proposal call path is now documented
- reusable adapter/core components have been identified
- one candidate pre-proposal Cordial interception seam is identified
- the result is concrete enough to open the next implementation issue
