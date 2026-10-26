# Cordial Miners demo

The earlier local-intercept demo used a modified `f1r3node` node with an
in-node `--consensus cordial-miners` selector. The current `f1r3node-rust`
checkout does not provide that selector. The local-intercept Compose file and
its verifier have been retired.

Use the [Docker runbook](docker/README.md) to build the native node image and
start the connected four-node cluster. Run the
[host-side ordering check](docs/cordial-miners/integration/13-four-node-ordering-convergence.md)
against that cluster. The [PoR shadow runner](docs/por-shadow-runner.md) can
observe the same finalized blocks.

The adapter conformance suite remains available without a live node:

```bash
cargo test -p cordial-f1r3node-adapter --test conformance
```
