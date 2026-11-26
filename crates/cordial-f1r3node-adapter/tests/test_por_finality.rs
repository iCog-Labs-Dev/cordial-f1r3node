use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{
        FinalizedRatingRound, PorFinalityError, PorFinalityTracker, finalized_output_for_wave,
        finalized_prefix_checkpoint, finalized_wave_from_output,
        validate_finalized_prefix_checkpoint,
    },
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

fn prefix_output(count: u8) -> OrderedFinalizedOutput {
    let blocks: Vec<_> = (1..=count).map(|tag| block(tag, None).identity).collect();
    let anchor = blocks.last().cloned();
    OrderedFinalizedOutput::new(blocks, anchor, WAVELENGTH, 4, usize::from(count)).with_timestamp(0)
}

#[test]
fn fixed_prefix_checkpoints_do_not_depend_on_poll_frequency() {
    let early = prefix_output(17);
    let late = prefix_output(35);
    let first = finalized_prefix_checkpoint(&early, 1).unwrap().unwrap();

    assert_eq!(first.finalized_prefix_len, 16);
    assert_eq!(first.terminal_block, early.blocks[15]);
    assert_eq!(first.rating_source().round, 1);
    assert_eq!(first.rating_source().finalized_prefix_len, 16);
    assert_eq!(
        first.rating_source().finalized_prefix_hash,
        first.prefix_hash
    );
    assert_eq!(
        finalized_prefix_checkpoint(&late, 1).unwrap(),
        Some(first.clone())
    );
    assert_eq!(finalized_prefix_checkpoint(&early, 2).unwrap(), None);
    assert_eq!(
        finalized_prefix_checkpoint(&late, 2)
            .unwrap()
            .unwrap()
            .terminal_block,
        late.blocks[31]
    );
    assert_eq!(finalized_prefix_checkpoint(&late, 3).unwrap(), None);
    validate_finalized_prefix_checkpoint(&late, &first).unwrap();
}

#[test]
fn checkpoint_replay_rejects_changed_or_missing_prefix() {
    let original = prefix_output(16);
    let checkpoint = finalized_prefix_checkpoint(&original, 1).unwrap().unwrap();
    let mut changed = prefix_output(17);
    changed.blocks[7] = block(99, None).identity;

    assert_eq!(
        validate_finalized_prefix_checkpoint(&changed, &checkpoint),
        Err(PorFinalityError::PrefixCheckpointMismatch)
    );
    assert_eq!(
        validate_finalized_prefix_checkpoint(&prefix_output(15), &checkpoint),
        Err(PorFinalityError::PrefixCheckpointMismatch)
    );
    assert_eq!(
        finalized_prefix_checkpoint(&original, 0),
        Err(PorFinalityError::InvalidPrefixCheckpointRound)
    );
    assert_eq!(
        finalized_prefix_checkpoint(&original, u64::MAX),
        Err(PorFinalityError::InvalidPrefixCheckpointRound)
    );
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
        vec![FinalizedRatingRound {
            finalized_wave: 0,
            rating_round: 1,
        }]
    );
    assert_eq!(tracker.last_opened_wave(), Some(0));
}

#[test]
fn finalized_wave_can_be_reported_without_opening_a_rating_round() {
    let mut blocklace = Blocklace::new();
    let leader = block(1, None);
    insert(&mut blocklace, &leader);
    let finalized = output(Some(leader.identity), WAVELENGTH);

    assert_eq!(
        finalized_wave_from_output(&blocklace, &finalized),
        Ok(Some(0))
    );
    assert_eq!(
        finalized_wave_from_output(&blocklace, &output(None, WAVELENGTH)),
        Ok(None)
    );
    assert_eq!(PorFinalityTracker::new().last_opened_wave(), None);
}

#[test]
fn repeated_finalized_output_is_idempotent() {
    let mut blocklace = Blocklace::new();
    let leader = block(1, None);
    insert(&mut blocklace, &leader);
    let output = output(Some(leader.identity.clone()), WAVELENGTH);
    let mut tracker = PorFinalityTracker::new();

    assert!(
        !tracker
            .observe_finalized_output(&blocklace, &output)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        tracker
            .observe_finalized_output(&blocklace, &output)
            .unwrap(),
        vec![]
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
        vec![FinalizedRatingRound {
            finalized_wave: 1,
            rating_round: 2,
        }]
    );
}

#[test]
fn output_without_a_final_leader_does_not_open_a_round() {
    let mut tracker = PorFinalityTracker::new();

    assert_eq!(
        tracker
            .observe_finalized_output(&Blocklace::new(), &output(None, WAVELENGTH))
            .unwrap(),
        vec![]
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

#[test]
fn skipped_finalized_wave_yields_gap_rounds_and_real_round() {
    // Build a chain of 3 waves with wavelength 3:
    //   wave 0: leader_a at depth 0
    //   wave 1: leader_b at depth 3
    //   wave 2: leader_c at depth 6
    let mut blocklace = Blocklace::new();

    let leader_a = block(1, None);
    let round_a1 = block(2, Some(&leader_a.identity));
    let round_a2 = block(3, Some(&round_a1.identity));

    let leader_b = block(4, Some(&round_a2.identity));
    let round_b1 = block(5, Some(&leader_b.identity));
    let round_b2 = block(6, Some(&round_b1.identity));

    let leader_c = block(7, Some(&round_b2.identity));

    for b in [
        &leader_a, &round_a1, &round_a2, &leader_b, &round_b1, &round_b2, &leader_c,
    ] {
        insert(&mut blocklace, b);
    }

    let mut tracker = PorFinalityTracker::new();

    // Observe wave 0 normally → one round returned
    let rounds_0 = tracker
        .observe_finalized_output(
            &blocklace,
            &output(Some(leader_a.identity.clone()), WAVELENGTH),
        )
        .unwrap();
    assert_eq!(
        rounds_0,
        vec![FinalizedRatingRound {
            finalized_wave: 0,
            rating_round: 1,
        }]
    );

    // Skip wave 1. Observe wave 2 directly → should return 2 rounds:
    //   [0] synthetic gap for wave 1 (round 2)
    //   [1] real round for wave 2 (round 3)
    let rounds_2 = tracker
        .observe_finalized_output(
            &blocklace,
            &output(Some(leader_c.identity.clone()), WAVELENGTH),
        )
        .unwrap();
    assert_eq!(
        rounds_2,
        vec![
            FinalizedRatingRound {
                finalized_wave: 1,
                rating_round: 2,
            },
            FinalizedRatingRound {
                finalized_wave: 2,
                rating_round: 3,
            },
        ]
    );
    assert_eq!(tracker.last_opened_wave(), Some(2));
}

#[test]
fn replays_an_earlier_wave_only_when_its_ordering_is_a_finalized_prefix() {
    let mut blocklace = Blocklace::new();
    let mut chain = vec![block(1, None)];
    for tag in 2..=7 {
        chain.push(block(tag, Some(&chain.last().unwrap().identity)));
    }
    for entry in &chain {
        insert(&mut blocklace, entry);
    }

    let bonds = HashMap::from([(NodeId(vec![1]), 1)]);
    let latest = OrderedFinalizedOutput::new(
        chain[..4]
            .iter()
            .map(|entry| entry.identity.clone())
            .collect(),
        Some(chain[3].identity.clone()),
        WAVELENGTH,
        1,
        chain.len(),
    )
    .with_timestamp(0);
    let replayed = finalized_output_for_wave(&blocklace, &latest, &bonds, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        replayed.anchor,
        Some(chain[0].identity.consensus_identity())
    );
    assert_eq!(
        replayed.blocks,
        vec![chain[0].identity.consensus_identity()]
    );
    assert_eq!(
        finalized_output_for_wave(&blocklace, &latest, &bonds, 1).unwrap(),
        Some(latest.clone())
    );
    assert_eq!(
        finalized_output_for_wave(&blocklace, &latest, &bonds, 2).unwrap(),
        None
    );

    let mut drifted = latest;
    drifted.blocks.remove(0);
    assert_eq!(
        finalized_output_for_wave(&blocklace, &drifted, &bonds, 0),
        Err(PorFinalityError::HistoricalPrefixMismatch { wave: 0 })
    );
}

#[test]
fn later_finality_cannot_supply_a_missing_wave_zero_leader() {
    let mut blocklace = Blocklace::new();
    let mut genesis = block(0, None);
    genesis.identity.creator = NodeId(Vec::new());
    let first = block(1, Some(&genesis.identity));
    let second = block(2, Some(&first.identity));
    let later_leader = block(4, Some(&second.identity));
    for entry in [&genesis, &first, &second, &later_leader] {
        insert(&mut blocklace, entry);
    }
    let latest = OrderedFinalizedOutput::new(
        vec![genesis.identity.clone(), first.identity, second.identity],
        Some(later_leader.identity),
        WAVELENGTH,
        1,
        4,
    );
    let bonds = HashMap::from([(NodeId(vec![4]), 1)]);
    assert_eq!(
        finalized_output_for_wave(&blocklace, &latest, &bonds, 0),
        Err(PorFinalityError::MissingFinalLeaderForWave { wave: 0 })
    );
}
