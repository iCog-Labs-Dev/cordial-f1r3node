#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
mkdir "$tmp_dir/mock"

for node in cordial-boot cordial.validator1 cordial.validator2 cordial.validator3 cordial.validator4; do
  printf 'x' >"$tmp_dir/$node.log"
done

cat >"$tmp_dir/mock/docker" <<'MOCK_DOCKER'
#!/bin/sh
set -eu
phase=before
if [ -f "$MOCK_ROOT/after" ]; then phase=after; fi
case "$1" in
  info) printf '%s\n' "$MOCK_ROOT" ;;
  inspect)
    if [ "$2" = --size ]; then
      if [ "$phase" = before ]; then echo 10; else echo 15; fi
    elif [ "$3" = '{{.State.Running}}' ]; then
      echo true
    elif [ "$3" = '{{.LogPath}}' ]; then
      printf '%s/%s.log\n' "$MOCK_ROOT" "$4"
    else
      exit 1
    fi
    ;;
  exec)
    if [ "$phase" = before ]; then echo '100 /var/lib/rnode'; else echo '120 /var/lib/rnode'; fi
    ;;
  system) echo 'mock Docker space report' ;;
  *) exit 1 ;;
esac
MOCK_DOCKER

cat >"$tmp_dir/mock/df" <<'MOCK_DF'
#!/bin/sh
echo Avail
if [ -f "$MOCK_ROOT/after" ]; then echo 900; else echo 1000; fi
MOCK_DF

cat >"$tmp_dir/mock/sleep" <<'MOCK_SLEEP'
#!/bin/sh
set -eu
for node in cordial-boot cordial.validator1 cordial.validator2 cordial.validator3 cordial.validator4; do
  printf 'abc' >>"$MOCK_ROOT/$node.log"
done
touch "$MOCK_ROOT/after"
MOCK_SLEEP

chmod +x "$tmp_dir/mock/docker" "$tmp_dir/mock/df" "$tmp_dir/mock/sleep"
MOCK_ROOT="$tmp_dir" PATH="$tmp_dir/mock:$PATH" \
  sh "$repo_root/docker/scripts/measure-four-node-disk.sh" 0 "$tmp_dir/report" \
  >"$tmp_dir/result"

awk -F '\t' 'NR > 1 && ($2 != 4 || $3 != 120 || $4 != 15) {exit 1} END {if (NR != 6) exit 1}' \
  "$tmp_dir/report/after.tsv"
grep -q 'Host free-space delta: -100 bytes' "$tmp_dir/result"
grep -q 'Full snapshots and Docker image/cache totals:' "$tmp_dir/result"
echo 'PASS: disk sampler reports logs, node data, layers, and host free-space changes.'
