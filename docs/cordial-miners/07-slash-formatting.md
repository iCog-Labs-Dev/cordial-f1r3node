# Cordial Miners Slash Formatting

## Current migration boundary

The core evidence pool retains conflicting Cordial blocks without host-node
types. `F1r3SlashDeployFormatter` receives that evidence through the adapter's
`SlashDeployFormatter` trait. With the migrated `f1r3node-rust` host, the
formatter rejects nonempty evidence with a clear error. It does not emit slash
system deploy bytes. An empty evidence list still produces an empty list.

The new host's `SlashSystemDeployDataProto` requires `invalidBlockHash`,
`issuerPublicKey`, and `targetActivationEpoch`. The host resolves the invalid
block hash in its own DAG and checks that the referenced block is marked
invalid, the offender is bonded, and the evidence and proposed block belong
to the target epoch. These checks live in the sibling checkout's
`casper/src/rust/slashing_authorization.rs`.

The Cordial `BlockIdentity.content_hash` is an internal hash. It is distinct
from the host wire hash validated in `grpc_ingest.rs`. The core evidence record
also has no verified host block number or activation epoch. Choosing a Cordial
content hash or filling the epoch with zero would produce a slash deploy that
the host cannot authorize.

## Required integration work

To enable real slash formatting, carry a verified host wire hash and host DAG
metadata for each relevant Cordial block across the adapter boundary. Derive
the target epoch from the host's epoch rules and confirm it matches the current
proposal epoch. The formatter must then choose an invalid host block, include
the correct issuer public key, and encode all three protobuf fields. The
execution request must retain the target epoch through RSpace translation.

Keep the original conflicting Cordial blocks available for proof transport
and audits. The host slash protobuf contains a block reference, not the full
conflicting signatures. Tests should cover wire hash mapping, epoch boundaries,
restarts, and receive-side host authorization before enabling slash emission.

## Tiered formatter API

`SlashDeployFormatter<V, P, Id>` exposes `to_slash_system_deploys` and
`to_tiered_slash_deploys(evidence, penalty_ratio_bps)`. The tiered API retains
adapter metadata in `FormattedSlashEvidence`: the ratio, host bytes, and raw
conflicting blocks. It does not calculate penalties or depend on PoR mathematics.

`F1r3SlashDeployFormatter` accepts `0..=10_000` basis points, where `2500`
means 25% and `10000` means 100%. It validates this range before invoking the
same host authorization boundary as the byte API. Both tiers reject nonempty
evidence without verified host metadata; empty evidence produces no envelopes.
`to_slash_evidence_envelopes` delegates with `10000`. The trait's default tiered
method returns an unsupported error, so legacy implementations keep compiling.

The ratio remains adapter metadata and is not encoded into the host protobuf.
The envelope type retains space for serialized conflicting blocks, preserving
their signatures when an authorized formatter is available. See
[Slashing Integration](./16-slashing-integration.md) for issue #234's scope and
the prepared request interface.

## Proposer behavior

The proposer asks the formatter for deploys before execution. If evidence is
present and lacks verified host metadata, formatting returns `SlashFormat` and
the proposer does not execute, sign, or broadcast that proposed block. Tests
can supply a test formatter to exercise the proposer's ordering independently
of production slash authorization.
