# Penalty-aware PoR transition

Issue [#283](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues/283)
formalizes the state-changing boundary that applies authorized equivocation
events to a Proof-of-Reputation snapshot. The model is in
`lean/LeanVerification/SlashingTransition.lean`.

The original #283 transition primitives below model a fixed identity domain and
assume an admissible pre-penalty candidate. The [#284 reference](10-executable-slashing-reference.md)
extends the shared contract to candidate-set growth and explicitly validates
candidate exclusion provenance. Use `SlashingRef.transition` for conformance
admission, including rounds that add newly rated validators.

## Model

`ValidatorReputation` records active reputation, retained reputation, and the
permanent-exclusion bit for one validator. `PenaltyReputationState` adds the
round, the unique validator identities, and their accounts. Its
`WellFormed` predicate requires strictly increasing identities (and therefore
uniqueness), zero active reputation for every excluded key, and zero retained
reputation for every active key. These are the invariants checked by Rust's
`validate_reputation_entries`. `wellFormed_uniqueValidatorIds` exposes the
uniqueness consequence, and `penaltySuccessor_wellFormed` preserves all three
requirements.

```lean
structure ValidatorReputation where
  activeReputation : Nat
  retainedReputation : Nat
  excluded : Bool

structure PenaltyReputationState where
  round : Nat
  validatorIds : List NodeId
  account : NodeId → ValidatorReputation

def PenaltyReputationState.WellFormed
    (state : PenaltyReputationState) : Prop
```

`AuthorizedEquivocationEvent context` retains an authorization witness from
`AuthorizedPenaltyEvent` in the evidence model. The context fixes the
cryptographic verifier and protocol policy for the entire transition. Each
witness contains a valid blocklace view and the original authorized event,
with proofs binding its offender, evidence bytes, offense depth, and
application round to the consumed fields. Constructing the event directly
requires the same witness as using `ofAuthorizedPenaltyEvent`; nonempty bytes
alone are insufficient. The transition still independently rejects empty
serialized evidence, as Rust does.

`transitionEvent_policy_bound` exposes the preserved policy authorization for
the exact consumed fields. `deniedPolicy_noTransitionEvent` and
`deniedAuthentication_noTransitionEvent` prove that events cannot be constructed
when the fixed context denies authorization or authentication.

`ValidPenaltyEvents` checks the successor round, stable validator identities,
unique offenders, nonempty evidence, known and active offenders, one-round
inactivity, and absence of overlap across penalty kinds.

`previousActiveTotal` and `previousCorrelatedWeightFor` read only the previous
snapshot. Therefore neither blended candidate weights nor event order can
affect correlation-tier selection.

`calculatePenaltyTransition` returns `Except PenaltyTransitionError State`.
It first checks that both the previous and candidate snapshots are well formed,
returning `invalidState` if either is malformed. It exposes the staged
`penaltySuccessor` only after state validation, event validation, and all
fixed-width arithmetic preconditions pass. `commitPenaltyTransition` models
the state-level atomic commit: either the complete successor is installed or
the complete current state is returned unchanged.

After state and event validation, configuration checks run even without
equivocations. The weight-total and slashing-product checks
run only for a nonempty equivocation set, matching Rust's guarded slashing
branch. In particular, empty and inactivity-only penalty rounds may advance
over zero active weight. For a slash, the checked products are the correlated
weight for the offense round multiplied by the scale, and the previous total
multiplied by the correlation threshold. There is no total-times-scale check.
The retained-balance bound checks the selected survival fraction.
`arithmeticReady_checkedSelect` and `arithmeticReady_checkedRetained` connect
these conditions to the checked-arithmetic results proved in issue #282.

## Safety theorems

- `slash_zeroes_activeReputation` and `slash_setsExcluded` show that every
  valid slash permanently removes the offender's active voting weight.
- `isolatedSlash_setsRetainedReputation` connects the below-threshold tier to
  the base-penalty retained balance proved in `SlashingArithmetic.lean`.
- `correlatedSlash_zeroesRetainedReputation` connects the above-threshold tier
  to a full slash and zero retained balance.
- `slash_preserves_nonOffenders` proves that active innocent accounts retain
  the pre-penalty candidate value.
- `excluded_persistent` and `retainedReputation_persistent` prove that later
  candidates cannot resurrect an excluded key or replace its retained balance.
- `penaltySuccessor_wellFormed` proves that a transition preserves the state
  representation invariants.
- `calculatePenaltyTransition_ok_wellFormed` proves that every successful
  calculation returns a well-formed successor, without separate caller
  assumptions about its inputs. `commitPenaltyTransition_wellFormed` preserves
  a well-formed current state even when the candidate is malformed.
- `invalidState_rejected` and `invalidState_preserve_entireState` prove that
  either malformed input is rejected while preserving the entire current state.
- `invalidEvents_preserve_entireState` and
  `arithmeticFailure_preserves_entireState` cover the complete state rather
  than a single account field.
- `duplicateOffender_rejected`, `overlappingPenalty_rejected`,
  `unknownOffender_rejected`, `alreadyExcluded_rejected`, and
  `emptyEquivocationEvidence_rejected` establish fail-closed event handling.
- `eventPermutation_invariant` proves that any permutation of the authorized
  equivocation list produces the identical complete successor state.

The companion `SlashingTransitionExamples.lean` module contains executable
examples for the isolated and correlated tiers, preservation of an innocent
validator, every required rejection case,
reversed event order, and an attempted later restoration of an ejected key.
Its authorization fixtures prove actual same-creator, same-depth forks in
valid blocklaces. The test context explicitly assumes successful external
authentication and separately authorizes the application round; no additional
axioms are introduced.

Review regressions also cover zero-weight empty and inactivity-only rounds,
zero-weight slash rejection, a valid maximum-u64 slash whose total-times-scale
product would overflow, actual correlated-product overflow with atomic
rejection, invalid configuration without equivocations, and rejection of
unsorted/duplicate identities and inconsistent active/retained balances.
The state-validation regressions exercise the executable transition with each
malformed previous or candidate snapshot, including the empty-event candidate
whose event and arithmetic checks both pass. They also establish complete-state
equality after rejection of a malformed previous or candidate snapshot.

## Rust correspondence

| Lean definition or theorem | Rust behavior justified |
| --- | --- |
| `PenaltyReputationState.WellFormed`, `penaltySuccessor_wellFormed` | `commitments::validate_reputation_entries` canonical ordering and exclusion/balance consistency |
| `calculatePenaltyTransition_ok_wellFormed`, `invalidState_rejected` | previous-vector validation in `audit::replay_reputation_transition_with_penalties` and structurally valid pipeline candidates |
| `commitPenaltyTransition_wellFormed`, `invalidState_preserve_entireState` | fail-closed state admission and atomic audited commit |
| `transitionEvent_has_authorizedSource`, `transitionEvent_policy_bound` | intact authorization checked by `event.validate_authorization` |
| `ValidPenaltyEvents` | `transition::apply_reputation_penalties` pre-mutation validation |
| `noEquivocations_arithmeticReady` | guarded slashing branch in `transition::apply_reputation_penalties` |
| `arithmeticReady_checkedSelect`, `arithmeticReady_checkedRetained` | exact checked products and retained-balance calculation in `penalties.rs` |
| `previousActiveTotal`, `previousCorrelatedWeightFor` | `transition::apply_reputation_penalties` previous-vector correlation aggregation |
| `isolatedSlash_setsRetainedReputation` | `penalties::compute_slash_penalty_wide` and `apply_slash_to_reputation` base tier |
| `correlatedSlash_zeroesRetainedReputation` | full tier in `compute_slash_penalty_wide` and `apply_slash_to_reputation` |
| `slash_zeroes_activeReputation`, `slash_setsExcluded` | offender updates in `transition::apply_reputation_penalties` |
| `excluded_exportsZeroWeight` | `weights::authorized_validator_weights` |
| `excluded_persistent`, `retainedReputation_persistent` | persistence logic in `audit::replay_reputation_transition_with_penalties` and `state::apply_reputation_vector` |
| `invalidEvents_preserve_entireState`, `arithmeticFailure_preserves_entireState` | staged clone in `apply_reputation_penalties` and audited atomic commit in `ReputationState` |
| rejection theorems | duplicate, overlap, unknown, empty-evidence, and already-ejected checks in `apply_reputation_penalties` |
| `eventPermutation_invariant` | canonical, order-independent penalty replay |

All declarations are proved without `sorry` or additional axioms.
