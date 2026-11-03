#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
benchmark_dir="$(mktemp -d)"
trap 'rm -rf -- "$benchmark_dir"' EXIT

cd "$repo_root"
CORDIAL_TRACE_DIR="$benchmark_dir" \
  cargo test -j 2 -p cordial-miners-core --features trace \
    --test generate_trace_fixtures generate_benchmark_15_fixture -- \
    --exact --ignored --nocapture --test-threads=1

trace="$benchmark_dir/benchmark_15.json"
weights="$benchmark_dir/benchmark_15.weights.json"
echo "[benchmark_15] trace lines=$(wc -l < "$trace") bytes=$(wc -c < "$trace")"

cd "$repo_root/lean"
timeout_seconds="${ISSUE188_BENCHMARK_TIMEOUT:-30}"
set +e
/usr/bin/time -f '[benchmark_15] replay elapsed=%e s max_rss=%M KB' \
  timeout "${timeout_seconds}s" lake exe replay_runner --trace "$trace" "$weights" benchmark_15
status=$?
set -e
if [[ $status -eq 124 ]]; then
  echo "[benchmark_15] replay exceeded ${timeout_seconds}s safety timeout" >&2
fi
exit "$status"
