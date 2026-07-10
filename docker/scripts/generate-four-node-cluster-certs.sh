#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DOCKER_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
CERTS_DIR="$DOCKER_DIR/certs"
UPSTREAM_CERTS_DIR="$DOCKER_DIR/../../f1r3node/docker/certs"

copy_bootstrap_cert() {
  src_dir="$UPSTREAM_CERTS_DIR/bootstrap"
  out_dir="$CERTS_DIR/bootstrap"
  mkdir -p "$out_dir"

  if [ ! -f "$src_dir/node.key.pem" ] || [ ! -f "$src_dir/node.certificate.pem" ]; then
    echo "ERROR: expected upstream bootstrap TLS files under $src_dir" >&2
    echo "The bootstrap certificate must match CORDIAL_BOOTSTRAP_NODE_ID." >&2
    exit 1
  fi

  cp "$src_dir/node.key.pem" "$out_dir/node.key.pem"
  cp "$src_dir/node.certificate.pem" "$out_dir/node.certificate.pem"
}

generate_key_only() {
  name="$1"
  out_dir="$CERTS_DIR/$name"
  mkdir -p "$out_dir"
  rm -f "$out_dir/node.certificate.pem"

  openssl genpkey \
    -algorithm EC \
    -pkeyopt ec_paramgen_curve:prime256v1 \
    -pkeyopt ec_param_enc:named_curve \
    -out "$out_dir/node.key.pem" \
    >/dev/null 2>&1
}

mkdir -p "$CERTS_DIR"

copy_bootstrap_cert
for name in validator1 validator2 validator3 validator4; do
  generate_key_only "$name"
done

echo "Prepared four-node cluster TLS material under $CERTS_DIR"
echo "Bootstrap uses the upstream fixed certificate; validators generate matching certificates from mounted keys on first boot."
