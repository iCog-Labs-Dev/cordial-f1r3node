
# cordial-por Architecture

## Purpose

`cordial-por` is the dedicated crate for Proof-of-Reputation (PoR) state, reputation-derived weights, and (future) audit data that feed the weighted path of Cordial Miners.  

Cordial Miners approval, ratification, finality, τ-ordering and blocklace consensus rules remain exclusively inside `cordial-miners-core`.  
`cordial-por` computes and exports weights only; it never implements consensus.

## Design Goals

- Keep reputation state and weight export behind a clean crate boundary.
- Supply `HashMap<NodeId, u64>` (aliased as `ReputationWeight`) that the existing weighted APIs of `cordial-miners-core` can consume without modification.
- Remain a pure library; no networking, no block production, no finality logic.
- Provide a stable scaffold that later PoR stages (Liquid Rank, penalties, reputation blocks, etc.) can be added to without changing the integration contract.

## High-Level Architecture

```mermaid
flowchart TD
    subgraph External["External / Future"]
        Ratings["Rating / evidence sources"]
        Audit["Reputation blocks / audit path"]
    end

    subgraph PoR["cordial-por"]
        State["ReputationState"]
        Config["PorConfig"]
        Export["reputation_weights()"]
    end

    subgraph Core["cordial-miners-core"]
        Weighted["Existing weighted APIs\n(finality, fork-choice, τ)"]
        Finality["Finality"]
        Tau["τ ordering"]
        Approval["Approval / ratification"]
        Blocklace["Blocklace rules"]
    end

    Ratings -.->|future| State
    Audit -.->|future| State
    Config --> State
    State --> Export
    Export -->|"HashMap&lt;NodeId, ReputationWeight&gt;"| Weighted
    Weighted --> Finality
    Weighted --> Tau
    Approval -.->|does not own| PoR
    Blocklace -.->|does not own| PoR
```

## Internal PoR Architecture

The current crate is an intentional scaffold. Only the modules that exist today are shown.

```mermaid
flowchart TD
    Config["config::PorConfig\n(scale, initial_reputation)"]
    Types["types\n(ReputationRound, ReputationWeight,\nReputationEntry)"]
    State["state::ReputationState\n(BTreeMap&lt;NodeId, ReputationWeight&gt;)"]
    Weights["weights::reputation_weights()"]
    Error["error::PorError"]

    Config --> State
    Types --> State
    State --> Weights
    Error -.->|placeholder| State
```

### Future PoR Stages

> **Implementation Note:**  
> The stages listed below are part of the intended PoR architecture described in the paper (arXiv:2108.03542 and related Liquid-Rank literature) but are **not yet implemented** in the current codebase.

- Rating ingestion & validation  
- Round aggregation  
- Normalization  
- Liquid-Rank calculation  
- Penalties / slashing  
- Clamping / fixed-point conversion (beyond the scale constant already present)  
- Reputation state transition  
- Reputation block / audit path  
- Committee selection  

## Module Responsibilities

| Module | Responsibility | Inputs | Outputs | Dependencies | Public interfaces |
|--------|----------------|--------|---------|--------------|-------------------|
| `config` | Holds fixed-point scale and initial reputation | scale, initial value | `PorConfig` | none | `PorConfig::{new, default}` |
| `types` | Minimal type aliases and entry struct | — | `ReputationRound`, `ReputationWeight`, `ReputationEntry` | `cordial-miners-core::NodeId` | re-exported types |
| `state` | In-memory reputation map keyed by `NodeId` | round, validator → weight | `ReputationState` | `types` | `new`, `round`, `reputations`, `reputation_of`, `set_reputation` |
| `weights` | Export current reputation map for the weighted path | `&ReputationState` | `HashMap<NodeId, ReputationWeight>` | `state`, `cordial-miners-core::NodeId` | `reputation_weights` |
| `error` | Placeholder error type for future validation | — | `PorError` | none | `PorError::InvalidConfiguration` |
| `lib` | Crate root, re-exports | — | public API surface | all of the above | `PorConfig`, `PorError`, `ReputationState`, types, `reputation_weights` |

## Data Flow

1. A `PorConfig` is created (defaults: scale = 1_000_000_000, initial_reputation = 200_000_000).
2. A `ReputationState` is instantiated for a given `ReputationRound`.
3. Callers (or future ingestion logic) populate the state via `set_reputation`.
4. `reputation_weights(&state)` produces a `HashMap<NodeId, ReputationWeight>` that is handed to the weighted APIs of `cordial-miners-core`.
5. No further processing (finality, τ, approval) occurs inside `cordial-por`.

## Ownership Boundaries

### cordial-por owns

- Reputation state representation (`ReputationState`).
- Fixed-point scale and initial-reputation configuration.
- Conversion of the current reputation map into the weight map expected by Cordial Miners.
- Future PoR algorithms (Liquid Rank, penalties, reputation blocks) once implemented.

### cordial-por does NOT own

- Approval mechanics  
- Ratification  
- Finality detection  
- τ ordering  
- Blocklace consensus rules  
- Equivocation detection / exclusion  
- Networking or block production  

## Integration Contract

**Current implemented interface**

```rust
pub fn reputation_weights(state: &ReputationState) -> HashMap<NodeId, ReputationWeight>
```

where `ReputationWeight = u64` and `NodeId` is the type defined by `cordial-miners-core`.

**Intended integration contract** (already satisfied by the current function)

- `cordial-por` exports `HashMap<NodeId, u64>`.
- `cordial-miners-core` consumes those weights through its existing weighted APIs (finality stake summation, fork-choice scoring, etc.).
- No consensus behaviour is altered; weights are only an input parameter.
- Refresh / update lifecycle is currently caller-driven (`set_reputation` + re-export). Persistence ownership remains outside the crate.
- Adapter layer is trivial: the returned map is already in the form expected by the weighted path.

## Relationship with Cordial Miners Consensus

`cordial-por` computes weights.  
It does **not** implement:

- consensus  
- approval  
- ratification  
- τ ordering  
- finality  
- blocklace rules  

All of the above remain the exclusive responsibility of `cordial-miners-core`. The only coupling is the consumption of the weight map.

## Open Design Decisions

### All-validator reputation weights vs committee weights

- **Current implementation:** All validators present in `ReputationState` are exported.
- **Paper design:** Highest-reputation nodes form a consensus committee.
- **Future work:** Policy flag (reputation-only / committee-only / stake × reputation) inside the weight exporter.

### >2/3 finality threshold vs >50% committee threshold

- **Current implementation:** Threshold logic lives entirely in `cordial-miners-core` (supermajority of honest stake).
- **Paper design:** Committee of high-reputation nodes may use a lower internal threshold.
- **Future work:** Decide whether PoR only supplies weights or also influences the threshold constant.

### Fixed-point scale

- **Current implementation:** `PorConfig::DEFAULT_SCALE = 1_000_000_000`.
- **Paper design:** Liquid Rank produces real-valued ranks that must be scaled for integer arithmetic.
- **Future work:** Confirm scale, clamping rules and overflow behaviour once Liquid Rank is implemented.

### Reputation sidechain vs payload references

- **Current implementation:** No reputation blocks or sidechain.
- **Paper design:** Reputation updates may be carried as a sidechain or as payload references inside the main blocklace.
- **Future work:** Choose the audit / publication path and the corresponding data structures.

## Future Extensions

Logical extension points that do not yet exist:

- Rating ingestion pipeline (signed ratings, validation, round aggregation).
- Liquid Rank iterative computation (weighted liquid democracy formula).
- Penalty / slashing application that mutates `ReputationState`.
- Reputation-block construction and audit trail.
- Committee selection policy that filters the exported weight map.
- Persistence layer (snapshot / restore of `ReputationState`).
- Configuration-driven weight policies (reputation-only, stake-times-reputation, capped stake, committee-only).

None of the above are present in the current scaffold; they are documented solely as planned extension points.
