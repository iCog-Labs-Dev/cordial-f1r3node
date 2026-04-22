#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CORE_DIR="${REPO_ROOT}/crates/cordial-miners-core"

if [[ ! -d "${CORE_DIR}" ]]; then
  echo "Core crate not found at ${CORE_DIR}" >&2
  exit 1
fi

if rg --line-number --glob '*.rs' 'f1r3node::|models::|casper::' "${CORE_DIR}"; then
  echo "Boundary violation: cordial-miners-core imports node-specific crates." >&2
  exit 1
fi

echo "Boundary check passed: cordial-miners-core is isolated from node-specific imports."
