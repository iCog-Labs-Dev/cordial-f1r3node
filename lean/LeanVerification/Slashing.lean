/-
Cordial tiered-slashing evidence and authorization model.

This module deliberately separates four concepts that an implementation must
not conflate:

1. `RawEquivocationEvidence` is an untrusted claim plus opaque proof material.
2. `ValidEquivocationEvidence` proves that the named blocks form the existing
   Cordial `Equivocation` and binds the claim to its creator and round.
3. `EvidenceAuthenticated` crosses an explicitly named cryptographic trust
   boundary for content-identity and signature verification.
4. `AuthorizedPenaltyEvent` records a separate protocol decision authorizing
   application in a PoR round.

The cryptographic algorithm is intentionally outside the Lean model. Its sole
trust assumption is `CryptographicVerificationAssumption.verifies`; in
particular, authentication is never inferred from `proofMaterial` being
nonempty (or from any other property of those bytes).

Cordial depths and PoR application rounds are distinct coordinate systems.
The model binds each to its source, but deliberately assumes no numeric ordering
between them.
-/

import LeanVerification.Equivocation

namespace CordialMiners

/-! ### Raw and structurally valid evidence -/

/--
An untrusted slash-evidence claim.

`proofMaterial` stands for the signatures, serialized blocks, and host mapping
data supplied by an implementation. Merely constructing this value establishes
nothing about those bytes or about the claimed offender.
-/
structure RawEquivocationEvidence where
  claimedOffender : NodeId
  firstBlock : BlockId
  secondBlock : BlockId
  offenseRound : Nat
  proofMaterial : List UInt8
  deriving DecidableEq

/--
Structural validity of raw evidence in a particular valid blocklace.

The individual conditions remain explicit at this boundary even where the
existing `Equivocation` predicate also entails them. This makes every check an
implementation must perform visible in the formal contract while reusing,
rather than redefining, Cordial equivocation.
-/
structure ValidEquivocationEvidence
    (B : Blocklace) (hV : ValidBlocklace B)
    (evidence : RawEquivocationEvidence) : Prop where
  firstPresent : evidence.firstBlock ∈ B.keys
  secondPresent : evidence.secondBlock ∈ B.keys
  distinct : evidence.firstBlock ≠ evidence.secondBlock
  sameCreator :
    creatorOf B evidence.firstBlock = creatorOf B evidence.secondBlock
  firstCreatorIsOffender :
    creatorOf B evidence.firstBlock = some evidence.claimedOffender
  secondCreatorIsOffender :
    creatorOf B evidence.secondBlock = some evidence.claimedOffender
  firstDoesNotObserveSecond :
    ¬ Observes B evidence.firstBlock evidence.secondBlock
  secondDoesNotObserveFirst :
    ¬ Observes B evidence.secondBlock evidence.firstBlock
  sameDepth :
    blockDepth B hV evidence.firstBlock = blockDepth B hV evidence.secondBlock
  offenseRoundIsFirstDepth :
    evidence.offenseRound = blockDepth B hV evidence.firstBlock
  equivocation : Equivocation B hV evidence.firstBlock evidence.secondBlock

/-- Structurally valid evidence contains an existing Cordial equivocation. -/
theorem validEvidence_implies_equivocation
    (B : Blocklace) (hV : ValidBlocklace B)
    (evidence : RawEquivocationEvidence)
    (hValid : ValidEquivocationEvidence B hV evidence) :
    Equivocation B hV evidence.firstBlock evidence.secondBlock :=
  hValid.equivocation

/-- Structurally valid evidence identifies its claimed offender as an equivocator. -/
theorem validEvidence_implies_equivocator
    (B : Blocklace) (hV : ValidBlocklace B)
    (evidence : RawEquivocationEvidence)
    (hValid : ValidEquivocationEvidence B hV evidence) :
    Equivocator B hV evidence.claimedOffender :=
  ⟨evidence.firstBlock, evidence.secondBlock, hValid.equivocation,
    hValid.firstCreatorIsOffender⟩

/--
Two valid claims naming the same first conflicting block cannot identify
different offenders.
-/
theorem validEvidence_offender_unique
    (B : Blocklace) (hV : ValidBlocklace B)
    (first second : RawEquivocationEvidence)
    (hFirst : ValidEquivocationEvidence B hV first)
    (hSecond : ValidEquivocationEvidence B hV second)
    (hSameBlock : first.firstBlock = second.firstBlock) :
    first.claimedOffender = second.claimedOffender := by
  have hSecondCreator :
      creatorOf B first.firstBlock = some second.claimedOffender := by
    simpa [hSameBlock] using hSecond.firstCreatorIsOffender
  exact Option.some.inj (hFirst.firstCreatorIsOffender.symm.trans hSecondCreator)

/-- Both conflicting blocks are bound to the raw evidence's offense round. -/
theorem validEvidence_round_bound
    (B : Blocklace) (hV : ValidBlocklace B)
    (evidence : RawEquivocationEvidence)
    (hValid : ValidEquivocationEvidence B hV evidence) :
    blockDepth B hV evidence.firstBlock = evidence.offenseRound ∧
      blockDepth B hV evidence.secondBlock = evidence.offenseRound := by
  constructor
  · exact hValid.offenseRoundIsFirstDepth.symm
  · exact hValid.sameDepth.symm.trans hValid.offenseRoundIsFirstDepth.symm

/-! ### Authentication boundary -/

/--
The named trust assumption for cryptographic evidence verification.

An implementation may assert `verifies B evidence` only after checking that the
supplied block contents map to the claimed identities and that both creators'
signatures verify. Lean treats the result of that external cryptographic
algorithm as an assumption; it does not model the algorithm and does not derive
the result from the opaque `proofMaterial` bytes. The concrete Rust counterpart
is the sealed `VerifiedEquivocationEvidence` value returned only by
`verify_equivocation_evidence`.
-/
structure CryptographicVerificationAssumption where
  verifies : Blocklace → RawEquivocationEvidence → Prop

/-- Successful content-identity and signature verification at the trust boundary. -/
def EvidenceAuthenticated
    (crypto : CryptographicVerificationAssumption)
    (B : Blocklace) (evidence : RawEquivocationEvidence) : Prop :=
  crypto.verifies B evidence

/-! ### Protocol authorization boundary -/

/-- The kind of consensus decision that can authorize a penalty application. -/
inductive PenaltyAuthorizationDecision where
  /-- A finalized Cordial block authorizes the penalty. -/
  | finalized (anchor : BlockId)
  /-- An abstract finalized PoR attestation authorizes the penalty. -/
  | attested (attestationId : Nat)
  deriving DecidableEq

/--
An abstract protocol policy relating an authorization decision, verified
evidence, and the PoR round in which the penalty may be applied.
-/
abbrev PenaltyAuthorizationPolicy :=
  PenaltyAuthorizationDecision → RawEquivocationEvidence → Nat → Prop

/-- A protocol decision authorizes applying this evidence in this PoR round. -/
def PenaltyApplicationAuthorized
    (policy : PenaltyAuthorizationPolicy)
    (decision : PenaltyAuthorizationDecision)
    (evidence : RawEquivocationEvidence)
    (applicationRound : Nat) : Prop :=
  policy decision evidence applicationRound

/--
A penalty event that crossed all three boundaries: structural validation,
cryptographic authentication, and protocol authorization.

The offense round records the Cordial depth where the equivocation occurred.
`applicationRound` is deliberately separate: it is a PoR round bound by the
authorization policy, not a value ordered against the Cordial depth.
-/
structure AuthorizedPenaltyEvent
    (B : Blocklace) (hV : ValidBlocklace B)
    (crypto : CryptographicVerificationAssumption)
    (policy : PenaltyAuthorizationPolicy) where
  evidence : RawEquivocationEvidence
  validEvidence : ValidEquivocationEvidence B hV evidence
  authenticatedEvidence : EvidenceAuthenticated crypto B evidence
  offender : NodeId
  offenderBound : offender = evidence.claimedOffender
  offenseRound : Nat
  offenseRoundBound : offenseRound = evidence.offenseRound
  applicationRound : Nat
  authorizationDecision : PenaltyAuthorizationDecision
  authorized :
    PenaltyApplicationAuthorized policy authorizationDecision evidence applicationRound

/-- Every authorized event still exposes the structurally valid evidence it contains. -/
theorem authorizedEvent_contains_validEvidence
    (B : Blocklace) (hV : ValidBlocklace B)
    (crypto : CryptographicVerificationAssumption)
    (policy : PenaltyAuthorizationPolicy)
    (event : AuthorizedPenaltyEvent B hV crypto policy) :
    ValidEquivocationEvidence B hV event.evidence :=
  event.validEvidence

/-- The event offender is the claimed and structurally established block creator. -/
theorem authorizedEvent_offender_bound
    (B : Blocklace) (hV : ValidBlocklace B)
    (crypto : CryptographicVerificationAssumption)
    (policy : PenaltyAuthorizationPolicy)
    (event : AuthorizedPenaltyEvent B hV crypto policy) :
    event.offender = event.evidence.claimedOffender ∧
      creatorOf B event.evidence.firstBlock = some event.offender ∧
      creatorOf B event.evidence.secondBlock = some event.offender := by
  refine ⟨event.offenderBound, ?_, ?_⟩
  · simpa [event.offenderBound] using event.validEvidence.firstCreatorIsOffender
  · simpa [event.offenderBound] using event.validEvidence.secondCreatorIsOffender

/-- The authorization decision binds this evidence to this exact PoR application round. -/
theorem authorizedEvent_applicationRound_bound
    (B : Blocklace) (hV : ValidBlocklace B)
    (crypto : CryptographicVerificationAssumption)
    (policy : PenaltyAuthorizationPolicy)
    (event : AuthorizedPenaltyEvent B hV crypto policy) :
    PenaltyApplicationAuthorized policy event.authorizationDecision
      event.evidence event.applicationRound :=
  event.authorized

end CordialMiners
