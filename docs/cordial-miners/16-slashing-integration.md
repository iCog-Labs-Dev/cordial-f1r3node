# Cordial Miners Slashing Integration

## Purpose

Cordial Miners treats equivocation as a consensus safety failure with an
economic consequence. The adapter prepares slash requests before normal user
work.

This integration connects the pure evidence retained by the Cordial core to the
adapter's prepared execution batch:

1. the core `EvidencePool` retains raw conflicting Cordial blocks,
2. the adapter `SlashDeployFormatter` converts that evidence into f1r3node
   slash system deploy bytes,
3. the proposer prepares those system deploys before user deploys,
4. the execution engine receives the prepared request,
5. the proposer signs and broadcasts the resulting Cordial block.

For [issue #234's clarified scope](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues/234#issuecomment-6159081010),
`penalty_ratio_bps` remains adapter metadata. The host protobuf still carries
only the invalid block hash and issuer key. Applying the ratio and guaranteeing
slash-before-user runtime/replay execution require a future integration change;
this issue does not change the sibling `f1r3node` repository.

## Cost of Corruption

In the paper, cordiality and finality depend on validators making observable,
non-equivocating progress through the blocklace. If a validator creates
conflicting blocks for the same round, honest nodes can retain both blocks as
cryptographic proof.

The default PoR policy computes a 25% penalty when equivocating validators hold
at most 30% of the round's active reputation weight, and 100% above that
threshold. These become `2500` and `10000` basis points in the adapter.
The prepared slash prefix expresses the intended order; it does not establish
that the host enforces the percentage or executes slashes before user contracts.

## Proposer Flow

The Cordial proposer pipeline in
`crates/cordial-f1r3node-adapter/src/proposer.rs` supports tiered preparation
through `with_slash_policy(policy)`. The existing `with_slashing` path remains
available without that policy.

```rust,ignore
use cordial_f1r3node_adapter::tiered_slashing::TieredSlashPolicy;

let policy = TieredSlashPolicy::new(por_config)?
    .with_round_weights(evidence_round, authorized_reputation_weights)?;
let proposer = proposer
    .with_slashing(evidence_source, slash_formatter)
    .with_slash_policy(policy);
```

The caller must supply the Cordial-authorized active reputation weights for each
core evidence round. Current proposal bonds, block numbers, and PoR rating
rounds are not substitutes. The adapter does not authenticate these snapshots.
Missing or invalid inputs fail explicitly; totals must fit `u64`, and penalty
fractions must convert exactly to whole basis points.

With a tiered policy configured, the flow is:

1. `TipSelector::select_tips`
   selects parent block identities from the blocklace. The proposer derives the
   execution pre-state before preparing slash requests.

2. `EvidenceSource::pending_evidence`
   queries the core evidence layer. The provided `EvidencePoolSource` wraps a
   core `EvidencePool<NodeId, Block, BlockIdentity>` and asks for evidence by
   validator. It returns raw `EquivocationEvidence` values without formatting or
   protobuf conversion.

3. `EvidenceSource::equivocating_validators_in_round`
   queries all locally retained offenders for each selected evidence round,
   including those whose proof was not selected. `TieredSlashPolicy` counts
   each validator's weight once and calls `cordial_por::compute_slash_penalty`.

4. `SlashDeployFormatter::to_tiered_slash_deploys`
   receives evidence in deterministic round/validator/proof order and the
   computed ratio. The proposer checks the output count, ratio, and block hash.

5. `DeployPool::select_for_block`
   selects normal user deploys after slash preparation, preserving the existing
   scope, expiration, and selection rules.

6. `PreparedExecutionRequest::batch()`
   exposes the adapter's preparation order:

   ```text
   [slash system deploys..., selected user deploys..., optional close block]
   ```

   User deploy order is preserved after the system prefix. Each slash item
   retains its validator, evidence round, ratio, and formatted proof package.

   The adapter preserves one slash request per evidence record. Multiple proofs
   for the same `(validator, round)` therefore produce multiple requests, not
   independent economic offenses. Before percentage enforcement is connected,
   host integration must consolidate punishment by `(validator, round)` or make
   it idempotent: repeated evidence, proposals, or replay must not cause repeated
   deductions for the same offense. This adapter does not implement that host
   punishment contract.

7. `ExecutionEngine::execute_prepared`
   receives the prepared request and returns an `ExecutionResult`. Its default
   implementation calls `execute` with `into_legacy_request()`, which drops the
   extra metadata and keeps separate user/system deploy lists. This compatibility
   path does not apply tiered penalties or enforce the prepared runtime order.

8. `BlockSigner::sign_block`
   signs the block content containing the execution results and selected parents.
   The proposer packages those results into `CordialBlockPayload` first.

9. `BlockBroadcaster::broadcast`
   publishes the signed block to peers. The proposer activates post-state bonds
   for the next proposal only after execution, signing, and broadcast succeed.

## Isolation Boundary

The integration keeps the core and adapter separated:

- `EvidencePool` stores and returns generic raw evidence.
- `EvidencePoolSource` only reads from that core pool.
- `SlashDeployFormatter` is the first f1r3node-specific formatting boundary.
- `ExecutionBatchItem::SlashSystemDeploy` retains the formatted evidence package.
- Penalty calculation stays in `TieredSlashPolicy`, outside the formatter.
- The core never imports adapter, node, RSpace, protobuf, or PoR types.

This lets consensus math remain pure while the adapter handles host-chain
serialization and execution details.

## Test Coverage

The focused tests inspect adapter requests, not real host punishment or replay:

- `tests/test_slashing.rs`: unchanged host bytes, retained ratios, original
  signatures, deterministic proof order, and invalid ratios.
- `tests/test_tiered_slashing.rs`: threshold boundaries, distinct round snapshots,
  duplicate offenders, complete round correlation, prepared slash-before-user
  order with and without close-block, input failures, formatter checks, legacy
  engine compatibility, and signed broadcast/peer validation using the existing
  mock runtime.
- `tests/test_proposer.rs`: existing proposal and execution-request regressions,
  including a real core `CordialEvidencePool` feeding `EvidencePoolSource`.

Run from the repository root:

```bash
cargo +nightly test --locked -j 1 -p cordial-f1r3node-adapter --test test_slashing --test test_tiered_slashing --test test_proposer
```

A single build job limits memory use when linking native host dependencies.
