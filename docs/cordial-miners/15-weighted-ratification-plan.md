# Weighted Ratification Implementation Plan

## Context

Issue 2 adds weighted ratification and weighted super-ratification for the
f1r3node compatibility path. The paper-native implementation already exists and
should stay intact:

- `approves(...)` in `crates/cordial-miners-core/src/consensus/approval.rs`
- `ratifies(...)` in `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `super_ratifies(...)` in `crates/cordial-miners-core/src/consensus/cordiality.rs`
- `is_supermajority(...)` in `crates/cordial-miners-core/src/consensus/cordiality.rs`

Leader selection and leader finality belong in
`crates/cordial-miners-core/src/consensus/finality.rs`. Ratification remains a
cordiality concern, with approval-specific helpers in the approval module.

## Plan

1. Keep the paper-native functions unchanged so existing protocol tests and
   leader finality behavior continue to use the original distinct-creator
   supermajority semantics.
2. Add a weighted approval helper in `consensus/approval.rs`:
   `weighted_approving_creators(...)`.
3. Add weighted ratification helpers in `consensus/cordiality.rs`:
   `weighted_ratifies(...)`, `weighted_super_ratifies(...)`, and
   `is_weighted_supermajority(...)`.
4. Use validator bond weights from the f1r3node compatibility layer. Unknown
   validators and zero-weight validators contribute no support.
5. Count each validator at most once, matching the distinct-creator behavior of
   the paper-native implementation while replacing creator cardinality with
   bonded stake.
6. Keep finality code focused on finality. Do not place weighted ratification
   logic or test modules inside finality.
7. Cover the weighted path with tests beside the existing approval and
   ratification tests.
