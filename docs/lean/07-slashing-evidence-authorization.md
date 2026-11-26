# Cordial Slashing Evidence and Authorization

Lean module: `LeanVerification/Slashing.lean`

Issue: [#281 — Define the Cordial slashing evidence and authorization model](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues/281)

## Why the boundaries are separate

A penalty calculation can be numerically correct and still slash the wrong
validator. The formal contract therefore does not let a raw byte vector stand
in for proof. It models four separate stages:

1. raw evidence claims an offender, two blocks, and an offense round;
2. structural validation proves that claim describes a Cordial equivocation;
3. authentication records successful external content-identity and signature
   verification;
4. protocol authorization permits applying the authenticated evidence in a
   particular PoR round.

No theorem in the module derives a later stage from an earlier one.

## Raw and valid evidence

`RawEquivocationEvidence` is deliberately untrusted:

```lean
structure RawEquivocationEvidence where
  claimedOffender : NodeId
  firstBlock : BlockId
  secondBlock : BlockId
  offenseRound : Nat
  proofMaterial : List UInt8
```

The opaque `proofMaterial` can represent serialized blocks, signatures, or
host mapping data. Its presence, length, and contents establish no proposition
in Lean.

`ValidEquivocationEvidence B hV evidence` proves all of the structural facts
needed at the slashing boundary:

- both block identities are present and distinct;
- both blocks have the same creator;
- that creator is the claimed offender;
- neither block observes the other;
- both blocks have the same computed Cordial depth;
- the recorded offense round is that depth;
- the pair satisfies the existing `Equivocation B hV` predicate.

The last field is important: the slashing model reuses the protocol's existing
equivocation definition instead of introducing a competing one.

## Authentication trust assumption

```lean
structure CryptographicVerificationAssumption where
  verifies : Blocklace → RawEquivocationEvidence → Prop

def EvidenceAuthenticated
    (crypto : CryptographicVerificationAssumption)
    (B : Blocklace) (evidence : RawEquivocationEvidence) : Prop :=
  crypto.verifies B evidence
```

`CryptographicVerificationAssumption.verifies` is the module's named trust
assumption. An implementation may cross this boundary only after verifying the
mapping from supplied block contents to their identities and checking both
creators' signatures. The Lean model does not attempt to prove the
cryptographic algorithm itself.

In particular, `EvidenceAuthenticated` does not inspect `proofMaterial` and is
not equivalent to a nonempty-byte check.

## Authorization and round binding

`PenaltyAuthorizationDecision` distinguishes a finalized Cordial anchor from
an abstract finalized PoR attestation. `PenaltyAuthorizationPolicy` is the
protocol-provided relation that states which decision authorizes which evidence
and application round.

`AuthorizedPenaltyEvent` contains:

- structurally valid evidence;
- a separate authentication proof;
- the bound offender and original offense round;
- a distinct PoR penalty application round;
- the finalization or attestation decision;
- a proof that the configured protocol policy authorizes the evidence in that
  exact application round.

Thus detection, authentication, and authorization remain independent facts.
The model does not compare the two round numbers: Cordial depth and PoR round
are separate coordinate systems, so a numeric ordering between them would be
an undocumented assumption.

## Key theorems

```lean
theorem validEvidence_implies_equivocation ... :
  Equivocation B hV evidence.firstBlock evidence.secondBlock
```

Structural validity exposes the exact existing Cordial equivocation carried by
the evidence.

```lean
theorem validEvidence_implies_equivocator ... :
  Equivocator B hV evidence.claimedOffender
```

The validated creator binding turns the block pair into an `Equivocator`
witness for the claimed offender.

```lean
theorem validEvidence_offender_unique ...
    (hSameBlock : first.firstBlock = second.firstBlock) :
  first.claimedOffender = second.claimedOffender
```

Two valid claims sharing a conflicting block cannot attribute that block to
different offenders, because `creatorOf` returns one creator.

```lean
theorem validEvidence_round_bound ... :
  blockDepth B hV evidence.firstBlock = evidence.offenseRound ∧
  blockDepth B hV evidence.secondBlock = evidence.offenseRound
```

The explicit offense round is the computed depth of both conflicting blocks.

```lean
theorem authorizedEvent_contains_validEvidence ... :
  ValidEquivocationEvidence B hV event.evidence
```

Authorization cannot erase or replace the structural proof.

```lean
theorem authorizedEvent_offender_bound ... :
  event.offender = event.evidence.claimedOffender ∧
  creatorOf B event.evidence.firstBlock = some event.offender ∧
  creatorOf B event.evidence.secondBlock = some event.offender
```

The event-level offender is tied to both conflicting block creators.

```lean
theorem authorizedEvent_applicationRound_bound ... :
  PenaltyApplicationAuthorized policy event.authorizationDecision
    event.evidence event.applicationRound
```

The authorization proof names the exact PoR application round. It does not
compare that round with the Cordial evidence depth.

## Lean-to-Rust contract mapping after the `dev` integration

The updated Rust tree now has explicit verification and authorization layers.
The table maps those concrete layers to the formal contract without claiming
full conformance where the models intentionally differ.

| Lean definition or theorem | Rust counterpart / obligation |
|---|---|
| `RawEquivocationEvidence` | `CordialEquivocationEvidence`, the untrusted evidence-pool record |
| `ValidEquivocationEvidence` | structural checks in `verify_equivocation_evidence`, plus the equal-depth condition described below |
| `validEvidence_implies_equivocation` | `equivocation_blocks_at_round` / `all_equivocations` structural result |
| `validEvidence_implies_equivocator` | verifier creator checks and evidence-pool offender indexing |
| `validEvidence_round_bound` | `blockDepth` versus the claimed evidence round; Rust currently uses the minimum depth as described below |
| `EvidenceAuthenticated` | sealed `VerifiedEquivocationEvidence`, constructible only through `verify_equivocation_evidence` |
| `CryptographicVerificationAssumption` | Rust content-hash, Secp256k1 signature, creator, complete-ancestry, identity, and incomparability checks |
| `PenaltyAuthorizationPolicy` | `EquivocationAuthorizationPolicy` and `CheckpointEquivocationAuthorizationPolicy` |
| `PenaltyAuthorizationDecision` | trusted finalized Cordial output or finalized checkpoint-prefix authorization |
| `AuthorizedPenaltyEvent` | `EquivocationPenalty` carrying its private `EquivocationAuthorization` capability |
| `authorizedEvent_offender_bound` | `EquivocationPenalty::authorize` derives the offender from `VerifiedEquivocationEvidence` |
| `authorizedEvent_applicationRound_bound` | the private capability binds `EquivocationAuthorizationContext.round` or the checkpoint source round |

### Remaining conformance boundary

The Lean issue defines a Cordial equivocation as two same-creator,
incomparable blocks at the same depth. Rust's new verifier authenticates two or
more same-creator, pairwise-incomparable blocks, but it intentionally accepts
blocks at different depths and records their minimum depth as the evidence
round. Such a Rust value satisfies the authentication boundary but does not, by
itself, construct `ValidEquivocationEvidence` in this formal model.

This distinction is now explicit rather than hidden behind the mapping table.
Closing it requires a separate protocol decision: either restrict the Rust
verifier to equal-depth pairs or deliberately broaden the formal definition.
Issue #281 keeps the existing Lean `Equivocation` predicate and therefore takes
the first interpretation as its contract.

The merged implementation does satisfy the separation principle: decoding
canonical bytes returns an unverified claim, only the verifier creates the
sealed authenticated value, and authorization requires a separate trusted
policy over finalized protocol output. Arbitrary nonempty evidence bytes cannot
create the private penalty capability.
