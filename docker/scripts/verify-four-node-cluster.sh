#!/bin/sh
set -eu

boot="cordial-boot"
validators="cordial-validator-1 cordial-validator-2 cordial-validator-3 cordial-validator-4"
expected_network="cordial-real-four-node"
reference_order=""

wait_for_status() {
  node="$1"
  i=0
  while [ "$i" -lt 120 ]; do
    if curl -fsS "http://${node}:40403/api/status" >/tmp/"${node}".status 2>/tmp/"${node}".status.err; then
      return 0
    fi
    i=$((i + 1))
    sleep 2
  done
  echo "ERROR: ${node} HTTP API did not become ready" >&2
  cat /tmp/"${node}".status.err >&2 || true
  return 1
}

wait_for_status "$boot"
for node in $validators; do
  wait_for_status "$node"
done

boot_network="$(jq -r '.networkId' /tmp/"${boot}".status)"
if [ "$boot_network" != "$expected_network" ]; then
  echo "ERROR: bootstrap joined ${boot_network}, expected ${expected_network}" >&2
  exit 1
fi

for node in $validators; do
  network_id="$(jq -r '.networkId' /tmp/"${node}".status)"
  is_validator="$(jq -r '.isValidator' /tmp/"${node}".status)"
  peers="$(jq -r '.peers // 0' /tmp/"${node}".status)"
  nodes="$(jq -r '.nodes // 0' /tmp/"${node}".status)"

  if [ "$network_id" != "$expected_network" ]; then
    echo "ERROR: ${node} joined ${network_id}, expected ${expected_network}" >&2
    exit 1
  fi
  if [ "$is_validator" != "true" ]; then
    echo "ERROR: ${node} is not a bonded validator" >&2
    jq . /tmp/"${node}".status >&2
    exit 1
  fi
  if [ "$peers" = "0" ] && [ "$nodes" = "0" ]; then
    echo "ERROR: ${node} appears isolated (peers=0, nodes=0)" >&2
    jq . /tmp/"${node}".status >&2
    exit 1
  fi
  echo "${node}: connected Cordial validator on ${network_id} (peers=${peers}, nodes=${nodes})"
done

for node in $validators; do
  echo "${node}: triggering local Cordial proposal through admin API"
  curl -fsS -X POST "http://${node}:40405/api/propose"
  echo
done

sleep 5

for node in $validators; do
  blocks="$(curl -fsS "http://${node}:40403/api/blocks/10")"
  order="$(printf '%s' "$blocks" | jq -c '[.[].blockInfo | {blockNumber, blockHash, sender, seqNum, deployCount, isFinalized}]')"
  hash_order="$(printf '%s' "$order" | jq -c '[.[].blockHash]')"
  finalized_count="$(printf '%s' "$order" | jq '[.[] | select(.isFinalized == true)] | length')"

  if [ "$finalized_count" -lt 1 ]; then
    echo "ERROR: ${node} did not expose a finalized Cordial block" >&2
    printf '%s\n' "$order" >&2
    exit 1
  fi

  echo "${node}: ordered block view ${hash_order}"
  if [ -z "$reference_order" ]; then
    reference_order="$order"
  elif [ "$order" != "$reference_order" ]; then
    echo "ERROR: ${node} ordered view diverged from cordial-validator-1" >&2
    echo "reference:" >&2
    printf '%s\n' "$reference_order" | jq . >&2
    echo "${node}:" >&2
    printf '%s\n' "$order" | jq . >&2
    exit 1
  fi
done

echo "PASS: four connected local f1r3node validators produced the same Cordial Miners ordered view."
printf '%s\n' "$reference_order" | jq .
