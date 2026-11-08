# PoR-Aware Leader Selection Comparison

## Status

PoR-aware leader selection is implemented as an optional comparison policy.
The deterministic weighted-fair scheduler is functional: it derives a
threshold-based leader-candidate set from activated PoR weights, constructs a
bounded schedule, resolves a leader for each wave, and can be compared with
canonical `NodeId` round robin through shadow status.

This feature is complete for its current observational scope. It deliberately
does not replace Cordial's authoritative leader selector, so finality and tau
ordering continue to use canonical round robin. Making PoR-aware selection an
authoritative consensus mode would be a separate protocol change requiring
network-wide configuration and activation at a committed future wave.

## Objective

Compare leader-selection policies without transferring validator-membership
authority from Cordial to Proof of Reputation:

1. canonical round robin;
2. deterministic PoR weighted-fair rotation over all positive weights;
3. deterministic PoR threshold-filtered weighted-fair rotation.

Both policies consume Cordial's authorized validator map. PoR may change the
weight and leadership frequency of an authorized validator, but it cannot add
an identity to that map.

## Relationship To The PoR Paper

Sections 4.1.1 and 4.1.2 of the Proof-of-Reputation paper first construct a
high-reputation consensus group whose collective reputation exceeds 50% of
network reputation. The paper then randomly selects a leader uniformly from
that group.

The Cordial policy is deliberately different:

- Cordial retains its complete authorized validator set.
- PoR reputation is already used as Cordial finality weight.
- The optional leader policy treats that group as a leader-candidate subset,
  not as a replacement for Cordial membership.
- The comparison policy uses reputation to allocate leadership frequency
  within the candidate subset.
- Selection is deterministic so all honest observers derive the same leader.

The threshold-filtered weighted-fair policy is therefore paper-guided
integration, not a paper-native implementation of the PoR consensus protocol.

Paper reference: <https://arxiv.org/abs/2108.03542>

## Policy A: Canonical Round Robin

Given the authorized validator set `A`, sort validators by ascending `NodeId`:

```text
V = sort(A)
leader(wave) = V[wave mod |V|]
```

This reproduces the existing Cordial behavior, including the presence of
zero-weight authorized keys in the rotation.

### Properties

- deterministic;
- independent of reputation distribution;
- bounded leadership opportunity for every authorized validator;
- simple historical replay;
- no additional PoR coupling.

### Limitation

A validator with very low or zero active reputation receives the same leader
frequency as a highly reputable validator. Weighted finality may prevent a
zero-weight leader from finalizing, but the wave can still lose liveness.

## Policy B: PoR Weighted-Fair Rotation

Start with Cordial's authorized validators and retain only positive-weight
entries:

```text
E = { validator in A | activated_weight(validator) > 0 }
```

The implementation constructs a bounded schedule of `S` slots:

1. Sort `E` by ascending `NodeId`.
2. Give every validator one baseline slot.
3. Allocate the remaining `S - |E|` slots proportionally to activated weight.
4. Use largest-remainder apportionment with `NodeId` as the final tie-breaker.
5. Arrange the quotas by earliest virtual finish using a deterministic heap.

For an epoch beginning at wave `e`:

```text
leader(wave) = schedule[(wave - e) mod S]
```

Waves before `e` must be resolved using the preceding schedule epoch.

### Example

For a ten-slot schedule:

```text
A = 60 reputation
B = 30 reputation
C = 10 reputation
```

The baseline-plus-proportional allocation produces:

```text
A = 5 slots
B = 3 slots
C = 2 slots
```

The exact slot order is deterministic and spreads each validator's quota
through the cycle.

Schedule construction costs `O(S log K)`, where `S` is the bounded schedule
length and `K` is the number of eligible validators. The heap stores `O(K)`
entries and the materialized schedule stores `O(S)` leaders. Construction
requires `K <= S <= 65,536`, which bounds both computation and memory.

### Properties

- deterministic from an agreed activated weight snapshot;
- bounded schedule size;
- every positive-weight validator receives a leadership opportunity;
- higher reputation generally yields more leadership opportunities;
- ejected and zero-weight validators receive no PoR leadership slots;
- no random seed or locally observed timing affects selection.

## Policy C: Threshold-Filtered PoR Weighted-Fair Rotation

This is the policy used by shadow comparison. Start with Cordial's authorized
validators and rank positive-weight entries by:

1. activated reputation descending;
2. `NodeId` ascending for an exact reputation tie.

For threshold `n/d`, select the smallest ranked prefix `G` satisfying:

```text
sum(reputation in G) > (n / d) * total_active_reputation
```

The comparison defaults to `1/2`, matching the paper's strict
greater-than-50% rule. Integer arithmetic computes the minimum accepted weight
without floating point:

```text
required = floor(total_active_reputation * n / d) + 1
```

The bounded weighted-fair schedule is then constructed over `G` only. Cordial's
authorized set is unchanged: validators outside `G` remain consensus members
but do not appear in this optional leader schedule.

### Example

```text
A = 50 reputation
B = 30 reputation
C = 20 reputation
threshold = 1/2
```

`A` alone reaches exactly 50%, which is not strictly greater than the
threshold. The selected candidate set is therefore `[A, B]` with collective
reputation 80%.

### Properties

- deterministic candidate selection;
- strict threshold semantics aligned with the paper's group boundary;
- canonical tie-breaking independent of map insertion order;
- zero-weight validators are never candidates;
- Cordial remains the authority for validator membership;
- deterministic weighted scheduling replaces the paper's random leader draw.

## Correctness Requirements

For every wave `w`, all honest observers must resolve the same tuple:

```text
(policy version,
 effective wave,
 reputation checkpoint hash,
 authorized weight commitment,
 schedule parameters)
```

Leader selection must then satisfy:

```text
leader_i(w) = leader_j(w)
```

for every pair of honest observers `i` and `j`.

Eventual synchrony only bounds message delay after the global stabilization
time. It does not repair disagreement about the schedule or reputation epoch.
Schedule activation must therefore be derived from finalized protocol data,
not local arrival time.

## Epoch Activation

A PoR leader schedule must be activated with the same future-only boundary as
its authorized weight projection:

```text
waves < effective_wave  -> old weights and old leader schedule
waves >= effective_wave -> new weights and new leader schedule
```

Required activation data includes:

- policy and algorithm version;
- effective wave;
- reputation checkpoint hash;
- authorized weight commitment;
- schedule length;
- schedule commitment or sufficient canonical inputs to reconstruct it.

Historical schedules must remain available for finality replay, tau ordering,
restart recovery, and audit. Missing history must fail closed instead of using
the latest schedule for an older wave.

## Equivocation And Unavailable Leaders

Leader failure does not change the selected leader inside the current wave.

- No runner-up is selected after observing equivocation.
- Multiple blocks from the selected leader prevent a unique leader block.
- An unavailable selected leader may prevent that wave from finalizing.
- Later waves continue according to the committed schedule.

Locally replacing a failed leader would make selection depend on message arrival
and would violate deterministic replay.

## Known Tradeoffs

### Weight Quantization

A bounded schedule cannot represent every fixed-point reputation ratio exactly.
Largest-remainder allocation minimizes the discrepancy but does not remove it.

### Baseline-Slot Distortion

Giving every positive-weight validator one slot ensures recurring leadership
opportunities within the selected policy set, but slightly overrepresents
validators with very small weights.

### Reputation Concentration

A high-reputation validator can receive many slots. Smooth rotation spreads
those slots but does not remove concentration.

### Schedule Length

A short schedule increases quantization error. A long schedule improves
proportional accuracy but increases storage, replay, and configuration costs.
The comparison implementation bounds a materialized schedule at `65,536`
slots. Any future authoritative protocol must commit both this bound and the
chosen schedule length.

### Paper Deviation

The threshold policy constructs the paper-guided greater-than-50% group but
uses it only for leader candidacy. It does not replace Cordial membership and
does not uniformly sample a leader from that group.

### No Local Fallback

Falling back to round robin because a node lacks PoR state would split leader
selection. Before the first activated PoR epoch, round robin is valid. After
activation, missing PoR schedule state is a recovery error.

## Comparison Measurements

Shadow comparison should record, per policy:

- selected leader per wave;
- selected threshold and ranked leader-candidate set;
- finalized waves and skipped waves;
- waves between successful final leaders;
- leadership frequency per validator;
- reputation share versus leadership share;
- leader equivocation and absence outcomes;
- finalized-prefix and tau-prefix agreement;
- schedule reconstruction agreement after restart.

## Requirements For Future Consensus Activation

If PoR-aware selection is later promoted from comparison mode to an
authoritative consensus policy, that separate integration must cover:

- finalized leader lookup;
- weighted tau and cached tau;
- old/new epoch-boundary replay;
- durable append-only schedule history;
- collision-safe schedule identity in ordering caches;
- network-wide policy configuration and migration.

Comparison mode requires none of these changes and remains fully functional.
An authoritative mode must not be enabled until every leader-dependent path
uses the same epoch-aware selector.
