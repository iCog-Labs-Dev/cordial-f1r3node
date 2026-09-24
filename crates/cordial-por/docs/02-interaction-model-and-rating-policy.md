# Proof-of-Reputation Interaction Model and Rating Admission Policy

## Document Context

- **Document ID**: `crates/cordial-por/docs/02-interaction-model-and-rating-policy.md`
- **Status**: Draft Architecture Specification
- **Related Specs**:
  - [`00-architecture.md`](./00-architecture.md)
  - [`data-structures.md`](./data-structures.md)
  - [`01-tiered-slashing-and-key-ejection.md`](./01-tiered-slashing-and-key-ejection.md)

---

## 1. Overview and Motivation

The core `cordial-por` reputation pipeline is implemented:

```text
RatingRecord
  -> RatingBatch
  -> RatingMatrix
  -> NormalizedRatingMatrix
  -> Liquid-Rank contribution
  -> alpha-blended transition
  -> sigmoid clamp
  -> ReputationState
  -> ReputationBlock
  -> audit replay
  -> exported Cordial Miners weights
```

The remaining design question is not mathematical. It is input governance:

```text
Which real protocol events are allowed to become RatingRecord values?
```

The Proof-of-Reputation paper describes ratings as judgments produced after
interactions between nodes. It intentionally leaves the meaning of
"interaction" dependent on the system context. For this project, the context is:

```text
f1r3node execution and networking
  -> cordial-f1r3node-adapter
  -> Cordial Miners blocklace validation, ordering, and finality
  -> cordial-por reputation weights
```

Therefore, PoR ratings must not be arbitrary opinions. A valid rating must be
derived from a replayable, protocol-observable interaction.

---

## 2. Paper Reference

Primary reference:

- Oladotun Aluko and Anton Kolonin, "Proof-of-Reputation: An Alternative
  Consensus Mechanism for Blockchain Systems", IJNSA, 2021.

Relevant paper guidance:

- **Section 2.2, Reputation Systems**
  - Reputation is based on ratings received with respect to a particular
    interaction.
  - A rating is a judgment from one node, the origin, to another node, the
    target, in a scope.
  - A reputation system aggregates these ratings and disseminates reputation
    values.

- **Section 4.1, Node Transaction**
  - A node gives another node a rating with respect to an interaction.
  - Ratings are signed by the rater.
  - Rating transactions for a round are collected before the consensus phase.

- **Section 4.2, Reputation System**
  - Reputation is determined by analyzing ratings received from others.
  - Ratings reflect trust based on previous interactions.
  - Reputation values are open to members of the network so they can be
    audited.

Paper URL:

```text
https://arxiv.org/abs/2108.03542
```

This project follows the paper for the reputation model, but does not adopt PoR
as a standalone consensus protocol. Cordial Miners remains the consensus
engine. PoR supplies weights.

---

## 3. Core Design Principle

In this implementation, an interaction is:

```text
a protocol-observable event involving validator behavior or validator-produced
output that can be linked to deterministic evidence
```

The interaction model must preserve these properties:

- **Scoped**: the rating describes a specific validator behavior in a specific
  round or window.
- **Signed**: the rating is attributable to the rater.
- **Bounded**: the score uses configured rating bounds.
- **Replayable**: validators can reconstruct the admitted rating set from
  recorded evidence.
- **Auditable**: the rating links to evidence through `interaction_ref`.
- **Deterministic where possible**: ratings should be derived from verifiable
  protocol facts, not subjective preference.
- **Separated from slashing**: severe objective faults such as equivocation are
  deterministic penalty inputs, not ordinary subjective ratings.

---

## 4. Terms

### Interaction

An interaction is a verified protocol event that creates enough context for one
node or policy component to judge another node's behavior.

Examples:

- a validator publishes a block
- a block references known tips
- a block carries a valid execution result
- a validator participates in observable protocol communication

### Rating

A rating is the PoR input produced from an admitted interaction:

```text
RatingRecord {
    round,
    rater,
    recipient,
    score,
    signature,
    interaction_ref,
}
```

### Rater

The node that signs the rating. For normal PoR ratings, the rater should be an
active validator for the relevant round or window.

### Recipient

The node whose behavior is being rated. In Cordial Miners integration this is
usually the validator that produced the block, message, or protocol output.

### Interaction Reference

`interaction_ref` is the audit link from a rating to the protocol evidence that
justified it.

Examples:

```text
block_hash
deploy_hash
state_transition_hash
cordial_validation_result_hash
equivocation_evidence_hash
inactivity_evidence_hash
```

The first implementation can keep `interaction_ref` as bytes, but the policy
must define what those bytes mean for every admitted interaction kind.

---

## 5. Interaction Categories

### 5.1 Normal Rating Interactions

Normal rating interactions are ordinary protocol observations that can produce
`RatingRecord` values.

| Interaction | Recipient | Rater | Evidence |
|---|---|---|---|
| Valid block production | block producer | observing validator | block hash |
| Cordial tip references | block producer | validator checking cordiality | block hash plus known-tip view |
| Valid execution result | block producer | validator replaying or verifying execution | state transition hash |
| Valid deploy inclusion behavior | block producer | validator observing deploy batch rules | deploy or batch hash |

These interactions should feed the rating pipeline only after admission checks
pass.

### 5.2 Penalty Interactions

Penalty interactions are objective faults. They should not be treated as normal
ratings.

| Event | Target | Handling |
|---|---|---|
| Equivocation | offender key | slashing evidence |
| Permanent key ejection | offender key | `ReputationState` exclusion |
| Inactivity over a policy window | inactive validator | inactivity penalty or decay |
| Invalid block/state transition | producer | reject block and optionally emit evidence |

This distinction matters because ratings are aggregated by Liquid Rank, while
slashing and inactivity policies apply deterministic penalties.

### 5.3 No Interaction

No interaction means no rating.

A node that receives no ratings in a round should not automatically be punished
through ordinary ratings. Sparse rounds are handled by the configured
no-rating fallback policy, currently `CarryForward` by default.

Genuine inactivity should be handled by a separate inactivity evidence module,
not by inventing negative ratings without interaction evidence.

---

## 6. Rating Admission Rules

A `RatingRecord` is admissible only if all of the following hold:

1. **Round match**
   - `rating.round` must match the target reputation round.

2. **Distinct rater and recipient**
   - `rating.rater != rating.recipient`.

3. **Known active rater**
   - the rater must be eligible to rate for the round or window.
   - ejected validator keys must not create new ratings.

4. **Known recipient**
   - the recipient must be a validator key known to the reputation state or
     admitted as a new validator under the configured join policy.

5. **Bounded score**
   - `PorConfig::minimum_rating <= rating.score <= PorConfig::maximum_rating`.

6. **Signed rating**
   - the rating must carry a non-empty signature.
   - the adapter's signed-rating ingress verifies that the signature belongs
     to `rating.rater` before verified batch construction.

7. **Auditable interaction reference**
   - `interaction_ref` must be present for policy-generated ratings.
   - the referenced evidence must be available for replay.

8. **One rating per pair per round/window**
   - duplicate `(round, rater, recipient)` ratings are rejected.

9. **Policy-known interaction kind**
   - the interaction must match a known interaction category.

10. **Replayable score derivation**
    - given the same evidence and policy configuration, honest validators must
      derive the same admitted rating or reject it in the same way.

The current `ratings.rs` module already enforces the basic syntactic rules:

```text
round match
no self-rating
non-empty signature
score bounds
one rating per rater-recipient pair per round
canonical ordering
```

The interaction policy layer will enforce the semantic rules:

```text
real interaction evidence
known interaction kind
active rater eligibility
interaction_ref interpretation
score derivation
```

---

## 7. Score Policy

The paper requires ratings to be bounded values that reflect the rater's
judgment after an interaction. In this project, the scoring policy should be as
deterministic as possible.

Recommended first policy:

| Evidence Result | Rating Meaning | Score |
|---|---|---|
| Interaction verified as valid | positive behavior | `maximum_rating` |
| Interaction verified as degraded but not slashable | partial behavior | policy-defined midpoint |
| No interaction | no rating | none |
| Objective severe fault | penalty path | no ordinary rating |

This keeps the first implementation simple and auditable:

```text
valid interaction -> positive rating
no interaction -> no rating
equivocation / severe fault -> slashing evidence
```

More nuanced scoring can be introduced later, but each score level must have a
replayable evidence rule.

---

## 8. Dynamic System Flow

PoR runs beside Cordial Miners. It does not replace Cordial Miners.

Recommended round lifecycle:

```text
1. ReputationState_k exports weights W_k
2. Cordial Miners uses W_k for weighted finality and tau ordering in wave k
3. f1r3node and the adapter observe protocol events during wave k
4. Cordial Miners finalizes wave k
5. Interaction policy admits evidence-backed interactions from finalized wave k
6. cordial-por maps finalized wave k to rating round k+1
7. cordial-por batches ratings and computes ReputationState_{k+1}
8. a ReputationBlock_{k+1} can be built and audited
9. ReputationState_{k+1} exports weights W_{k+1}
10. Cordial Miners uses W_{k+1} in subsequent waves
```

The important timing rule is:

```text
wave k interactions
  -> finalized wave k
  -> rating round k+1
  -> ReputationState k+1
  -> subsequent Cordial Miners weights
```

Ratings must not affect the weights used to finalize the wave that produced
them. Advancing the finalized wave index with
`rating_round_from_finalized_wave` keeps this boundary explicit and prevents a
circular dependency between ratings and finality.

---

## 9. Ownership Boundaries

### `cordial-por` owns

- PoR data structures.
- Rating validation and deterministic batching.
- Rating matrix construction.
- Normalization.
- Liquid-Rank contribution.
- Alpha-blended transition.
- Sigmoid clamping.
- Reputation state application.
- Reputation block construction and audit replay.
- Weight export to Cordial Miners.
- This interaction policy specification.

### `cordial-por` does not own

- f1r3node networking.
- f1r3node block production.
- RSpace execution.
- Cordial Miners finality.
- Cordial Miners tau ordering.
- Cordial Miners blocklace validation.
- Evidence extraction from live protocol traffic.
- Validator private-key custody, rating signing, or signature verification
  against f1r3node key material.

### `cordial-miners-core` owns

- Blocklace data structures and validation.
- Cordiality checks.
- Weighted finality and weighted tau ordering.
- Equivocation detection primitives and evidence structures.

### `cordial-f1r3node-adapter` owns

- Translating f1r3node events into Cordial Miners structures.
- Observing execution and block-production results.
- Supplying protocol evidence that can become `interaction_ref`.
- Extracting and admitting interactions from finalized Cordial output.
- Validator private-key custody and canonical rating signatures.
- Verifying signed ratings before deterministic batch construction.
- Bridging a finalized wave atomically into its local signed rating batch.
- Collecting local and received evidence-backed ratings into a round batch.

---

## 10. Implemented Interaction Pipeline

The implemented block-production path is:

```text
OrderedFinalizedOutput
  -> canonical InteractionEvidence per producer
  -> AdmittedInteraction
  -> deterministic score
  -> canonical signed payload
  -> validator signature
  -> verified RatingRecord
  -> evidence-backed round collection
  -> deterministic multi-validator RatingBatch
```

`cordial-por/src/interactions.rs` owns the interaction vocabulary, admission,
and score policy. The adapter's `por_interactions.rs` extracts finalized
evidence, while `por_ratings.rs` owns signing, verification, and the atomic
`build_finalized_block_production_rating_batch` orchestration entry point.
The adapter's `por_rating_collector.rs` accepts local or received ratings,
reconstructs their finalized block-production evidence, and closes them into
one canonical round batch.

The collector enforces:

- a finalized-output anchor matching the opened rating round;
- a reputation state from the immediately preceding round;
- a valid signature from the declared rater;
- active, known raters and recipients through interaction admission;
- an exact match to the canonical finalized interaction reference;
- the deterministic score for that admitted interaction;
- one non-conflicting rating per `(rater, recipient)` pair;
- atomic insertion of a supplied per-validator batch.

The collector does not decide when enough ratings have arrived. Quorum,
deadline, and round-closing policy remain external so they can be defined with
the eventual rating transport protocol.

This preserves the required direction:

```text
finalized protocol evidence -> admitted and signed RatingRecord
```

and excludes:

```text
arbitrary node opinion -> RatingRecord
```

---

## 11. Relationship to Slashing

Ratings and slashing both affect reputation, but they are different mechanisms.

| Mechanism | Input | Purpose |
|---|---|---|
| Rating | interaction-backed score | gradual reputation learning |
| Liquid Rank | rating matrix and rater reputation | weight ratings by rater trust |
| Alpha blend | contribution and previous reputation | temporal smoothing |
| Clamp | next reputation value | bounded reputation growth |
| Slashing | objective fault evidence | deterministic punishment |
| Key ejection | slash result | remove unsafe key from weighted path |

Equivocation should not be converted into a subjective low rating. It should be
processed as evidence for the slashing path. Ordinary ratings should capture
normal protocol behavior that is not already handled by deterministic penalty
logic.

---

## 12. Canonical Rating-Signing Protocol

The version 1 signed-rating protocol resolves the signing boundary as follows:

1. The signed fields are `round`, `rater`, `recipient`, `score`, and
   `interaction_ref`. The signature field is excluded from its own payload.
2. The canonical encoding is a fixed-order binary layout. Integers and
   variable-field length prefixes are unsigned 64-bit big-endian values.
   Optional interaction references have a one-byte presence tag before their
   length and bytes.
3. Every payload begins with the domain separator
   `cordial-por:rating:v1`. Encoding revisions must use a new versioned domain.
4. The canonical bytes are hashed with Blake2b-256 before signing.
5. Ratings use secp256k1 ECDSA prehash signatures encoded as DER, matching the
   primary f1r3node validator identity convention.
6. `rating.rater` signs the rating. Its `NodeId` bytes are the SEC1-encoded
   secp256k1 public key used for verification.
7. Private-key access and signing remain adapter-owned. The adapter verifies
   locally produced ratings and verifies remotely supplied ratings before its
   verified batch entry point calls the structural `cordial-por` batch builder.
8. Empty, malformed, wrong-key, and payload-mismatched signatures are rejected.
9. Key parsing and signing failures propagate through the adapter's explicit
   signed-rating error instead of producing a partial rating or batch.
10. `interaction_ref` is mandatory for this interaction-derived signing path
    and is covered by the signature.

The canonical v1 byte layout is:

```text
"cordial-por:rating:v1"
|| round_u64_be
|| rater_len_u64_be || rater_bytes
|| recipient_len_u64_be || recipient_bytes
|| score_u64_be
|| interaction_ref_presence_u8
|| [interaction_ref_len_u64_be || interaction_ref_bytes]
```

This protocol deliberately does not place validator private keys in
`cordial-por`. That crate defines the deterministic payload; the adapter owns
the signing infrastructure and algorithm integration.

---

## 13. Remaining Open Design Decisions

The following decisions remain open beyond the version 1 signing protocol:

1. **Exact evidence encoding**
   - Should `interaction_ref` stay as raw bytes, or should it become a typed
     enum?

2. **Exact score levels**
   - Should the first policy be binary positive/no-rating, or should it include
     partial scores?

3. **Rater eligibility**
   - Should only active consensus validators rate, or can observer nodes submit
     ratings?

4. **Interaction window**
   - Should "one rating per pair" mean per round, per block, or per configured
     time window?

5. **Evidence availability**
   - Which evidence must be included in reputation blocks, and which evidence
     can be referenced by hash?

---

## 14. Acceptance Criteria for This Specification

- Defines "interaction" for this PoR implementation.
- Cites the PoR paper sections that guide the model.
- Explains why ratings must not be arbitrary opinions.
- Defines normal rating interactions separately from penalty interactions.
- Defines rating admission rules.
- Defines the role of `interaction_ref`.
- Explains that no interaction means no rating.
- Explains the one-round delay between observed interactions and exported
  Cordial Miners weights.
- Preserves the boundary that PoR supplies weights while Cordial Miners remains
  the consensus engine.
- Identifies future implementation locations without requiring code in this
  specification issue.
