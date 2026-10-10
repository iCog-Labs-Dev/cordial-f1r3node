#!/bin/sh
set -eu

usage() {
  echo "Usage: $0 [seconds=60] [report-dir=/tmp/cordial-cluster-disk-TIMESTAMP] [por-data-root]" >&2
}

if [ "$#" -gt 3 ]; then
  usage
  exit 2
fi
interval="${1:-60}"
case "$interval" in
  ''|*[!0-9]*) usage; exit 2 ;;
esac
report_dir="${2:-/tmp/cordial-cluster-disk-$(date -u +%Y%m%dT%H%M%SZ)}"
por_data_root="${3:-}"
mkdir -p "$report_dir"

nodes="cordial-boot cordial.validator1 cordial.validator2 cordial.validator3 cordial.validator4"
docker_root="$(docker info --format '{{.DockerRootDir}}')"

snapshot() {
  phase="$1"
  table="$report_dir/$phase.tsv"
  printf 'node\tlog_bytes\tvolume_bytes\tlayer_bytes\n' >"$table"
  for node in $nodes; do
    running="$(docker inspect --format '{{.State.Running}}' "$node")"
    if [ "$running" != "true" ]; then
      echo "ERROR: $node is not running; start the four-node cluster first" >&2
      exit 1
    fi
    log_path="$(docker inspect --format '{{.LogPath}}' "$node")"
    if [ -z "$log_path" ] || ! [ -r "$log_path" ]; then
      echo "ERROR: cannot read Docker log for $node; run this sampler with sudo" >&2
      exit 1
    fi
    # Include rotated json-file logs, not just the active log returned by inspect.
    log_bytes="$(find "$(dirname "$log_path")" -maxdepth 1 -type f \
      -name "$(basename "$log_path")*" -printf '%s\n' |
      awk '{total += $1} END {print total + 0}')"
    volume_bytes="$(docker exec "$node" du -sB1 /var/lib/rnode | awk '{print $1}')"
    layer_bytes="$(docker inspect --size --format '{{.SizeRw}}' "$node")"
    printf '%s\t%s\t%s\t%s\n' "$node" "$log_bytes" "$volume_bytes" "$layer_bytes" >>"$table"
  done
  df -B1 --output=avail "$docker_root" | tail -n 1 >"$report_dir/$phase.free-bytes"
  docker system df -v >"$report_dir/$phase.docker-system-df.txt"
  if [ -n "$por_data_root" ]; then
    if [ -d "$por_data_root" ]; then
      du -sB1 "$por_data_root" | awk '{print $1}' >"$report_dir/$phase.por-bytes"
    else
      printf '0\n' >"$report_dir/$phase.por-bytes"
    fi
  fi
}

echo "Sampling cluster disk usage now and after ${interval}s"
snapshot before
sleep "$interval"
snapshot after

printf '\n%-22s %14s %14s %14s\n' 'Node' 'Log delta' 'Volume delta' 'Layer delta'
awk -F '\t' '
  FNR == 1 {next}
  FILENAME == ARGV[1] {logs[$1] = $2; volumes[$1] = $3; layers[$1] = $4; next}
  {printf "%-22s %+14d %+14d %+14d\n", $1, $2 - logs[$1], $3 - volumes[$1], $4 - layers[$1]}
' "$report_dir/before.tsv" "$report_dir/after.tsv"

awk 'NR == 1 {before = $1} NR == 2 {printf "Host free-space delta: %+d bytes\n", $1 - before}' \
  "$report_dir/before.free-bytes" "$report_dir/after.free-bytes"
if [ -n "$por_data_root" ]; then
  awk 'NR == 1 {before = $1} NR == 2 {printf "PoR data delta: %+d bytes\n", $1 - before}' \
    "$report_dir/before.por-bytes" "$report_dir/after.por-bytes"
fi
echo "Full snapshots and Docker image/cache totals: $report_dir"
