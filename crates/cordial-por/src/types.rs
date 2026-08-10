//! Proof-of-Reputation deterministic data model.
//!
//! This module defines the paper-aligned PoR vocabulary:
//! ratings, reputation snapshots, penalties, and reputation blocks.
//!
//! No reputation calculation logic exists here.
//! Committee selection and leader selection are implemented in future modules.

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

    pub rater: NodeId,

    pub recipient: NodeId,

    pub score: RatingScore,

    pub signature: Vec<u8>,

    /// Optional interaction reference.
    pub interaction_ref: Option<Vec<u8>>,
}

impl RatingRecord {
    pub fn new(
        round: ReputationRound,
        rater: NodeId,
        recipient: NodeId,
        score: RatingScore,
        std_signature: Vec<u8>,
    ) -> Self {
        Self {
            round,
            rater,
            recipient,
            score,
            interaction_ref: None,
            signature: std_signature,
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
    pub fn new(node_id: NodeId, reputation: ReputationWeight) -> Self {
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
/// Values are expected in canonical `NodeId` order.
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

    pub previous_reputation_hash: Option<Vec<u8>>,

    pub ratings_hash: Vec<u8>,

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusGroupMember {
    pub node_id: NodeId,
    pub reputation: ReputationWeight,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusGroup {
    pub round: ReputationRound,
    pub members: Vec<ConsensusGroupMember>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingMatrix {
    pub round: ReputationRound,
    pub ratings: Vec<RatingRecord>,
}

/// A rating matrix entry after paper-guided normalization.
///
/// `score` preserves the original fixed-point rating value.
/// `normalized_score` stores the fixed-point normalized value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRatingEntry {
    pub rater: NodeId,
    pub recipient: NodeId,
    pub score: RatingScore,
    pub normalized_score: RatingScore,
}

/// Rating matrix with normalized fixed-point scores.
///
/// This is prepared for Liquid-Rank contribution calculation.
/// It does not contain final reputation updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRatingMatrix {
    pub round: ReputationRound,
    pub ratings: Vec<NormalizedRatingEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderSelection {
    pub round: ReputationRound,

    pub leader: NodeId,
}
