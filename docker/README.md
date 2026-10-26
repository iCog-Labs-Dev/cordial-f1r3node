# Docker development cluster

These images build the native `f1r3node-rust` node. Cordial and PoR run as
host-side observers of its finalized blocks. The node keeps its own consensus;
these Compose files do not enable in-node Cordial consensus or enforce PoR
weights.

Place the checkouts beside each other:

```text
cordial-miners/
├── cordial-f1r3node/
└── f1r3node-rust/
```

The Docker build context is their parent directory. `Dockerfile.dockerignore`
limits the context to the source required by both Rust workspaces. The node
includes its default configuration and genesis resources in the binary. The
Compose files mount only local overrides and node data.

## Build and check the image

Run these commands from the `cordial-f1r3node` root:

```bash
test -e docker/.env || cp docker/.env.example docker/.env
docker compose --env-file docker/.env -f docker/standalone.yml build cordial-standalone
just demo-cordial-image-check
```

The first build compiles the release node from the pinned `f1r3node-rust`
checkout. `just demo-cordial-image-check` checks the native node CLI in the
built image. CI builds the same runtime image, in addition to checking the
Compose syntax.

## One node

```bash
just demo-cordial-up
just demo-cordial-wait
just demo-cordial-blocks
just demo-cordial-down
```

`demo-cordial-up` builds the image. Use `demo-cordial-up-prebuilt` after a
successful build if you want to start the existing local image. The standalone
node publishes ports `40400` through `40405` on the host. Stop the node before
starting another service on those ports.

Run adapter conformance separately with `just demo-cordial-conformance`. The
`docker/conformance.yml` service uses the same two source trees and runs the
adapter tests inside the build image.

## Connected four-node cluster

This path starts one bootstrap node and four bonded validators. Each node has
its own data volume. The cluster publishes HTTP and gRPC ports on `127.0.0.1`.

```bash
./docker/scripts/generate-four-node-cluster-certs.sh
just demo-cordial-four-node-cluster-config
just demo-cordial-four-node-cluster-up
just demo-cordial-four-node-cluster-wait
just demo-cordial-four-node-cluster-verify
just demo-cordial-four-node-cluster-ordering
```

The certificate script reads the bootstrap key and certificate from the
`f1r3node-rust` sibling. It keeps existing validator TLS keys. The bootstrap
certificate must match `CORDIAL_BOOTSTRAP_NODE_ID` in `docker/.env`.

The cluster verifier checks network membership and a shared finalized block
window. The ordering script runs on the host. It mirrors the same recent block
window from the four validator gRPC endpoints and compares the Cordial view.
See [the convergence check](../docs/cordial-miners/integration/13-four-node-ordering-convergence.md)
for its parameters. Run the PoR shadow check separately as described in
[the PoR shadow runner guide](../docs/por-shadow-runner.md).

Stop and restart without deleting node data:

```bash
just demo-cordial-four-node-cluster-down
just demo-cordial-four-node-cluster-up
```

Run `just demo-cordial-four-node-cluster-reset` only when you want fresh node
data and a new genesis. The stopped cluster keeps five named volumes.

The local image tag defaults to `cordial-f1r3node:local`. Build it before you
start the cluster. `just demo-cordial-image-check` rejects an image that still
advertises the old `--consensus` flag.

## Disk use

The cluster rotates Docker logs at 20 MB per file, with three files per node.
The named node volumes hold the blockchain data. Measure both while the nodes
run:

```bash
docker system df -v
docker ps --size --filter name=cordial
sudo sh docker/scripts/measure-four-node-disk.sh 300 \
  /tmp/cordial-cluster-disk-report "$HOME/cordial-por-four-node"
```

The measurement script reports Docker logs, node data, writable layers, free
space, and optional PoR shadow data before and after the interval. It does not
stop containers or delete data.

## Files

| File | Purpose |
|---|---|
| `Dockerfile` and `Dockerfile.dockerignore` | Build the native node and adapter conformance image from sibling checkouts |
| `standalone.yml` and `prebuilt-standalone.yml` | Run one native node |
| `conformance.yml` | Run adapter conformance tests in Docker |
| `four-node-cluster.yml` | Run the connected bootstrap and validator cluster |
| `conf/cordial-standalone.conf` | Standalone node overrides |
| `conf/cordial-four-node-cluster.conf` | Connected-cluster overrides |
| `scripts/generate-four-node-cluster-certs.sh` | Prepare cluster TLS material |
| `scripts/verify-four-node-cluster.sh` | Check connected-cluster finality and agreement |

The old local-intercept demo used a modified node and an in-node
`--consensus cordial-miners` flag. That mode is not part of `f1r3node-rust`.
The connected cluster and host-side ordering script replace that demo.
