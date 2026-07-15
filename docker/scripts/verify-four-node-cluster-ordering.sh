#!/bin/sh
set -eu

depth="${CORDIAL_ORDER_DEPTH:-64}"
height_batch_size="${CORDIAL_ORDER_HEIGHT_BATCH_SIZE:-64}"
height_window="${CORDIAL_ORDER_HEIGHT_WINDOW:-256}"
fragment_only="${CORDIAL_ORDER_FRAGMENT_ONLY:-true}"
trusted_window_boundary="${CORDIAL_ORDER_TRUSTED_WINDOW_BOUNDARY:-true}"
window_ordering_fragment="${CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT:-true}"
node_timeout="${CORDIAL_ORDER_NODE_TIMEOUT:-300}"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

nodes_file="$tmp_dir/nodes.txt"
cat >"$nodes_file" <<'NODES'
cordial-validator-1 http://127.0.0.1:51401 http://127.0.0.1:51403
cordial-validator-2 http://127.0.0.1:52401 http://127.0.0.1:52403
cordial-validator-3 http://127.0.0.1:53401 http://127.0.0.1:53403
cordial-validator-4 http://127.0.0.1:54401 http://127.0.0.1:54403
NODES

baseline="$tmp_dir/baseline.ordered.json"
reference_node=""
common_target_height=""

echo "Verifying four-node Cordial window-order convergence"
echo "===================================================="
echo "Depth:        $depth"
echo "Height batch: $height_batch_size"
echo "Height window: $height_window"
echo "Fragment:     $fragment_only"
echo "Boundary:     $trusted_window_boundary"
echo "Window order: $window_ordering_fragment"
echo "Node timeout: ${node_timeout}s"
echo

while read -r node _grpc_url http_url; do
  [ -n "${node:-}" ] || continue
  max_height="$(curl -fsS "$http_url/api/blocks/$depth" |
    jq '[.[] | (.blockInfo // .) | .blockNumber] | max')"

  if [ "$max_height" = "null" ] || [ -z "$max_height" ]; then
    echo "ERROR: $node did not expose a recent block height" >&2
    exit 1
  fi

  echo "$node: latest visible height=$max_height"
  if [ -z "$common_target_height" ] || [ "$max_height" -lt "$common_target_height" ]; then
    common_target_height="$max_height"
  fi
done < "$nodes_file"

echo "Common target height: $common_target_height"
echo

while read -r node grpc_url http_url; do
  [ -n "${node:-}" ] || continue

  output_file="$tmp_dir/$node.ordered.json"
  log_file="$tmp_dir/$node.log"

  echo "$node: mirroring $grpc_url and exporting bounded Cordial window order"
  boundary_arg=""
  if [ "$trusted_window_boundary" = "true" ]; then
    boundary_arg="--trusted-window-boundary"
  fi
  window_order_arg=""
  if [ "$window_ordering_fragment" = "true" ]; then
    window_order_arg="--window-ordering-fragment"
  fi

  if [ "$fragment_only" = "true" ]; then
    if ! timeout "$node_timeout" cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- \
      --grpc-url "$grpc_url" \
      --http-url "$http_url" \
      --depth "$depth" \
      --height-bootstrap \
      --height-batch-size "$height_batch_size" \
      --height-bootstrap-window "$height_window" \
      --height-bootstrap-target "$common_target_height" \
      --skip-http-compare \
      --ordering-preview 3 \
      --ordering-fragment-only \
      $boundary_arg \
      $window_order_arg \
      --write-ordered-file "$output_file" \
      >"$log_file" 2>&1; then
      echo "ERROR: $node ordering export failed or timed out" >&2
      cat "$log_file" >&2
      exit 1
    fi
  else
    if ! timeout "$node_timeout" cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- \
      --grpc-url "$grpc_url" \
      --http-url "$http_url" \
      --depth "$depth" \
      --height-bootstrap \
      --height-batch-size "$height_batch_size" \
      --height-bootstrap-window "$height_window" \
      --height-bootstrap-target "$common_target_height" \
      --skip-http-compare \
      --ordering-preview 3 \
      $boundary_arg \
      $window_order_arg \
      --write-ordered-file "$output_file" \
      >"$log_file" 2>&1; then
      echo "ERROR: $node ordering export failed or timed out" >&2
      cat "$log_file" >&2
      exit 1
    fi
  fi

  ordered_count="$(jq 'length' "$output_file")"
  mirror_lfb="$(grep '^Mirror LFB:' "$log_file" | sed 's/^Mirror LFB:[[:space:]]*//')"

  if [ "$ordered_count" -eq 0 ]; then
    echo "ERROR: $node produced an empty Cordial window order" >&2
    cat "$log_file" >&2
    exit 1
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
echo "PASS: four real f1r3node validators produced the same bounded mirrored Cordial window order."
