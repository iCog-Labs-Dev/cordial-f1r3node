use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{
        is_weighted_supermajority, weighted_approving_creators, weighted_ratifies,
        weighted_super_ratifies,
    },
    crypto::CryptoVerifier,
};
use std::collections::{HashMap, HashSet};

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

fn block(creator_id: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
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

fn bonds(weights: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    weights
        .iter()
        .map(|(id, weight)| (node(*id), *weight))
        .collect()
}

fn all_blocks(blocklace: &Blocklace) -> HashSet<Block> {
    blocklace
        .dom()
        .into_iter()
        .filter_map(|id| blocklace.get(id))
        .collect()
}

#[test]
fn weighted_approving_creators_returns_positive_weight_supporters() {
    let mut blocklace = Blocklace::new();
    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver_2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver_3 = block(3, 3, HashSet::from([target.identity.clone()]));
    let zero_weight = block(4, 4, HashSet::from([target.identity.clone()]));
    let unknown = block(9, 9, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver_2);
    insert(&mut blocklace, &approver_3);
    insert(&mut blocklace, &zero_weight);
    insert(&mut blocklace, &unknown);

    let supporters = weighted_approving_creators(
        &blocklace,
        &all_blocks(&blocklace),
        &target.identity,
        &bonds(&[(2, 4), (3, 3), (4, 0)]),
    );

    assert_eq!(supporters, HashSet::from([node(2), node(3)]));
}

#[test]
fn weighted_ratifies_succeeds_with_strict_two_thirds_stake() {
    let mut blocklace = Blocklace::new();
    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver_2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver_3 = block(3, 3, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver_2);
    insert(&mut blocklace, &approver_3);

    let ratifier = block(
        9,
        9,
        HashSet::from([approver_2.identity.clone(), approver_3.identity.clone()]),
    );
    insert(&mut blocklace, &ratifier);

    assert!(weighted_ratifies(
        &blocklace,
        &ratifier,
        &target,
        &bonds(&[(2, 4), (3, 3), (4, 3)]),
    ));
}

#[test]
fn weighted_ratifies_fails_below_strict_two_thirds_stake() {
    let mut blocklace = Blocklace::new();
    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver_2 = block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver_2);

    let ratifier = block(9, 9, HashSet::from([approver_2.identity.clone()]));
    insert(&mut blocklace, &ratifier);

    assert!(!weighted_ratifies(
        &blocklace,
        &ratifier,
        &target,
        &bonds(&[(2, 4), (3, 3), (4, 3)]),
    ));
}

#[test]
fn weighted_super_ratifies_succeeds_with_weighted_ratifier_majority() {
    let mut blocklace = Blocklace::new();
    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver_2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver_3 = block(3, 3, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver_2);
    insert(&mut blocklace, &approver_3);

    let ratifier_2 = block(
        2,
        20,
        HashSet::from([approver_2.identity.clone(), approver_3.identity.clone()]),
    );
    let ratifier_3 = block(
        3,
        30,
        HashSet::from([approver_2.identity.clone(), approver_3.identity.clone()]),
    );
    insert(&mut blocklace, &ratifier_2);
    insert(&mut blocklace, &ratifier_3);

    let ratifiers = HashSet::from([ratifier_2, ratifier_3]);

    assert!(weighted_super_ratifies(
        &blocklace,
        &ratifiers,
        &target,
        &bonds(&[(2, 4), (3, 3), (4, 3)]),
    ));
}

#[test]
fn weighted_super_ratifies_fails_below_weighted_ratifier_majority() {
    let mut blocklace = Blocklace::new();
    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver_2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver_3 = block(3, 3, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver_2);
    insert(&mut blocklace, &approver_3);

    let ratifier_2 = block(
        2,
        20,
        HashSet::from([approver_2.identity.clone(), approver_3.identity.clone()]),
    );
    insert(&mut blocklace, &ratifier_2);

    let ratifiers = HashSet::from([ratifier_2]);

    assert!(!weighted_super_ratifies(
        &blocklace,
        &ratifiers,
        &target,
        &bonds(&[(2, 4), (3, 3), (4, 3)]),
    ));
}

#[test]
fn weighted_supermajority_is_strictly_more_than_two_thirds() {
    let weights = bonds(&[(1, 3), (2, 3), (3, 3)]);

    assert!(!is_weighted_supermajority(
        &HashSet::from([node(1), node(2)]),
        &weights,
    ));
    assert!(is_weighted_supermajority(
        &HashSet::from([node(1), node(2), node(3)]),
        &weights,
    ));
}

#[test]
fn weighted_supermajority_returns_false_for_zero_total_weight() {
    assert!(!is_weighted_supermajority(
        &HashSet::from([node(1)]),
        &bonds(&[(1, 0)]),
    ));
}
