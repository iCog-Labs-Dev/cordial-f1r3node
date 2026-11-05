//! Immutable, replayable explanation of a complete checkpoint rating candidate.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use cordial_miners_core::{Blocklace, NodeId};
use cordial_por::{
    POR_FINALIZED_BLOCKS_PER_CHECKPOINT, PorConfig, ReputationState, reputation_block_hash,
    score_admitted_checkpoint_interaction,
};
use serde::{Deserialize, Serialize};

use crate::ordered_output::OrderedFinalizedOutput;

use super::{
    interactions::{
        admit_checkpoint_block_production_interactions, validate_finalized_checkpoint_window,
    },
    lifecycle::checkpoint::CompletedCheckpointRatingRound,
    ratings::validate_checkpoint_signed_rating,
};

const AUDIT_DIRECTORY: &str = "checkpoint-audits";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditedRating {
    pub rater: String,
    pub recipient: String,
    pub score: u64,
    pub evidence_ref: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditedReputation {
    pub node: String,
    pub reputation: u64,
    pub retained_reputation: u64,
    pub is_excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditedEquivocation {
    pub offender: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditedInactivity {
    pub offender: String,
    pub missed_rounds: u64,
}

/// A candidate report is written before commit. Activation must be checked in
/// shadow-status.json; this report alone never claims active consensus weights.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRatingAudit {
    pub schema_version: u16,
    pub por_round: u64,
    pub source_prefix_len: u64,
    pub source_prefix_hash: String,
    pub window_block_hashes: Vec<String>,
    pub eligible_raters: Vec<String>,
    /// One signed rating for every admissible rater/recipient pair. Empty
    /// batches from eligible raters are represented by `eligible_raters`.
    pub ratings: Vec<AuditedRating>,
    pub equivocations: Vec<AuditedEquivocation>,
    pub inactivity: Vec<AuditedInactivity>,
    pub reputations: Vec<AuditedReputation>,
    pub checkpoint_hash: String,
    pub ratings_hash: String,
    pub penalties_hash: String,
    pub reputation_root: String,
    pub weight_commitment: String,
    pub weights: BTreeMap<String, u64>,
}

impl CheckpointRatingAudit {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        blocklace: &Blocklace,
        output: &OrderedFinalizedOutput,
        completed: &CompletedCheckpointRatingRound,
        preceding: &ReputationState,
        candidate: &ReputationState,
        config: &PorConfig,
        authorized: &BTreeSet<NodeId>,
        weight_commitment: [u8; 32],
        weights: &BTreeMap<String, u64>,
    ) -> Result<Self> {
        completed.validate_preceding_state(preceding)?;
        let checkpoint = completed.checkpoint();
        validate_finalized_checkpoint_window(blocklace, output, checkpoint)?;
        ensure!(
            candidate.round() == checkpoint.por_round,
            "candidate round differs from source checkpoint"
        );
        ensure!(
            completed.batch().round == checkpoint.por_round,
            "rating batch round differs from source checkpoint"
        );
        let source = checkpoint.rating_source();
        let mut supplied = BTreeMap::new();
        for rating in &completed.batch().ratings {
            validate_checkpoint_signed_rating(rating, &source, config)?;
            ensure!(
                supplied
                    .insert((rating.rater.clone(), rating.recipient.clone()), rating)
                    .is_none(),
                "duplicate checkpoint rating pair"
            );
        }

        let eligible: Vec<_> = authorized
            .iter()
            .filter(|rater| !preceding.is_ejected(rater))
            .collect();
        let mut ratings = Vec::new();
        for rater in &eligible {
            cordial_por::validate_checkpoint_rater(&source, rater, preceding)?;
            for interaction in admit_checkpoint_block_production_interactions(
                blocklace, output, checkpoint, rater, preceding,
            )? {
                let evidence = interaction.evidence();
                let rating = supplied
                    .remove(&(evidence.rater.clone(), evidence.recipient.clone()))
                    .context("missing expected checkpoint rating")?;
                ensure!(
                    rating.score == score_admitted_checkpoint_interaction(&interaction, config)?
                        && rating.interaction_ref.as_deref()
                            == Some(evidence.evidence_ref.as_slice()),
                    "signed checkpoint rating differs from finalized evidence"
                );
                ratings.push(AuditedRating {
                    rater: hex::encode(&rating.rater.0),
                    recipient: hex::encode(&rating.recipient.0),
                    score: rating.score,
                    evidence_ref: hex::encode(&evidence.evidence_ref),
                    signature: hex::encode(&rating.signature),
                });
            }
        }
        ensure!(
            supplied.is_empty(),
            "checkpoint batch contains unexpected rating pairs"
        );
        ratings.sort_by(|a, b| (&a.rater, &a.recipient).cmp(&(&b.rater, &b.recipient)));

        let block = candidate
            .latest_block()
            .context("candidate has no reputation block")?;
        ensure!(
            block.header.source_checkpoint == Some(source),
            "candidate block has a different rating source"
        );
        let checkpoint_hash = reputation_block_hash(block)?;
        let mut equivocations = Vec::new();
        let mut inactivity = Vec::new();
        if let Some(penalties) = completed.penalties() {
            ensure!(
                penalties.round == source.round,
                "candidate penalties have a different round"
            );
            equivocations = penalties
                .equivocations
                .iter()
                .map(|event| AuditedEquivocation {
                    offender: hex::encode(&event.offender.0),
                    evidence: hex::encode(&event.evidence),
                })
                .collect();
            inactivity = penalties
                .inactivity
                .iter()
                .map(|event| AuditedInactivity {
                    offender: hex::encode(&event.offender.0),
                    missed_rounds: event.missed_rounds,
                })
                .collect();
            equivocations
                .sort_by(|a, b| (&a.offender, &a.evidence).cmp(&(&b.offender, &b.evidence)));
            inactivity.sort_by(|a, b| {
                (&a.offender, a.missed_rounds).cmp(&(&b.offender, b.missed_rounds))
            });
        }
        let end = usize::try_from(source.finalized_prefix_len)?;
        let window = usize::try_from(POR_FINALIZED_BLOCKS_PER_CHECKPOINT)?;
        let reputations = candidate
            .reputation_list()
            .entries
            .iter()
            .map(|entry| AuditedReputation {
                node: hex::encode(&entry.node_id.0),
                reputation: entry.reputation,
                retained_reputation: entry.retained_reputation,
                is_excluded: entry.is_excluded,
            })
            .collect();
        Ok(Self {
            schema_version: 1,
            por_round: source.round,
            source_prefix_len: source.finalized_prefix_len,
            source_prefix_hash: hex::encode(source.finalized_prefix_hash),
            window_block_hashes: output.blocks[end - window..end]
                .iter()
                .map(|identity| hex::encode(identity.content_hash))
                .collect(),
            eligible_raters: eligible.iter().map(|rater| hex::encode(&rater.0)).collect(),
            ratings,
            equivocations,
            inactivity,
            reputations,
            checkpoint_hash: hex::encode(checkpoint_hash),
            ratings_hash: hex::encode(block.header.ratings_hash),
            penalties_hash: hex::encode(block.header.penalties_hash),
            reputation_root: hex::encode(block.header.reputation_root),
            weight_commitment: hex::encode(weight_commitment),
            weights: weights.clone(),
        })
    }

    pub fn path(data_dir: &Path, round: u64) -> PathBuf {
        data_dir
            .join("por")
            .join(AUDIT_DIRECTORY)
            .join(format!("round-{round:020}.json"))
    }

    /// A repeated poll or restart must produce identical bytes for this round.
    pub fn persist(&self, data_dir: &Path) -> Result<PathBuf> {
        let path = Self::path(data_dir, self.por_round);
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::create_dir_all(path.parent().context("audit path has no parent")?)?;
        if path.exists() {
            ensure!(
                fs::read(&path)? == bytes,
                "checkpoint audit changed at round {}",
                self.por_round
            );
            return Ok(path);
        }
        let temporary = path.with_extension("json.tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        File::open(path.parent().context("audit path has no parent")?)?.sync_all()?;
        Ok(path)
    }
}
