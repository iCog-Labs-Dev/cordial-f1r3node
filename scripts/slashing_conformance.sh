#!/usr/bin/env bash
# Validate canonical Rust fixtures and independently replay the penalty boundary.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# Regeneration is always explicit, never an incidental effect of CI validation.
unset POR_REGENERATE_SLASHING
cargo +nightly-2026-02-09 test --locked -p cordial-por --test slashing_fixtures
(
  cd lean
  lake build LeanVerification slashing_replay
  lake exe slashing_replay
)
python3 scripts/tests/test_slashing_replay.py
