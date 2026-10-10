# Cordial PoR Tiered Slashing & Permanent Key Ejection Specification

## Document Context

- **Document ID**: `crates/cordial-por/docs/01-tiered-slashing-and-key-ejection.md`
- **Status**: Approved Architecture Specification
- **Related Specs**:
  - [`00-architecture.md`](./00-architecture.md)
  - [`data-structures.md`](./data-structures.md)
  - [`docs/cordial-miners/16-slashing-integration.md`](../../../docs/cordial-miners/16-slashing-integration.md)

---

## 1. Overview & Motivation

### Problem Statement

The initial Cordial consensus slashing implementation applies an immediate 100% binary zero-reputation slash upon detecting any equivocation. While economically secure against malicious double-signing, this model is brittle:

- A minor software bug, failover configuration error, or network partition causing a single honest node operator to double-sign results in total capital wipeout.
- If multiple honest nodes experience the same client bug concurrently, the network risks losing its consensus supermajority ($>66.7\%$ active weight), causing a catastrophic liveness halt.

### Inspiration from Ethereum PoS

This specification transitions `cordial-por` from binary slashing to a **Tiered Correlation Slashing with Permanent Key Ejection** model **inspired by Ethereum Proof-of-Stake (PoS)** anti-correlation principles (Casper FFG / LMD-GHOST):

**What Ethereum actually does:**

- **Anti-Correlation Scaling**: Ethereum applies a small immediate penalty (~1/32 of effective balance ≈ 1 ETH out of 32 ETH), followed by a later **correlation penalty** proportional to the total slashings within a ~36-day removal window. An isolated double-sign results in a total penalty well below 0.1% of stake, while correlated equivocations involving a large fraction of the validator set scale up toward a 100% penalty (full 32 ETH burn).
- **Permanent Ejection**: Slashed Ethereum validators are forcibly exited and barred from re-entering under the same key. The operator retains their remaining (largely un-slashed) capital and can register a fresh validator key, but Ethereum does not define a specific percentage-based transfer mechanism.
- **Inactivity Leak**: Offline validators undergo gradual quadratic balance decay without slashing, preserving liveness without sudden state locks.

**What Cordial defines on top of these principles:**

- **25% Step Function Penalty**: Rather than Ethereum's continuous proportional formula, Cordial applies a fixed 25% immediate reputation reduction for isolated faults (see §2).
- **30% Correlation Threshold**: If >30% of active weight equivocates in a single round, the penalty escalates to 100% (complete wipeout).
- **75% Capital Transfer to Fresh Key**: Operators retain 75% of their reputation/capital after an isolated fault and can transfer it to a newly registered validator key.
- **Inactivity Decay**: Offline nodes undergo slow fixed-point decay ($\gamma$) without key ejection, preserving liveness without sudden state locks.

---

## 2. Slashing & Penalty Model

### Metric Matrix

| Metric | Legacy Binary Model | Proposed Cordial Model (Ethereum-inspired) |
| :--- | :--- | :--- |
| **Initial Equivocation Penalty** | 100% loss | **25% immediate reputation/stake reduction** |
| **Network Correlation Scaling** | None | **If $>30\%$ of active weight equivocates in round $k$, penalty scales to 100%** |
| **Validator Key Status** | Immediate zero-reputation | **Permanent key ejection (`is_excluded = true`)** |
| **Operator Recovery Path** | None (Destroyed) | **Re-register fresh validator key with remaining 75% capital** |
| **Downtime / Liveness Policy** | Undefined | **Inactivity leak (fixed-point reputation decay $\gamma$)** |

### Anti-Correlation Formula

For round $k$, let $W_{\text{equivocating}}$ be the total active reputation weight of nodes equivocating in round $k$, and $W_{\text{total}}$ be the total active network reputation weight:

$$\text{CorrelatedRatio}(k) = \frac{W_{\text{equivocating}}}{W_{\text{total}}}$$

The slash penalty ratio $\text{PenaltyRatio}(k)$ applied to all equivocating nodes in round $k$ can be evaluated using either a step function or a continuous linear scale:

#### Primary Step Function Model

$$\text{PenaltyRatio}(k) = \begin{cases} 0.25 & \text{if } \text{CorrelatedRatio}(k) \le 0.30 \\ 1.00 & \text{if } \text{CorrelatedRatio}(k) > 0.30 \end{cases}$$

#### Post-Slash Reputation Update Formula

For each equivocating node $i$:

$$\text{Reputation}_{\text{new}, i} = \text{Reputation}_{\text{old}, i} \times \left(1 - \text{PenaltyRatio}(k)\right)$$

---

• If CorrelatedRatio (k) ≤ 30% (Isolated Fault): Node retains 75% of its reputation/capital and can register a new key.
• If CorrelatedRatio (k) > 30% (Coordinated Attack): Node retains 0% (complete wipeout).
• Key Ejection: Regardless of the ratio, the validator key is marked is_excluded = true and exports weight 0.

---

## 3. Validator Key Lifecycle & State Machine

```mermaid
stateDiagram-v2
    direction LR

    [*] --> Active: Register Validator Key

    state Active {
        direction TB
        [*] --> Participating
        Participating --> Participating: Flawless Participation 
        Participating --> Participating: Inactive Period (Decay γ) 
    }

    Active --> Ejected: Equivocation Detected

    state Ejected {
        direction TB
        [*] --> Retired
    }

    Ejected --> [*]: Key Permanently Retired(Weight = 0)
    Ejected --> Active: Register NEW Key(75% reputation retained)

    note right of Active
        Reputation accrues or decays
        continuously while active
    end note

    note right of Ejected
        Slashing is permanent for the
        equivocating key — capital
        survives, identity does not
    end note

    classDef activeState fill:#2E7D32,stroke:#1B5E20,color:#000,font-weight:bold
    classDef ejectedState fill:#C62828,stroke:#8E0000,color:#000,font-weight:bold
    classDef terminalState fill:#616161,stroke:#333,color:#fff

    class Active activeState
    class Ejected ejectedState
```

### Lifecycle Rules

1. **Active State**: Node participates in consensus, receives ratings, and exports weight $W = \text{ReputationValue}$.
2. **Equivocation Event**:
   - `ReputationState` calculates $\text{PenaltyRatio}(k)$.
   - $\text{ReputationValue}_{\text{new}} = \text{ReputationValue}_{\text{old}} \times (1 - \text{PenaltyRatio}(k))$.
   - Node status is marked as `is_excluded = true`.
3. **Ejection & Weight Export**:
   - `reputation_weights()` in `cordial-por` maps `is_excluded` nodes to weight `0` (or omits them from the active validator set).
   - Ejected keys are permanently barred from consensus proposing or voting.
4. **Key Rotation & Re-registration**:
   - The operator retains $\text{ReputationValue}_{\text{new}}$ (75% for isolated faults).
   - To resume validation, the operator generates a new cryptographic public key (`NodeId_new`) and submits a key registration deploy transferring the remaining balance/reputation to `NodeId_new`.

---

## 4. Architectural Boundaries & Crate Responsibilities

The implementation enforces strict separation of concerns across workspace crates:

```mermaid
flowchart TD
    subgraph Core["cordial-miners-core"]
        EvidencePool["EvidencePool<br/>Group equivocations by round"]
    end

    subgraph Adapter["cordial-f1r3node-adapter"]
        Formatter["F1r3SlashDeployFormatter<br/>(SlashSystemDeploy protobuf)"]
        Proposer["Proposer Batching<br/>(Top-of-batch system deploys)"]
        RSpace["RSpace Host Execution"]
    end

    subgraph PoR["️ cordial-por"]
        State["ReputationState<br/>(is_excluded flag)"]
        Transition["transition.rs<br/>(Apply Tiered Slash & Decay)"]
        Weights["weights.rs<br/>(reputation_weights ⇒ weight 0)"]
        Audit["audit.rs<br/>(Replay Audit Verification)"]
    end

    EvidencePool -->|"equivocation evidence"| Formatter
    Formatter -->|"formatted deploy"| Proposer
    Proposer -->|"batched"| RSpace
    RSpace -->|"executes"| State
    State -->|"drives"| Transition
    Transition -->|"zeroes out"| Weights
    State -.->|"verified by"| Audit

    classDef coreStyle fill:#1565C0,stroke:#0D47A1,color:#fff,font-weight:bold
    classDef adapterStyle fill:#6A1B9A,stroke:#4A148C,color:#fff,font-weight:bold
    classDef porStyle fill:#2E7D32,stroke:#1B5E20,color:#fff,font-weight:bold

    class EvidencePool coreStyle
    class Formatter,Proposer,RSpace adapterStyle
    class State,Transition,Weights,Audit porStyle
```

### 1. `cordial-por` (Pure Math & State Container)
- **`src/types.rs`**: Adds `is_excluded: bool` to `ReputationEntry` and `ReputationState`.
- **`src/transition.rs`**: Applies fixed-point $\text{PenaltyRatio}$ reduction and inactivity decay factor $\gamma$.
- **`src/weights.rs`**: Exports `0` weight for `is_excluded` validator nodes.
- **`src/audit.rs`**: Verifies proposed `ReputationBlock` instances against the tiered transition replay.

### 2. `cordial-miners-core` (Evidence Collection)
- Retains generic `EquivocationEvidence`.
- Aggregates equivocating validator IDs by round $k$ to supply total equivocating weight calculations.

### 3. `cordial-f1r3node-adapter` (Host System Deploy Integration)
- **`src/slashing.rs`**: Formats tiered slash evidence into `SlashSystemDeployDataProto` system deploy bytes.
- **`src/proposer.rs`**: Places slash system deploys at the top of host execution batches in RSpace prior to user deploys.

---

## 5. Work Breakdown & Issues

| Issue ID | Scope / Crate | Title | Dependencies |
| :--- | :--- | :--- | :--- |
| **Issue #1** | `cordial-por` | Add Permanent Key Ejection State & `0`-Weight Export | None |
| **Issue #2** | `cordial-por` | Implement Tiered Slashing Math & Inactivity Decay in `transition.rs` | Issue #1 |
| **Issue #3** | `cordial-miners-core` | Round-Correlated Evidence Aggregation in `EvidencePool` | None |
| **Issue #4** | `cordial-f1r3node-adapter` | Slash System Deploy Formatter & Top-of-Batch Proposer Wiring | Issues #1, #3 |
| **Issue #5** | Integration Tests | End-to-End Tiered Slashing & Key Rotation Harness | Issues #2, #4 |

---

## 6. Verification & Test Plan

1. **Isolated Fault Test**:
   - Inject equivocation evidence for 1 validator node ($<30\%$ weight).
   - Verify node reputation drops by 25%.
   - Verify `reputation_weights()` omits the ejected key and the authorized validator projection gives it weight `0`.
   - Verify remaining 75% capital can be attached to a newly registered validator key.

2. **Coordinated Attack Test**:
   - Inject equivocation evidence for $>30\%$ of active validator weight in round $k$.
   - Verify all offending nodes suffer 100% reputation reduction.
   - Verify all offending keys are marked `is_excluded = true`.

3. **Audit Replay Conformance Test**:
   - Verify `verify_reputation_transition()` passes for blocks incorporating tiered slashes and rejects blocks with incorrect penalty math.


## 7. Penalty calculation API

`transition::{compute_slash_penalty, apply_slash_to_reputation,
compute_inactivity_decay}` exposes checked fixed-point arithmetic. Penalty
fractions are scaled to `PorConfig::scale`; invalid fractions and zero total
slash weight return the existing `PorError::InvalidConfiguration` variant.
The correlation threshold uses exact cross multiplication before rounding.

`ReputationPenaltyEvents` carries externally authenticated events for one
reputation round. Pass the same event set to
`replay_reputation_transition_with_penalties`,
`build_reputation_block_with_penalties`, and
`verify_reputation_transition_with_penalties`. Existing entry points delegate
with `None`, which commits to the canonical empty event set for that round.
`ReputationState::apply_reputation_block_with_penalties` audits before mutation.

The host must supply an agreed, finalized event set and authenticate equivocation
proofs. The library validates the round, offender membership and event shape.
It rejects empty or oversized evidence, duplicate/overlapping offenders, and
unknown or already-ejected keys. Inactivity requires absence from both sides of
the rating batch and exactly one missed round per consecutive transition.

Penalties run after blend/clamp and use previous active weights for both the
correlation ratio and deductions. An equivocator receives zero active reputation
and a permanent exclusion; its post-slash capital is stored separately in
`ReputationEntry::retained_reputation`. Audited state application commits the
list, exclusion registry and latest block atomically. Retained balances survive
later rounds and snapshots, cannot be overwritten by reputation assignment, and
never enter consensus-weight exports. Transfer to a fresh key still requires a
separate authenticated host lifecycle; this implementation does not authorize
or perform transfers. Explicit inactivity decays active reputation without ejecting.

`PorConfig::validate()` rejects invalid scale, alpha, initial reputation, rating
bounds and penalty fractions on every replay, blend, block construction and
verification, including rounds without penalty events. Runtime startup validates
the configuration before opening durable state.

The v2 block header includes `penalties_hash`, a canonical commitment to the
round, event categories, offender IDs, exact evidence bytes and missed-round
counts. Substituting or omitting events fails verification even if the arithmetic
result is identical. Event order within each category does not affect the hash.
The host must retain the evidence for replay; the block stores its commitment.

Configuration, reputation-list and block commitments use v2 domains. Block
headers, block wire envelopes and state snapshots are version 2. Retained balances
are covered by the reputation root and persisted in both block and state encodings.
Legacy v1 blocks and snapshots are rejected. Deployment requires a coordinated
upgrade and an explicitly prepared v2 checkpoint; no automatic v1 migration is
provided. Rating signatures and the adapter's outer signed publication format
remain v1.
