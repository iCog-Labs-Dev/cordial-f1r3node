# Cordial Miners Progress Report

**Date:** May 29, 2026  
**Scope:** Core Cordial Miners consensus, dissemination simulation, and f1r3node/Casper integration progress.

## Executive Summary

The Cordial Miners work has progressed from the core blocklace data structure toward a usable consensus pipeline. The main path now covers blocklace construction, structural validation, equivocation detection, cordial support predicates, wave-based leader finality, tau ordering, dissemination helpers, simulation coverage, and adapter-facing snapshot output.

In short, the implemented path is:

```text
Blocklace DAG
  -> validation and equivocation handling
  -> cordial predicates
  -> wave leader finality
  -> tau ordering
  -> dissemination support and simulation
  -> f1r3node/Casper snapshot integration
```

## Feature Status

| Area | Status | Summary |
| --- | --- | --- |
| Blocklace data structure | Completed | Core DAG structure, block identities, predecessor links, closure/observation behavior, and local blocklace views are implemented. |
| Validation | Completed | Block insertion is protected by structural validation, missing predecessor checks, creator-chain checks, and configurable validation behavior. |
| Equivocation detection | Completed | Same-creator conflicting blocks can be detected, including visible and hidden equivocation cases used by cordiality logic. |
| Round/depth model | Completed | Blocks are grouped by DAG depth, which provides the round model used by waves and finality. |
| Wave model | Completed | Rounds are grouped into waves with configurable wavelength, supporting the eventual synchrony path. |
| Approval predicates | Completed | Blocks can approve targets only when observation is valid and equivocation conflicts are excluded. |
| Ratification and super-ratification | Completed | Cordial support predicates are implemented for leader support and finality checks. |
| Leader finality | Completed | Wave leader blocks can be selected and checked for finality through super-ratification. |
| Tau ordering | Completed | Final leader blocks anchor deterministic topological ordering while excluding equivocations. |
| Ordering cache | Completed | Tau and previous-leader calculations include cache support to avoid repeated expensive traversal. |
| Weighted support path | Completed | Stake-weighted approval/finality/order paths exist alongside the paper-style count-based path. |
| Dissemination helpers | Completed | Predecessor selection, acknowledgement thresholds, proposal construction, and pending block buffering are implemented. |
| Dissemination simulation | Completed | Simulation module covers local nodes, delivery queues, buffering, recovery, weighted/unweighted paths, and adversarial scenarios. |
| f1r3node/Casper adapter snapshot | Completed | Snapshot construction exposes blocklace-derived state, final ordering, bonds, validators, and DAG representation to the adapter layer. |
| Full production networking | In progress | Core dissemination logic exists, but full live networking behavior remains an integration task. |
| End-to-end f1r3node execution | In progress | Adapter-side tests exist, but full real Rholang execution through f1r3node runtime is still a follow-up. |

## Core Consensus Progress

### 1. Blocklace Foundation

The blocklace is the structural foundation of the protocol. It represents each node's local view as a DAG instead of a single chain. Blocks point to earlier blocks through predecessor hashes, and this creates the observation relation used by later consensus logic.

Implemented files include:

- `crates/cordial-miners-core/src/blocklace.rs`
- `crates/cordial-miners-core/src/block.rs`
- `crates/cordial-miners-core/src/consensus/round.rs`
- `crates/cordial-miners-core/src/consensus/wave.rs`

Accomplished:

- Core block and block identity model.
- Blocklace insertion and traversal.
- Ancestor/closure-style observation behavior.
- Depth/round computation.
- Wave grouping over rounds.

### 2. Validation and Equivocation Handling

Validation protects the structural soundness of the DAG before blocks are accepted into the local blocklace. Equivocation handling detects when a creator produces incompatible block histories.

Implemented files include:

- `crates/cordial-miners-core/src/consensus/validation.rs`
- `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `crates/cordial-miners-core/src/consensus/approval.rs`

Accomplished:

- Missing predecessor detection.
- Creator-chain and structural consistency checks.
- Same-round/same-creator equivocation detection.
- Hidden equivocation detection through observed history.
- Approval logic that excludes equivocating targets.

### 3. Cordial Predicates

Cordial predicates provide the support model used to move from local DAG evidence to finality.

Implemented files include:

- `crates/cordial-miners-core/src/consensus/approval.rs`
- `crates/cordial-miners-core/src/consensus/cordiality.rs`

Accomplished:

- Approval: a block observes a target without observing a conflicting equivocation.
- Ratification: a block's closure contains a supermajority of approving blocks.
- Super-ratification: a witness set contains a supermajority of ratifying blocks.
- Count-based supermajority path.
- Stake-weighted supermajority path.

Key rule:

```text
supermajority support > (n + f) / 2
```

For weighted mode, support is calculated by bond/stake weight instead of simple validator count.

### 4. Leader Finality

Leader finality connects waves, leader selection, and super-ratification. For the eventual synchrony path, waves are modeled with a fixed wavelength, and the leader block of a wave becomes final when it is super-ratified within that wave.

Implemented files include:

- `crates/cordial-miners-core/src/consensus/finality.rs`
- `crates/cordial-miners-core/src/consensus/wave.rs`

Accomplished:

- Leader block lookup for a wave.
- Final leader detection.
- Latest final leader discovery.
- Weighted final leader path.
- Tests for leader finality behavior.

### 5. Tau Ordering

Tau ordering turns the partially ordered blocklace into a deterministic linear output. It uses final leader blocks as anchors, topologically sorts approved reachable blocks, excludes equivocating blocks, and preserves previous output as a prefix.

Implemented files include:

- `crates/cordial-miners-core/src/consensus/ordering.rs`

Accomplished:

- `xsort` for deterministic topological ordering.
- `tau` and `weighted_tau`.
- Previous final leader chaining.
- Equivocation-aware ordered fragments.
- Append-only/monotonic output behavior.
- Memoization cache for repeated ordering operations.

## Dissemination Progress

Dissemination work focuses on how nodes create blocks, disclose known history, recover missing predecessors, and eventually converge local blocklace views.

Implemented files include:

- `crates/cordial-miners-core/src/consensus/dissemination.rs`
- `crates/cordial-miners-core/src/simulation/dissemination.rs`

Accomplished:

- Validator-visible tip selection.
- Deterministic predecessor selection.
- Sorted predecessor output for stable block content.
- Required acknowledgement threshold.
- Weighted acknowledgement threshold.
- `next_block_predecessors` helper.
- `build_block_candidate` helper.
- Pending block buffer for missing predecessors.
- Retry flow for buffered blocks once dependencies arrive.

Simulation coverage includes:

- Local node block receipt.
- Missing predecessor buffering.
- Retrying buffered blocks.
- Network delivery queues.
- Partition and heal behavior.
- Equivocation-style adversarial scenarios.
- Weighted and unweighted finality/order paths.

## Integration Progress

The integration work exposes consensus output to the f1r3node/Casper-facing layer without forcing the pure consensus core to depend on f1r3node runtime internals.

Implemented files include:

- `crates/cordial-f1r3node-adapter/src/snapshot.rs`
- `crates/cordial-f1r3node-adapter/tests/test_snapshot.rs`
- `crates/cordial-f1r3node-adapter/tests/test_casper_adapter.rs`
- `crates/cordial-f1r3node-adapter/tests/test_block_translation.rs`
- `crates/cordial-f1r3node-adapter/tests/test_crypto_bridge.rs`
- `crates/cordial-f1r3node-adapter/tests/test_grpc_ingest.rs`
- `crates/cordial-f1r3node-adapter/tests/test_shard_conf.rs`

Accomplished:

- Snapshot construction from blocklace state.
- DAG representation for adapter-facing consumers.
- Latest finalized block lookup.
- Ordered finalized block hashes.
- Casper shard configuration model.
- On-chain Casper state representation.
- Validator/bond ordering.
- Adapter-side tests for snapshot, translation, crypto bridge, gRPC ingestion, and shard configuration.

Current integration boundary:

```text
cordial-miners-core
  -> pure consensus, ordering, dissemination, simulation

cordial-f1r3node-adapter
  -> snapshot/API layer for Casper/f1r3node-facing consumers
```

This split keeps the consensus core independent while allowing adapter-specific logic to evolve separately.

## Testing Progress

Core consensus test coverage includes:

- Block and blocklace behavior.
- Hashing and signatures.
- Depth/round computation.
- Wave structure.
- Validation.
- Approval.
- Ratification and weighted ratification.
- Cordiality and equivocation behavior.
- Finality.
- Ordering and tau behavior.
- Dissemination helpers.
- Dissemination simulation.
- Consensus simulation.

Adapter test coverage includes:

- Snapshot construction.
- Casper adapter behavior.
- Block translation.
- Crypto bridge.
- gRPC ingestion.
- Shard configuration.

## In Progress

| Area | Current State | Next Action |
| --- | --- | --- |
| Live dissemination integration | Core helper and simulation logic exists. | Wire the simulation-backed behavior into the live node/network path. |
| f1r3node runtime execution | Adapter boundary exists and snapshot tests exist. | Add an end-to-end test that executes a real deploy through f1r3node/Rholang runtime. |
| Production adapter completeness | Snapshot and translation pieces exist. | Continue closing trait/API gaps required by the real Casper integration surface. |
| Documentation | Presentation and progress docs exist. | Keep docs aligned with implementation as issues land. |

## Remaining Risks and Open Questions

- Live networking may reveal timing and buffering cases that are not fully captured by the simulation.
- The adapter snapshot exposes consensus output, but full f1r3node runtime execution still needs an end-to-end validation path.
- Weighted and unweighted paths both exist; downstream users must be clear which mode they are using.
- Tau ordering is local and deterministic over a node's local blocklace. Nodes converge as dissemination fills missing history and final leader evidence aligns.

## Recommended Next Steps

1. Wire dissemination behavior into the live node/network layer.
2. Add an end-to-end f1r3node execution test using real runtime state.
3. Expand integration tests around snapshot-to-adapter consumption.
4. Add a small architecture diagram to the docs showing core consensus vs adapter boundary.
5. Keep issue tracking split by feature area: dissemination, adapter runtime, weighted mode, and production networking.

## Current Takeaway

The core Cordial Miners consensus path is largely implemented from DAG construction through finality and ordering. The main remaining work is less about the mathematical core and more about production integration: live dissemination behavior, full f1r3node execution, and adapter completeness.
