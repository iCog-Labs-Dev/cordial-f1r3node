//! Proof-of-Reputation deterministic data model.
//!
//! This module defines the paper-aligned PoR vocabulary:
//! ratings, reputation snapshots, penalties, and reputation blocks.
//!
//! No reputation calculation logic exists here.
//! Liquid-rank, committee selection, and leader selection are implemented in future modules.

use cordial_miners_core::NodeId;

/// Logical PoR processing round.
pub type ReputationRound = u64;

/// Fixed-point reputation value.
///
/// Example:
/// scale = 1_000_000_000
///
/// 500_000_000 represents 0.5 reputation.
pub type ReputationWeight = u64;

/// Fixed-point rating value.
pub type RatingScore = u64;



// ============================================================
// Rating model
// ============================================================

/// A single rating transaction.
///
/// Paper concept:
/// node i rates node j after interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingRecord {
    pub round: ReputationRound,

    /// Node providing the rating.
    pub rater: NodeId,

    /// Node receiving the rating.
    pub recipient: NodeId,

    /// Fixed-point rating score.
    pub score: RatingScore,

    /// Optional future evidence reference.
    pub interaction_ref: Option<Vec<u8>>,
}


impl RatingRecord {
    pub fn new(
        round: ReputationRound,
        rater: NodeId,
        recipient: NodeId,
        score: RatingScore,
    ) -> Self {
        Self {
            round,
            rater,
            recipient,
            score,
            interaction_ref: None,
        }
    }
}


/// Collection of ratings belonging to one round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingBatch {
    pub round: ReputationRound,
    pub ratings: Vec<RatingRecord>,
}



// ============================================================
// Reputation snapshot model
// ============================================================

/// Reputation value assigned to one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationEntry {
    pub node_id: NodeId,
    pub reputation: ReputationWeight,
}


impl ReputationEntry {
    pub fn new(
        node_id: NodeId,
        reputation: ReputationWeight,
    ) -> Self {
        Self {
            node_id,
            reputation,
        }
    }
}



/// Complete reputation snapshot for a round.
///
/// Paper concept:
/// ReputationList_i contains all nodes and their reputation values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationList {
    pub round: ReputationRound,
    pub entries: Vec<ReputationEntry>,
}



/// Mathematical reputation vector representation.
///
/// This is a paper-aligned structure only.
/// No calculation is performed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationVector {
    pub round: ReputationRound,
    pub values: Vec<ReputationEntry>,
}



// ============================================================
// Penalty placeholders
// ============================================================

/// Placeholder for equivocation evidence.
///
/// No slashing logic exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivocationPenalty {
    pub offender: NodeId,
    pub evidence: Vec<u8>,
}



/// Placeholder for inactivity penalties.
///
/// No punishment logic exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InactivityPenalty {
    pub offender: NodeId,
    pub missed_rounds: u64,
}



// ============================================================
// Reputation block model
// ============================================================

/// Metadata describing a reputation block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationBlockHeader {
    pub round: ReputationRound,

    /// Hash of previous reputation state.
    pub previous_reputation_hash: Vec<u8>,

    /// Hash of ratings included in this round.
    pub ratings_hash: Vec<u8>,

    /// Root commitment of reputation entries.
    pub reputation_root: Vec<u8>,
}



/// Reputation block.
///
/// Paper concept:
///
/// ReputationBlock = Header + ReputationList
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationBlock {
    pub header: ReputationBlockHeader,
    pub reputation_list: ReputationList,
}