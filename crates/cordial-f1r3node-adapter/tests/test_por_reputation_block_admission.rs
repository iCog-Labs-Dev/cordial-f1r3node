use std::{cell::RefCell, collections::HashSet};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        AdmittedPorReputationBlock, CompletedPorRatingRound, DurablePorState, DurablePorStateError,
        PorFinalityTracker, PorRatingRoundCoordinator, PorRatingRoundCutoffPolicy,
        PorReputationBlockAdmissionCoordinator, PorReputationBlockAdmissionError,
        PorReputationBlockObservation, PorReputationBlockQuorumPolicy, RatingEnvelopeBroadcaster,
        ReputationBlockPublicationV1, apply_completed_reputation_round,
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, PorError, ReputationBlock, ReputationState, reputation_block_hash};
use k256::ecdsa::SigningKey;
use tempfile::tempdir;

const WAVELENGTH: u64 = 3;
const SHARD_ID: &[u8] = b"root";

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

#[derive(Default)]
struct RecordingBroadcaster {
    envelopes: RefCell<Vec<Vec<u8>>>,
}

impl RatingEnvelopeBroadcaster for RecordingBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.envelopes.borrow_mut().push(envelope.to_vec());
        Ok(())
    }
}

struct Fixture {
    blocklace: Blocklace,
    output: OrderedFinalizedOutput,
    state: ReputationState,
    config: PorConfig,
}

fn private_key(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&private_key(seed)).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn cordial_block(tag: u8, creator_seed: u8, predecessor: Option<&BlockIdentity>) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_seed),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessor.into_iter().cloned().collect::<HashSet<_>>(),
        },
    }
}

fn fixture() -> Fixture {
    let mut blocklace = Blocklace::new();
    let leader = cordial_block(1, 1, None);
    let second = cordial_block(2, 2, Some(&leader.identity));
    let third = cordial_block(3, 1, Some(&second.identity));

    for block in [&leader, &second, &third] {
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
    }

    let output = OrderedFinalizedOutput::new(
        vec![
            third.identity.clone(),
            leader.identity.clone(),
            second.identity.clone(),
        ],
        Some(leader.identity),
        WAVELENGTH,
        3,
        3,
    )
    .with_timestamp(0);

    let mut state = ReputationState::new(0);
    for (seed, reputation) in [(1, 40), (2, 30), (8, 20), (9, 10)] {
        state.set_reputation(node(seed), reputation);
    }

    Fixture {
        blocklace,
        output,
        state,
        config: PorConfig::default(),
    }
}

fn completed_round(fixture: &Fixture) -> CompletedPorRatingRound {
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&fixture.blocklace, &fixture.output)
        .unwrap()
        .unwrap();
    let mut coordinator = PorRatingRoundCoordinator::new(
        &fixture.blocklace,
        &fixture.output,
        opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();
    coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap();
    coordinator
        .broadcast_pending(&RecordingBroadcaster::default())
        .unwrap();
    coordinator
        .close_at_finalized_wave(
            &PorRatingRoundCutoffPolicy::default(),
            opened.finalized_wave + 1,
        )
        .unwrap();
    coordinator.into_completed().unwrap()
}

fn candidate_block(fixture: &Fixture, completed: &CompletedPorRatingRound) -> ReputationBlock {
    let mut state = fixture.state.clone();
    apply_completed_reputation_round(completed, &mut state, &fixture.config, SHARD_ID)
        .unwrap()
        .block
}

fn eligible_publishers() -> Vec<NodeId> {
    vec![node(1), node(2), node(8), node(9)]
}

fn publication(seed: u8, block: &ReputationBlock) -> ReputationBlockPublicationV1 {
    ReputationBlockPublicationV1::sign(node(seed), block.clone(), &private_key(seed)).unwrap()
}

fn admit(
    state: &ReputationState,
    completed: &CompletedPorRatingRound,
    config: &PorConfig,
    block: &ReputationBlock,
) -> AdmittedPorReputationBlock {
    let mut coordinator = PorReputationBlockAdmissionCoordinator::new(
        state,
        completed,
        config,
        SHARD_ID,
        eligible_publishers(),
        PorReputationBlockQuorumPolicy::default(),
    )
    .unwrap();
    coordinator.observe(publication(1, block)).unwrap();
    coordinator.observe(publication(2, block)).unwrap();
    coordinator.into_admitted().unwrap()
}

#[test]
fn policy_and_eligible_set_validation_fail_closed() {
    assert_eq!(
        PorReputationBlockQuorumPolicy::new(0, 3),
        Err(PorReputationBlockAdmissionError::InvalidThreshold {
            numerator: 0,
            denominator: 3,
        })
    );
    assert_eq!(
        PorReputationBlockQuorumPolicy::new(3, 3),
        Err(PorReputationBlockAdmissionError::InvalidThreshold {
            numerator: 3,
            denominator: 3,
        })
    );

    let fixture = fixture();
    let completed = completed_round(&fixture);
    let deduplicated = PorReputationBlockAdmissionCoordinator::new(
        &fixture.state,
        &completed,
        &fixture.config,
        SHARD_ID,
        vec![node(1), node(1)],
        PorReputationBlockQuorumPolicy::default(),
    )
    .unwrap();
    assert_eq!(deduplicated.progress().eligible_publishers, 1);
    assert_eq!(deduplicated.progress().total_eligible_weight, 40);
    assert_eq!(deduplicated.progress().required_weight, 27);
    assert!(matches!(
        PorReputationBlockAdmissionCoordinator::new(
            &fixture.state,
            &completed,
            &fixture.config,
            SHARD_ID,
            Vec::new(),
            PorReputationBlockQuorumPolicy::default(),
        ),
        Err(PorReputationBlockAdmissionError::EmptyEligiblePublisherSet)
    ));
    assert!(matches!(
        PorReputationBlockAdmissionCoordinator::new(
            &fixture.state,
            &completed,
            &fixture.config,
            SHARD_ID,
            vec![node(7)],
            PorReputationBlockQuorumPolicy::default(),
        ),
        Err(PorReputationBlockAdmissionError::UnknownEligiblePublisher(publisher))
            if publisher == node(7)
    ));

    let mut excluded = fixture.state.clone();
    excluded.eject_validator(&node(8)).unwrap();
    assert!(matches!(
        PorReputationBlockAdmissionCoordinator::new(
            &excluded,
            &completed,
            &fixture.config,
            SHARD_ID,
            vec![node(8)],
            PorReputationBlockQuorumPolicy::default(),
        ),
        Err(PorReputationBlockAdmissionError::ExcludedEligiblePublisher(publisher))
            if publisher == node(8)
    ));

    let mut zero_weight = fixture.state.clone();
    for seed in [1, 2, 8, 9] {
        zero_weight.set_reputation(node(seed), 0);
    }
    assert!(matches!(
        PorReputationBlockAdmissionCoordinator::new(
            &zero_weight,
            &completed,
            &fixture.config,
            SHARD_ID,
            eligible_publishers(),
            PorReputationBlockQuorumPolicy::default(),
        ),
        Err(PorReputationBlockAdmissionError::ZeroEligibleWeight)
    ));
}

#[test]
fn distinct_audited_publishers_reach_strict_weighted_quorum() {
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let block = candidate_block(&fixture, &completed);
    let expected_hash = reputation_block_hash(&block).unwrap();
    let mut coordinator = PorReputationBlockAdmissionCoordinator::new(
        &fixture.state,
        &completed,
        &fixture.config,
        SHARD_ID,
        eligible_publishers(),
        PorReputationBlockQuorumPolicy::default(),
    )
    .unwrap();

    assert_eq!(coordinator.progress().total_eligible_weight, 100);
    assert_eq!(coordinator.progress().required_weight, 67);
    assert_eq!(
        coordinator.observe(publication(1, &block)),
        Ok(PorReputationBlockObservation::Counted)
    );
    assert_eq!(coordinator.progress().signed_weight, 40);
    assert!(!coordinator.progress().is_reached());

    assert_eq!(
        coordinator.observe(publication(1, &block)),
        Ok(PorReputationBlockObservation::Duplicate)
    );
    assert_eq!(coordinator.progress().signed_weight, 40);
    assert_eq!(
        coordinator.observe(publication(2, &block)),
        Ok(PorReputationBlockObservation::Counted)
    );
    assert!(coordinator.progress().is_reached());
    assert_eq!(coordinator.progress().signed_weight, 70);
    assert_eq!(
        coordinator.observe(publication(8, &block)),
        Err(PorReputationBlockAdmissionError::QuorumAlreadyReached)
    );

    let admitted = coordinator.into_admitted().unwrap();
    assert_eq!(admitted.block(), &block);
    assert_eq!(admitted.block_hash(), expected_hash);
    assert_eq!(admitted.publications().len(), 2);
    assert_eq!(admitted.progress().required_weight, 67);
    let mut expected_publishers = vec![node(1), node(2)];
    expected_publishers.sort();
    assert_eq!(
        admitted.publishers().cloned().collect::<Vec<_>>(),
        expected_publishers
    );
}

#[test]
fn coordinator_cannot_emit_a_certificate_below_quorum() {
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let block = candidate_block(&fixture, &completed);
    let mut coordinator = PorReputationBlockAdmissionCoordinator::new(
        &fixture.state,
        &completed,
        &fixture.config,
        SHARD_ID,
        eligible_publishers(),
        PorReputationBlockQuorumPolicy::default(),
    )
    .unwrap();
    coordinator.observe(publication(1, &block)).unwrap();

    assert_eq!(
        coordinator.into_admitted(),
        Err(PorReputationBlockAdmissionError::QuorumNotReached {
            signed_weight: 40,
            required_weight: 67,
        })
    );
}

#[test]
fn unexpected_and_invalid_publications_do_not_change_progress() {
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let block = candidate_block(&fixture, &completed);
    let mut coordinator = PorReputationBlockAdmissionCoordinator::new(
        &fixture.state,
        &completed,
        &fixture.config,
        SHARD_ID,
        eligible_publishers(),
        PorReputationBlockQuorumPolicy::default(),
    )
    .unwrap();

    assert!(matches!(
        coordinator.observe(publication(7, &block)),
        Err(PorReputationBlockAdmissionError::UnexpectedPublisher(publisher))
            if publisher == node(7)
    ));
    assert_eq!(coordinator.progress().signed_weight, 0);
    assert_eq!(coordinator.progress().candidate_hash, None);

    let mut invalid = block.clone();
    invalid.header.ratings_hash[0] ^= 1;
    assert_eq!(
        coordinator.observe(publication(1, &invalid)),
        Err(PorReputationBlockAdmissionError::Audit(
            PorError::ReputationBlockRatingsHashMismatch
        ))
    );
    assert_eq!(coordinator.progress().signed_weight, 0);
    assert_eq!(coordinator.progress().candidate_hash, None);

    assert_eq!(
        coordinator.observe(publication(1, &block)),
        Ok(PorReputationBlockObservation::Counted)
    );
    assert_eq!(coordinator.progress().signed_weight, 40);

    assert_eq!(
        coordinator.observe(publication(2, &invalid)),
        Err(PorReputationBlockAdmissionError::Audit(
            PorError::ReputationBlockRatingsHashMismatch
        ))
    );
    assert_eq!(coordinator.progress().signed_weight, 40);
    assert_eq!(coordinator.progress().publishers, vec![node(1)]);
}

#[test]
fn a_publishers_conflicting_signature_is_reported_without_replacing_its_vote() {
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let block = candidate_block(&fixture, &completed);
    let first_hash = reputation_block_hash(&block).unwrap();
    let mut conflicting = block.clone();
    conflicting.header.ratings_hash[0] ^= 1;
    let conflicting_hash = reputation_block_hash(&conflicting).unwrap();
    let mut coordinator = PorReputationBlockAdmissionCoordinator::new(
        &fixture.state,
        &completed,
        &fixture.config,
        SHARD_ID,
        eligible_publishers(),
        PorReputationBlockQuorumPolicy::default(),
    )
    .unwrap();

    let first_publication = publication(1, &block);
    let conflicting_publication = publication(1, &conflicting);
    coordinator.observe(first_publication.clone()).unwrap();
    let error = coordinator
        .observe(conflicting_publication.clone())
        .unwrap_err();
    let PorReputationBlockAdmissionError::ConflictingPublication(evidence) = error else {
        panic!("expected signed conflict evidence");
    };
    assert_eq!(evidence.publisher(), &node(1));
    assert_eq!(evidence.first_hash(), first_hash);
    assert_eq!(evidence.conflicting_hash(), conflicting_hash);
    assert_eq!(evidence.first_publication(), &first_publication);
    assert_eq!(evidence.conflicting_publication(), &conflicting_publication);
    assert_eq!(coordinator.progress().signed_weight, 40);
    assert_eq!(coordinator.progress().candidate_hash, Some(first_hash));
    assert_eq!(coordinator.progress().publishers, vec![node(1)]);
}

#[test]
fn admitted_block_is_reaudited_and_durably_recovered() {
    let directory = tempdir().unwrap();
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let block = candidate_block(&fixture, &completed);
    let admitted = admit(&fixture.state, &completed, &fixture.config, &block);
    let initial = fixture.state.clone();
    let mut runtime = DurablePorState::open(directory.path(), initial.clone()).unwrap();

    assert!(matches!(
        runtime.apply_admitted_block(&admitted, &completed, &fixture.config, b"other"),
        Err(DurablePorStateError::Transition(
            PorError::ReputationBlockShardMismatch
        ))
    ));
    assert_eq!(runtime.state().unwrap(), &initial);
    assert!(runtime.history().is_empty().unwrap());

    let applied = runtime
        .apply_admitted_block(&admitted, &completed, &fixture.config, SHARD_ID)
        .unwrap();
    let committed = runtime.state().unwrap().clone();
    assert_eq!(applied.block, block);
    assert_eq!(committed.latest_block(), Some(&applied.block));
    assert_eq!(runtime.history().latest().unwrap(), Some(applied.block));
    drop(runtime);

    let reopened = DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
    assert_eq!(reopened.state().unwrap(), &committed);
}
