# Ratification Math

This note documents the ratification scope for Issue 2. It keeps the
paper-native Cordial Miners predicates intact and adds weighted variants for the
f1r3node validator model.

## Paper-Native Predicates

The existing implementation keeps the paper-native functions as the source of
truth:

- `approves(...)`
- `ratifies(...)`
- `super_ratifies(...)`
- `is_supermajority(...)`

The Cordial Miners paper defines approval through the observed block closure and
equivocation relation. It defines ratification over the inclusive closure of the
ratifier, and super-ratification over a supermajority of ratifying blocks. The
implementation preserves those predicates and does not change their threshold
semantics.

## Weighted f1r3node Variants

The weighted functions are added beside the paper-native functions:

- `weighted_approving_creators(...)`
- `weighted_ratifies(...)`
- `weighted_super_ratifies(...)`
- `is_weighted_supermajority(...)`

`weighted_approving_creators(...)` does not define approval itself. It delegates
to the existing paper-native `approves(...)` predicate. Any details about
observed conflicts, incomparability, and equivocation remain owned by
`approves(...)`.

The weighted path changes only support accounting. Instead of counting distinct
creators equally, it sums bonded stake from the active validator bond map:

- unknown creators contribute no support
- zero-weight creators contribute no support
- each creator contributes at most once
- the denominator is the total active stake in the supplied bond map

## Weighted Threshold

`is_weighted_supermajority(...)` uses a strict integer two-thirds check:

```text
support_weight * 3 > total_weight * 2
```

This avoids floating-point rounding and preserves strictness at the exact
boundary. For total stake 10:

- support 6 fails because `6 * 3 == 10 * 2`
- support 7 passes because `7 * 3 > 10 * 2`

Checked arithmetic is used for accumulation and multiplication. Overflow fails
closed by returning `false`.

## Closure and Witness Sets

`weighted_ratifies(...)` follows the paper-native ratification shape: it inspects
the ratifier block's inclusive causal closure, then applies weighted support to
the blocks in that closure that approve the target.

`weighted_super_ratifies(...)` receives the witness block set from its caller.
That set should come from the relevant round, wave, or f1r3node adapter context.
The weighted ratification module does not select leaders, finality windows,
waves, pruning bounds, networking state, or ordering policy.

## Memoization Scope

Weighted super-ratification may ask repeated ratification questions over the
same blocklace snapshot. The implementation uses a private per-call memo for
weighted ratification results.

The memo is intentionally local. It must not be reused globally across DAG
updates, bond map changes, validator epoch changes, or threshold policy changes.

## Out of Scope

This issue does not implement leader finality, compatibility finality detection,
ordering, pruning, networking, or adapter policy. Those concerns belong in a
separate issue and PR.
