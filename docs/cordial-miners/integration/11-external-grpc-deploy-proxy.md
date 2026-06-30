# External gRPC deploy proxy

This step moves deploy-side interception fully outside `f1r3node`.

Instead of patching node handlers, the adapter now provides an external gRPC
proxy that speaks the same public `DeployService` interface and forwards calls
to a real upstream node.

## Goal

Preserve the sidecar architecture while keeping the public gRPC contract
unchanged.

## What it does

- exposes the same `casper.v1.DeployService` method names
- observes `doDeploy` requests through `live_deploy_ingress`
- forwards the same deploy request to the real upstream `f1r3node`
- passes through the remaining `DeployService` calls unchanged

## What it does not do

- it does not modify `f1r3node`
- it does not rename or reshape upstream gRPC methods
- it does not change native Casper admission behavior

## Added pieces

- `crates/cordial-f1r3node-adapter/src/live_deploy_proxy.rs`
- `crates/cordial-f1r3node-adapter/src/bin/live_deploy_proxy.rs`
- `crates/cordial-f1r3node-adapter/src/bin/submit_live_deploy.rs`
- `crates/cordial-f1r3node-adapter/tests/test_live_deploy_proxy.rs`

## How it works

The proxy accepts a normal `doDeploy` request, records it in Cordial-side
observer state, and then forwards it upstream using the real
`DeployServiceClient`.

This means Cordial can see live deploy traffic without requiring any changes
inside the node itself.

## Run

Start a local `f1r3node` first so the proxy has a real upstream deploy
service to forward into.

Then start the proxy:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_deploy_proxy -- \
  --bind-addr 127.0.0.1:40411 \
  --upstream-grpc-url http://127.0.0.1:40401
```

Clients can then submit deploys to the proxy on `127.0.0.1:40411`, while the
proxy forwards those deploys to the real node on `127.0.0.1:40401`.

## Submit a live deploy

Use the deploy submitter binary to send a valid secp256k1-signed deploy
through the proxy:

```bash
cargo run -p cordial-f1r3node-adapter --bin submit_live_deploy -- \
  --grpc-url http://127.0.0.1:40411
```

By default this uses the same canonical secp256k1 deploy construction path as
`f1r3node`, so the upstream node can accept it as a real deploy rather than
rejecting it as a malformed test payload.

Useful optional flags:

- `--term` — override the submitted Rholang term
- `--phlo-price` — set the deploy phlo price
- `--phlo-limit` — set the deploy phlo limit
- `--shard-id` — choose the shard id
- `--valid-after-block-number` — set the deploy validity floor

Example:

```bash
cargo run -p cordial-f1r3node-adapter --bin submit_live_deploy -- \
  --grpc-url http://127.0.0.1:40411 \
  --term '@0!("hello cordial")'
```

## Expected result

When the path is working correctly, the submitter should print a successful
deploy response from the upstream node:

```text
Deploy accepted: Success!
DeployId is: ...
```

This proves:

- the client reached the Cordial proxy
- the proxy observed the `doDeploy` request
- the proxy forwarded the request upstream
- the upstream `f1r3node` accepted the deploy

## Why this matters

This is the cleanest deploy-side interception shape for the current project:

- Cordial remains an external sidecar
- the upstream node remains untouched
- method names and public API stay aligned with `f1r3node`
- future work can connect observed deploys to proposal and block inclusion
