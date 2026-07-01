#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DOCKER_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
CERTS_DIR="$DOCKER_DIR/certs"

generate_cert() {
  name="$1"
  out_dir="$CERTS_DIR/$name"
  mkdir -p "$out_dir"

  openssl req \
    -x509 \
    -newkey ec \
    -pkeyopt ec_paramgen_curve:prime256v1 \
    -pkeyopt ec_param_enc:named_curve \
    -keyout "$out_dir/node.key.pem" \
    -out "$out_dir/node.certificate.pem" \
    -sha256 \
    -days 365 \
    -nodes \
    -subj "/CN=$name" \
    >/dev/null 2>&1
}

mkdir -p "$CERTS_DIR"

for name in bootstrap validator1 validator2 validator3 validator4; do
  generate_cert "$name"
done

echo "Generated EC node certificates under $CERTS_DIR"
