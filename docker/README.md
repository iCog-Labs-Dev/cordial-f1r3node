# Cordial Miners Docker Demo

This Docker folder runs the local Cordial Miners integration through the real
`f1r3node` binary.

The authoritative standalone path builds a local f1r3node image from two sibling
source trees:

- `../f1r3node` for the node, RSpace, API, and transport crates.
- `.` for the Cordial Miners core and adapter crates.

A registry or old local image is not enough unless it was built with the current
Cordial Miners integration.

## Files

| File | Purpose |
|---|---|
| `.env.example` | Development-only demo keys and image names |
| `Dockerfile` | Builds `f1r3node` with the sibling `cordial-f1r3node` path dependencies |
| `Dockerfile.dockerignore` | Keeps the parent build context restricted to the two required source trees |
| `standalone.yml` | Builds and starts one standalone node from the sibling `f1r3node` checkout plus this repository |
| `prebuilt-standalone.yml` | Optional shortcut for a local image that was already built from the current integration branch |
| `conformance.yml` | Runs the Cordial Miners conformance test suite inside Docker |
| `four-node-intercept.yml` | Starts four local f1r3node runtimes and verifies their Cordial ordered views match |
| `conf/cordial-standalone.conf` | Minimal standalone node config |
| `conf/cordial-four-node.conf` | Four-node local-intercept config for the KR convergence demo |
| `genesis/cordial-bonds.txt` | Bonds the demo validator public key |
| `genesis/cordial-wallets.txt` | Empty wallet file for the no-deploy standalone demo |
| `scripts/verify-four-node-order.sh` | Containerized four-node ordered-view verifier |

## Quick Commands

From the `cordial-f1r3node` repository root:

```bash
cp -n docker/.env.example docker/.env
docker compose --env-file docker/.env -f docker/standalone.yml config
docker compose --env-file docker/.env -f docker/conformance.yml run --rm cordial-conformance
docker compose --env-file docker/.env -f docker/standalone.yml up -d --build
curl -s http://127.0.0.1:40403/api/status | jq
curl -s -X POST http://127.0.0.1:40405/api/propose
curl -s http://127.0.0.1:40403/api/blocks/10 | jq
docker compose --env-file docker/.env -f docker/standalone.yml down -v
```

The `Justfile` wraps these commands as `just demo-cordial-*`.

For the four-node local-intercept KR demo:

```bash
just demo-cordial-up
just demo-cordial-four-node-config
just demo-cordial-four-node-up
just demo-cordial-four-node-verify
just demo-cordial-four-node-blocks
just demo-cordial-four-node-down
```

`just demo-cordial-up` is included first because it source-builds the local
`cordial-f1r3node:local` image used by the four-node compose file. If that image
already exists from the current integration branch, start at
`just demo-cordial-four-node-config`.

The four-node demo is a non-breaking logic prototype. It starts four local
f1r3node runtimes with `--consensus cordial-miners`, triggers the same local
proposal path on each runtime, then compares the ordered block views returned by
`/api/blocks/10`. It does not replace f1r3node peer discovery or the production
consensus networking layer.

The Dockerfile starts from `rust:slim-bookworm`, installs the f1r3node pinned
`nightly-2026-02-09` toolchain with retry support, and removes local
`rust-toolchain.toml` overrides before compiling. This keeps both copied source
trees on the same compiler inside Docker. Override `CORDIAL_RUST_BASE` and
`RUST_TOOLCHAIN` at build time if a pinned internal base image is available.
