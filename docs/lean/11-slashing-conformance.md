# Rust-to-Lean slashing conformance (#285)

Run the complete suite from the repository root:

```bash
just slashing-conformance
# Equivalent, without just:
bash scripts/slashing_conformance.sh
```

The command validates the checked-in Rust fixtures byte for byte, builds the
Lean library and `slashing_replay`, replays every fixture, and runs mutations
against the compiled Lean executable. Any schema or observed-result mismatch
returns a nonzero exit status. The Lean CI workflow runs this command for PoR,
Lean, and harness changes. Rust uses the repository's pinned
`nightly-2026-02-09`; Lean uses `lean/lean-toolchain`. The normal workspace path
dependencies and cached/downloadable Cargo and Mathlib dependencies are needed.

## Boundary and trust

`lean/slashing/fixtures.v1.json` is a separate, versioned structural slashing
suite, independent of the existing consensus-event traces. Rust computes the
whole production rating/blend/clamp pipeline twice: once without penalties to
capture the candidate, once with penalties to obtain the expected result. Lean
uses that explicit candidate as the input to `SlashingRef.transition`; it does
not independently verify Liquid Rank, blending, clamping, or rating signatures.

Current Rust APIs require an authenticated, authorized equivocation capability.
The exporter builds deterministic signed, same-depth forks and admits them
through `verify_equivocation_evidence` and `EquivocationPenalty::authorize` using
an explicit test authorization policy. These are fixture preparation steps,
not cryptographic conformance claims. The fixtures project offender identity,
offense/application rounds, and opaque evidence **length**. Empty and oversized
evidence cases mutate an existing capability and must fail Rust replay.

Lean instantiates synthetic valid depth-zero forks with the projected offender
and byte length. Its test-only crypto verifier and authorization policy explicitly
return `True`; the structural fork validity proof uses existing proved lemmas.
This supplies the typed authorization witness required by `SlashingRef` without
adding axioms or inferring authentication from byte length. The fork payloads,
hashes, signatures, and byte contents are not translated between languages.

**This suite does not test semantic evidence authentication**, invalid
signatures, offender/signature binding, live evidence admission, finalization,
or host slash-deploy execution. Inactivity authorization remains a trusted host
input. Version 1 restricts equivocation fixtures to one depth-zero correlation
group; later schema versions can extend that boundary.

Cryptographic commitments are outside the Lean model and are deliberately not
serialized as purported Lean expectations. The Rust exporter still checks
production block construction, audit, wire roundtrip, event-order-independent
blocks (including their commitments), and rejection of a forged successor even
when its reputation root is recomputed. No claim of Lean hash conformance is
made.

## Version 1 schema

The root object has exactly these fields:

| Field | Meaning |
| --- | --- |
| `schema_version` | Integer `1`; unsupported versions fail |
| `boundary` | Literal `cordial-por/slashing-structural-v1` |
| `fixtures` | Nonempty array, strictly sorted by unique `name` |

Each fixture has exactly `name`, `config`, `previous`, `candidate`, `ratings`,
`events`, and `expected`. Every object rejects missing and unknown fields.
All numeric values are exact nonnegative JSON integers bounded by `u64`, never
floats or decimal strings. Lean's arithmetic uses checked `u128` bounds from
`SlashingRef`. Booleans are JSON booleans, not integers.

| Object | Fields |
| --- | --- |
| `config` | `scale`, `initial`, `alpha`, `minimum_rating`, `maximum_rating`, `missing_entry_policy`, `threshold`, `base_penalty`, `gamma` |
| `previous`, `candidate`, `expected.state` | `round`, `entries` |
| State entry | `id`, `active`, `retained`, `excluded` |
| Rating | `round`, `rater`, `recipient`, `score`, `signature` (byte array) |
| `events` | `round`, `equivocations`, `inactivity` |
| Equivocation | `id`, `evidence_length`, `offense_round`, `application_round` |
| Inactivity | `id`, `missed_rounds` |
| `expected` | `status`, `state`, `weights`, `tiers` |
| Weight | `id`, `weight` |
| Tier | `id`, `penalty` (fixed-point numerator over `config.scale`) |

`id` is a small natural-number alias, assigned in lexicographic public-key order
by the deterministic Rust key helper. Identities, state entries, weights, and
tiers are strictly sorted. Events preserve their supplied order so permutation
fixtures exercise reversed input. Serialization sorts JSON object keys and
never iterates over a Rust `HashMap` to emit entries or weights.
`missing_entry_policy` is `carry_forward`; other policies fail parsing in v1.
`offense_round` is zero. Evidence lengths up to 65,537 are parseable so the
65,536-byte maximum has an explicit rejection fixture without unbounded parser
allocation.

`expected.state` is the successor on success and the complete previous modeled
snapshot on failure. `expected.weights` projects every successor identity onto
Cordial's authorized validator set, including excluded keys with zero weight.
`expected.tiers` is empty on failure or when there are no equivocations; on
success it records each offender's selected penalty ratio. Unknown or excluded
offenders never produce a tier.

Invalid-configuration fixtures use a candidate produced with the default valid
fixture configuration. The configuration gate rejects before that candidate
can affect a result. Rating inputs in v1 are otherwise valid; participants are
used to check inactivity conflicts. This is not a rating-parser conformance
suite.

### Stable error categories

| Rust result | Lean category |
| --- | --- |
| Success | `ok` |
| `InvalidConfiguration` | `invalid_configuration` |
| `InvalidPenaltyEvents` | `invalid_penalty_events` |
| `SlashOverflow`, `InactivityDecayOverflow` | `arithmetic_overflow` |

Lean's more precise wrong-round, evidence-size, duplicate/overlap, unknown,
already-ejected, and inactivity errors collapse to Rust's actual public
`InvalidPenaltyEvents` category. It does not compare unstable diagnostic text.
Invalid state/set results remain `invalid_state`, preventing accidental mapping
to a tested event error. The Rust exporter panics on unmapped error variants.

## Production correspondence and observations

| Exporter/checker | Boundary exercised |
| --- | --- |
| `replay_reputation_transition` | Independent pre-penalty candidate |
| `replay_reputation_transition_with_penalties` | Acceptance and expected successor |
| `compute_slash_penalty` | Expected tier from production arithmetic |
| `build_reputation_block_with_penalties` | Production construction/commitments |
| `verify_reputation_transition_with_penalties` | Independent block audit |
| `ReputationState::apply_reputation_block_with_penalties` | Audited atomic application |
| `authorized_validator_weights` | Actual production weight projection |
| `SlashingReplay.replayFixture` | Execute `SlashingRef.transition`, checked tier selection, and weight export |

For every rejection, the exporter attempts both audit and real state application
using a structurally valid proposal and verifies the same stable category as
replay. Equality of the **entire cloned `ReputationState`** before and after
failure checks round, reputation list, retained balances, permanent exclusion
registry, pending ratings, and latest block. Failure fixtures include a state
with an existing audited ejection, retained balance, latest block, and pending
rating. Lean compares the finite previous snapshot, including identities and
all account fields, as its pure failure result. Rust-only state fields are
covered by Rust equality, not claimed as modeled Lean fields.

Successful cases also attempt a forged block and check complete-state atomicity
before applying the valid block. An existing retained balance is established by
an actual audited prior-round slash, rather than hand-forging state flags.

The 34 cases cover isolated/exact/above-threshold slashing, correlated and
unequal weights, non-divisible rounding, innocents, mixed penalties, inactivity
and both event permutations, duplicate/overlapping/unknown/ejected offenders,
wrong rounds, empty/oversized evidence, invalid configuration fields, checked
arithmetic overflow, rating rewards ignored for slash balances, new rated
validators, inactive rating participants, and preservation of retained ejections.
Mutation tests change every expected observable and rejected-state atomicity,
and reject malformed schema, unsupported versions, unknown/missing fields,
wrong numeric/boolean types, bounds violations, duplicate/unsorted identities,
and empty suites.

## Regenerating fixtures

```bash
POR_REGENERATE_SLASHING=1 cargo +nightly-2026-02-09 test --locked \
  -p cordial-por --test slashing_fixtures
just slashing-conformance
git diff -- lean/slashing/fixtures.v1.json
```

Normal tests and CI only validate; they never rewrite fixtures. The harness
unsets the regeneration variable so a shell environment cannot silently bless
changed expected outputs. Review fixture diffs as behavior changes; do not
regenerate merely to hide a Rust/Lean disagreement.
