use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    WeightedEpochBoundary, WeightedEpochError, weighted_final_leader_for_wave, weighted_tau,
    weighted_tau_after_epoch_boundary, weighted_tau_at_leader,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
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

fn block(creator: u8, hash: u8, predecessors: &[&Block]) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = creator;
    content_hash[1] = hash;
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: Vec::new(),
        },
        content: BlockContent {
            payload: Vec::new(),
            predecessors: predecessors
                .iter()
                .map(|predecessor| predecessor.identity.clone())
                .collect::<HashSet<_>>(),
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace.insert(block.clone(), &MockVerifier).unwrap();
}

fn weights(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, weight)| (node(*id), *weight))
        .collect()
}

fn leader(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

#[test]
fn future_weight_epoch_keeps_old_finality_and_changes_a_later_wave() {
    let mut lace = Blocklace::new();
    let old = weights(&[(1, 1), (2, 3), (3, 3), (4, 3), (5, 1)]);
    let new = weights(&[(1, 1), (2, 1), (3, 1), (4, 1), (5, 8)]);

    // Only validators 2-4 ratify wave 0. Their old weight finalizes its
    // leader; their new weight does not.
    let w0 = block(1, 1, &[]);
    insert(&mut lace, &w0);
    let r1_2 = block(2, 2, &[&w0]);
    let r1_3 = block(3, 3, &[&w0]);
    let r1_4 = block(4, 4, &[&w0]);
    for block in [&r1_2, &r1_3, &r1_4] {
        insert(&mut lace, block);
    }
    let r2_2 = block(2, 5, &[&r1_2, &r1_3, &r1_4]);
    let r2_3 = block(3, 6, &[&r1_2, &r1_3, &r1_4]);
    let r2_4 = block(4, 7, &[&r1_2, &r1_3, &r1_4]);
    for block in [&r2_2, &r2_3, &r2_4] {
        insert(&mut lace, block);
    }
    assert_eq!(
        weighted_final_leader_for_wave(&lace, 0, 3, &old, leader),
        Some(w0.identity.clone())
    );
    assert_eq!(
        weighted_final_leader_for_wave(&lace, 0, 3, &new, leader),
        None
    );
    let sealed = weighted_tau_at_leader(&lace, &w0.identity, 3, &old, leader).unwrap();

    let w1 = block(1, 8, &[&r2_2, &r2_3, &r2_4]);
    insert(&mut lace, &w1);
    let boundary = || WeightedEpochBoundary {
        final_leader: &w0.identity,
        ordered_prefix: &sealed,
        effective_wave: 1,
    };
    let (pending, anchor) =
        weighted_tau_after_epoch_boundary(&lace, 3, &old, &new, boundary(), leader).unwrap();
    assert_eq!(pending, sealed);
    assert_eq!(anchor, w0.identity);

    // Validators 2 and 5 now carry enough new weight to finalize wave 1.
    // Their old weight cannot finalize that wave.
    let w1_r1_2 = block(2, 9, &[&w1]);
    let w1_r1_5 = block(5, 10, &[&w1]);
    insert(&mut lace, &w1_r1_2);
    insert(&mut lace, &w1_r1_5);
    let w1_r2_2 = block(2, 11, &[&w1_r1_2, &w1_r1_5]);
    let w1_r2_5 = block(5, 12, &[&w1_r1_2, &w1_r1_5]);
    insert(&mut lace, &w1_r2_2);
    insert(&mut lace, &w1_r2_5);
    assert_eq!(
        weighted_final_leader_for_wave(&lace, 1, 3, &old, leader),
        None
    );
    assert_eq!(
        weighted_final_leader_for_wave(&lace, 1, 3, &new, leader),
        Some(w1.identity.clone())
    );

    let (ordered, anchor) =
        weighted_tau_after_epoch_boundary(&lace, 3, &old, &new, boundary(), leader).unwrap();
    assert_eq!(anchor, w1.identity);
    assert!(ordered.starts_with(&sealed));
    assert!(ordered.len() > sealed.len());
    assert!(ordered.contains(&w1.identity));
    assert_eq!(
        weighted_tau_after_epoch_boundary(&lace, 3, &old, &new, boundary(), leader).unwrap(),
        (ordered, anchor),
        "replay must recover the same output"
    );
    assert_eq!(
        weighted_tau(&lace, 3, &old, leader).unwrap(),
        sealed,
        "the old table cannot finalize wave 1"
    );
}

#[test]
fn epoch_boundary_rejects_a_changed_prefix_or_past_effective_wave() {
    let mut lace = Blocklace::new();
    let old = weights(&[(1, 1)]);
    let w0 = block(1, 1, &[]);
    insert(&mut lace, &w0);
    let w0_r1 = block(1, 2, &[&w0]);
    insert(&mut lace, &w0_r1);
    let w0_r2 = block(1, 3, &[&w0_r1]);
    insert(&mut lace, &w0_r2);
    let sealed = weighted_tau_at_leader(&lace, &w0.identity, 3, &old, leader).unwrap();
    assert!(!sealed.is_empty());

    assert_eq!(
        weighted_tau_after_epoch_boundary(
            &lace,
            3,
            &old,
            &old,
            WeightedEpochBoundary {
                final_leader: &w0.identity,
                ordered_prefix: &[],
                effective_wave: 1,
            },
            leader,
        ),
        Err(WeightedEpochError::PrefixMismatch)
    );
    assert_eq!(
        weighted_tau_after_epoch_boundary(
            &lace,
            3,
            &old,
            &old,
            WeightedEpochBoundary {
                final_leader: &w0.identity,
                ordered_prefix: &sealed,
                effective_wave: 0,
            },
            leader,
        ),
        Err(WeightedEpochError::InvalidBoundary)
    );
    assert_eq!(
        weighted_tau_after_epoch_boundary(
            &lace,
            3,
            &old,
            &weights(&[(2, 1)]),
            WeightedEpochBoundary {
                final_leader: &w0.identity,
                ordered_prefix: &sealed,
                effective_wave: 1,
            },
            leader,
        ),
        Err(WeightedEpochError::ValidatorSetMismatch)
    );
}
