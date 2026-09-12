use std::collections::BTreeSet;

use cordial_miners_core::NodeId;

use crate::{
    error::PorError,
    types::{
        RatingRecord, ReputationBlock, ReputationEntry, ReputationList, ReputationRound,
        ReputationVector, ReputationWeight,
    },
};

/// Local Proof-of-Reputation state.
///
/// This stores PoR data only.
/// It does not calculate reputation.
///
/// ## Ejection invariant
///
/// `excluded_keys` is the **sole and permanent source of truth** for key
/// ejection. It is never cleared by `apply_reputation_vector`, `set_reputation`,
/// or any other mutation path. Once a `NodeId` appears in `excluded_keys` it
/// stays there for the lifetime of this state object.
///
/// The `reputation_list` is a replaceable snapshot. External code must not
/// rely on `is_excluded` flags inside the list for security decisions — use
/// `is_ejected()` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationState {
    current_round: ReputationRound,

    reputation_list: ReputationList,

    /// Permanent ejection registry.
    ///
    /// Keys present here are excluded from the consensus weighted path
    /// regardless of what `reputation_list` contains.
    excluded_keys: BTreeSet<NodeId>,

    pending_ratings: Vec<RatingRecord>,

    latest_block: Option<ReputationBlock>,
}

impl ReputationState {
    pub fn new(round: ReputationRound) -> Self {
        Self {
            current_round: round,

            reputation_list: ReputationList {
                round,
                entries: Vec::new(),
            },

            excluded_keys: BTreeSet::new(),

            pending_ratings: Vec::new(),

            latest_block: None,
        }
    }

    pub fn round(&self) -> ReputationRound {
        self.current_round
    }

    pub fn reputation_list(&self) -> &ReputationList {
        &self.reputation_list
    }

    pub fn pending_ratings(&self) -> &[RatingRecord] {
        &self.pending_ratings
    }

    pub fn latest_block(&self) -> Option<&ReputationBlock> {
        self.latest_block.as_ref()
    }

    pub fn add_rating(&mut self, rating: RatingRecord) {
        self.pending_ratings.push(rating);
    }

    /// Returns `true` if `node_id` has been permanently ejected.
    ///
    /// This consults the permanent `excluded_keys` registry, not the
    /// `is_excluded` flag on individual `ReputationEntry` values.
    pub fn is_ejected(&self, node_id: &NodeId) -> bool {
        self.excluded_keys.contains(node_id)
    }

    /// Returns a view of all currently ejected node IDs.
    pub fn excluded_keys(&self) -> &BTreeSet<NodeId> {
        &self.excluded_keys
    }

    /// Insert or update a validator's reputation weight.
    ///
    /// If `node_id` is present in the permanent ejection registry this call
    /// is a no-op — ejection cannot be reversed through reputation assignment.
    pub fn set_reputation(&mut self, node_id: NodeId, reputation: ReputationWeight) {
        // Permanently ejected keys must never be re-inserted as active.
        if self.excluded_keys.contains(&node_id) {
            return;
        }

        match self
            .reputation_list
            .entries
            .binary_search_by(|entry| entry.node_id.cmp(&node_id))
        {
            Ok(index) => {
                self.reputation_list.entries[index].reputation = reputation;
            }
            Err(index) => {
                self.reputation_list
                    .entries
                    .insert(index, ReputationEntry::new(node_id, reputation));
            }
        }
    }

    /// Permanently eject a validator key from the active set.
    ///
    /// Records `node_id` in the `excluded_keys` registry so that ejection
    /// survives future calls to `apply_reputation_vector` and
    /// `set_reputation` regardless of whether the node appears in incoming
    /// vectors.
    ///
    /// If the node is currently present in `reputation_list` its weight is
    /// zeroed and its `is_excluded` flag is set for consistency. If the node
    /// is absent from the list it is still recorded in `excluded_keys`.
    ///
    /// Returns `PorError::UnknownNode` if the node is neither present in
    /// `reputation_list` nor already in `excluded_keys`.
    pub fn eject_validator(&mut self, node_id: &NodeId) -> Result<(), PorError> {
        // If already ejected, idempotent success.
        if self.excluded_keys.contains(node_id) {
            return Ok(());
        }

        // Record in the permanent registry first.
        let found_in_list = self
            .reputation_list
            .entries
            .binary_search_by(|entry| entry.node_id.cmp(node_id))
            .ok();

        if found_in_list.is_none() {
            // Node is not in the list and not yet ejected — unknown.
            return Err(PorError::UnknownNode);
        }

        self.excluded_keys.insert(node_id.clone());

        // Zero out in the reputation list for the derived view.
        if let Some(index) = found_in_list {
            self.reputation_list.entries[index].is_excluded = true;
            self.reputation_list.entries[index].reputation = 0;
        }

        Ok(())
    }

    /// Apply a finalized reputation vector as the current state snapshot.
    ///
    /// The vector is expected to be the already-computed output of the
    /// calculation pipeline. This method validates canonical ordering, takes
    /// ownership of the vector entries, and replaces the state's reputation
    /// list; it does not recompute ratings, Liquid Rank, transition, or
    /// clamping.
    ///
    /// ## Ejection preservation
    ///
    /// The `excluded_keys` registry is **never modified** by this method.
    /// After replacing the list, all entries whose `NodeId` appears in
    /// `excluded_keys` are zeroed and flagged `is_excluded = true`.
    /// Ejected nodes that are **absent** from the incoming vector are
    /// re-inserted as ejected tombstone entries so that the derived
    /// `is_excluded` view remains consistent with the registry.
    pub fn apply_reputation_vector(&mut self, vector: ReputationVector) -> Result<(), PorError> {
        validate_reputation_vector(&vector)?;

        // Snapshot the excluded registry before we touch anything.
        let excluded = &self.excluded_keys;

        // Build the new list from the incoming vector, re-applying ejection
        // for any node that appears in the permanent registry.
        let mut new_entries: Vec<ReputationEntry> = vector
            .values
            .into_iter()
            .map(|mut entry| {
                if excluded.contains(&entry.node_id) {
                    entry.is_excluded = true;
                    entry.reputation = 0;
                }
                entry
            })
            .collect();

        // Re-insert tombstone entries for ejected nodes that were omitted from
        // the incoming vector so they cannot be silently resurrected by a
        // later set_reputation call that finds the node absent from the list.
        for ejected_id in excluded.iter() {
            let already_present = new_entries
                .binary_search_by(|e| e.node_id.cmp(ejected_id))
                .is_ok();

            if !already_present {
                // Insert in sorted position to preserve canonical ordering.
                let insert_pos = new_entries.partition_point(|e| e.node_id < *ejected_id);
                new_entries.insert(insert_pos, ReputationEntry::ejected(ejected_id.clone()));
            }
        }

        self.current_round = vector.round;
        self.reputation_list = ReputationList {
            round: vector.round,
            entries: new_entries,
        };

        Ok(())
    }
}

impl Default for ReputationState {
    fn default() -> Self {
        Self::new(0)
    }
}

fn validate_reputation_vector(vector: &ReputationVector) -> Result<(), PorError> {
    for entries in vector.values.windows(2) {
        match entries[0].node_id.cmp(&entries[1].node_id) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(PorError::DuplicateReputationEntry),
            std::cmp::Ordering::Greater => return Err(PorError::UnsortedReputationVector),
        }
    }

    Ok(())
}
