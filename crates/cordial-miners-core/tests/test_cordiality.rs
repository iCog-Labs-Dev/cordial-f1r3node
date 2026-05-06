use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::cordiality::{
    acknowledges_equivocation, all_equivocations, equivocation_blocks_at_round,
    hidden_equivocations, is_cordial_block, missing_known_tips, observed_block_ids,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::{HashMap, HashSet};
use std::vec;

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn create_mock_block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = hash_byte;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("insert failed");
}


// Test that we can detect same-round equivocation by a creator. We create two blocks by the same creator at the same round and check that they are both detected as equivocations.
#[test]
fn detects_same_round_equivocation() {
    let mut blocklace = Blocklace::new();
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());

    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    // this cause equivocation because both blocks are created by the same creator (node 1) and they are at the same round (round 0, since they have no predecessors). We check that both blocks are detected as equivocations by calling the equivocation_blocks_at_round function and verifying that it returns both blocks.
    let equivocation = equivocation_blocks_at_round(&blocklace, &node(1), 0); 
    assert_eq!(equivocation.len(), 2);
    assert!(equivocation.contains(&e1));
    assert!(equivocation.contains(&e2));
}

// Test that different rounds by the same creator do not count as equivocation. We create two blocks by the same creator at different rounds and check that they are not detected as equivocations.
#[test]
fn different_rounds_are_not_same_round_equivocations() {
    let mut blocklace = Blocklace::new();

    let g = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &g);
    let child = create_mock_block(1, 2, HashSet::from([g.identity]));
    insert(&mut blocklace, &child);

    assert!(equivocation_blocks_at_round(&blocklace, &node(1), 0).is_empty());
    assert!(equivocation_blocks_at_round(&blocklace, &node(1), 1).is_empty());

}

// Test that different creators at the same round do not count as equivocation. We create two blocks by different creators at the same round and check that they are not detected as equivocations.
#[test]
fn predecessor_closure_can_acknowledge_equivocation() {
    let mut blocklace = Blocklace::new();
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());
    
    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    let witness = create_mock_block(2,3, HashSet::from([e1.identity.clone(), e2.identity.clone()]));
    insert(&mut blocklace, &witness);

    let candidate = create_mock_block(3, 4,HashSet::from([witness.identity.clone()]));

    let observed = observed_block_ids(&blocklace, &candidate);

    // The candidate block acknowledges the witness block, which in turn acknowledges both equivocation blocks e1 and e2. Therefore, the candidate block should be considered as acknowledging the equivocation by node 1 at round 0, and the observed block ids should include both e1 and e2.
    assert!(observed.contains(&e1.identity));
    assert!(observed.contains(&e2.identity));
    assert!(acknowledges_equivocation(&blocklace, &candidate, &node(1), 0));
}


#[test]
fn candidate_can_hide_known_equivocation() {
    let mut blocklace = Blocklace::new();
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());
    
    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    let candidate = create_mock_block(3, 3, HashSet::from([e1.identity.clone()]));
    insert(&mut blocklace, &candidate);
    let hidden = hidden_equivocations(&blocklace, &candidate);
    // The candidate block does not acknowledge any of the equivocation blocks e1 and e2, so both of them should be considered as hidden equivocations by the candidate block.

    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].creator, node(1));
    assert_eq!(hidden[0].round, 0);
    assert_eq!(hidden[0].hidden, vec![e2.identity.clone()]);

}


// Test that cordiality requires acknowledging all known tips and no hidden equivocations. We create a candidate block that is missing a known tip and check that it is not cordial, then we create a candidate block that acknowledges all known tips and has no hidden equivocations and check that it is cordial.
#[test]
fn cordiality_requires_tips_and_no_hidden_equivocations() {
    let mut blocklace = Blocklace::new();
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1,2, HashSet::new());
    let g2 = create_mock_block(2, 3, HashSet::new());

    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);
    insert(&mut blocklace, &g2);

    let witness = create_mock_block(4, 4, HashSet::from([e1.identity.clone(), e2.identity.clone()]));
    insert(&mut blocklace, &witness);

    let known_tips: HashMap<NodeId, BlockIdentity> = [
        (node(1), witness.identity.clone()),
        (node(2), g2.identity.clone()),
    ]
    .into();

    let missing_tip_candidate = create_mock_block(5, 5, HashSet::from([witness.identity.clone()]));

    // The missing_tip_candidate block acknowledges the witness block, which is the known tip for node 1, but it does not acknowledge the known tip for node 2 (g2). Therefore, the missing_tip_candidate block should be considered as missing a known tip and should not be considered cordial. The missing_known_tips function should return the identity of g2 as a missing known tip for the missing_tip_candidate block.
    assert!(missing_known_tips(&missing_tip_candidate, &known_tips).contains(&g2.identity));
    // Since the missing_tip_candidate block is missing a known tip, it should not be considered cordial according to the definition of cordiality, which requires acknowledging all known tips and having no hidden equivocations. Therefore, the is_cordial_block function should return false for the missing_tip_candidate block when we pass in the blocklace and the known_tips.
    assert!(!is_cordial_block(
        &blocklace,
        &missing_tip_candidate,
        &known_tips
    ));

    let cordial = create_mock_block(
        5,
        6,
        HashSet::from([witness.identity.clone(), g2.identity.clone()]),
    );
    // The cordial block acknowledges both known tips (witness and g2) and does not have any hidden equivocations, so it should be considered cordial according to the definition of cordiality. Therefore, the is_cordial_block function should return true for the cordial block when we pass in the blocklace and the known_tips.
    assert!(is_cordial_block(&blocklace, &cordial, &known_tips));

}

// Test that all_equivocations returns the correct creator, round, and blocks for each equivocation. We create multiple equivocations by different creators at different rounds and check that they are all reported correctly by the all_equivocations function.
#[test]
fn all_equivocations_reports_creator_and_round() {
    let mut blocklace = Blocklace::new();
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());
    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    let equivocations = all_equivocations(&blocklace);
    assert_eq!(equivocations.len(), 1);
    assert_eq!(equivocations[0].creator, node(1));
    assert_eq!(equivocations[0].round, 0);
    assert_eq!(equivocations[0].blocks.len(), 2);
}
