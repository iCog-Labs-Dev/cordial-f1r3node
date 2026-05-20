use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{CheckpointGc, checkpoint_after_finality, tau};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

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

fn make_id(creator: &NodeId, tag: u64) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    hash[8] = creator.0[0];

    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: tag.to_le_bytes().to_vec(),
    }
}

fn genesis(creator: &NodeId, tag: u64) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors: HashSet::new(),
        },
    }
}

fn child(creator: &NodeId, tag: u64, parents: &[&Block]) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors: parents.iter().map(|block| block.identity.clone()).collect(),
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace
        .insert(block.clone(), &MockVerifier)
        .expect("test block should insert");
}

fn leader_node1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

#[test]
fn checkpoint_prune_removes_old_blocks_and_observe_stops_at_boundary() {
    let mut blocklace = Blocklace::new();

    let genesis = genesis(&node(1), 1);
    let parent = child(&node(2), 2, &[&genesis]);
    let checkpoint = child(&node(1), 3, &[&parent]);
    let tip = child(&node(2), 4, &[&checkpoint]);

    for block in [&genesis, &parent, &checkpoint, &tip] {
        insert(&mut blocklace, block);
    }

    assert!(blocklace.observe(&tip.identity).contains(&genesis.identity));

    let report = blocklace
        .prune_below_checkpoint(&checkpoint.identity)
        .expect("checkpoint should prune");

    assert!(report.removed.contains(&genesis.identity));
    assert!(report.removed.contains(&parent.identity));
    assert_eq!(blocklace.current_checkpoint(), Some(&checkpoint.identity));
    assert!(blocklace.get(&genesis.identity).is_none());
    assert!(blocklace.get(&parent.identity).is_none());
    assert!(blocklace.get(&checkpoint.identity).is_some());
    assert!(blocklace.get(&tip.identity).is_some());
    assert!(blocklace.is_closed());

    let observed = blocklace.observe(&tip.identity);
    assert!(observed.contains(&tip.identity));
    assert!(observed.contains(&checkpoint.identity));
    assert!(!observed.contains(&parent.identity));
    assert!(!observed.contains(&genesis.identity));
    assert!(blocklace.observe(&genesis.identity).is_empty());
}

#[test]
fn checkpoint_after_finality_prunes_latest_final_leader_history() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3;
    let n = 4;
    let f = 1;

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);
    let v4 = node(4);

    let w0_leader = genesis(&v1, 1);
    insert(&mut blocklace, &w0_leader);

    let w0_r1_v2 = child(&v2, 2, &[&w0_leader]);
    let w0_r1_v3 = child(&v3, 3, &[&w0_leader]);
    let w0_r1_v4 = child(&v4, 4, &[&w0_leader]);
    for block in [&w0_r1_v2, &w0_r1_v3, &w0_r1_v4] {
        insert(&mut blocklace, block);
    }

    let w0_r2_v2 = child(&v2, 5, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v3 = child(&v3, 6, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v4 = child(&v4, 7, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    for block in [&w0_r2_v2, &w0_r2_v3, &w0_r2_v4] {
        insert(&mut blocklace, block);
    }

    let w1_leader = child(&v1, 8, &[&w0_r2_v2, &w0_r2_v3, &w0_r2_v4]);
    insert(&mut blocklace, &w1_leader);

    let w1_r1_v2 = child(&v2, 9, &[&w1_leader]);
    let w1_r1_v3 = child(&v3, 10, &[&w1_leader]);
    let w1_r1_v4 = child(&v4, 11, &[&w1_leader]);
    for block in [&w1_r1_v2, &w1_r1_v3, &w1_r1_v4] {
        insert(&mut blocklace, block);
    }

    let w1_r2_v2 = child(&v2, 12, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    let w1_r2_v3 = child(&v3, 13, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    let w1_r2_v4 = child(&v4, 14, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    for block in [&w1_r2_v2, &w1_r2_v3, &w1_r2_v4] {
        insert(&mut blocklace, block);
    }

    let before = tau(&blocklace, wavelength, n, f, leader_node1).expect("tau should order");
    let report = checkpoint_after_finality(&mut blocklace, wavelength, n, f, leader_node1)
        .expect("finality checkpoint should not error")
        .expect("latest final leader should prune");
    let after = tau(&blocklace, wavelength, n, f, leader_node1).expect("tau should order");

    assert_eq!(report.checkpoint, w1_leader.identity);
    assert!(report.removed.contains(&w0_leader.identity));
    assert!(blocklace.get(&w0_leader.identity).is_none());
    assert_eq!(after, before);
}

#[test]
fn frequent_checkpoint_pruning_keeps_block_memory_bounded() {
    let mut blocklace = Blocklace::new();
    let prune_interval = 100usize;
    let total_blocks = 10_000usize;

    let mut previous = genesis(&node(1), 1);
    let first_id = previous.identity.clone();
    insert(&mut blocklace, &previous);

    let mut max_retained = blocklace.dom().len();

    for index in 2..=total_blocks {
        let creator = node(((index % 4) + 1) as u8);
        let next = child(&creator, index as u64, &[&previous]);
        insert(&mut blocklace, &next);
        previous = next;

        if index % prune_interval == 0 {
            let report = blocklace
                .prune_below_checkpoint(&previous.identity)
                .expect("checkpoint should prune");
            assert!(report.retained_blocks <= prune_interval);
            assert!(blocklace.is_closed());
        }

        max_retained = max_retained.max(blocklace.dom().len());
    }

    assert!(max_retained <= prune_interval);
    assert!(blocklace.dom().len() <= prune_interval);
    assert!(blocklace.get(&first_id).is_none());
    assert_eq!(blocklace.current_checkpoint(), Some(&previous.identity));
}
