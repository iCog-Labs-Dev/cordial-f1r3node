use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::finality::leader_block_for_wave;
use cordial_miners_core::consensus::{
    CheckpointGc, checkpoint_after_finality, checkpoint_after_weighted_finality, depth,
    latest_final_leader, latest_weighted_final_leader, leader_blocks_of_wave,
    previous_final_leader, tau, wave_of_round, weighted_previous_final_leader, weighted_tau,
};
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

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, stake)| (node(*id), *stake))
        .collect()
}

struct WeightedCheckpointGraph {
    w0_leader: Block,
    w1_leader: Block,
    w1_tip: Block,
}

fn insert_weighted_two_wave_checkpoint_graph(blocklace: &mut Blocklace) -> WeightedCheckpointGraph {
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);
    let v4 = node(4);
    let v5 = node(5);

    let w0_leader = genesis(&v1, 1);
    insert(blocklace, &w0_leader);

    let w0_r1_v2 = child(&v2, 2, &[&w0_leader]);
    let w0_r1_v3 = child(&v3, 3, &[&w0_leader]);
    let w0_r1_v4 = child(&v4, 4, &[&w0_leader]);
    for block in [&w0_r1_v2, &w0_r1_v3, &w0_r1_v4] {
        insert(blocklace, block);
    }

    let w0_r2_v2 = child(&v2, 5, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v3 = child(&v3, 6, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v4 = child(&v4, 7, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    for block in [&w0_r2_v2, &w0_r2_v3, &w0_r2_v4] {
        insert(blocklace, block);
    }

    let w1_leader = child(&v1, 8, &[&w0_r2_v2, &w0_r2_v3, &w0_r2_v4]);
    insert(blocklace, &w1_leader);

    let w1_r1_v2 = child(&v2, 9, &[&w1_leader]);
    let w1_r1_v3 = child(&v3, 10, &[&w1_leader]);
    let w1_r1_v4 = child(&v4, 11, &[&w1_leader]);
    let w1_r1_v5 = child(&v5, 12, &[&w1_leader]);
    for block in [&w1_r1_v2, &w1_r1_v3, &w1_r1_v4, &w1_r1_v5] {
        insert(blocklace, block);
    }

    let w1_r2_v2 = child(&v2, 13, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4, &w1_r1_v5]);
    let w1_r2_v3 = child(&v3, 14, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4, &w1_r1_v5]);
    let w1_r2_v4 = child(&v4, 15, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4, &w1_r1_v5]);
    let w1_tip = child(&v5, 16, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4, &w1_r1_v5]);
    for block in [&w1_r2_v2, &w1_r2_v3, &w1_r2_v4, &w1_tip] {
        insert(blocklace, block);
    }

    WeightedCheckpointGraph {
        w0_leader,
        w1_leader,
        w1_tip,
    }
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
fn checkpoint_prune_preserves_ancestor_referenced_unsigned_by_retained_fork() {
    let mut blocklace = Blocklace::new();

    let ancestor = genesis(&node(1), 1);
    let checkpoint = child(&node(2), 2, &[&ancestor]);
    for block in [&ancestor, &checkpoint] {
        insert(&mut blocklace, block);
    }

    let mut unsigned_ancestor = ancestor.identity.clone();
    unsigned_ancestor.signature.clear();
    let retained_fork = Block {
        identity: make_id(&node(3), 3),
        content: BlockContent {
            payload: vec![3],
            predecessors: HashSet::from([unsigned_ancestor]),
        },
    };
    insert(&mut blocklace, &retained_fork);

    let report = blocklace
        .prune_below_checkpoint(&checkpoint.identity)
        .expect("checkpoint should prune without breaking retained forks");

    assert!(!report.removed.contains(&ancestor.identity));
    assert!(blocklace.get(&ancestor.identity).is_some());
    assert!(blocklace.get(&retained_fork.identity).is_some());
    assert!(blocklace.is_closed());
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
fn weighted_checkpoint_prune_preserves_weighted_tau_prefix() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3;
    let weights = bonds(&[(1, 1), (2, 1), (3, 1), (4, 1), (5, 100)]);
    let graph = insert_weighted_two_wave_checkpoint_graph(&mut blocklace);

    let before =
        weighted_tau(&blocklace, wavelength, &weights, leader_node1).expect("weighted tau");
    let report =
        checkpoint_after_weighted_finality(&mut blocklace, wavelength, &weights, leader_node1)
            .expect("weighted finality checkpoint should not error")
            .expect("latest weighted final leader should prune");
    let after = weighted_tau(&blocklace, wavelength, &weights, leader_node1).expect("weighted tau");

    assert_eq!(report.checkpoint, graph.w1_leader.identity);
    assert_eq!(report.tau_prefix_len, 0);
    assert_eq!(report.weighted_tau_prefix_len, before.len());
    assert!(report.removed.contains(&graph.w0_leader.identity));
    assert!(blocklace.get(&graph.w0_leader.identity).is_none());
    assert_eq!(after, before);

    let observed = blocklace.observe(&graph.w1_tip.identity);
    assert!(observed.contains(&graph.w1_leader.identity));
    assert!(!observed.contains(&graph.w0_leader.identity));
}

#[test]
fn weighted_tau_does_not_replay_unweighted_checkpoint_prefix() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3;
    let n = 4;
    let f = 1;
    let weights = bonds(&[(1, 1), (2, 1), (3, 1), (4, 1), (5, 100)]);
    let graph = insert_weighted_two_wave_checkpoint_graph(&mut blocklace);

    let unweighted_before =
        tau(&blocklace, wavelength, n, f, leader_node1).expect("tau should order");
    let weighted_before =
        weighted_tau(&blocklace, wavelength, &weights, leader_node1).expect("weighted tau");
    assert!(!weighted_before.is_empty());

    let report = blocklace
        .prune_below_checkpoint(&graph.w1_leader.identity)
        .expect("manual checkpoint should prune");
    assert_eq!(report.tau_prefix_len, unweighted_before.len());
    assert_eq!(report.weighted_tau_prefix_len, 0);

    let weighted_after =
        weighted_tau(&blocklace, wavelength, &weights, leader_node1).expect("weighted tau");
    assert_eq!(
        weighted_after,
        vec![graph.w1_leader.identity.consensus_identity()]
    );
    assert_ne!(weighted_after, unweighted_before);
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

/// Build `rounds` rounds of a fully connected history for `validators`:
/// every block references all blocks of the previous round.
fn fully_connected_rounds(validators: &[NodeId], rounds: u64) -> Vec<Block> {
    let mut history = Vec::new();
    let mut previous: Vec<Block> = Vec::new();
    let mut tag = 1u64;

    for round in 0..rounds {
        let parents: Vec<&Block> = previous.iter().collect();
        let mut current = Vec::new();
        for creator in validators {
            let block = if round == 0 {
                genesis(creator, tag)
            } else {
                child(creator, tag, &parents)
            };
            tag += 1;
            current.push(block.clone());
            history.push(block);
        }
        previous = current;
    }

    history
}

fn round_robin_leader(validators: &[NodeId]) -> impl Fn(u64) -> Option<NodeId> + Copy + '_ {
    move |wave| Some(validators[(wave as usize) % validators.len()].clone())
}

/// Feed `blocks` one at a time into a pruned blocklace (advancing the
/// checkpoint after every insert via `advance`) and an unpruned reference,
/// and assert at every step that the pruned output equals the reference
/// output and extends the previous pruned output. Returns how many times the
/// checkpoint advanced.
fn assert_pruned_output_matches_reference(
    blocks: &[Block],
    mut advance: impl FnMut(&mut Blocklace) -> bool,
    order: impl Fn(&Blocklace) -> Vec<BlockIdentity>,
    label: &str,
) -> usize {
    let mut pruned = Blocklace::new();
    let mut reference = Blocklace::new();
    let mut previous = Vec::new();
    let mut advances = 0usize;

    for (step, block) in blocks.iter().enumerate() {
        insert(&mut pruned, block);
        insert(&mut reference, block);
        if advance(&mut pruned) {
            advances += 1;
        }

        let output = order(&pruned);
        let expected = order(&reference);
        assert_eq!(
            output, expected,
            "{label}: pruned output differs from the unpruned reference at step {step}"
        );
        assert!(
            output.starts_with(&previous),
            "{label}: output dropped finalized blocks at step {step}"
        );
        previous = output;
    }

    advances
}

#[test]
fn repeated_weighted_checkpoint_advances_preserve_finalized_prefix() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let weights = bonds(&[(1, 25), (2, 25), (3, 25), (4, 25)]);
    let leader = round_robin_leader(&validators);
    let advances = assert_pruned_output_matches_reference(
        &fully_connected_rounds(&validators, 15),
        |lace| {
            checkpoint_after_weighted_finality(lace, 3, &weights, leader)
                .expect("weighted checkpoint advance should not error")
                .is_some()
        },
        |lace| weighted_tau(lace, 3, &weights, leader).expect("weighted tau"),
        "weighted",
    );
    assert!(
        advances >= 3,
        "expected repeated checkpoint advances, got {advances}"
    );
}

#[test]
fn repeated_checkpoint_advances_preserve_finalized_prefix() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let leader = round_robin_leader(&validators);
    let advances = assert_pruned_output_matches_reference(
        &fully_connected_rounds(&validators, 15),
        |lace| {
            checkpoint_after_finality(lace, 3, 4, 1, leader)
                .expect("checkpoint advance should not error")
                .is_some()
        },
        |lace| tau(lace, 3, 4, 1, leader).expect("tau"),
        "unweighted",
    );
    assert!(
        advances >= 3,
        "expected repeated checkpoint advances, got {advances}"
    );
}

#[test]
fn repeated_weighted_checkpoint_advances_match_reference_with_unequal_stake() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let weights = bonds(&[(1, 40), (2, 30), (3, 20), (4, 10)]);
    let leader = round_robin_leader(&validators);
    let advances = assert_pruned_output_matches_reference(
        &fully_connected_rounds(&validators, 15),
        |lace| {
            checkpoint_after_weighted_finality(lace, 3, &weights, leader)
                .expect("weighted checkpoint advance should not error")
                .is_some()
        },
        |lace| weighted_tau(lace, 3, &weights, leader).expect("weighted tau"),
        "weighted, unequal stake",
    );
    assert!(
        advances >= 3,
        "expected repeated checkpoint advances, got {advances}"
    );
}

#[test]
fn explicit_non_leader_checkpoint_is_not_a_previous_final_leader() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let leader = round_robin_leader(&validators);
    let mut lace = Blocklace::new();

    for block in fully_connected_rounds(&validators, 15) {
        insert(&mut lace, &block);
    }

    let l = latest_final_leader(&lace, 3, 4, 1, leader).expect("should find a latest final leader");
    let l_round = depth(&lace, &l).expect("depth of L");
    let l_wave = wave_of_round(l_round, 3).expect("wave of L");
    assert!(l_wave >= 2, "expected L in wave >= 2, got wave {l_wave}");

    // Pick a block X that is not a leader block, in wave < l_wave, and precedes L.
    let non_leader_round = (l_wave - 1) * 3 + 1;
    let x = lace
        .dom()
        .into_iter()
        .find(|id| depth(&lace, id) == Some(non_leader_round))
        .expect("should find a block at non-leader round")
        .clone();
    let wave_x = wave_of_round(non_leader_round, 3).expect("wave of X");
    assert!(wave_x < l_wave);
    assert!(lace.precedes(&x, &l));
    assert!(
        !leader_blocks_of_wave(&lace, wave_x, 3, leader)
            .iter()
            .any(|b| b.identity == x)
    );

    lace.prune_below_checkpoint(&x)
        .expect("prune below non-leader checkpoint should succeed");

    let prev = previous_final_leader(&lace, &l, 3, 4, 1, leader);
    assert_ne!(
        prev,
        Some(x),
        "explicit non-leader checkpoint must not be returned as previous final leader"
    );
}

#[test]
fn explicit_non_leader_checkpoint_is_not_a_weighted_previous_final_leader() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let weights = bonds(&[(1, 25), (2, 25), (3, 25), (4, 25)]);
    let leader = round_robin_leader(&validators);
    let mut lace = Blocklace::new();

    for block in fully_connected_rounds(&validators, 15) {
        insert(&mut lace, &block);
    }

    let l = latest_weighted_final_leader(&lace, 3, &weights, leader)
        .expect("should find a latest weighted final leader");
    let l_round = depth(&lace, &l).expect("depth of L");
    let l_wave = wave_of_round(l_round, 3).expect("wave of L");
    assert!(l_wave >= 2, "expected L in wave >= 2, got wave {l_wave}");

    // Pick a block X that is not a leader block, in wave < l_wave, and precedes L.
    let non_leader_round = (l_wave - 1) * 3 + 1;
    let x = lace
        .dom()
        .into_iter()
        .find(|id| depth(&lace, id) == Some(non_leader_round))
        .expect("should find a block at non-leader round")
        .clone();
    let wave_x = wave_of_round(non_leader_round, 3).expect("wave of X");
    assert!(wave_x < l_wave);
    assert!(lace.precedes(&x, &l));
    assert!(
        !leader_blocks_of_wave(&lace, wave_x, 3, leader)
            .iter()
            .any(|b| b.identity == x)
    );

    lace.prune_below_checkpoint(&x)
        .expect("prune below non-leader checkpoint should succeed");

    let prev = weighted_previous_final_leader(&lace, &l, 3, &weights, leader);
    assert_ne!(
        prev,
        Some(x),
        "explicit non-leader checkpoint must not be returned as weighted previous final leader"
    );
}

#[test]
fn equivocating_leader_checkpoint_is_not_a_previous_final_leader() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let leader = round_robin_leader(&validators);
    let mut lace = Blocklace::new();

    let history = fully_connected_rounds(&validators, 15);
    for block in &history {
        insert(&mut lace, block);
    }

    let l = latest_final_leader(&lace, 3, 4, 1, leader).expect("should find a latest final leader");
    let l_round = depth(&lace, &l).expect("depth of L");
    let l_wave = wave_of_round(l_round, 3).expect("wave of L");
    assert!(l_wave >= 2, "expected L in wave >= 2, got wave {l_wave}");

    let w = l_wave - 1;
    let r = w * 3;
    let elected_creator = leader(w).expect("elected leader for wave w");
    let c = history[r as usize * 4..r as usize * 4 + 4]
        .iter()
        .find(|b| b.identity.creator == elected_creator)
        .expect("should find block C created by leader(w)")
        .clone();

    let parents: Vec<&Block> = history[(r - 1) as usize * 4..(r - 1) as usize * 4 + 4]
        .iter()
        .collect();
    let e = child(&elected_creator, 10_000, &parents);
    insert(&mut lace, &e);

    assert_eq!(leader_block_for_wave(&lace, w, 3, leader), None);

    lace.prune_below_checkpoint(&c.identity)
        .expect("prune below checkpoint should succeed");

    let prev = previous_final_leader(&lace, &l, 3, 4, 1, leader);
    assert_ne!(
        prev,
        Some(c.identity.clone()),
        "equivocating leader checkpoint must not be returned as previous final leader"
    );
}

#[test]
fn equivocating_leader_checkpoint_is_not_a_weighted_previous_final_leader() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let weights = bonds(&[(1, 25), (2, 25), (3, 25), (4, 25)]);
    let leader = round_robin_leader(&validators);
    let mut lace = Blocklace::new();

    let history = fully_connected_rounds(&validators, 15);
    for block in &history {
        insert(&mut lace, block);
    }

    let l = latest_weighted_final_leader(&lace, 3, &weights, leader)
        .expect("should find a latest weighted final leader");
    let l_round = depth(&lace, &l).expect("depth of L");
    let l_wave = wave_of_round(l_round, 3).expect("wave of L");
    assert!(l_wave >= 2, "expected L in wave >= 2, got wave {l_wave}");

    let w = l_wave - 1;
    let r = w * 3;
    let elected_creator = leader(w).expect("elected leader for wave w");
    let c = history[r as usize * 4..r as usize * 4 + 4]
        .iter()
        .find(|b| b.identity.creator == elected_creator)
        .expect("should find block C created by leader(w)")
        .clone();

    let parents: Vec<&Block> = history[(r - 1) as usize * 4..(r - 1) as usize * 4 + 4]
        .iter()
        .collect();
    let e = child(&elected_creator, 10_000, &parents);
    insert(&mut lace, &e);

    assert_eq!(leader_block_for_wave(&lace, w, 3, leader), None);

    lace.prune_below_checkpoint(&c.identity)
        .expect("prune below checkpoint should succeed");

    let prev = weighted_previous_final_leader(&lace, &l, 3, &weights, leader);
    assert_ne!(
        prev,
        Some(c.identity.clone()),
        "equivocating leader checkpoint must not be returned as weighted previous final leader"
    );
}

#[test]
fn legitimate_checkpoint_is_still_the_previous_final_leader_floor() {
    let validators = vec![node(1), node(2), node(3), node(4)];
    let leader = round_robin_leader(&validators);
    let mut lace = Blocklace::new();

    let history = fully_connected_rounds(&validators, 21);
    for block in &history[..15 * 4] {
        insert(&mut lace, block);
    }

    let report = checkpoint_after_finality(&mut lace, 3, 4, 1, leader)
        .expect("checkpoint_after_finality should not error")
        .expect("15 rounds should produce a checkpoint");
    let c = report.checkpoint;

    let c_round = depth(&lace, &c).expect("depth of C");
    let c_wave = wave_of_round(c_round, 3).expect("wave of C");
    assert_eq!(
        leader_block_for_wave(&lace, c_wave, 3, leader),
        Some(c.clone()),
        "unique leader must still be found after pruning"
    );

    for block in &history[15 * 4..] {
        insert(&mut lace, block);
    }

    let l = latest_final_leader(&lace, 3, 4, 1, leader).expect("should find a latest final leader");
    let l_round = depth(&lace, &l).expect("depth of L");
    let l_wave = wave_of_round(l_round, 3).expect("wave of L");
    assert!(
        l_wave > c_wave,
        "expected L wave > C wave ({l_wave} > {c_wave})"
    );

    let mut current = l;
    let mut reached_c = false;
    while let Some(prev) = previous_final_leader(&lace, &current, 3, 4, 1, leader) {
        if prev == c {
            reached_c = true;
            break;
        }
        current = prev;
    }
    assert!(
        reached_c,
        "previous_final_leader search should have reached checkpoint floor C"
    );
}
