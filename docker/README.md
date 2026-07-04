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
| `four-node-intercept.yml` | Starts four local f1r3node runtimes in local-intercept mode and verifies their Cordial ordered views match |
| `four-node-cluster.yml` | Starts a real connected local cluster: bootstrap + four validators + verifier |
| `conf/cordial-standalone.conf` | Minimal standalone node config |
| `conf/cordial-four-node.conf` | Four-node local-intercept config for the KR convergence demo |
| `conf/f1r3node-defaults.conf` | Upstream runtime defaults expected by the Rust node binary |
| `conf/f1r3node-kamon.conf` | Upstream metrics/runtime config expected by the Rust node binary |
| `genesis/cordial-bonds.txt` | Bonds the four demo validator public keys |
| `genesis/cordial-wallets.txt` | Empty wallet file for the no-deploy standalone demo |
| `scripts/generate-four-node-cluster-certs.sh` | Generates local EC TLS certs for bootstrap + validators |
| `scripts/verify-four-node-order.sh` | Containerized four-node ordered-view verifier for the local-intercept demo |
| `scripts/verify-four-node-cluster.sh` | Containerized verifier for the real connected four-node cluster |

## Quick Commands

From the repository root:

```bash
cp -n docker/.env.example docker/.env
docker-compose --env-file docker/.env -f docker/standalone.yml config
docker-compose --env-file docker/.env -f docker/conformance.yml run --rm cordial-conformance
docker-compose --env-file docker/.env -f docker/standalone.yml up -d --build
curl -s http://127.0.0.1:40403/api/status | jq
curl -s -X POST http://127.0.0.1:40405/api/propose
curl -s http://127.0.0.1:40403/api/blocks/10 | jq
docker-compose --env-file docker/.env -f docker/standalone.yml down -v
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
`f1r3node` runtimes with `--consensus cordial-miners`, each using its own bonded
validator identity, then compares the ordered block views returned by
`/api/blocks/10`. It does not replace `f1r3node` peer discovery or the
production consensus networking layer.

For the connected four-node cluster demo:

```bash
./docker/scripts/generate-four-node-cluster-certs.sh
just demo-cordial-four-node-cluster-config
just demo-cordial-four-node-cluster-up
just demo-cordial-four-node-cluster-wait
just demo-cordial-four-node-cluster-status
just demo-cordial-four-node-cluster-verify
just demo-cordial-four-node-cluster-blocks
just demo-cordial-four-node-cluster-down
```

This path is heavier than the local-intercept demo. It launches:

- one bootstrap node
- four bonded validators with distinct validator keys
- a verifier that checks:
  - all validators joined the expected network
  - validators are bonded
  - validators are not isolated (`peers` / `nodes` visibility)
  - finalized ordered views converge

The real-cluster path now depends on local EC TLS certificates for each node.
They are generated into `docker/certs/` by
`docker/scripts/generate-four-node-cluster-certs.sh` and are intentionally kept
out of git.

## Docker CLI Compatibility

Some environments provide the legacy `docker-compose` command instead of
`docker compose`. If a `Justfile` recipe fails with an error such as
`unknown flag: --env-file`, run the equivalent command directly with
`docker-compose`.

Example cluster shutdown:

```bash
docker-compose --env-file docker/.env -f docker/four-node-cluster.yml down -v
```

The `cp -n` warning that may appear while creating `docker/.env` is harmless.

If you rebuild the Docker image after changing runtime packaging, use:

```bash
docker-compose --env-file docker/.env -f docker/standalone.yml build cordial-standalone
```

The Dockerfile starts from `rust:slim-bookworm`, installs the f1r3node pinned
`nightly-2026-02-09` toolchain with retry support, and removes local
`rust-toolchain.toml` overrides before compiling. This keeps both copied source
trees on the same compiler inside Docker. Override `CORDIAL_RUST_BASE` and
`RUST_TOOLCHAIN` at build time if a pinned internal base image is available.
