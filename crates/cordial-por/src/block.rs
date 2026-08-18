use crate::{
    error::PorError,
    types::{ReputationBlock, ReputationBlockHeader, ReputationList},
};

/// Build a reputation block from a finalized reputation list.
///
/// This function only validates and assembles the already-finalized PoR
/// snapshot. It does not compute Liquid Rank, apply transition blending,
/// clamp reputation values, mutate state, or publish the block.
pub fn build_reputation_block(
    header: ReputationBlockHeader,
    reputation_list: ReputationList,
) -> Result<ReputationBlock, PorError> {
    validate_reputation_block_inputs(&header, &reputation_list)?;

    Ok(ReputationBlock {
        header,
        reputation_list,
    })
}

fn validate_reputation_block_inputs(
    header: &ReputationBlockHeader,
    reputation_list: &ReputationList,
) -> Result<(), PorError> {
    if header.round != reputation_list.round {
        return Err(PorError::InvalidReputationBlockRound);
    }

    if header.ratings_hash.is_empty() {
        return Err(PorError::MissingReputationBlockRatingsHash);
    }

    if header.reputation_root.is_empty() {
        return Err(PorError::MissingReputationBlockRoot);
    }

    for entries in reputation_list.entries.windows(2) {
        match entries[0].node_id.cmp(&entries[1].node_id) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(PorError::DuplicateReputationEntry),
            std::cmp::Ordering::Greater => return Err(PorError::UnsortedReputationVector),
        }
    }

    Ok(())
}
