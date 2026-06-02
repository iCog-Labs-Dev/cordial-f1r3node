# Cordial Miners Demo

The demo covers two layers:

1. The deterministic Cordial Miners conformance tests, which exercise the
   protocol math and adapter path without a live node.
2. A live standalone f1r3node process, built from the sibling `../f1r3node`
   checkout and this Cordial Miners workspace, started with the Cordial Miners
   consensus selector.
3. A four-runtime Docker demo that starts four local f1r3node processes,
   intercepts the same local proposal stream through the Cordial adapter, and
   verifies that all four expose the exact same ordered view.

## Repository Layout

Run all commands from:

```bash
cd /home/matania/Desktop/rho-calculus/cordial-f1r3node
```

The Docker build expects this sibling layout:

```text
/home/matania/Desktop/rho-calculus/
  f1r3node/
  cordial-f1r3node/
```

This matters because `f1r3node/node/Cargo.toml` path-depends on the Cordial
Miners crates in `../cordial-f1r3node`. A generic registry image is not enough
for this demo unless it was built with those local path dependencies.

## Demo Files

| File | Purpose |
|---|---|
| `docker/.env.example` | Demo image names, token metadata, and development validator key |
| `docker/Dockerfile` | Builds a local f1r3node binary with this Cordial Miners workspace |
| `docker/Dockerfile.dockerignore` | Keeps the parent build context from sending `target/`, `.git/`, and `node_modules/` |
| `docker/standalone.yml` | Builds and runs one node from `../f1r3node` plus this workspace |
| `docker/prebuilt-standalone.yml` | Optional shortcut for a local image that was already built from the current integration branch |
| `docker/conformance.yml` | Runs the Cordial Miners conformance test suite in Docker |
| `docker/four-node-intercept.yml` | Runs four local f1r3node runtimes and a verifier for the KR 3 convergence demo |
| `docker/conf/cordial-standalone.conf` | Minimal standalone node config |
| `docker/conf/cordial-four-node.conf` | Four-node local-intercept config using the Cordial Miners selector |
| `docker/genesis/cordial-bonds.txt` | Bonds the demo validator public key so it can propose |
| `docker/genesis/cordial-wallets.txt` | Empty wallet file for no-deploy startup/propose smoke demos |
| `docker/scripts/verify-four-node-order.sh` | Containerized verifier that compares all four nodes' ordered block views |
| `Justfile` | Short demo commands under `just demo-cordial-*` |

## Requirements

Install these on the host:

```bash
docker --version
docker compose version
just --version
jq --version
curl --version
```

The local Rust build inside Docker installs its own native build packages and
the f1r3node pinned `nightly-2026-02-09` Rust toolchain. No host Rust toolchain
is required for the Docker path. The Dockerfile starts from `rust:slim-bookworm`
and installs the pinned nightly with retry support because the workspace pulls
dependencies that require nightly.

## Demo 1: Render The Docker Configuration

Create a local environment file:

```bash
just demo-cordial-env
```

Validate the compose files:

```bash
just demo-cordial-config
```

Expected result:

```text
Cordial Miners compose files are valid.
```

Why this matters: rendering the config catches path, YAML, and environment
mistakes before the node starts or an expensive Rust source-build begins.

## Demo 2: Run The Cordial Miners Conformance Suite In Docker

```bash
just demo-cordial-conformance
```

Expected result: the `cordial-f1r3node-adapter` conformance test binary runs and
the scenarios pass. This is the protocol sanity check:

- honest majority finalizes the expected leader,
- equivocation is rejected from super-ratification and yields slash evidence,
- extending the DAG leaves the previously calculated tau prefix invariant.

Why this matters: this proves the Cordial Miners paper-level behavior through
the adapter boundary before starting the live f1r3node runtime.

If Docker image pulls are slow, use the same test target on the host:

```bash
just demo-cordial-conformance-local
```

## Demo 3: Build And Start A Live Cordial Miners Node

```bash
just demo-cordial-up
```

This builds `cordial-f1r3node:local` and starts one standalone node with:

```text
--consensus cordial-miners
--validator-private-key 0101010101010101010101010101010101010101010101010101010101010101
```

For a quick check of an already-built local image, run:

```bash
just demo-cordial-image-check
just demo-cordial-up-prebuilt
```

Use the prebuilt shortcut only when that image was built from the current
integration branch. The authoritative demo command is `just demo-cordial-up`.

Wait for the HTTP API to respond:

```bash
just demo-cordial-wait
```

Expected log markers:

```bash
just demo-cordial-logs
```

Look for:

```text
Cordial Miners consensus selected; using Cordial adapter for deploy, propose, and block queries
Launching Cordial Miners through the selected consensus adapter
Skipping Casper engine initialization for selected consensus
All tasks started. Node is now running.
```

Why this matters: those lines prove f1r3node did not fall back to CBC Casper.
The node selected the Cordial adapter and skipped the Casper launch/engine tasks.

## Demo 4: Check Node Status

```bash
just demo-cordial-status
```

Expected fields:

```json
{
  "networkId": "cordial-demo",
  "peers": 0,
  "nodes": 0,
  "isValidator": true
}
```

`peers: 0` and `nodes: 0` are correct for this standalone demo. They mean the
node has no other peers in its discovery table; they do not mean the consensus
adapter failed.

## Demo 5: Propose A Cordial Miners Block

Force a proposal through the admin HTTP API:

```bash
just demo-cordial-propose
```

Expected result:

```text
Success! Block <hash> created and added.
```

Then query recent blocks:

```bash
just demo-cordial-blocks
```

Expected result: at least one block is returned. For an empty-deploy smoke test,
`preStateHash` and `postStateHash` can be the same because RSpace had no user
deploys to execute. That is normal; a state hash changes only when deploys or
system deploys change state.

## Demo 6: Stop And Reset

```bash
just demo-cordial-down
```

This stops the node and removes the standalone/conformance demo volumes so the
next demo starts from a clean block store and does not retain large Cargo target
caches.

## Demo 7: Four Local Nodes Produce The Same Order

This is the last KR demo:

```text
Run a 4-node local cluster and verify that all nodes produce the exact same
total order for a set of intercepted transactions.
```

Start four local f1r3node runtimes from the already-built Cordial image:

```bash
just demo-cordial-four-node-up
```

The four nodes expose their HTTP/admin APIs on separate host port ranges:

| Node | HTTP API | Admin API |
|---|---:|---:|
| `cordial-node-1` | `51403` | `51405` |
| `cordial-node-2` | `52403` | `52405` |
| `cordial-node-3` | `53403` | `53405` |
| `cordial-node-4` | `54403` | `54405` |

Check that all four selected Cordial Miners and are bonded validators:

```bash
just demo-cordial-four-node-status
```

Expected fields for every node:

```json
{
  "networkId": "cordial-demo-four-node",
  "isValidator": true,
  "peers": 0,
  "nodes": 0
}
```

Run the verifier:

```bash
just demo-cordial-four-node-verify
```

Expected result:

```text
PASS: four local f1r3node runtimes produced the same Cordial Miners ordered view.
```

The verifier performs the KR check end to end:

1. Waits for all four f1r3node HTTP APIs.
2. Confirms every runtime is on `cordial-demo-four-node` and is a validator.
3. Calls the existing admin proposal endpoint on each runtime.
4. Reads `/api/blocks/10` from all four runtimes.
5. Compares the ordered block views exactly: block number, hash, sender,
   sequence number, deploy count, and finalization flag.

This is intentionally a non-breaking local-intercept prototype. The four
containers use the same bonded demo validator identity so the same intercepted
input stream produces byte-identical proposed blocks. The separate conformance
suite is the four-validator Cordial Miners math proof for honest majority,
equivocation rejection, slash evidence, and tau prefix invariance.

Inspect the ordered views yourself:

```bash
just demo-cordial-four-node-blocks
```

Stop and reset the four-node demo:

```bash
just demo-cordial-four-node-down
```

## Local Non-Docker Shortcut

If the sibling `../f1r3node` checkout already builds on the host, this starts
the same node without Docker:

```bash
just demo-cordial-local-clean
just demo-cordial-local-node
```

Use the same HTTP/admin commands from Demo 4 and Demo 5.

## Troubleshooting

If `just demo-cordial-config` fails, fix the compose/YAML issue before building.

If Docker build cannot find `f1r3node`, verify the sibling layout:

```bash
ls ../f1r3node/node/Cargo.toml
```

If the node logs say the configured Cordial Miners validator key is not present
in the bonds file, verify `docker/genesis/cordial-bonds.txt` contains the public
key from `docker/.env`.

If `nodes` and `peers` stay at zero in standalone mode, that is expected. Use a
multi-node shard when peer discovery itself is the demonstration target.

In the four-node KR demo, `nodes` and `peers` can also remain zero because the
prototype intentionally compares local Cordial adapter output without replacing
f1r3node's production peer discovery or consensus networking. The verifier is
the source of truth for this demo: all four runtimes must expose the same
ordered view.

If `preStateHash` and `postStateHash` are identical, submit a user deploy or
generate slash evidence before proposing. Empty blocks do not change RSpace
state.

## Validation Progress In This Checkout

This section is updated as the demo commands are run from this repository.

## Commit And CI Trace

The demo work is split into atomic semantic commits on the `integrations`
branch:

| Commit type | Scope | Purpose |
|---|---|---|
| `fix(just)` | Rust verification recipes | Repairs the existing `fmt` and `clippy` recipes so local CI commands call the intended tools. |
| `chore(docker)` | Demo infrastructure | Adds Dockerfiles, compose files, configs, genesis fixtures, and the four-node ordered-view verifier. |
| `build(just)` | Demo commands | Adds `just demo-cordial-*` commands for config validation, conformance, live node startup, four-node verification, logs, and cleanup. |
| `docs(demo)` | Runbooks | Documents the standalone Cordial demo, four-node local-intercept demo, troubleshooting notes, and validation evidence. |

Before pushing this branch, run:

```bash
just ci
git diff --check
just demo-cordial-config
just demo-cordial-four-node-config
```

For the Docker-backed KR proof, also run:

```bash
just demo-cordial-four-node-up
just demo-cordial-four-node-verify
just demo-cordial-four-node-down
```

| Command | Result |
|---|---|
| `just --list` | Passed. All `demo-cordial-*` recipes are registered. |
| `just demo-cordial-config` | Passed. `standalone.yml`, `prebuilt-standalone.yml`, `conformance.yml`, and `four-node-intercept.yml` render successfully. |
| `just demo-cordial-four-node-config` | Passed. The four-node compose file renders successfully after adding the verifier service and moving host ports to `514xx`, `524xx`, `534xx`, and `544xx`. |
| `just demo-cordial-four-node-up` | Passed. First attempt exposed a real local conflict on `41402`; the demo was corrected to the `51xxx`-`54xxx` ranges, partial containers were removed with `just demo-cordial-four-node-down`, and the retry started all four nodes. |
| `just demo-cordial-four-node-wait` and `just demo-cordial-four-node-status` | Passed. All four nodes reported `networkId=cordial-demo-four-node`, `isValidator=true`, `peers=0`, `nodes=0`, and `lastFinalizedBlockNumber=-1` before proposing. |
| `just demo-cordial-four-node-verify` | Passed. The verifier triggered local proposals on all four runtimes and every node produced ordered block view `[e07e1f6821c7786c4f6e159aa028cc991c4ddfe95182ffd9694b42e41a8a29e9]`. |
| `just demo-cordial-four-node-blocks` | Passed. Host-facing block queries on `51403`, `52403`, `53403`, and `54403` all returned the same finalized block projection: block number `0`, sender `041b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f70beaf8f588b541507fed6a642c5ab42dfdf8120a7f639de5122d47a69a8e8d1`, sequence number `0`, deploy count `0`, and finalized `true`. |
| `just demo-cordial-four-node-down` | Passed. Four-node containers and volumes were removed after verification. |
| `just demo-cordial-conformance-local` | Passed locally: 3 scenarios passed. |
| `just demo-cordial-conformance` | Passed in Docker: 3 scenarios passed. Fixes made from the failed runs: absolute Cargo path inside the container, `libprotobuf-dev` for `google/protobuf/descriptor.proto`, pinned nightly instead of stable, retry support for `rustup`, and `CARGO_INCREMENTAL=0` to avoid filling Docker storage with incremental query caches. |
| `just demo-cordial-image-check` | Passed after the source build for `cordial-f1r3node:local`; it advertises `--consensus cordial-miners`. The recipe captures `run --help` before `grep`, so it no longer fails with `write /dev/stdout: broken pipe`. This is only a shortcut check. |
| `just demo-cordial-up` | Passed. Built `cordial-f1r3node:local` from `../f1r3node` plus this workspace in Docker, then started `cordial.standalone`. The release `node` build completed in the source image and the container came up on `40400-40405`. |
| `just demo-cordial-up-prebuilt` | Passed after `CORDIAL_F1R3NODE_IMAGE` was pointed at the source-built `cordial-f1r3node:local` image. The shortcut node responded to `/api/status` with `networkId=cordial-demo` and `isValidator=true`. |
| `just demo-cordial-local-node` plus status/propose/blocks | Passed earlier in this checkout with the current host-built f1r3node binary: the node selected Cordial Miners, skipped Casper engine initialization, accepted an admin propose request, and returned the proposed block from `/api/blocks/10`. |
| `just demo-cordial-wait` | Passed against the source-built Docker node. HTTP status responded with `networkId=cordial-demo`, `isValidator=true`, `peers=0`, `nodes=0`, and `lastFinalizedBlockNumber=-1` before proposing. |
| `just demo-cordial-logs` | Passed. Logs showed Cordial Miners blocklace recovery, query-state seeding, `Cordial Miners consensus selected; using Cordial adapter for deploy, propose, and block queries`, `Launching Cordial Miners through the selected consensus adapter`, and Casper engine/task skips for `cordial-miners`. |
| `just demo-cordial-propose` against Docker | Passed. Created block `e07e1f6821c7786c4f6e159aa028cc991c4ddfe95182ffd9694b42e41a8a29e9`. |
| `just demo-cordial-status` against Docker | Passed after proposing. `lastFinalizedBlockNumber=0`, `isValidator=true`, `peers=0`, `nodes=0`. |
| `just demo-cordial-blocks` against Docker | Passed. Returned the signed finalized Cordial block from the bonded validator key with `sigAlgorithm=secp256k1`, `deployCount=0`, and matching pre/post state hashes for the empty-deploy smoke block. |
| `just demo-cordial-local-clean` | Passed. Fresh data dir: `/tmp/cordial-f1r3node-demo`. |
| `just demo-cordial-local-node` | Passed. Startup logs showed `Cordial Miners consensus selected; using Cordial adapter for deploy, propose, and block queries`, repository recovery, query-state seeding, Casper engine skip, and `All tasks started. Node is now running.` |
| `just demo-cordial-status` | Passed against the host-local node. `networkId=cordial-demo`, `isValidator=true`, `peers=0`, `nodes=0`. |
| `just demo-cordial-propose` | Passed. Created block `e07e1f6821c7786c4f6e159aa028cc991c4ddfe95182ffd9694b42e41a8a29e9`. |
| `just demo-cordial-blocks` | Passed. Returned the signed finalized Cordial block with the bonded validator public key. |
| `just demo-cordial-down` | Passed. Standalone container, standalone data volume, and conformance cache volumes are removed after cleanup. |
