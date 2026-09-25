use crate::{
    commitments::{
        config_commitment, rating_batch_commitment, reputation_block_hash,
        reputation_list_commitment,
    },
    config::PorConfig,
    error::PorError,
    ratings::rating_round_from_finalized_wave,
    types::{RatingBatch, ReputationBlock, ReputationBlockHeader, ReputationList},
};

/// Canonical reputation-block format emitted by this crate.
pub const REPUTATION_BLOCK_VERSION: u16 = 1;

/// Bound chain-context input before it is copied into every block header.
pub const MAX_REPUTATION_BLOCK_SHARD_ID_LEN: usize = 256;

/// External finalized context required to build or audit a reputation block.
#[derive(Debug, Clone, Copy)]
pub struct ReputationBlockContext<'a> {
    pub shard_id: &'a [u8],
    pub source_finalized_wave: u64,
    pub previous_block: Option<&'a ReputationBlock>,
}

/// Build a fully committed reputation block from finalized protocol inputs.
///
/// The caller supplies protocol data, never precomputed commitment bytes. This
/// function derives the previous-block hash, configuration commitment, rating
/// batch commitment, and reputation-list root using the canonical v1 formats.
pub fn build_reputation_block(
    context: ReputationBlockContext<'_>,
    ratings: &RatingBatch,
    reputation_list: ReputationList,
    config: &PorConfig,
) -> Result<ReputationBlock, PorError> {
    validate_shard_id(context.shard_id)?;
    let round = rating_round_from_finalized_wave(context.source_finalized_wave)?;
    if ratings.round != round {
        return Err(PorError::InvalidRatingRound);
    }
    if reputation_list.round != round {
        return Err(PorError::InvalidReputationBlockRound);
    }

    let previous_reputation_hash = match context.previous_block {
        Some(previous) => {
            if previous.header.shard_id != context.shard_id {
                return Err(PorError::PreviousReputationBlockShardMismatch);
            }
            if previous.header.round.checked_add(1) != Some(round) {
                return Err(PorError::InvalidPreviousReputationBlockRound);
            }
            Some(reputation_block_hash(previous)?)
        }
        None => None,
    };
    let config_hash = config_commitment(config);
    let ratings_hash = rating_batch_commitment(ratings, config)?;
    let reputation_root = reputation_list_commitment(&reputation_list)?;
    let block = ReputationBlock {
        header: ReputationBlockHeader {
            version: REPUTATION_BLOCK_VERSION,
            shard_id: context.shard_id.to_vec(),
            source_finalized_wave: context.source_finalized_wave,
            round,
            previous_reputation_hash,
            config_hash,
            ratings_hash,
            reputation_root,
        },
        reputation_list,
    };

    validate_reputation_block(&block)?;
    Ok(block)
}

/// Validate the self-contained structural commitments of a reputation block.
///
/// Full audit additionally requires the expected shard, finalized wave,
/// previous block, rating batch, previous reputation, and protocol config;
/// [`crate::verify_reputation_transition`] performs those checks.
pub fn validate_reputation_block(block: &ReputationBlock) -> Result<(), PorError> {
    let header = &block.header;
    if header.version != REPUTATION_BLOCK_VERSION {
        return Err(PorError::UnsupportedReputationBlockVersion(header.version));
    }
    validate_shard_id(&header.shard_id)?;

    if rating_round_from_finalized_wave(header.source_finalized_wave) != Ok(header.round) {
        return Err(PorError::InvalidReputationBlockSourceWave);
    }
    if header.round != block.reputation_list.round {
        return Err(PorError::InvalidReputationBlockRound);
    }

    let expected_root = reputation_list_commitment(&block.reputation_list)?;
    if header.reputation_root != expected_root {
        return Err(PorError::ReputationBlockRootMismatch);
    }

    Ok(())
}

fn validate_shard_id(shard_id: &[u8]) -> Result<(), PorError> {
    if shard_id.is_empty() {
        return Err(PorError::MissingReputationBlockShardId);
    }
    if shard_id.len() > MAX_REPUTATION_BLOCK_SHARD_ID_LEN {
        return Err(PorError::ReputationBlockShardIdTooLong);
    }
    Ok(())
}
