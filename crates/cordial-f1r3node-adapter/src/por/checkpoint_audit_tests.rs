//! Checkpoint audit fixture and immutable-report checks.

use std::collections::{BTreeSet, HashSet};

use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, ReputationState, authorized_validator_weights};
use k256::ecdsa::SigningKey;

use crate::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        activation::PorWeightActivationRecord, collector::CheckpointBlockProductionRatingCollector,
        finality::finalized_prefix_checkpoint,
        lifecycle::checkpoint::CompletedCheckpointRatingRound,
        ratings::build_finalized_checkpoint_rating_batch,
        transition::stage_completed_checkpoint_round,
    },
};

use super::checkpoint_audit::CheckpointRatingAudit;

struct AcceptAll;

impl CryptoVerifier for AcceptAll {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(seed: u8) -> NodeId {
    NodeId(
        SigningKey::from_slice(&[seed; 32])
            .unwrap()
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

#[test]
fn report_binds_complete_signed_window_and_rejects_conflicting_restart() {
    let mut blocklace = Blocklace::new();
    let mut blocks: Vec<BlockIdentity> = Vec::new();
    for tag in 1..=16u8 {
        let block = Block {
            identity: BlockIdentity {
                content_hash: [tag; 32],
                creator: node(if tag % 2 == 0 { 2 } else { 1 }),
                signature: vec![tag],
            },
            content: BlockContent {
                payload: vec![tag],
                predecessors: blocks.last().cloned().into_iter().collect::<HashSet<_>>(),
            },
        };
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
        blocks.push(block.identity);
    }
    let output = OrderedFinalizedOutput::new(blocks.clone(), blocks.last().cloned(), 3, 16, 16);
    let checkpoint = finalized_prefix_checkpoint(&output, 1).unwrap().unwrap();
    let config = PorConfig::default();
    let mut state = ReputationState::new(0);
    let validators = [node(1), node(2)];
    for validator in &validators {
        state.set_reputation(validator.clone(), config.initial_reputation);
    }
    let mut collector = CheckpointBlockProductionRatingCollector::new(
        &blocklace,
        &output,
        checkpoint.clone(),
        validators.clone(),
        &state,
        &config,
    )
    .unwrap();
    for (seed, validator) in [1u8, 2].into_iter().zip(&validators) {
        collector
            .insert_batch(
                build_finalized_checkpoint_rating_batch(
                    &blocklace,
                    &output,
                    &checkpoint,
                    validator,
                    &state,
                    &config,
                    &[seed; 32],
                )
                .unwrap(),
            )
            .unwrap();
    }
    let completed = CompletedCheckpointRatingRound::new(
        checkpoint,
        collector.build_complete_batch().unwrap(),
        BTreeSet::new(),
        &state,
    );
    let (candidate_state, _) =
        stage_completed_checkpoint_round(&completed, &state, &config, b"audit-test").unwrap();
    let weights = authorized_validator_weights(&candidate_state, &validators).unwrap();
    let record =
        PorWeightActivationRecord::from_state_and_weights(&candidate_state, &weights).unwrap();
    let authorized = validators.into_iter().collect();
    let reported_weights = weights
        .iter()
        .map(|(node, value)| (hex::encode(&node.0), *value))
        .collect();
    let report = CheckpointRatingAudit::build(
        &blocklace,
        &output,
        &completed,
        &state,
        &candidate_state,
        &config,
        &authorized,
        record.weights_commitment(),
        &reported_weights,
    )
    .unwrap();
    assert_eq!(report.window_block_hashes.len(), 16);
    assert_eq!(report.eligible_raters.len(), 2);
    assert_eq!(report.ratings.len(), 2);
    assert_eq!(
        report.checkpoint_hash,
        hex::encode(record.checkpoint_hash().unwrap())
    );

    let mut tampered_batch = completed.batch().clone();
    tampered_batch.ratings[0].signature[0] ^= 1;
    let tampered = CompletedCheckpointRatingRound::new(
        completed.checkpoint().clone(),
        tampered_batch,
        BTreeSet::new(),
        &state,
    );
    assert!(
        CheckpointRatingAudit::build(
            &blocklace,
            &output,
            &tampered,
            &state,
            &candidate_state,
            &config,
            &authorized,
            record.weights_commitment(),
            &reported_weights,
        )
        .is_err()
    );

    let directory = tempfile::tempdir().unwrap();
    let path = report.persist(directory.path()).unwrap();
    assert_eq!(report.persist(directory.path()).unwrap(), path);
    let mut changed = report.clone();
    changed.ratings[0].signature.push('0');
    assert!(
        changed
            .persist(directory.path())
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
}
