# Cordial F1R3node

A Rust workspace for integrating **Cordial Miners** with the F1R3FLY node stack. The project is split into a pure consensus core crate and adapter crates for node/runtime integration.

## What it does

- `cordial-miners-core` provides core types, blocklace DAG structures, validation, fork choice, finality, execution abstractions, and generic consensus traits.
- `cordial-f1r3node-adapter` provides f1r3node-facing adapters, block translation, snapshot/shard config helpers, crypto bridge, and consensus-mode factory wiring.
- `cordial-f1r3space-adapter` provides real runtime/storage bridging for RSpace/Rholang integration against f1r3node crates.
- Boundary checks ensure node-specific imports do not leak into the pure core crate.

## Repo layout

This is a Cargo workspace with three crates, layered:

| Crate | What | Depends on |
|-------|------|------------|
| `crates/cordial-miners-core` | Pure consensus/core crate: DAG/block types, consensus logic, execution abstractions, generic engine traits | No f1r3node crates |
| `crates/cordial-f1r3node-adapter` | Node-facing adapter crate: translation, Casper adapter, snapshot/shard config, consensus mode selection | `cordial-miners-core`, optional f1r3node crates |
| `crates/cordial-f1r3space-adapter` | Storage/runtime adapter crate for real RSpace integration | `cordial-miners-core`, `cordial-f1r3node-adapter`, and f1r3node path deps |

If you only want the consensus protocol, depend on `cordial-miners-core`. If you want node-facing integration hooks, add `cordial-f1r3node-adapter`. If you want real Rholang/RSpace execution wiring, add `cordial-f1r3space-adapter`.

## Quick start

### Clone and run core tests

```bash
git clone  https://github.com/iCog-Labs-Dev/cordial-f1r3node.git
cd cordial-f1r3node
cargo +nightly-2025-06-15 test -p cordial-miners-core
```

This builds and tests the standalone core crate. No f1r3node checkout is required for this step.

### Full workspace including f1r3node integration

You'll need:

- `protoc` on PATH — `sudo apt install protobuf-compiler` (Linux) or `brew install protobuf` (Mac)
- f1r3node checked out at `../f1r3node` relative to this repo (i.e. they're sibling directories)

Then:

```bash
cargo +nightly-2025-06-15 test --workspace
```

First build takes about 5 minutes — it compiles f1r3node's full Rholang interpreter and RSpace tuplespace. Incremental builds are fast.

If something breaks during build, see the troubleshooting table in [`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md).

## Useful commands

This repo includes a `Justfile` with common workflows:

```bash
just build
just test
just test-core
just test-adapter
just test-consensus-flag
just check-core-boundaries
just ci
```

## Documentation

- **[`docs/implementation.md`](docs/implementation.md)** — what's implemented, organized by module. Start here if you want to understand the codebase.
- **[`docs/cordial-miners-vs-cbc-casper.md`](docs/cordial-miners-vs-cbc-casper.md)** — protocol comparison and integration roadmap. Read this if you're integrating into f1r3node or comparing the two consensus mechanisms.
- **[`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md)** — open tasks for new contributors, ordered by impact. Each task has scope, difficulty, and files to read.
- **[`crates/cordial-miners-core/src/network/NETWORK.md`](crates/cordial-miners-core/src/network/NETWORK.md)** — the P2P networking module's wire protocol and flow diagrams.
- **[`CONTRIBUTING.md`](CONTRIBUTING.md)** — how to set up your environment, run tests, structure commits, and submit changes.

## Status

Phases 1-3 of the integration roadmap are complete. The core consensus crate and adapter crates compile and test successfully, including consensus-mode feature-flag/factory wiring and core-boundary CI checks.

Outstanding work is tracked in [`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md). The most impactful next task is an end-to-end test that actually executes a Rholang deploy through the RSpace adapter — the translation layer is done, but we don't yet exercise it against a live RuntimeManager.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Short version: pick a task from `INTEGRATION_NEXT_STEPS.md`, branch off `master`, keep PRs scoped to one logical change, run `cargo test --workspace` before pushing.

## License

See [`LICENSE`](LICENSE) (if present) or contact the repository owners.
