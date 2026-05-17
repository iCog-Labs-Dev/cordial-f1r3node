# 09 - Ratification Math

## Scope

Issue 2 adds weighted ratification and weighted super-ratification for the
f1r3node compatibility path. The paper-native Cordial Miners implementation is
kept intact, and the weighted path is added beside it.

Leader selection and leader finality are separate concerns and live in
`crates/cordial-miners-core/src/consensus/finality.rs`. Ratification remains a
cordiality concern, with approval-specific helpers in the approval module.

## Paper-Native Functions

The original paper-native functions stay unchanged:

- `approves(...)` in `crates/cordial-miners-core/src/consensus/approval.rs`
- `ratifies(...)` in `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `super_ratifies(...)` in `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `is_supermajority(...)` in `crates/cordial-miners-core/src/consensus/cordiality.rs`

These functions use the Cordial Miners paper's distinct-creator supermajority
semantics. They are still the implementation used by paper-native leader
finality checks.

## Weighted Variants

The weighted compatibility path adds these functions:

- `weighted_approving_creators(...)` in
  `crates/cordial-miners-core/src/consensus/approval.rs`
- `weighted_ratifies(...)` in
  `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `weighted_super_ratifies(...)` in
  `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `is_weighted_supermajority(...)` in
  `crates/cordial-miners-core/src/consensus/cordiality.rs`

These functions reuse the same approval relation as the paper-native path, then
replace distinct-creator counting with bonded stake.

## Approval

Ratification depends on approval. A block approves a target when:

```text
approves(approver, target) is true iff:
  target is in closure(approver), and
  closure(approver) does not include an incomparable conflicting block
  by the target creator.
```

`weighted_approving_creators(...)` does not redefine approval. It filters a
provided block set through `approves(...)`, then returns only creators that have
positive bond weight.

Unknown validators and zero-weight validators may produce approving blocks, but
they contribute no weighted support.

## Weighted Ratification

`weighted_ratifies(ratifier, target, bonds)` asks whether the ratifier's
inclusive closure contains enough bonded approval:

```text
ApprovingCreators(ratifier, target) = {
  creator(block) |
  block in closure(ratifier) and approves(block, target)
}

weighted_ratifies(ratifier, target, bonds) =
  sum(bonds[v] for v in ApprovingCreators) / total_bond_weight > 2/3
```

Each validator is counted at most once. If a validator has multiple approving
blocks in the closure, its bond weight is added once.

## Weighted Super-Ratification

`weighted_super_ratifies(blocks, target, bonds)` applies the same weighted
support rule one level higher:

```text
RatifyingCreators(blocks, target) = {
  creator(block) |
  block in blocks and weighted_ratifies(block, target, bonds)
}

weighted_super_ratifies(blocks, target, bonds) =
  sum(bonds[v] for v in RatifyingCreators) / total_bond_weight > 2/3
```

The caller supplies the block set for the relevant round, wave, or adapter
context. The weighted helper does not select leaders or wave boundaries.

## Threshold Arithmetic

Weighted supermajority is strict two-thirds:

```text
support_weight * 3 > total_weight * 2
```

With total bond weight `10`, support weight `7` passes because `7 * 3 > 10 * 2`.
Support weight `6` fails because `6 * 3 <= 10 * 2`.

The implementation uses integer arithmetic instead of floating point. Weight
accumulation and multiplication are checked; if arithmetic overflows, the query
fails closed.

Zero total bond weight never passes.

## Implementation Plan

1. Keep the paper-native functions unchanged so existing protocol tests and
   leader finality behavior continue to use distinct-creator supermajority.
2. Add weighted approval support in `consensus/approval.rs` through
   `weighted_approving_creators(...)`.
3. Add weighted ratification support in `consensus/cordiality.rs` through
   `weighted_ratifies(...)`, `weighted_super_ratifies(...)`, and
   `is_weighted_supermajority(...)`.
4. Use validator bond weights from the f1r3node compatibility layer.
5. Count each validator at most once, matching the distinct-creator behavior of
   the paper-native implementation while replacing creator cardinality with
   bonded stake.
6. Keep finality code focused on finality. Do not place weighted ratification
   logic or tests inside finality.
7. Cover the weighted path with tests beside the existing approval and
   ratification tests.

## Tests

The focused weighted ratification tests cover:

- positive-weight approving creators,
- weighted ratification success and failure,
- weighted super-ratification success and failure,
- strict two-thirds threshold behavior, and
- zero total bond weight.
