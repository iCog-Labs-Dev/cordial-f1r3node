#!/bin/sh
set -eu

boot="cordial-boot"
validators="cordial-validator-1 cordial-validator-2 cordial-validator-3 cordial-validator-4"
expected_network="cordial-real-four-node"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

wait_for_status() {
  node="$1"
  i=0
  while [ "$i" -lt 120 ]; do
    if curl -fsS --max-time 10 "http://${node}:40403/api/status" >"$tmp_dir/${node}.status" 2>"$tmp_dir/${node}.status.err"; then
      return 0
    fi
    i=$((i + 1))
    sleep 2
  done
  echo "ERROR: ${node} HTTP API did not become ready" >&2
  cat "$tmp_dir/${node}.status.err" >&2 || true
  return 1
}

wait_for_status "$boot"
for node in $validators; do
  wait_for_status "$node"
done

boot_network="$(jq -r '.networkId' "$tmp_dir/${boot}.status")"
if [ "$boot_network" != "$expected_network" ]; then
  echo "ERROR: bootstrap joined ${boot_network}, expected ${expected_network}" >&2
  exit 1
fi

for node in $validators; do
  network_id="$(jq -r '.networkId' "$tmp_dir/${node}.status")"
  is_validator="$(jq -r '.isValidator | if . == null then "unknown" else tostring end' "$tmp_dir/${node}.status")"
  peers="$(jq -r '.peers // 0' "$tmp_dir/${node}.status")"
  nodes="$(jq -r '.nodes // 0' "$tmp_dir/${node}.status")"

  if [ "$network_id" != "$expected_network" ]; then
    echo "ERROR: ${node} joined ${network_id}, expected ${expected_network}" >&2
    exit 1
  fi
  if [ "$is_validator" = "false" ]; then
    echo "ERROR: ${node} reports it is not a validator" >&2
    jq . "$tmp_dir/${node}.status" >&2
    exit 1
  fi
  if [ "$peers" = "0" ] && [ "$nodes" = "0" ]; then
    echo "ERROR: ${node} appears isolated (peers=0, nodes=0)" >&2
    jq . "$tmp_dir/${node}.status" >&2
    exit 1
  fi
  echo "${node}: connected Cordial node on ${network_id} (validator status=${is_validator}, peers=${peers}, nodes=${nodes})"
done

for node in $validators; do
  echo "${node}: triggering local Cordial proposal through admin API"
  propose_body="$(mktemp)"
  propose_status="$(curl -sS -o "$propose_body" -w "%{http_code}" -X POST "http://${node}:40405/api/propose")"
  if [ "$propose_status" -ge 200 ] && [ "$propose_status" -lt 300 ]; then
    cat "$propose_body"
    echo
  elif grep -q "NoNewDeploys" "$propose_body"; then
    echo "${node}: no pending deploys; heartbeat proposer is already producing blocks"
  else
    cat "$propose_body" >&2 || true
    rm -f "$propose_body"
    exit 1
  fi
  rm -f "$propose_body"
done

read_lfb() {
  node="$1"
  curl -fsS --max-time 10 "http://${node}:40403/api/last-finalized-block" |
    jq -r '[.blockInfo.blockNumber, .blockInfo.blockHash] | @tsv'
}

# Fix a height before comparing nodes so their moving tips cannot create a
# false divergence. Require progress past genesis before choosing the target.
target_height=""
target_hash=""
i=0
while [ "$i" -lt 120 ]; do
  if lfb="$(read_lfb cordial-validator-1 2>/dev/null)"; then
    target_height="$(printf '%s' "$lfb" | cut -f1)"
    target_hash="$(printf '%s' "$lfb" | cut -f2)"
    if [ -n "$target_hash" ] && [ "$target_height" -gt 0 ] 2>/dev/null; then
      break
    fi
  fi
  i=$((i + 1))
  sleep 2
done
if [ -z "$target_hash" ] || ! [ "$target_height" -gt 0 ] 2>/dev/null; then
  echo "ERROR: cordial-validator-1 did not finalize a block after genesis" >&2
  exit 1
fi

for node in $validators; do
  i=0
  while [ "$i" -lt 120 ]; do
    if lfb="$(read_lfb "$node" 2>/dev/null)"; then
      height="$(printf '%s' "$lfb" | cut -f1)"
      if [ "$height" -ge "$target_height" ] 2>/dev/null; then
        echo "${node}: finalized height ${height} (target ${target_height})"
        break
      fi
    fi
    i=$((i + 1))
    sleep 2
  done
  if [ "$i" -eq 120 ]; then
    echo "ERROR: ${node} did not reach finalized height ${target_height}" >&2
    exit 1
  fi
done

start_height=$((target_height - 9))
if [ "$start_height" -lt 0 ]; then
  start_height=0
fi
reference_file="$tmp_dir/cordial-validator-1.finalized"

for node in $validators; do
  blocks_file="$tmp_dir/${node}.blocks"
  finalized_file="$tmp_dir/${node}.finalized"
  curl -fsS --max-time 20 "http://${node}:40403/api/blocks/${start_height}/${target_height}" >"$blocks_file"
  jq -e 'type == "array" and all(.[]; (.blockInfo // .) | (.blockNumber | type == "number") and (.blockHash | type == "string" and length > 0))' "$blocks_file" >/dev/null
  : >"$finalized_file"
  jq -r '.[] | (.blockInfo // .) | [.blockNumber, .blockHash] | @tsv' "$blocks_file" |
    while IFS="$(printf '\t')" read -r height hash; do
      finalized="$(curl -fsS --max-time 10 "http://${node}:40403/api/is-finalized/${hash}" |
        jq -r 'if type == "boolean" then tostring else error("invalid finality response") end')"
      if [ "$finalized" = "true" ]; then
        printf '%s %s\n' "$height" "$hash" >>"$finalized_file"
      fi
    done

  if ! grep -Fxq "${target_height} ${target_hash}" "$finalized_file"; then
    echo "ERROR: ${node} has not finalized reference anchor ${target_hash} at height ${target_height}" >&2
    exit 1
  fi
  echo "${node}: finalized window ${start_height}..${target_height} ($(wc -l <"$finalized_file") blocks)"
  if [ "$node" != "cordial-validator-1" ] && ! cmp -s "$reference_file" "$finalized_file"; then
    echo "ERROR: ${node} finalized window differs from cordial-validator-1" >&2
    echo "cordial-validator-1:" >&2
    cat "$reference_file" >&2
    echo "${node}:" >&2
    cat "$finalized_file" >&2
    exit 1
  fi
done

echo "PASS: four connected local f1r3node nodes share the finalized block window through height ${target_height}."
cat "$reference_file"
