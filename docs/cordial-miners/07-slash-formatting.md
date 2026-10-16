# Cordial Miners Slash Formatting

## Purpose

The core evidence pool keeps `EquivocationEvidence` free of F1R3FLY types. The
adapter is the boundary where that pure evidence becomes a host-chain system
deploy that F1R3FLY can execute and replay.

CBC Casper creates slashing deploys in `BlockCreator.prepareSlashingDeploys` by
taking an invalid block hash and wrapping it in:

```text
SlashDeploy(invalidBlockHash, issuerPublicKey, slashRandomSeed)
```

During replay, F1R3FLY stores and replays the block-body form as
`ProcessedSystemDeployProto` containing:

```text
SystemDeployDataProto::slashSystemDeploy {
  invalidBlockHash,
  issuerPublicKey
}
```

The Cordial adapter emits that exact protobuf byte payload.

Tiered penalties are adapter metadata only; the host protobuf remains unchanged.
See [Slashing Integration](./16-slashing-integration.md) for issue #234's scope.

## Formatter API

`SlashDeployFormatter<V, P, Id>` exposes:

```rust
to_slash_system_deploys(
    evidence: &[EquivocationEvidence<V, P, Id>],
) -> anyhow::Result<Vec<Vec<u8>>>
```

For Cordial Miners, `F1r3SlashDeployFormatter` implements the trait for:

```rust
EquivocationEvidence<NodeId, Block, BlockIdentity>
```

`to_tiered_slash_deploys(evidence, penalty_ratio_bps)` returns
`FormattedSlashEvidence` containing the supplied ratio, existing host bytes,
and serialized conflicting blocks. It accepts `0..=10_000` basis points:
`2500` means 25%, and `10000` means 100%. Both produce the same host bytes.
The formatter does not calculate the ratio or depend on PoR mathematics.
The existing byte API remains available; `to_slash_evidence_envelopes` uses
`10000`. The trait's default tiered method returns an unsupported error, so
legacy formatter implementations continue compiling.

The formatter is constructed with the issuer public key. This mirrors CBC
Casper: the slash deploy identifies the invalid block being reported, while the
issuer key is the validator proposing the system deploy.

## Evidence Mapping

For each evidence record:

1. Require at least two conflicting blocks.
2. Require every conflicting block to be from the evidence validator.
3. Select the lowest conflicting content hash as the `invalidBlockHash`.
4. Serialize the conflicting blocks in identity order for the evidence envelope.
5. Encode a successful `ProcessedSystemDeployProto` with a slash system deploy.

The deterministic selection keeps repeated formatting stable even if the same
evidence arrives in a different block order.

## Signature Preservation

The F1R3FLY slash protobuf carries only `invalidBlockHash` and
`issuerPublicKey`; it does not carry raw conflicting block signatures. For that
reason, the adapter also exposes an evidence envelope containing:

- the byte-identical F1R3FLY slash system deploy bytes,
- serialized raw Cordial blocks from the evidence record.

Those raw block bytes preserve the original signatures and payloads for later
proof transport, auditing, and cryptographic verification. RSpace consumes the
slash system deploy bytes; proof packaging can consume the retained block bytes.

## Boundary Rules

- The core evidence pool remains pure and has no dependency on F1R3FLY models.
- The adapter uses real F1R3FLY protobuf/model types for slash deploy encoding.
- The formatter does not re-sign, re-hash, or mutate the original Cordial
  blocks.
- The protobuf encoding is deterministic and covered by byte-for-byte tests.

