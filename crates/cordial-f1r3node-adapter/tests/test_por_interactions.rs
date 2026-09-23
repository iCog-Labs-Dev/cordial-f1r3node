use std::collections::HashSet;

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_interactions::{PorInteractionError, extract_block_production_evidence},
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{InteractionKind, ReputationState, admit_interaction_evidence};

const WAVELENGTH: u64 = 3;

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

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn block(tag: u8, creator: u8, predecessor: Option<&BlockIdentity>) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessor.into_iter().cloned().collect::<HashSet<_>>(),
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace
        .insert(block.clone(), &AcceptAll)
        .expect("test block should be accepted");
}

fn output(blocks: &[&Block], anchor: Option<&Block>, wavelength: u64) -> OrderedFinalizedOutput {
    OrderedFinalizedOutput::new(
        blocks.iter().map(|block| block.identity.clone()).collect(),
        anchor.map(|block| block.identity.clone()),
        wavelength,
        3,
        blocks.len(),
    )
    .with_timestamp(0)
}

fn wave_zero_chain() -> (Blocklace, Block, Block, Block) {
    let mut blocklace = Blocklace::new();
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 1, Some(&second.identity));

    for block in [&leader, &second, &third] {
        insert(&mut blocklace, block);
    }

    (blocklace, leader, second, third)
}

#[test]
fn extracts_one_canonical_block_production_interaction_per_recipient() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let output = output(&[&third, &second, &leader], Some(&leader), WAVELENGTH);
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&blocklace, &output)
        .unwrap()
        .unwrap();

    let evidence =
        extract_block_production_evidence(&blocklace, &output, opened, &node(9)).unwrap();

    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0].finalized_wave, 0);
    assert_eq!(evidence[0].round, 1);
    assert_eq!(evidence[0].kind, InteractionKind::BlockProduction);
    assert_eq!(evidence[0].rater, node(9));
    assert_eq!(evidence[0].recipient, node(1));
    assert_eq!(
        evidence[0].evidence_ref,
        leader.identity.content_hash.to_vec()
    );
    assert_eq!(evidence[1].recipient, node(2));
    assert_eq!(
        evidence[1].evidence_ref,
        second.identity.content_hash.to_vec()
    );
}

#[test]
fn extraction_is_independent_of_finalized_output_order() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let ordered = output(&[&leader, &second, &third], Some(&leader), WAVELENGTH);
    let shuffled = output(&[&third, &leader, &second], Some(&leader), WAVELENGTH);
    let opened = FinalizedRatingRound {
        finalized_wave: 0,
        rating_round: 1,
    };

    let expected =
        extract_block_production_evidence(&blocklace, &ordered, opened, &node(9)).unwrap();
    let actual =
        extract_block_production_evidence(&blocklace, &shuffled, opened, &node(9)).unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn skips_self_interactions() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let output = output(&[&leader, &second, &third], Some(&leader), WAVELENGTH);
    let opened = FinalizedRatingRound {
        finalized_wave: 0,
        rating_round: 1,
    };

    let evidence =
        extract_block_production_evidence(&blocklace, &output, opened, &node(1)).unwrap();

    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].recipient, node(2));
}

#[test]
fn cumulative_output_only_produces_evidence_for_the_opened_wave() {
    let (mut blocklace, leader, second, third) = wave_zero_chain();
    let next_leader = block(4, 3, Some(&third.identity));
    insert(&mut blocklace, &next_leader);
    let output = output(
        &[&leader, &second, &third, &next_leader],
        Some(&next_leader),
        WAVELENGTH,
    );
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&blocklace, &output)
        .unwrap()
        .unwrap();

    let evidence =
        extract_block_production_evidence(&blocklace, &output, opened, &node(9)).unwrap();

    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].finalized_wave, 1);
    assert_eq!(evidence[0].round, 2);
    assert_eq!(evidence[0].recipient, node(3));
    assert_eq!(
        evidence[0].evidence_ref,
        next_leader.identity.content_hash.to_vec()
    );
}

#[test]
fn extracted_evidence_passes_por_admission_for_known_active_validators() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let output = output(&[&leader, &second, &third], Some(&leader), WAVELENGTH);
    let opened = FinalizedRatingRound {
        finalized_wave: 0,
        rating_round: 1,
    };
    let mut state = ReputationState::new(0);
    state.set_reputation(node(1), 100);
    state.set_reputation(node(2), 100);
    state.set_reputation(node(9), 100);

    let evidence =
        extract_block_production_evidence(&blocklace, &output, opened, &node(9)).unwrap();

    for interaction in evidence {
        assert!(admit_interaction_evidence(interaction, &state).is_ok());
    }
}

#[test]
fn rejects_an_opened_round_from_another_finalized_wave() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let output = output(&[&leader, &second, &third], Some(&leader), WAVELENGTH);
    let mismatched = FinalizedRatingRound {
        finalized_wave: 1,
        rating_round: 2,
    };

    assert_eq!(
        extract_block_production_evidence(&blocklace, &output, mismatched, &node(9)),
        Err(PorInteractionError::FinalizedRoundMismatch)
    );
}

#[test]
fn rejects_output_without_a_final_leader() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let output = output(&[&leader, &second, &third], None, WAVELENGTH);

    assert_eq!(
        extract_block_production_evidence(
            &blocklace,
            &output,
            FinalizedRatingRound {
                finalized_wave: 0,
                rating_round: 1,
            },
            &node(9),
        ),
        Err(PorInteractionError::MissingFinalLeader)
    );
}

#[test]
fn rejects_finalized_output_block_missing_from_the_blocklace() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let missing = block(99, 4, Some(&third.identity));
    let output = output(
        &[&leader, &second, &third, &missing],
        Some(&leader),
        WAVELENGTH,
    );

    assert_eq!(
        extract_block_production_evidence(
            &blocklace,
            &output,
            FinalizedRatingRound {
                finalized_wave: 0,
                rating_round: 1,
            },
            &node(9),
        ),
        Err(PorInteractionError::UnknownFinalizedBlock)
    );
}

#[test]
fn rejects_zero_wavelength() {
    let (blocklace, leader, second, third) = wave_zero_chain();
    let output = output(&[&leader, &second, &third], Some(&leader), 0);

    assert_eq!(
        extract_block_production_evidence(
            &blocklace,
            &output,
            FinalizedRatingRound {
                finalized_wave: 0,
                rating_round: 1,
            },
            &node(9),
        ),
        Err(PorInteractionError::InvalidWavelength)
    );
}
