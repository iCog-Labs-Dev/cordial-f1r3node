# Cordial Miners Evidence Pool

## Purpose

Cordiality checks can detect that a validator equivocated, but slashing needs
more than a boolean decision. The network must retain the original conflicting
`CordialBlock` values so a later slashing path can verify the signatures,
validator identity, round, payload, and parent set exactly as they were observed.

The evidence pool is the pure-core retention layer for that proof. It does not
know how F1R3FLY serializes blocks on the network, how an adapter encodes them,
or how a slashing transaction will later be submitted. It only keeps the raw
core block objects and indexes them deterministically.

## Data Model

`EquivocationEvidence<V, P, Id>` contains:

- `validator`: the validator accused of producing conflicting blocks.
- `round`: the Cordial Miners round where the conflict happened.
- `blocks`: the original conflicting block objects.

The type is generic so the core can retain native block objects without pulling
host-chain serialization types into the consensus math. For the in-tree Cordial
core, `CordialEquivocationEvidence` is an alias for:

```rust
EquivocationEvidence<NodeId, Block, BlockIdentity>
```

`Block` is stored directly. The pool does not convert payloads into bytes, does
not re-sign blocks, and does not rebuild the structure. This is important
because later signature verification and slashing proof generation must operate
on the same block data that triggered equivocation detection.

## Pool Interface

The `EvidencePool<V, P, Id>` trait exposes two operations:

- `record_equivocation(validator, round, blocks)` records one conflicting block
  set for a validator and round.
- `evidence_for(validator)` returns all retained evidence for that validator in
  stable deterministic order.

`InMemoryEvidencePool` is keyed first by `(validator, round)`. Inside each
bucket, records are keyed by the sorted block identities in the conflicting set.
That gives two properties:

- Replaying the exact same conflicting pair does not duplicate evidence.
- Replaying the same pair in the opposite order still maps to the same evidence
  key.

The pool ignores records with fewer than two distinct block identities because a
single block is not cryptographic evidence of equivocation.

## Deterministic Query Order

The implementation uses `BTreeMap` for both the outer `(validator, round)` index
and the inner block-identity index. Querying by validator therefore returns a
stable ordering:

1. validator key order from the outer map,
2. increasing round for the queried validator,
3. sorted conflicting block-identity sets within the round.

This deterministic ordering matters for tests, replay, audits, and future proof
packaging.

## Boundary With F1R3FLY

The evidence pool is intentionally below the adapter boundary:

- It imports only Cordial Miners core types.
- It has no dependency on node, adapter, gRPC, protobuf, or network wire types.
- It performs no byte-array payload formatting.
- It does not decide slashing economics or build a slashing transaction.

The expected lifecycle is:

1. Cordiality logic detects conflicting blocks for a validator in a round.
2. The caller records the original blocks in `EvidencePool`.
3. Finality or slashing logic later asks `evidence_for(validator)`.
4. A higher layer verifies signatures and packages the retained raw blocks into
   whatever proof format the host chain requires.

