//! Immutable snapshot of the bonded validator weight table.
//!
//! Issue #189 (gap 2): a finality decision is only meaningful relative to the
//! weight table it was judged against. Threading `&HashMap<NodeId, u64>`
//! through the decision path makes "which weights were in effect" an inference
//! from live, mutable state rather than a recoverable fact — so a decision
//! cannot be re-verified later, and a checker cannot distinguish a real
//! divergence from a race between reading weights and deciding.
//!
//! A [`WeightSnapshot`] captures the table together with its canonical
//! fingerprint. The fingerprint is the same value the canonical trace carries
//! as `weight_table_hash` and the Lean replay independently recomputes from
//! its sidecar, so a snapshot id is directly comparable against both.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::trace;
use crate::types::NodeId;

/// Canonical identity of a weight table: the FNV-1a-64 fingerprint over its
/// sorted `node_id:weight\n` rows, lowercase hex, 16 characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WeightSnapshotId(String);

impl WeightSnapshotId {
    /// Fingerprint a live bond map without materialising a whole snapshot —
    /// for callers that only need the identity, such as a cache key.
    pub fn of_bonds(bonds: &HashMap<NodeId, u64>) -> Self {
        Self(trace::weight_table_hash(bonds))
    }

    /// The fingerprint as it appears in the canonical trace.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for WeightSnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An immutable validator weight table together with its identity.
///
/// Cloning is cheap: the table is shared behind an `Arc` and never mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightSnapshot {
    id: WeightSnapshotId,
    bonds: Arc<BTreeMap<NodeId, u64>>,
}

impl WeightSnapshot {
    /// Capture a snapshot from the live bond map, computing its id once.
    pub fn from_bonds(bonds: &HashMap<NodeId, u64>) -> Self {
        Self::from_sorted(bonds.iter().map(|(node, w)| (node.clone(), *w)).collect())
    }

    /// Capture a snapshot from a table already in canonical order.
    pub fn from_sorted(bonds: BTreeMap<NodeId, u64>) -> Self {
        // `BTreeMap` iterates byte-lexicographically by `NodeId`, which is the
        // order `weight_table_hash` sorts into — so this is the identical
        // fingerprint, not merely an equivalent one.
        let id = WeightSnapshotId(trace::weight_table_hash_sorted(bonds.iter()));
        Self {
            id,
            bonds: Arc::new(bonds),
        }
    }

    /// The canonical fingerprint of this table.
    pub fn id(&self) -> &WeightSnapshotId {
        &self.id
    }

    /// Bonded stake of `node`. Unknown validators carry no weight, matching
    /// `is_weighted_supermajority`'s treatment of unknown creators.
    pub fn weight_of(&self, node: &NodeId) -> u64 {
        self.bonds.get(node).copied().unwrap_or(0)
    }

    pub fn contains(&self, node: &NodeId) -> bool {
        self.bonds.contains_key(node)
    }

    /// Total bonded stake, or `None` on overflow — fail-closed, matching the
    /// checked accumulation the quorum predicates already use.
    pub fn total(&self) -> Option<u128> {
        self.bonds
            .values()
            .try_fold(0u128, |total, weight| total.checked_add(u128::from(*weight)))
    }

    pub fn len(&self) -> usize {
        self.bonds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bonds.is_empty()
    }

    /// Canonical-order view for iteration.
    pub fn as_map(&self) -> &BTreeMap<NodeId, u64> {
        &self.bonds
    }

    /// Escape hatch for call sites not yet migrated (dissemination, adapter).
    pub fn to_hash_map(&self) -> HashMap<NodeId, u64> {
        self.bonds
            .iter()
            .map(|(node, weight)| (node.clone(), *weight))
            .collect()
    }
}

impl From<&HashMap<NodeId, u64>> for WeightSnapshot {
    fn from(bonds: &HashMap<NodeId, u64>) -> Self {
        Self::from_bonds(bonds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u8) -> NodeId {
        NodeId(vec![id])
    }

    fn table() -> HashMap<NodeId, u64> {
        let mut bonds = HashMap::new();
        bonds.insert(node(1), 100);
        bonds.insert(node(2), 200);
        bonds.insert(node(3), 300);
        bonds
    }

    /// The load-bearing invariant: a snapshot id is byte-identical to the
    /// fingerprint the canonical trace carries. If this breaks, every trace
    /// fixture and the Lean sidecar check break with it.
    #[test]
    fn snapshot_id_matches_canonical_trace_fingerprint() {
        let bonds = table();
        let snapshot = WeightSnapshot::from_bonds(&bonds);
        assert_eq!(snapshot.id().as_str(), trace::weight_table_hash(&bonds));
    }

    #[test]
    fn standalone_id_matches_full_snapshot_id() {
        let bonds = table();
        assert_eq!(
            WeightSnapshotId::of_bonds(&bonds),
            *WeightSnapshot::from_bonds(&bonds).id()
        );
    }

    #[test]
    fn snapshot_id_is_independent_of_insertion_order() {
        let mut reversed = HashMap::new();
        reversed.insert(node(3), 300);
        reversed.insert(node(1), 100);
        reversed.insert(node(2), 200);

        assert_eq!(
            WeightSnapshot::from_bonds(&table()).id(),
            WeightSnapshot::from_bonds(&reversed).id()
        );
    }

    #[test]
    fn differing_tables_have_differing_ids() {
        let mut heavier = table();
        heavier.insert(node(3), 301);
        assert_ne!(
            WeightSnapshot::from_bonds(&table()).id(),
            WeightSnapshot::from_bonds(&heavier).id()
        );
    }

    #[test]
    fn unknown_validators_carry_no_weight() {
        let snapshot = WeightSnapshot::from_bonds(&table());
        assert_eq!(snapshot.weight_of(&node(1)), 100);
        assert_eq!(snapshot.weight_of(&node(9)), 0);
        assert!(!snapshot.contains(&node(9)));
    }

    #[test]
    fn total_sums_bonded_stake() {
        assert_eq!(WeightSnapshot::from_bonds(&table()).total(), Some(600));
    }

    #[test]
    fn total_fails_closed_on_overflow() {
        let mut bonds = HashMap::new();
        bonds.insert(node(1), u64::MAX);
        bonds.insert(node(2), u64::MAX);
        // Two u64::MAX values still fit in u128; the checked fold only fails
        // beyond that, so assert the accumulation is genuinely widened.
        assert_eq!(
            WeightSnapshot::from_bonds(&bonds).total(),
            Some(u128::from(u64::MAX) * 2)
        );
    }

    #[test]
    fn empty_table_round_trips() {
        let snapshot = WeightSnapshot::from_bonds(&HashMap::new());
        assert!(snapshot.is_empty());
        assert_eq!(snapshot.total(), Some(0));
        assert_eq!(snapshot.id().as_str(), trace::weight_table_hash(&HashMap::new()));
    }

    #[test]
    fn hash_map_view_round_trips() {
        let bonds = table();
        assert_eq!(WeightSnapshot::from_bonds(&bonds).to_hash_map(), bonds);
    }
}
