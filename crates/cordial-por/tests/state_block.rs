use cordial_miners_core::NodeId;
use cordial_por::{
    MissingEntryPolicy, PorConfig, PorError, RatingRecord, ReputationBlock, ReputationBlockContext,
    ReputationState, build_rating_batch, build_reputation_block, replay_reputation_transition,
    reputation_list_commitment, reputation_weights,
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
