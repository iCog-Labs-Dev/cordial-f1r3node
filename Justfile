set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

toolchain := "nightly-2025-06-15"

default:
    just --list

fmt:
    cargo +{{toolchain}} fmt -p cordial-miners-core -p cordial-miners-core -p cordial-f1r3node-adapter -p cordial-f1r3space-adapter --check

clippy:
    cargo +{{toolchain}} cordial-miners-core -p cordial-f1r3node-adapter -p cordial-f1r3space-adapter --all-targets --all-features --no-deps -- -D warnings

build:
    cargo +{{toolchain}} build --workspace

test:
    cargo +{{toolchain}} test --workspace

test-core:
    cargo +{{toolchain}} test -p cordial-miners-core

test-adapter:
    cargo +{{toolchain}} test -p cordial-f1r3node-adapter

test-consensus-flag:
    cargo +{{toolchain}} test -p cordial-f1r3node-adapter parses_consensus_flag_for_cordial_miners -- --exact --nocapture

test-cordial-startup:
    cargo +{{toolchain}} test -p cordial-f1r3node-adapter startup_with_cordial_mode_returns_cordial_stub -- --exact --nocapture

check-core-boundaries:
    ./scripts/check_core_boundaries.sh

ci:
    just fmt
    just clippy
    just build
    just test
    just check-core-boundaries
