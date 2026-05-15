# 09 - Weighted Ratification Math

## Paper Reference

Definition 22 of the Cordial Miners paper defines two recursive finality
operators:

- A block is **ratified** when the closure of a witness contains a
  supermajority of blocks that approve it.
- A block is **super-ratified** when the supplied blocklace or witness set
  contains a supermajority of blocks that ratify it.

The paper counts miners. F1R3Node consensus is proof-of-stake based, so the
implementation in `cordial-miners-core/src/finality.rs` uses validator weight
instead of one-validator-one-vote counting.

## Pure Inputs

The weighted finality module is deliberately isolated from node, storage,
networking, crypto, and runtime crates. It only needs:

- `CordialBlock<VId, P, Id>`: block id, creator, round, parents, and opaque
  payload.
- `Blocklace<VId, P, Id>`: a read-only way to resolve blocks and request the
  inclusive causal closure of a block id.
- `ValidatorSet<VId>`: active stake weight for each validator and the total
  active weight for the decision context.
- `ApprovalThreshold`: a validated rational threshold such as strict
  two-thirds.

Adapters are responsible for mapping real DAG/storage data and epoch validator
weights into these pure interfaces. The denominator for weighted ratification
is always `ValidatorSet::total_weight()`: the active validator weight for the
decision context, not locally observed non-equivocating stake.

## Base Approval Predicate

Ratification depends on approval. In the weighted finality module:

```text
approve(approver, candidate) is true iff:
  candidate is in closure(approver), and
  closure(approver) contains no different block with the same
  candidate creator and candidate round.
```

Approval is local to the approver's observed closure. A conflicting candidate
that exists elsewhere in the blocklace but is not observed by the approver does
not invalidate that approver's approval. The module exposes one canonical
generic approval predicate for this math layer: `approve_with_memo` delegates to
the same approval logic used by `approve`.

## Weighted Ratify

`ratify(witness, candidate)` asks whether the witness observes enough weighted
approval:

```text
ApprovingValidators(witness, candidate) = {
  creator(block) |
  block in closure(witness) and approve(block, candidate)
}

ratify(witness, candidate) =
  sum(weight(v) for v in ApprovingValidators) / total_weight > threshold
```

Each validator is counted at most once. If the closure contains several
approving blocks from validator `A`, only `weight(A)` is added. Unknown
validators and validators with zero weight may approve, but they add no support.

## Weighted Super-Ratify

`super_ratify(witness_set, candidate)` applies the same weighted support rule
one level higher:

```text
RatifyingValidators(witness_set, candidate) = {
  creator(witness) |
  witness in witness_set and ratify(witness, candidate)
}

super_ratify(witness_set, candidate) =
  sum(weight(v) for v in RatifyingValidators) / total_weight > threshold
```

The caller supplies the witness set for the relevant Cordial Miners wave or
round. Wave selection, leader choice, total ordering, pruning, storage, and
networking are outside this math module. Missing witness ids are ignored and do
not contribute support.

## Strict Threshold Arithmetic

Threshold checks use exact integer comparison:

```text
support_weight * denominator > total_weight * numerator
```

For strict two-thirds, the threshold is `2/3`. With total weight `10`, support
weight `7` passes because `7 * 3 > 10 * 2`, while support weight `6` fails
because `6 * 3 <= 10 * 2`.

The implementation avoids floating point nondeterminism and uses a small
wide-multiplication helper so the comparison remains exact for `u128` inputs.
The helper is covered by direct boundary tests, including cases that would
overflow direct `u128` multiplication.

Invalid thresholds are configuration errors. Use `ApprovalThreshold::try_new`
or the built-in `strict_two_thirds()` constructor; zero denominators and
thresholds greater than or equal to `1` are rejected. Zero total active weight
never passes.

Support weights are accumulated with checked addition. If a validator set would
overflow `u128` support accumulation, the query fails closed instead of
saturating and accidentally creating apparent finality.

## Memoization Strategy

Ratification and super-ratification reuse many of the same graph questions. A
single `FinalityMemo<Id>` is passed through the recursive calls:

```rust
approve_cache[(approver_id, candidate_id)] -> bool
ratify_cache[(witness_id, candidate_id)] -> bool
```

The public convenience wrappers allocate a fresh memo. The `_with_memo`
functions accept a mutable memo so callers can reuse cached results while
evaluating several witness sets or candidates over the same blocklace.

A `FinalityMemo` is scoped to one immutable blocklace snapshot, one active
validator set, and one threshold. Reusing a memo after changing any of those
inputs can return stale results because cache keys are only block pairs.

Expected behavior:

- Each approval pair is computed once per memo.
- Each ratification pair is computed once per memo.
- Memoization does not affect results; it only prevents repeated closure scans.
- With unique closure ids, one candidate, and a fixed witness set, runtime is
  approximately linear in the number of visited block pairs plus the cost of
  closure traversal.

## Safety Regression

Two different candidates from the same leader and round must not both
super-ratify in the same decision context. A witness that observes both
same-leader, same-round candidates approves neither. Weighted strict
supermajorities also count validator creators only once, so duplicate blocks by
one validator cannot manufacture extra support.

The focused tests in `finality.rs` cover:

- weighted ratification success and failure,
- weighted super-ratification success and failure,
- duplicate blocks by the same validator,
- unknown and zero-weight validators,
- missing candidates, witnesses, and closure parents,
- zero total weight,
- invalid threshold construction,
- support overflow fail-closed behavior,
- wide multiplication boundary cases,
- strict threshold boundaries,
- same-leader same-round candidate safety, and
- memo reuse across repeated `super_ratify_with_memo` calls.
