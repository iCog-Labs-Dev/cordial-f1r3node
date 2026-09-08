#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
mutation_dir="$(mktemp -d)"
trap 'rm -rf -- "$mutation_dir"' EXIT

cd "$repo_root"
CORDIAL_TRACE_DIR="$mutation_dir" \
  cargo test -j 2 -p cordial-miners-core \
    --features trace-threshold-mutation \
    --test generate_trace_fixtures generate_weakened_threshold_fixture -- \
    --exact --nocapture --test-threads=1

set +e
replay_output="$({
  cd "$repo_root/lean"
  lake exe replay_runner --trace \
    "$mutation_dir/weakened_threshold.json" \
    "$mutation_dir/weakened_threshold.weights.json" \
    weakened_threshold
} 2>&1)"
replay_status=$?
set -e

printf '%s\n' "$replay_output"

if [[ $replay_status -eq 0 ]]; then
  echo "mutation failure: Lean accepted the weakened Rust threshold" >&2
  exit 1
fi

if ! grep -q "insufficient quorum" <<<"$replay_output"; then
  echo "mutation failure: Lean rejected for an unexpected reason" >&2
  exit 1
fi

echo "[mutation/weakened-threshold] rejected by independent Lean quorum ✓"
