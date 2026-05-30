#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ ! -d "../f1r3node" ]]; then
  cat >&2 <<'EOF'
Missing ../f1r3node checkout.

The GitHub workflow checks out F1R3FLY-io/f1r3node next to this repository
because the adapter crates use f1r3node path dependencies. Clone f1r3node as a
sibling directory before pushing.
EOF
  exit 1
fi

if ! command -v protoc >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Missing protoc.

Install protobuf-compiler before pushing. The GitHub workflow installs it with:
sudo apt-get update && sudo apt-get install -y protobuf-compiler
EOF
  exit 1
fi

echo "==> Checking formatting"
cargo fmt -p cordial-miners-core --check
cargo fmt -p cordial-f1r3node-adapter --check
cargo fmt -p cordial-f1r3space-adapter --check

echo "==> Running clippy"
cargo clippy -p cordial-miners-core -p cordial-f1r3node-adapter -p cordial-f1r3space-adapter --all-targets --all-features --no-deps -- -D warnings

echo "==> Building workspace"
cargo build --verbose

echo "==> Running workspace tests"
cargo test --verbose
