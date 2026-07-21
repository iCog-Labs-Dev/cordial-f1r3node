# Ordered Output File Export

This note documents the first external access seam for finalized Cordial tau
ordering output.

It builds on:

- [13-ordered-output-export.md](./13-ordered-output-export.md)
- [14-ordered-output-consumer-boundary.md](./14-ordered-output-consumer-boundary.md)
- [15-shared-ordered-output-reader.md](./15-shared-ordered-output-reader.md)

## Purpose

The adapter can now expose ordered finalized output as a JSON file that another
sidecar process, demo script, or inspection tool can read.

This keeps the integration observer-first:

- no upstream `f1r3node` changes
- no proposer takeover
- no HTTP or gRPC server lifecycle yet
- no recomputation by the consumer

## Implementation

The reusable writer lives in:

```text
crates/cordial-f1r3node-adapter/src/ordered_output_file.rs
```

It exports:

- `write_ordered_output_file(...)`
- `write_latest_ordered_output_file(...)`
- `OrderedOutputFileError`

The writer accepts the read-only `ReadOrderedOutput` seam and serializes the
latest `OrderedFinalizedOutput` as pretty JSON.

## CLI Usage

The inspection binary can write the same output to a file:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_ordered_output -- \
  --grpc-url http://127.0.0.1:40401 \
  --depth 128 \
  --write-output-file /tmp/cordial-ordered-output.json
```

By default, the command refuses to write an empty ordered output. For early
startup or no-finality-yet demos, allow empty output explicitly:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_ordered_output -- \
  --grpc-url http://127.0.0.1:40401 \
  --depth 128 \
  --write-output-file /tmp/cordial-ordered-output.json \
  --allow-empty-output
```

For full live-node bootstrap and large finalized histories, use
`live_mirror_check` instead:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- \
  --grpc-url http://127.0.0.1:40401 \
  --http-url http://127.0.0.1:40403 \
  --depth 64 \
  --height-bootstrap \
  --height-batch-size 64 \
  --skip-http-compare \
  --ordering-preview 3 \
  --write-ordered-file /tmp/cordial-ordered-output.json
```

That harness writes a JSON array of ordered block hashes, so inspect it with:

```bash
jq 'length' /tmp/cordial-ordered-output.json
jq '.[0:3]' /tmp/cordial-ordered-output.json
jq '.[-3:]' /tmp/cordial-ordered-output.json
```

## Output Shape

The file contains the stable `OrderedFinalizedOutput` model:

- `blocks`: finalized block identities in tau order
- `anchor`: latest finalized leader anchoring the fragment
- `wavelength`: wave length used for finality
- `bond_count`: validator bond count used by the mirror
- `total_mirrored_blocks`: total mirrored DAG size
- `computed_at_ns`: export timestamp

## Intended Consumer

The first consumer is a sidecar reader that watches or periodically reads the
JSON file and treats the `blocks` field as append-only finalized output.

The consumer must not reinterpret blocklace internals. It should rely on the
export seam and enforce the same prefix rule:

```text
new_output.blocks starts with previous_output.blocks
```

## Out Of Scope

This file-export seam does not yet:

- serve ordered output over HTTP or gRPC
- push updates
- feed ordered output back into f1r3node execution
- replace CBC Casper

Those are follow-up integration steps.
