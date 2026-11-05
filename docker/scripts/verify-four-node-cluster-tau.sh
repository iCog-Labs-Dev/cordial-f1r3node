#!/bin/sh
set -eu

evidence_root="${CORDIAL_TAU_EVIDENCE_DIR:-}"
if [ -n "$evidence_root" ]; then
  if [ -e "$evidence_root" ]; then
    echo "ERROR: evidence directory already exists: $evidence_root" >&2
    exit 1
  fi
  mkdir -p "$evidence_root"
else
  evidence_root="$(mktemp -d "${TMPDIR:-/tmp}/cordial-tau-evidence.XXXXXX")"
fi

finish() {
  result=$?
  trap - EXIT
  if [ "$result" -eq 0 ]; then outcome="PASS"; else outcome="FAIL"; fi
  jq -n --arg outcome "$outcome" --argjson exit_code "$result" \
    --arg finished_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{outcome: $outcome, exit_code: $exit_code, finished_at: $finished_at}' \
    >"$evidence_root/result.json"
  echo "Tau evidence: $evidence_root"
  exit "$result"
}
trap finish EXIT

bonds_file="${CORDIAL_ORDER_BONDS_FILE:-docker/genesis/cordial-bonds.txt}"
nodes_file="${CORDIAL_ORDER_NODES_FILE:-}"
min_new_heights="${CORDIAL_TAU_MIN_NEW_HEIGHTS:-12}"
progress_timeout="${CORDIAL_TAU_PROGRESS_TIMEOUT:-300}"
if [ ! -r "$bonds_file" ]; then
  echo "ERROR: cannot read bonds file: $bonds_file" >&2
  exit 1
fi

run_snapshot() {
  label="$1"
  CORDIAL_ORDER_BONDS_FILE="$bonds_file" \
    CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT=false \
    CORDIAL_ORDER_FRAGMENT_ONLY=true \
    CORDIAL_ORDER_HEIGHT_WINDOW=0 \
    CORDIAL_ORDER_TRUSTED_WINDOW_BOUNDARY=false \
    CORDIAL_ORDER_EVIDENCE_DIR="$evidence_root/$label" \
    ./docker/scripts/verify-four-node-cluster-ordering.sh \
    >"$evidence_root/$label.log" 2>&1 || {
      tail -50 "$evidence_root/$label.log" >&2
      return 1
    }
}

echo "Running first full-history finalized weighted tau snapshot"
run_snapshot first
first_height="$(jq -r '.target_height' "$evidence_root/first/inputs.json")"

# The second snapshot must observe new native blocks on every validator.
# The prefix check below also requires at least one new finalized tau block.
if [ -n "$nodes_file" ]; then
  progress_nodes="$nodes_file"
else
  progress_nodes="$evidence_root/first/nodes.txt"
fi
deadline="$(($(date +%s) + progress_timeout))"
while :; do
  common_height=""
  while read -r node _grpc_url http_url; do
    [ -n "${node:-}" ] || continue
    height="$(curl -fsS --max-time 10 "$http_url/api/blocks/16" |
      jq '[.[] | (.blockInfo // .) | .blockNumber] | max')"
    if [ "$height" = null ] || [ -z "$height" ]; then
      echo "ERROR: $node has no visible block height" >&2
      exit 1
    fi
    if [ -z "$common_height" ] || [ "$height" -lt "$common_height" ]; then
      common_height="$height"
    fi
  done < "$progress_nodes"
  printf '%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$common_height" \
    >>"$evidence_root/progress.tsv"
  if [ "$common_height" -ge "$((first_height + min_new_heights))" ]; then break; fi
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "ERROR: validators did not advance $min_new_heights common heights within ${progress_timeout}s" >&2
    exit 1
  fi
  sleep 5
done

echo "Running second full-history finalized weighted tau snapshot"
run_snapshot second

: >"$evidence_root/prefix-checks.jsonl"
while read -r node _grpc_url _http_url; do
  [ -n "${node:-}" ] || continue
  first="$evidence_root/first/$node.ordered.json"
  second="$evidence_root/second/$node.ordered.json"
  jq -c -n --arg node "$node" --slurpfile before "$first" --slurpfile after "$second" \
    '{node: $node, before_count: ($before[0] | length),
      after_count: ($after[0] | length),
      is_prefix: ($after[0][0:($before[0] | length)] == $before[0]),
      grew: (($after[0] | length) > ($before[0] | length))}' \
    >>"$evidence_root/prefix-checks.jsonl"
done < "$progress_nodes"
jq -s . "$evidence_root/prefix-checks.jsonl" >"$evidence_root/prefix-checks.json"
if ! jq -e 'length == 4 and all(.[]; .is_prefix and .grew)' \
  "$evidence_root/prefix-checks.json" >/dev/null; then
  echo "ERROR: finalized tau output did not grow as an append-only prefix on all four mirrors" >&2
  exit 1
fi

echo "PASS: four mirrors agreed on nonempty finalized weighted tau at both snapshots, and each output grew by prefix extension."
