use std::collections::HashSet;

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityError, PorFinalityTracker},
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};

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

fn block(tag: u8, predecessor: Option<&BlockIdentity>) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: NodeId(vec![tag]),
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

fn output(anchor: Option<BlockIdentity>, wavelength: u64) -> OrderedFinalizedOutput {
    OrderedFinalizedOutput::new(Vec::new(), anchor, wavelength, 1, 0).with_timestamp(0)
}

#[test]
fn finalized_leader_opens_the_following_rating_round() {
    let mut blocklace = Blocklace::new();
    let leader = block(1, None);
    insert(&mut blocklace, &leader);

    let mut tracker = PorFinalityTracker::new();
    let opened = tracker
        .observe_finalized_output(
            &blocklace,
            &output(Some(leader.identity.clone()), WAVELENGTH),
        )
        .unwrap();

    assert_eq!(
        opened,
        Some(FinalizedRatingRound {
            finalized_wave: 0,
            rating_round: 1,
        })
    );
    assert_eq!(tracker.last_opened_wave(), Some(0));
}

#[test]
fn repeated_finalized_output_is_idempotent() {
    let mut blocklace = Blocklace::new();
    let leader = block(1, None);
    insert(&mut blocklace, &leader);
    let output = output(Some(leader.identity.clone()), WAVELENGTH);
    let mut tracker = PorFinalityTracker::new();

    assert!(
        tracker
            .observe_finalized_output(&blocklace, &output)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        tracker
            .observe_finalized_output(&blocklace, &output)
            .unwrap(),
        None
    );
}

#[test]
fn newer_finalized_leader_uses_its_actual_blocklace_wave() {
    let mut blocklace = Blocklace::new();
    let leader_0 = block(1, None);
    let round_1 = block(2, Some(&leader_0.identity));
    let round_2 = block(3, Some(&round_1.identity));
    let leader_1 = block(4, Some(&round_2.identity));

    for block in [&leader_0, &round_1, &round_2, &leader_1] {
        insert(&mut blocklace, block);
    }

    let mut tracker = PorFinalityTracker::new();
    tracker
        .observe_finalized_output(
            &blocklace,
            &output(Some(leader_0.identity.clone()), WAVELENGTH),
        )
        .unwrap();

    assert_eq!(
        tracker
            .observe_finalized_output(
                &blocklace,
                &output(Some(leader_1.identity.clone()), WAVELENGTH),
            )
            .unwrap(),
        Some(FinalizedRatingRound {
            finalized_wave: 1,
            rating_round: 2,
        })
    );
}

#[test]
fn output_without_a_final_leader_does_not_open_a_round() {
    let mut tracker = PorFinalityTracker::new();

    assert_eq!(
        tracker
            .observe_finalized_output(&Blocklace::new(), &output(None, WAVELENGTH))
            .unwrap(),
        None
    );
}

#[test]
fn rejects_conflicting_final_leaders_for_the_same_wave() {
    let mut blocklace = Blocklace::new();
    let leader_a = block(1, None);
    let leader_b = block(2, None);
    insert(&mut blocklace, &leader_a);
    insert(&mut blocklace, &leader_b);

    let mut tracker = PorFinalityTracker::new();
    tracker
        .observe_finalized_output(
            &blocklace,
            &output(Some(leader_a.identity.clone()), WAVELENGTH),
        )
        .unwrap();

    assert_eq!(
        tracker.observe_finalized_output(
            &blocklace,
            &output(Some(leader_b.identity.clone()), WAVELENGTH),
        ),
        Err(PorFinalityError::ConflictingFinalLeader { wave: 0 })
    );
}

#[test]
fn rejects_an_anchor_missing_from_the_blocklace() {
    let missing = block(1, None);
    let mut tracker = PorFinalityTracker::new();

    assert_eq!(
        tracker.observe_finalized_output(
            &Blocklace::new(),
            &output(Some(missing.identity), WAVELENGTH),
        ),
        Err(PorFinalityError::UnknownFinalLeader)
    );
}

#[test]
fn rejects_zero_wavelength_for_a_finalized_anchor() {
    let mut blocklace = Blocklace::new();
    let leader = block(1, None);
    insert(&mut blocklace, &leader);
    let mut tracker = PorFinalityTracker::new();

    assert_eq!(
        tracker.observe_finalized_output(&blocklace, &output(Some(leader.identity.clone()), 0),),
        Err(PorFinalityError::InvalidWavelength)
    );
}
