# Cordial Miners

A Rust implementation of the **Cordial Miners** consensus protocol, built around a blocklace DAG. Designed to run standalone as a consensus library, or as a pluggable alternative to CBC Casper inside [f1r3node](https://github.com/F1R3FLY-io/f1r3node).

## What it does

- Implements the Cordial Miners consensus from [arXiv:2205.09174](https://arxiv.org/abs/2205.09174): a Byzantine fault-tolerant DAG consensus where validators reference all known tips (the "cordial condition") rather than picking a single fork.
- Provides supermajority finality (> 2/3 of honest stake), structural equivocation detection via the chain axiom, and a deploy pool for transaction selection.
- Integrates with f1r3node via a Casper trait adapter, so Cordial Miners blocks can drive f1r3node's existing engine, proposer, and block processor.
- Optionally executes Rholang against a real RSpace tuplespace through f1r3node's `RuntimeManager`.

## Repo layout

This is a Cargo workspace with three crates, layered:

| Crate | What | Depends on |
|-------|------|------------|
| `crates/blocklace` | Standalone consensus library: blocks, fork choice, finality, validation, deploy pool, P2P networking | Nothing f1r3node-related |
| `crates/blocklace-f1r3node` | Casper trait adapter, block translation, snapshot construction, crypto bridge | `blocklace` only (uses mirror types of f1r3node) |
| `crates/blocklace-f1r3rspace` | Real `RuntimeManager` adapter delegating to f1r3node's RSpace | `blocklace`, `blocklace-f1r3node`, and 6 f1r3node crates as path deps |

If you only want the consensus protocol, depend on `blocklace`. If you want f1r3node integration without the full RSpace runtime, add `blocklace-f1r3node`. If you want actual Rholang execution, add `blocklace-f1r3rspace` (which also pulls in f1r3node's heavy dependency tree).

## Quick start

### Just the consensus library

```bash
git clone  https://github.com/iCog-Labs-Dev/cordial-f1r3node.git
cd blocklace
cargo test -p blocklace
```

This builds and tests the standalone consensus library. No f1r3node checkout required.

### Full workspace including f1r3node integration

You'll need:

- `protoc` on PATH — `sudo apt install protobuf-compiler` (Linux) or `brew install protobuf` (Mac)
- f1r3node checked out at `../f1r3node` relative to this repo (i.e. they're sibling directories)

Then:

```bash
cargo test --workspace
```

First build takes about 5 minutes — it compiles f1r3node's full Rholang interpreter and RSpace tuplespace. Incremental builds are fast.

If something breaks during build, see the troubleshooting table in [`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md).

## Documentation

- **[`docs/implementation.md`](docs/implementation.md)** — what's implemented, organized by module. Start here if you want to understand the codebase.
- **[`docs/cordial-miners-vs-cbc-casper.md`](docs/cordial-miners-vs-cbc-casper.md)** — protocol comparison and integration roadmap. Read this if you're integrating into f1r3node or comparing the two consensus mechanisms.
- **[`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md)** — open tasks for new contributors, ordered by impact. Each task has scope, difficulty, and files to read.
- **[`crates/blocklace/src/network/NETWORK.md`](crates/blocklace/src/network/NETWORK.md)** — the P2P networking module's wire protocol and flow diagrams.
- **[`CONTRIBUTING.md`](CONTRIBUTING.md)** — how to set up your environment, run tests, structure commits, and submit changes.

## Status

Phases 1–3 of the integration roadmap are complete. The standalone consensus is fully implemented and tested; the f1r3node Casper adapter is in place; the RSpace runtime adapter compiles against real f1r3node types. **252 tests across the workspace.**

Outstanding work is tracked in [`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md). The most impactful next task is an end-to-end test that actually executes a Rholang deploy through the RSpace adapter — the translation layer is done, but we don't yet exercise it against a live RuntimeManager.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Short version: pick a task from `INTEGRATION_NEXT_STEPS.md`, branch off `master`, keep PRs scoped to one logical change, run `cargo test --workspace` before pushing.

## License

See [`LICENSE`](LICENSE) (if present) or contact the repository owners.
