#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

cat >"$tmp_dir/curl" <<'MOCK_CURL'
#!/bin/sh
set -eu

url=""
body_file=""
previous=""
for arg in "$@"; do
  if [ "$previous" = "-o" ]; then body_file="$arg"; fi
  case "$arg" in http://*) url="$arg" ;; esac
  previous="$arg"
done

case "$url" in
  */api/status)
    case "$url" in
      *cordial-validator-2*)
        if [ "${MOCK_UNBONDED:-0}" = 1 ]; then
          printf '{"networkId":"cordial-real-four-node","isValidator":false,"peers":1,"nodes":1}'
          exit 0
        fi
        ;;
    esac
    printf '{"networkId":"cordial-real-four-node","peers":1,"nodes":1}'
    ;;
  */api/propose)
    printf 'proposed\n' >"$body_file"
    printf '200'
    ;;
  */api/last-finalized-block)
    printf '{"blockInfo":{"blockNumber":1,"blockHash":"anchor"}}'
    ;;
  */api/blocks/0/1)
    case "$url" in
      *cordial-validator-2*)
        if [ "${MOCK_DIVERGE:-0}" = 1 ]; then
          printf '[{"blockNumber":0,"blockHash":"extra"},{"blockNumber":1,"blockHash":"anchor"}]'
          exit 0
        fi
        ;;
    esac
    printf '[{"blockNumber":1,"blockHash":"anchor"}]'
    ;;
  */api/is-finalized/*)
    printf 'true'
    ;;
  *)
    echo "Unexpected URL: $url" >&2
    exit 1
    ;;
esac
MOCK_CURL
chmod +x "$tmp_dir/curl"

PATH="$tmp_dir:$PATH" sh "$repo_root/docker/scripts/verify-four-node-cluster.sh" >"$tmp_dir/pass.log"
grep -q '^PASS:' "$tmp_dir/pass.log"

if MOCK_DIVERGE=1 PATH="$tmp_dir:$PATH" sh "$repo_root/docker/scripts/verify-four-node-cluster.sh" >"$tmp_dir/diverge.log" 2>&1; then
  echo 'Expected divergent finalized blocks to fail' >&2
  exit 1
fi
grep -q 'finalized window differs' "$tmp_dir/diverge.log"

if MOCK_UNBONDED=1 PATH="$tmp_dir:$PATH" sh "$repo_root/docker/scripts/verify-four-node-cluster.sh" >"$tmp_dir/unbonded.log" 2>&1; then
  echo 'Expected an explicit non-validator status to fail' >&2
  exit 1
fi
grep -q 'reports it is not a validator' "$tmp_dir/unbonded.log"

echo 'PASS: cluster verifier accepts a shared finalized window and rejects divergence and explicit non-validator status.'
