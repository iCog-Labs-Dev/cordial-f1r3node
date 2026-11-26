use cordial_miners_core::NodeId;
use cordial_por::{
    CheckpointReputationBlockContext, MissingEntryPolicy, PorConfig, PorError, RatingRecord,
    RatingSourceCheckpoint, ReputationBlock, ReputationBlockContext, ReputationState,
    build_checkpoint_reputation_block, build_rating_batch, build_reputation_block,
    replay_reputation_transition, reputation_list_commitment, reputation_weights,
};

const ROUND: u64 = 1;
const SOURCE_WAVE: u64 = ROUND - 1;
const SHARD_ID: &[u8] = b"root";

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 0,
        liquid_rank_alpha: 50,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
        ..PorConfig::new(100, 0)
    }
}

fn ratings() -> Vec<RatingRecord> {
    vec![
        RatingRecord::new(ROUND, node(1), node(2), 80, vec![0x01]),
        RatingRecord::new(ROUND, node(2), node(1), 60, vec![0x02]),
    ]
}

fn state() -> ReputationState {
    let mut state = ReputationState::new(ROUND - 1);
    state.set_reputation(node(1), 40);
    state.set_reputation(node(2), 60);
    state
}

fn valid_block(state: &ReputationState) -> ReputationBlock {
    let previous = cordial_por::ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let list = replay_reputation_transition(&previous, &ratings(), ROUND, &config()).unwrap();
    let batch = build_rating_batch(ROUND, ratings(), &config()).unwrap();

    build_reputation_block(
        ReputationBlockContext {
            shard_id: SHARD_ID,
            source_finalized_wave: SOURCE_WAVE,
            previous_block: state.latest_block(),
        },
        &batch,
        list,
        &config(),
    )
    .unwrap()
}

fn checkpoint_context<'a>(
    previous_block: Option<&'a ReputationBlock>,
) -> CheckpointReputationBlockContext<'a> {
    CheckpointReputationBlockContext {
        shard_id: SHARD_ID,
        source: RatingSourceCheckpoint {
            round: ROUND,
            finalized_prefix_len: 16,
            finalized_prefix_hash: [0x31; 32],
        },
        previous_block,
    }
}

fn valid_checkpoint_block(state: &ReputationState) -> ReputationBlock {
    let previous = cordial_por::ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let list = replay_reputation_transition(&previous, &ratings(), ROUND, &config()).unwrap();
    let batch = build_rating_batch(ROUND, ratings(), &config()).unwrap();
    build_checkpoint_reputation_block(
        checkpoint_context(state.latest_block()),
        &batch,
        list,
        &config(),
    )
    .unwrap()
}

#[test]
fn audited_block_application_advances_state_and_records_latest_block() {
    let mut state = state();
    let block = valid_block(&state);

    state
        .apply_reputation_block(SHARD_ID, SOURCE_WAVE, &ratings(), block.clone(), &config())
        .unwrap();

    assert_eq!(state.round(), ROUND);
    assert_eq!(state.reputation_list(), &block.reputation_list);
    assert_eq!(state.latest_block(), Some(&block));
    assert_eq!(reputation_weights(&state).len(), 2);
}

#[test]
fn failed_block_audit_leaves_state_unchanged() {
    let mut state = state();
    let before = state.clone();
    let mut block = valid_block(&state);
    block.reputation_list.entries[0].reputation += 1;
    block.header.reputation_root = reputation_list_commitment(&block.reputation_list).unwrap();

    assert_eq!(
        state.apply_reputation_block(SHARD_ID, SOURCE_WAVE, &ratings(), block, &config()),
        Err(PorError::ReputationValueMismatch)
    );
    assert_eq!(state, before);
}

#[test]
fn audited_checkpoint_block_advances_state_atomically() {
    let mut state = state();
    let block = valid_checkpoint_block(&state);
    state
        .apply_checkpoint_reputation_block(
            SHARD_ID,
            checkpoint_context(None).source,
            &ratings(),
            block.clone(),
            &config(),
        )
        .unwrap();
    assert_eq!(state.round(), ROUND);
    assert_eq!(state.latest_block(), Some(&block));
    assert_eq!(state.reputation_list(), &block.reputation_list);
}

#[test]
fn failed_checkpoint_audit_leaves_state_unchanged() {
    let mut state = state();
    let before = state.clone();
    let block = valid_checkpoint_block(&state);
    let mut changed_source = checkpoint_context(None).source;
    changed_source.finalized_prefix_hash = [0x32; 32];
    assert_eq!(
        state.apply_checkpoint_reputation_block(
            SHARD_ID,
            changed_source,
            &ratings(),
            block,
            &config(),
        ),
        Err(PorError::ReputationBlockSourceCheckpointMismatch)
    );
    assert_eq!(state, before);
}

#[test]
fn checkpoint_application_must_extend_the_stored_tip() {
    let mut state = state();
    let first = valid_block(&state);
    state
        .apply_reputation_block(SHARD_ID, SOURCE_WAVE, &ratings(), first, &config())
        .unwrap();
    let before = state.clone();
    let source = RatingSourceCheckpoint {
        round: ROUND + 1,
        finalized_prefix_len: 32,
        finalized_prefix_hash: [0x32; 32],
    };
    let next_ratings: Vec<_> = ratings()
        .into_iter()
        .map(|mut rating| {
            rating.round = ROUND + 1;
            rating
        })
        .collect();
    let previous = cordial_por::ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let list =
        replay_reputation_transition(&previous, &next_ratings, ROUND + 1, &config()).unwrap();
    let batch = build_rating_batch(ROUND + 1, next_ratings.clone(), &config()).unwrap();
    let unlinked = build_checkpoint_reputation_block(
        CheckpointReputationBlockContext {
            shard_id: SHARD_ID,
            source,
            previous_block: None,
        },
        &batch,
        list.clone(),
        &config(),
    )
    .unwrap();
    assert_eq!(
        state.apply_checkpoint_reputation_block(
            SHARD_ID,
            source,
            &next_ratings,
            unlinked,
            &config(),
        ),
        Err(PorError::ReputationBlockPreviousHashMismatch)
    );
    assert_eq!(state, before);

    let linked = build_checkpoint_reputation_block(
        CheckpointReputationBlockContext {
            shard_id: SHARD_ID,
            source,
            previous_block: state.latest_block(),
        },
        &batch,
        list,
        &config(),
    )
    .unwrap();
    state
        .apply_checkpoint_reputation_block(
            SHARD_ID,
            source,
            &next_ratings,
            linked.clone(),
            &config(),
        )
        .unwrap();
    assert_eq!(state.latest_block(), Some(&linked));
}
