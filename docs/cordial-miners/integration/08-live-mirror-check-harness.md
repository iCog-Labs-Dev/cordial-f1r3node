# Live Mirror Check Harness

This note documents the live mirror verification harness for the
`cordial-f1r3node-adapter`.

The harness lives at:

- `crates/cordial-f1r3node-adapter/src/bin/live_mirror_check.rs`

Its purpose is to connect to a running `f1r3node`, reconstruct a local
Cordial blocklace mirror from live node-visible block APIs, and compare the
result against node-visible finality state.

## What The Harness Does

The harness performs four logical steps:

1. query the node's current gRPC block view
2. bootstrap a local Cordial mirror from the live block stream
3. compute Cordial finality and, optionally, Cordial ordering
4. compare the mirror result against node-visible HTTP or gRPC state

This makes it the main executable diagnostic tool for live integration.

## Current Bootstrap Strategy

The harness supports two bootstrap styles:

1. height-based bootstrap
   - fetches blocks from low heights upward using `get_blocks_by_heights`
   - this is the preferred mode
   - it reconstructs the DAG in the same direction the mirror expects

2. predecessor backfill
   - starts from a recent window and recursively fetches missing predecessors
   - useful as a fallback diagnostic path
   - slower and less natural for long single-parent histories

In practice, height-based bootstrap is what made the live mirror converge
reliably.

## How To Run

Typical run:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- \
  --grpc-url http://127.0.0.1:40401 \
  --http-url http://127.0.0.1:40403 \
  --depth 64 \
  --height-bootstrap \
  --height-batch-size 64
```

Useful diagnostic run that skips expensive post-bootstrap phases:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- \
  --grpc-url http://127.0.0.1:40401 \
  --http-url http://127.0.0.1:40403 \
  --depth 64 \
  --height-bootstrap \
  --height-batch-size 64 \
  --skip-ordering \
  --skip-http-compare
```

## Parameters

### Connection parameters

- `--grpc-url`
  - gRPC endpoint for live block access
  - default: `http://127.0.0.1:40401`

- `--http-url`
  - HTTP endpoint for observer comparison
  - default: `http://127.0.0.1:40403`

### Recent-view parameters

- `--depth`
  - recent block depth used to discover the live tip / max height
  - also used when height bootstrap is disabled
  - default: `64`

### Bootstrap parameters

- `--height-bootstrap`
  - enables forward height-based bootstrap
  - default: `true`

- `--height-batch-size`
  - number of heights fetched per bootstrap batch
  - default: `64`

- `--parents-only-bootstrap`
  - reconstructs trusted blocks using parent edges only, excluding
    justification edges from hard predecessor requirements
  - useful for debugging bootstrap assumptions
  - default: `false`

### Backfill parameters

- `--max-backfill-rounds`
  - maximum recursive predecessor-recovery rounds
  - default: `8`

- `--max-backfill-blocks`
  - maximum total predecessor blocks fetched during backfill
  - default: `256`

These are mainly relevant when height bootstrap is disabled or when the
mirror still has unresolved predecessors after bootstrap.

### Post-bootstrap diagnostics

- `--skip-ordering`
  - skips weighted `tau` ordering computation
  - useful for isolating finality vs ordering cost

- `--skip-http-compare`
  - skips HTTP observer comparison
  - useful for isolating mirror/finality behavior from HTTP fetch cost

## Output Phases

The harness prints explicit phase markers:

- `[phase] querying initial gRPC last finalized block`
- `[height-bootstrap] ...`
- `[phase] bootstrap complete`
- `[phase] computing mirror last finalized block`
- `[phase] querying gRPC last finalized block`
- `[phase] computing ordered finalized blocks`
- `[phase] querying HTTP observer and comparing mirror`

These messages are intentional. They help isolate which phase is expensive or
misaligned during live debugging.

## Output Meaning

Important fields in the final report:

- `Mirrored blocks`
  - number of blocks accepted into the local mirror

- `Pending blocks`
  - blocks still waiting on missing predecessors

- `Unresolved preds`
  - unresolved predecessor hashes after bootstrap/backfill

- `Initial gRPC LFB`
  - node last-finalized block at the beginning of the run

- `Mirror LFB`
  - Cordial finality result over the mirrored blocklace

- `gRPC LFB`
  - node last-finalized block queried again after bootstrap

- `Mirror LFB Meta`
  - creator, block number, round, and wave for the mirror's final leader

- `gRPC LFB Meta`
  - creator and block number for the node's later finalized block

## Interpreting Drift

During live runs, the node may continue producing blocks while the harness is
bootstrapping.

So there are two different comparisons:

1. baseline comparison
   - `Mirror LFB` vs `Initial gRPC LFB`
   - this is the correct frozen-in-time comparison

2. moving-head comparison
   - `Mirror LFB` vs later `gRPC LFB`
   - this may differ simply because the node advanced during the run

Recent runs showed that:

- the mirror can match the initial node LFB exactly
- later mismatch can be explained by node advancement during bootstrap

That means a post-bootstrap mismatch is not automatically a correctness bug.

## Intended Use

This harness is intended for:

- validating live mirror reconstruction
- checking that the mirror reaches predecessor closure
- confirming Cordial finality over mirrored live blocks
- comparing mirror state to node-visible APIs at a known point in time
- debugging performance boundaries between bootstrap, finality, ordering, and
  HTTP comparison

It is not intended to be the final production runtime path. It is a diagnostic
and validation executable for the integration track.

## Current Conclusion

At this stage, the harness demonstrates that:

- live gRPC block access works
- height-based bootstrap reconstructs a closed local mirror
- Cordial finality can be computed over that mirrored state
- frozen-baseline finality can match the node snapshot taken at the start of
  the run

This makes the harness a successful validation tool for the live block
mirroring milestone.
