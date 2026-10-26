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

Start a local `f1r3node-rust` first so the proxy has a real upstream deploy
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

Use the standalone development account from the sibling checkout's fixture.
The submitter reads its key from an environment variable and never prints it:

```bash
export STANDALONE_PRIVATE_KEY="$(sed -n 's/^STANDALONE_PRIVATE_KEY=//p' ../f1r3node-rust/docker/.env.example)"
cargo run -p cordial-f1r3node-adapter --bin submit_live_deploy -- \
  --grpc-url http://127.0.0.1:40411 \
  --private-key-env STANDALONE_PRIVATE_KEY
```

The same command can submit directly to the standalone node with
`--grpc-url http://127.0.0.1:40401`. The key must belong to an account funded
in that node's genesis. The submitter constructs and signs the deploy with the
same Casper helper used by `f1r3node-rust`.

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
  --private-key-env STANDALONE_PRIVATE_KEY \
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

Acceptance does not prove execution or canonical finalization. Copy the
returned deploy ID and check both endpoints:

```bash
DEPLOY_ID="paste-the-hex-id-printed-by-submit_live_deploy"
curl -fsS "http://127.0.0.1:40403/api/deploy/$DEPLOY_ID" | jq '{blockHash, blockNumber, errored, cost}'
curl -fsS "http://127.0.0.1:40403/api/deploy-finalization-status/$DEPLOY_ID" | jq .
```

Wait for `state` to become `Finalized`. A `Failed` state can follow a
successful `doDeploy` response when the account cannot pay. To check the
native mirror, run `live_native_mirror --once` with the same `--data-dir` used
before the deploy, then run it again to verify recovery. On 2026-10-17, a
funded standalone deploy sent through direct gRPC executed with cost 200 in
block 127 and reached canonical `Finalized` status. The mirror persisted that
block and its deploy signature, then recovered them after restart.

The proxy path was checked on the same node on 2026-10-17. A funded deploy
submitted to `127.0.0.1:40411` executed with cost 197 in block 161 and
reached canonical `Finalized` status. The native mirror persisted the block's
hash mapping and deploy signature and retained both after restart. The four
`test_live_deploy_proxy` tests pass, including the observer and forwarding
test. The live proxy currently has no endpoint to inspect its in-memory
observed-deploy list.

## Why this matters

This is the cleanest deploy-side interception shape for the current project:

- Cordial remains an external sidecar
- the upstream node remains untouched
- method names and public API stay aligned with `f1r3node`
- future work can connect observed deploys to proposal and block inclusion
