# Cordial F1R3node

[![Rust CI](https://github.com/iCog-Labs-Dev/cordial-f1r3node/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/iCog-Labs-Dev/cordial-f1r3node/actions/workflows/rust.yml)
[![Lean checks](https://github.com/iCog-Labs-Dev/cordial-f1r3node/actions/workflows/lean.yml/badge.svg)](https://github.com/iCog-Labs-Dev/cordial-f1r3node/actions/workflows/lean.yml)
[![Benchmarks](https://github.com/iCog-Labs-Dev/cordial-f1r3node/actions/workflows/benchmarks.yml/badge.svg?branch=main)](https://github.com/iCog-Labs-Dev/cordial-f1r3node/actions/workflows/benchmarks.yml)
![Workspace crates](https://img.shields.io/badge/workspace-5%20crates-287b61)
![Benchmark targets](https://img.shields.io/badge/bench%20targets-19-b26a35)
![Property suites](https://img.shields.io/badge/property%20suites-5-476b91)
![Rust edition](https://img.shields.io/badge/Rust%20edition-2024-555555)

[Cordial Miners research paper](https://arxiv.org/abs/2205.09174)

A Rust implementation of Cordial Miners with Proof-of-Reputation (PoR)
weighting, a Lean verification model, and adapter-side integration with
`f1r3node-rust`. The live adapter observes node blocks and deploys, builds a
local blocklace, and exports finalized Cordial order to read-only consumers
and an application-event interface.

The workflow badges report their GitHub Actions runs, not test coverage. The
count badges track workspace members, registered benchmark targets, and
dedicated property-test suites; CI checks them against the repository. We do
not publish a code-coverage percentage.

## Workspace

| Crate | Purpose |
| --- | --- |
| `cordial-miners-core` | Blocklace DAG, validation, equivocation handling, weighted finality, and tau ordering |
| `cordial-por` | Reputation updates, weight export, and penalty state |
| `cordial-app-runtime` | Deterministic processing of finalized app events, receipts, cursors, and snapshots |
| `cordial-f1r3node-adapter` | Live block mirror, deploy observation, ordered-output export, and app-event extraction |
| `cordial-f1r3space-adapter` | RSpace/Rholang runtime and storage bridge to `f1r3node-rust` |

The consensus core remains separate from node-specific integration. The
adapter consumes public node block views and provides an observer-side path for
finality, ordering, and PoR experiments. The `lean/` project models protocol
properties and checks selected Rust decisions by replaying traces; it does not
generate or formally verify the Rust implementation.

## Quick start

Install the pinned Rust toolchain with
`rustup toolchain install nightly-2026-02-09 --component clippy --component rustfmt`
and install `protoc` (`sudo apt install protobuf-compiler` on Linux or
`brew install protobuf` on macOS). Cargo resolves path dependencies for the
workspace even when testing a single crate, so `f1r3node-rust` must be a
sibling checkout:

```bash
git clone https://github.com/iCog-Labs-Dev/cordial-f1r3node.git
cd cordial-f1r3node
git clone https://github.com/F1R3FLY-io/f1r3node-rust.git ../f1r3node-rust
git -C ../f1r3node-rust checkout 525f8e8b5419bac7b36a354c2325ef61ed73201f
cargo +nightly-2026-02-09 test --workspace
```

The node commit above matches Rust CI. The first workspace build also compiles
the node's Rholang and RSpace dependencies.

## Common checks

The `Justfile` provides these commands:

```bash
just test                     # Rust workspace tests
just ci                       # Rust checks and core boundary check
just lean-build               # Lean model and proofs (requires Lean/Lake)
just trace-conformance        # Rust traces checked by the Lean reference
just bench-smoke              # Benchmark fixture preflight
```

For live-node and four-validator Docker commands, use
[docker/README.md](docker/README.md). Live cluster checks are distinct from
the fast workspace tests. The bounded ordering verifier compares a recent
shared window, not the entire historical tau order.

## Documentation

- [Integration guide](docs/cordial-miners/integration/00-index.md): live mirror, deploy observation, ordered-output export, and recovery.
- [Four-node ordering check](docs/cordial-miners/integration/13-four-node-ordering-convergence.md): scope and commands for the bounded live comparison.
- [PoR shadow observer](docs/por-shadow-runner.md): checkpoint ratings, weight commitments, and restart checks.
- [Lean verification](lean/README.md): model, proofs, trace replay, and conformance limits.
- [Application interface](crates/cordial-app-runtime/docs/01-architecture.md): finalized events to app state.
- [Benchmarks](docs/BENCHMARKS.md): workloads, smoke checks, and reproducible results.

## Status

The connected four-validator cluster has produced matching bounded Cordial
window order. A separate four-observer PoR run agreed on a checkpoint, weight
map, and commitment across observer and validator restarts. The native mirror
also has restart recovery for persisted blocks and its height cursor. These
are specific integration checks, not a claim that every node execution path
has been formally verified or production-qualified.

Work and reviews are tracked in [GitHub issues](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues)
and [pull requests](https://github.com/iCog-Labs-Dev/cordial-f1r3node/pulls).
