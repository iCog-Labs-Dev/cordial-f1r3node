# Executable slashing reference

Issue [#284](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues/284)
adds `lean/LeanVerification/SlashingRef.lean` and executable regressions in
`SlashingRefExamples.lean`. Both are exported by `LeanVerification.lean`.

## Shared contract

The reference reuses the evidence, arithmetic, and transition definitions from
issues #281–#283. It does not introduce a parallel equivocation relation,
threshold policy, or slash calculation. In particular:

- `checkEvidence` calls `CMRef.checkEquivocation`, whose finite observation
  computation is already proved equivalent to `Equivocation`. It additionally
  checks the claimed offender and offense depth.
- `validateConfig`, `selectTier`, and `retained` are aliases of the checked
  functions in `SlashingArithmetic.lean`.
- `transition` decides `ReferencePenaltyReady` in the shared
  `SlashingTransition.lean` specification, then exposes its pure successor.
  The slashing stage is the existing `penaltySuccessor`. The full successor
  publishes the candidate identity list and copies newly rated accounts from
  that candidate, while existing keys use the shared slashing/inactivity stages.

The shared specification adds the supported inactivity stage: one missed round
uses `old * (scale - gamma) / scale`, with the **previous** active balance.
`gamma` is an explicit input and must not exceed the validated scale.
`ratingParticipants` explicitly supplies the union of raters and recipients;
an inactive offender may not appear in it. Inactivity decisions themselves
remain externally finalized inputs, as in the Rust penalty API.

`ValidPenaltyCandidate` enforces the pre-penalty candidate invariant for every
registered candidate identity: new and previously active keys must still be
active before penalties run. Well-formedness alone does not establish this
provenance. A hand-built candidate that invents an exclusion is rejected with
`invalidState`, even when there are no events or when the supplied slash names
a different validator. Existing registered exclusions are restored from the
previous snapshot, preserving their retained balances.

`ValidReferencePenaltyEvents` permits a canonical candidate set containing all
previous identities, including newly rated identities interleaved in sorted
order. It reuses #283's event checks through `penaltyValidationCandidate`,
which projects only the validation identity list to the previous domain; round,
duplicate, overlap, evidence, and prior-offender rules retain their original
meaning. Removing an old identity is rejected with `invalidValidatorSet`.

A newly rated key cannot be penalized in that same transition: Rust's
`prior_entry` requires each offender to exist in the previous snapshot. New
candidate weights do not enter the previous-round correlation denominator or
numerator. Values of the previous account function outside its registered
identity domain cannot establish an exclusion or replace a new account.

```lean
def checkEvidence (B : Blocklace) (hV : ValidBlocklace B)
    (evidence : RawEquivocationEvidence) : Bool

def transition (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (ratingParticipants : List NodeId) :
    Except Error PenaltyReputationState

structure ReferencePenaltyTransition (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (ratingParticipants : List NodeId)
    (successor : PenaltyReputationState) : Prop
```

Admission includes canonical state ordering and balance consistency, successor
round agreement, preservation of all previous validator identities, unique
offenders across both event categories, known active offenders, nonempty evidence, and supported
inactivity. It additionally checks u64 rounds and serialized balances,
the 64 KiB per-evidence limit, the one-million-event limit, and the checked
wide arithmetic preconditions from #283. Empty event sets are valid; the
nonempty requirement applies to each equivocation's serialized evidence.

`ReferencePenaltyTransition` is a relational contract: admission plus the
successor round, identities, and account values, specified fieldwise. It has
no dependency on `SlashingRef.transition`.

## Explicit trust inputs

`authorizeEvidence` accepts two explicit `Decidable` values for the exact
`EvidenceAuthenticated` and `PenaltyApplicationAuthorized` propositions. A
positive value contains the corresponding trusted proof. A negative value
produces a separate authentication or authorization error. Structural checking
never supplies either proof, and nonempty bytes never imply authentication.

The returned `AuthorizedEquivocationEvent context` preserves the full source
witness and all offender, evidence, offense-round, and application-round
bindings from #281. Test-only contexts stipulate successful crypto verification
and bind attestation and application round separately; they add no axioms.

## Equivalence and safety

- `checkEvidence_iff`: the Boolean checker succeeds exactly for
  `ValidEquivocationEvidence`. Its proof uses the existing equivocation-checker
  equivalence and reconstructs all evidence fields, including incomparability.
- `authorizeEvidence_success_iff`: admission succeeds exactly when structural
  validity, authentication, and protocol authorization all hold.
- `validateConfig_iff`: configuration checking accepts exactly the validated
  slashing fields with their Rust widths.
- `selectTier_iff`: checked tier selection succeeds exactly for valid bounded
  inputs and returns `selectSlashPenalty`. The existing rational comparison
  theorem gives its mathematical interpretation, including the exact 30%
  boundary before integer division.
- `retained_iff`: checked retained-balance calculation succeeds exactly for
  bounded inputs and returns the specified floor division. Both directions
  include width, penalty-range, and intermediate-product conditions.
- `validateTransition_iff`: executable admission is equivalent to the shared
  complete precondition.
- `transition_iff`: a successful executable result satisfies the complete
  relational transition, and every relational successor is reproduced.
  `referencePenaltyTransition_iff` converts fieldwise equality into full state
  equality, including the complete account function.
- `transition_noInactivity`: on admitted rounds without inactivity and with
  unchanged identity sets, the result is exactly the #283 executable result.
  `inactivity_checked_refines` connects the new stage to #282's checked arithmetic; u64 input bounds imply that its
  product fits u128.
- `ready_permutation`, `successor_permutation`, and `transition_permutation`:
  permuting either event category preserves admission and the entire successful
  successor. Correlation uses the existing finite-set sums of previous weights.
  `diagnose_permutation` and `transition_permutation_eq` also prove identical
  failure categories, giving equality of the complete `Except` result.
- `failure_atomic` and `failure_no_successor`: an error contains no successor,
  admits no relational successor, and `commit` returns the complete previous
  state unchanged.
- `successor_newValidator_preserved`: newly registered candidate identities keep
  their candidate accounts, independently of unknown previous-account values.
- `transition_preserves_previousIds`: every successful transition retains all
  previously registered identities, including excluded keys.
- `successor_excluded_persistent`: prior **registered** exclusions and retained
  balances take priority over the candidate and over inactivity.
- `successor_exclusion_iff` and `transition_exclusion_iff`: an exported state's
  registered key is excluded exactly when it was previously registered and
  excluded, or has an authorized equivocation event. `newExclusion_requires_event`
  proves that every new exclusion has such an event.
- `successor_wellFormed` and `transition_ok_wellFormed`: admitted transitions
  preserve canonical identities and consistent active/retained/excluded fields,
  including mixed inactivity and slashing rounds.
- `exportedWeights_equiv`: the finite output view is exactly the existing
  formal active-weight projection; unknown and excluded identities export zero.

## Error mapping

The arithmetic aliases retain `SlashingArithmeticError.invalidConfiguration`
and `.overflow` from #282; these map to the corresponding configuration and
checked-arithmetic Rust errors. Transition and evidence admission use the
finer `SlashingRef.Error` categories below.

The diagnostic precedence is fixed in `SlashingRef.diagnose`. It checks whole
categories with finite predicates, so the selected category does not depend on
which offender appears first. Acceptance is decided independently from this
diagnostic function by the shared contract.

| Lean error | Rust conformance category |
| --- | --- |
| `invalidConfiguration` | `InvalidConfiguration` (including zero slash total) |
| `invalidState` | malformed snapshot or a candidate inventing a new exclusion |
| `invalidValidatorSet` | candidate drops a previously registered identity |
| `invalidEventRound` | `InvalidPenaltyEvents`, successor/application round mismatch |
| `emptyEvidence`, `oversizedEvidence`, `tooManyEvents` | `InvalidPenaltyEvents`, invalid evidence/event size |
| `duplicateOrOverlappingOffender` | `InvalidPenaltyEvents`, repeated offender across either category |
| `unknownOffender`, `alreadyEjectedOffender` | `InvalidPenaltyEvents`, prior-entry admission failure |
| `unsupportedInactivity`, `inactiveRatingParticipant` | `InvalidPenaltyEvents`, missed-round/rating rule |
| `arithmeticOverflow` | `SlashOverflow`/`InactivityDecayOverflow`, or an input outside Rust's widths |
| `structurallyInvalidEvidence` | invalid equivocation proof structure |
| `unauthenticatedEvidence` | failed content-identity/signature verification |
| `unauthorizedEvidence` | failed protocol authorization |

## Rust correspondence and model boundary

| Lean definition/theorem | Rust operation |
| --- | --- |
| `checkEvidence_iff` | structural equivocation checks in `cordial-por::evidence` and Cordial same-depth equivocation |
| `authorizeEvidence_success_iff` | evidence verification and `EquivocationPenalty::authorize` trust boundary |
| `validateConfig_iff` | penalty-related fields of `PorConfig::validate` |
| `selectTier_iff` | `penalties::compute_slash_penalty_wide` |
| `retained_iff`, `inactivity_checked_refines` | `apply_slash_to_reputation`, `compute_inactivity_decay` |
| `BoundedPenaltyEvidence` | evidence/event limits in `commitments::penalty_events_commitment` |
| `validateTransition_iff`, `transition_iff` | validated staged application in `transition::apply_reputation_penalties` |
| `transition_noInactivity` | agreement with the #283 penalty model on unchanged identity domains |
| `successor_newValidator_preserved` | union of identities in `blend_reputation_transition`, including newly rated keys |
| `transition_exclusion_iff`, `newExclusion_requires_event` | `ReputationEntry::new` active flags, restored prior exclusions, and event-driven ejection in audited replay |
| permutation theorems | previous-round per-offense aggregation and canonical event handling |
| atomicity theorems | staged clone/atomic publication in the audited transition |
| `successor_excluded_persistent` | permanent ejection in audited replay/state application |
| `exportedWeights_equiv` | per-identity projection in `weights::authorized_validator_weights` |

The model uses natural-number node identities and supplied valid blocklaces.
It does not model signature algorithms, byte-identity codecs, total encoded
payload size, the rating/blend/clamp pipeline, or RSpace execution. Candidates
represent the output of the preceding reputation pipeline; domain inclusion
and exclusion provenance are explicitly checked at the reference boundary.
The reference admits candidate-set growth and changes only penalty fields
within candidate accounts. The weight projection does not implement Rust's
additional wrapper checks for missing authorized identities or zero total
authorized weight. These boundaries are unchanged from the preceding models.
Rust fixture generation is outside this issue.

## Validation

Run `cd lean && lake build` and `just lean-check-sorry`.
The refinement-theorem axiom audit (`#print axioms`) reports only Lean's
standard `propext`, `Classical.choice`, and `Quot.sound`; it reports no
`sorryAx` or project-specific axioms. Choice appears in proof dependencies,
not as a noncomputable executable decision. The finite reference functions
compile and the examples below execute with `native_decide`.
`SlashingRefExamples.lean` evaluates valid and forged evidence, denied crypto
and authorization, isolated/correlated slashes, mixed inactivity, reversed
events, duplicate/overlapping/unknown/ejected offenders, rating overlap,
evidence-size boundaries, exact threshold/rounding cases, arithmetic overflow,
full-state failure observations, zero-weight rounds, malformed states, and
permanent exclusion/weight export. PR #299 regressions also cover the candidate
balances from Rust's `seeds_a_new_node_from_initial_reputation` test, interleaved
identity growth, mixed penalties during growth, correlation unaffected by new
weight, rejection of penalties against new keys, dropped old identities,
unknown prior-account values, and atomic rejection of forged old/new exclusions.
