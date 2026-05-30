# Cordial Miners Slashing Integration

## Purpose

Cordial Miners treats equivocation as a consensus safety failure with an
economic consequence. Detecting the fault is not enough: the next proposer must
make the cost of corruption executable by placing a slash system transaction in
front of normal user work.

This integration connects the pure evidence retained by the Cordial core to the
host f1r3node execution batch:

1. the core `EvidencePool` retains raw conflicting Cordial blocks,
2. the adapter `SlashDeployFormatter` converts that evidence into f1r3node
   slash system deploy bytes,
3. the proposer prepends those system deploys before user deploys,
4. RSpace executes the ordered batch and produces the block payload,
5. the proposer signs and broadcasts the resulting Cordial block.

## Cost of Corruption

In the paper, cordiality and finality depend on validators making observable,
non-equivocating progress through the blocklace. If a validator creates
conflicting blocks for the same round, honest nodes can retain both blocks as
cryptographic proof. f1r3node turns that proof into economic pressure by
executing a slash system deploy that removes or burns the attacker's stake.

The ordering is intentional: slash deploys are system transactions, not normal
mempool transactions. They represent protocol safety work and therefore run
before user smart contracts. A validator cannot delay punishment by filling the
block with user deploys first.

## Proposer Flow

The Cordial proposer pipeline in
`crates/cordial-f1r3node-adapter/src/proposer.rs` is:

1. `TipSelector::select_tips`
   selects live parent block identities from the blocklace.

2. `EvidenceSource::pending_evidence`
   queries the core evidence layer. The provided `EvidencePoolSource` wraps a
   core `EvidencePool<NodeId, Block, BlockIdentity>` and asks for evidence by
   validator. It returns raw `EquivocationEvidence` values without formatting or
   protobuf conversion.

3. `SlashDeployFormatter::to_slash_system_deploys`
   crosses the adapter boundary. It receives the raw evidence and emits
   f1r3node slash system deploy bytes. The core pool still has no dependency on
   f1r3node protobufs, RSpace, or node crates.

4. `DeploySource::pending_deploys`
   pulls normal user deploys from the mempool after slash evidence has already
   been queried and formatted.

5. `execution_batch`
   creates the exact execution order:

   ```text
   [slash system deploys..., user deploys...]
   ```

   Slash deploy order follows the deterministic evidence ordering supplied by
   the core pool and formatter. User deploy order is preserved after the system
   prefix.

6. `ExecutionEngine::execute`
   sends the ordered batch to the execution layer. For real f1r3node integration
   this is the RSpace-facing step that returns the `CordialBlockPayload`
   containing state hashes, processed user deploys, rejected deploys, and
   processed system deploys.

7. `BlockSigner::sign`
   wraps the payload bytes and selected parents into `BlockContent`, signs it,
   and returns the final Cordial block.

8. `BlockBroadcaster::broadcast`
   publishes the signed block to peers.

## Isolation Boundary

The integration keeps the core and adapter separated:

- `EvidencePool` stores and returns generic raw evidence.
- `EvidencePoolSource` only reads from that core pool.
- `SlashDeployFormatter` is the first f1r3node-specific formatting boundary.
- `ExecutionBatchItem::SlashSystemDeploy` stores already formatted system bytes.
- The core never imports adapter, node, RSpace, or protobuf types.

This lets consensus math remain pure while the adapter handles host-chain
serialization and execution details.

## Test Coverage

The proposer tests cover the issue requirements:

- evidence is queried and formatted before user deploys are pulled,
- formatted slash system deploy bytes are the first items in the execution
  batch,
- a real core `CordialEvidencePool` can feed the proposer through
  `EvidencePoolSource`,
- adapter formatting is mocked through the `SlashDeployFormatter` trait, proving
  that the core pool and formatter stay isolated by traits.
