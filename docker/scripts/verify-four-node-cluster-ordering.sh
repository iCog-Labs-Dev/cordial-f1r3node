#!/bin/sh
set -eu

depth="${CORDIAL_ORDER_DEPTH:-64}"
height_batch_size="${CORDIAL_ORDER_HEIGHT_BATCH_SIZE:-64}"
height_window="${CORDIAL_ORDER_HEIGHT_WINDOW:-256}"
fragment_only="${CORDIAL_ORDER_FRAGMENT_ONLY:-true}"
trusted_window_boundary="${CORDIAL_ORDER_TRUSTED_WINDOW_BOUNDARY:-true}"
window_ordering_fragment="${CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT:-true}"
node_timeout="${CORDIAL_ORDER_NODE_TIMEOUT:-300}"
bonds_file="${CORDIAL_ORDER_BONDS_FILE:-}"
evidence_dir="${CORDIAL_ORDER_EVIDENCE_DIR:-}"
max_height_lag="${CORDIAL_ORDER_MAX_HEIGHT_LAG:-8}"
started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if [ -n "$evidence_dir" ]; then
  if [ -e "$evidence_dir" ]; then
    echo "ERROR: evidence directory already exists: $evidence_dir" >&2
    exit 1
  fi
  mkdir -p "$evidence_dir"
  tmp_dir="$evidence_dir"
else
  tmp_dir="$(mktemp -d)"
fi

finish() {
  result=$?
  trap - EXIT
  if [ -n "$evidence_dir" ]; then
    if [ "$result" -eq 0 ]; then outcome="PASS"; else outcome="FAIL"; fi
    jq -n --arg outcome "$outcome" --argjson exit_code "$result" \
      --arg finished_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      '{outcome: $outcome, exit_code: $exit_code, finished_at: $finished_at}' \
      >"$tmp_dir/result.json"
  else
    rm -rf "$tmp_dir"
  fi
  exit "$result"
}
trap finish EXIT

if [ "$window_ordering_fragment" = "false" ]; then
  if [ "$fragment_only" != "true" ] || [ -z "$bonds_file" ]; then
    echo "ERROR: finalized tau mode requires CORDIAL_ORDER_FRAGMENT_ONLY=true and CORDIAL_ORDER_BONDS_FILE" >&2
    exit 1
  fi
fi
if [ -n "$bonds_file" ] && [ ! -r "$bonds_file" ]; then
  echo "ERROR: cannot read CORDIAL_ORDER_BONDS_FILE=$bonds_file" >&2
  exit 1
fi

nodes_file="${CORDIAL_ORDER_NODES_FILE:-$tmp_dir/nodes.txt}"
if [ -n "${CORDIAL_ORDER_NODES_FILE:-}" ]; then
  if [ ! -r "$nodes_file" ]; then
    echo "ERROR: cannot read CORDIAL_ORDER_NODES_FILE=$nodes_file" >&2
    exit 1
  fi
else
  cat >"$nodes_file" <<'NODES'
cordial-validator-1 http://127.0.0.1:51401 http://127.0.0.1:51403
cordial-validator-2 http://127.0.0.1:52401 http://127.0.0.1:52403
cordial-validator-3 http://127.0.0.1:53401 http://127.0.0.1:53403
cordial-validator-4 http://127.0.0.1:54401 http://127.0.0.1:54403
NODES
fi

if [ "$nodes_file" != "$tmp_dir/nodes.txt" ]; then
  cp "$nodes_file" "$tmp_dir/nodes.txt"
fi
nodes_file="$tmp_dir/nodes.txt"
if [ -n "$bonds_file" ]; then
  cp "$bonds_file" "$tmp_dir/bonds.txt"
  bonds_file="$tmp_dir/bonds.txt"
fi

baseline="$tmp_dir/baseline.ordered.json"
reference_node=""
common_target_height=""
highest_visible_height="0"
reference_anchor=""

echo "Verifying four-node Cordial window-order convergence"
echo "===================================================="
echo "Depth:        $depth"
echo "Height batch: $height_batch_size"
echo "Height window: $height_window"
echo "Fragment:     $fragment_only"
echo "Boundary:     $trusted_window_boundary"
echo "Window order: $window_ordering_fragment"
echo "Bonds file:   ${bonds_file:-<recent senders>}"
echo "Node timeout: ${node_timeout}s"
if [ -n "$evidence_dir" ]; then echo "Evidence:     $evidence_dir"; fi
echo

while read -r node _grpc_url http_url; do
  [ -n "${node:-}" ] || continue
  curl -fsS --max-time 10 "$http_url/api/last-finalized-block" \
    >"$tmp_dir/$node.native-lfb.json"
  max_height="$(curl -fsS "$http_url/api/blocks/$depth" |
    jq '[.[] | (.blockInfo // .) | .blockNumber] | max')"

  if [ "$max_height" = "null" ] || [ -z "$max_height" ]; then
    echo "ERROR: $node did not expose a recent block height" >&2
    exit 1
  fi

  echo "$node: latest visible height=$max_height"
  printf '%s\t%s\n' "$node" "$max_height" >>"$tmp_dir/heights.tsv"
  if [ -z "$common_target_height" ] || [ "$max_height" -lt "$common_target_height" ]; then
    common_target_height="$max_height"
  fi
  if [ "$max_height" -gt "$highest_visible_height" ]; then
    highest_visible_height="$max_height"
  fi
done < "$nodes_file"

echo "Common target height: $common_target_height"
if [ -n "$evidence_dir" ]; then
  if [ -n "$bonds_file" ]; then bonds_sha="$(sha256sum "$bonds_file" | cut -d' ' -f1)"; else bonds_sha=""; fi
  jq -n --arg started_at "$started_at" \
    --arg git_commit "$(git rev-parse HEAD)" \
    --arg verifier_sha256 "$(sha256sum docker/scripts/verify-four-node-cluster-ordering.sh | cut -d' ' -f1)" \
    --arg mirror_source_sha256 "$(sha256sum crates/cordial-f1r3node-adapter/src/bin/live_mirror_check.rs | cut -d' ' -f1)" \
    --arg mode "$(if [ "$window_ordering_fragment" = false ]; then echo finalized-weighted-tau; else echo window-xsort; fi)" \
    --arg bonds_sha256 "$bonds_sha" \
    --argjson target_height "$common_target_height" \
    --argjson height_window "$height_window" \
    --argjson trusted_window_boundary "$trusted_window_boundary" \
    --argjson max_height_lag "$max_height_lag" \
    '{started_at: $started_at, git_commit: $git_commit,
      verifier_sha256: $verifier_sha256, mirror_source_sha256: $mirror_source_sha256,
      mode: $mode,
      target_height: $target_height, height_window: $height_window,
      trusted_window_boundary: $trusted_window_boundary,
      max_height_lag: $max_height_lag, bonds_sha256: $bonds_sha256,
      nodes_file: "nodes.txt", bonds_file: (if $bonds_sha256 == "" then null else "bonds.txt" end)}' \
    >"$tmp_dir/inputs.json"
fi
if [ "$window_ordering_fragment" = "false" ] &&
  [ "$((highest_visible_height - common_target_height))" -gt "$max_height_lag" ]; then
  echo "ERROR: node visible-height lag exceeds $max_height_lag; see heights.tsv" >&2
  exit 1
fi
echo

while read -r node grpc_url http_url; do
  [ -n "${node:-}" ] || continue

  output_file="$tmp_dir/$node.ordered.json"
  log_file="$tmp_dir/$node.log"

  echo "$node: mirroring $grpc_url and exporting $(if [ "$window_ordering_fragment" = false ]; then echo 'finalized weighted tau'; else echo 'bounded window order'; fi)"
  set -- --grpc-url "$grpc_url" --http-url "$http_url" --depth "$depth" \
    --height-bootstrap --height-batch-size "$height_batch_size" \
    --height-bootstrap-target "$common_target_height" --skip-http-compare \
    --ordering-preview 3 --write-ordered-file "$output_file"
  if [ "$height_window" -gt 0 ]; then
    set -- "$@" --height-bootstrap-window "$height_window"
  fi
  if [ "$fragment_only" = "true" ]; then set -- "$@" --ordering-fragment-only; fi
  if [ "$trusted_window_boundary" = "true" ]; then set -- "$@" --trusted-window-boundary; fi
  if [ "$window_ordering_fragment" = "true" ]; then set -- "$@" --window-ordering-fragment; fi
  if [ -n "$bonds_file" ]; then set -- "$@" --bonds-file "$bonds_file"; fi

  if ! timeout "$node_timeout" cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- "$@" \
    >"$log_file" 2>&1; then
    echo "ERROR: $node ordering export failed or timed out; log: $log_file" >&2
    tail -80 "$log_file" >&2
    exit 1
  fi

  ordered_count="$(jq 'length' "$output_file")"
  mirror_lfb="$(grep '^Mirror LFB:' "$log_file" | sed 's/^Mirror LFB:[[:space:]]*//')"
  ordered_anchor="$(sed -n 's/^Ordered output anchor:[[:space:]]*//p' "$log_file" | tail -1)"
  bond_count="$(sed -n 's/^Bond count:[[:space:]]*//p' "$log_file" | tail -1)"
  output_sha="$(sha256sum "$output_file" | cut -d' ' -f1)"
  image_id="$(docker inspect --format '{{.Image}}' "$node" 2>/dev/null || true)"
  jq -n --arg node "$node" --arg grpc_url "$grpc_url" --arg http_url "$http_url" \
    --arg anchor "$ordered_anchor" --arg mirror_lfb "$mirror_lfb" \
    --arg image_id "$image_id" \
    --arg output_sha256 "$output_sha" --argjson ordered_count "$ordered_count" \
    --argjson bond_count "${bond_count:-0}" \
    '{node: $node, grpc_url: $grpc_url, http_url: $http_url,
      ordered_count: $ordered_count, anchor: $anchor, mirror_lfb: $mirror_lfb,
      bond_count: $bond_count, image_id: $image_id,
      output_sha256: $output_sha256}' \
    >"$tmp_dir/$node.summary.json"

  if [ "$ordered_count" -eq 0 ]; then
    echo "ERROR: $node produced an empty ordered output; anchor=$ordered_anchor, bonds=$bond_count; log: $log_file" >&2
    exit 1
  fi
  if [ "$(jq 'unique | length' "$output_file")" -ne "$ordered_count" ]; then
    echo "ERROR: $node ordered output contains duplicate hashes" >&2
    exit 1
  fi
  if [ "$window_ordering_fragment" = "false" ]; then
    if [ "$bond_count" -ne 4 ]; then
      echo "ERROR: $node finalized tau used $bond_count bonds; expected four authorized validators" >&2
      exit 1
    fi
    if [ -z "$ordered_anchor" ] || [ "$ordered_anchor" = "<none>" ]; then
      echo "ERROR: $node finalized tau output has no Cordial anchor" >&2
      exit 1
    fi
    if [ -z "$reference_anchor" ]; then
      reference_anchor="$ordered_anchor"
    elif [ "$ordered_anchor" != "$reference_anchor" ]; then
      echo "ERROR: $node Cordial tau anchor differs from $reference_node" >&2
      exit 1
    fi
  fi

  echo "$node: ordered=$ordered_count mirror_lfb=$mirror_lfb"

  if [ -z "$reference_node" ]; then
    cp "$output_file" "$baseline"
    reference_node="$node"
    continue
  fi

  if ! cmp -s "$baseline" "$output_file"; then
    echo "ERROR: $node Cordial window order diverged from $reference_node" >&2
    echo "Reference length: $(jq 'length' "$baseline")" >&2
    echo "$node length: $(jq 'length' "$output_file")" >&2
    jq -n \
      --slurpfile reference "$baseline" \
      --slurpfile current "$output_file" \
      '
      def mismatch:
        [range(0; ([($reference[0] | length), ($current[0] | length)] | min)) |
          select($reference[0][.] != $current[0][.])][0];
      {
        first_mismatch_index: mismatch,
        reference_hash: (if mismatch == null then null else $reference[0][mismatch] end),
        current_hash: (if mismatch == null then null else $current[0][mismatch] end)
      }
      ' >&2
    exit 1
  fi
done < "$nodes_file"

echo
if [ "$window_ordering_fragment" = "false" ]; then
  echo "PASS: four mirrors produced the same nonempty finalized weighted tau output and Cordial anchor."
else
  echo "PASS: four real f1r3node validators produced the same bounded mirrored Cordial window order."
fi
