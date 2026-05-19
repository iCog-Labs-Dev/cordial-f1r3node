use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{approved_blocks_for_leader, previous_final_leader, tau, xsort};
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

fn block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
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

fn leader_node1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

#[test]
fn approved_blocks_for_leader_returns_empty_for_unknown_leader() {
    let blocklace = Blocklace::new();
    let missing_leader = block(1, 1, HashSet::new());

    let result = approved_blocks_for_leader(&blocklace, &missing_leader.identity);

    assert!(result.is_empty());
}

#[test]
fn approved_blocks_for_leader_includes_blocks_approved_by_leader() {
    let mut blocklace = Blocklace::new();

    let leader = block(1, 1, HashSet::new());
    insert(&mut blocklace, &leader);

    let approved_a = block(2, 2, HashSet::from([leader.identity.clone()]));
    let approved_b = block(3, 3, HashSet::from([approved_a.identity.clone()]));
    let unrelated = block(4, 4, HashSet::new());
    insert(&mut blocklace, &approved_a);
    insert(&mut blocklace, &approved_b);
    insert(&mut blocklace, &unrelated);

    let result = approved_blocks_for_leader(&blocklace, &approved_b.identity);

    assert!(result.contains(&leader));
    assert!(result.contains(&approved_a));
    assert!(result.contains(&approved_b));
    assert!(!result.contains(&unrelated));
}

#[test]
fn approved_blocks_for_leader_excludes_blocks_not_approved_due_to_equivocation() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    let conflicting = block(1, 2, HashSet::new());
    insert(&mut blocklace, &target);
    insert(&mut blocklace, &conflicting);

    let leader = block(
        2,
        3,
        HashSet::from([target.identity.clone(), conflicting.identity.clone()]),
    );
    insert(&mut blocklace, &leader);

    let result = approved_blocks_for_leader(&blocklace, &leader.identity);

    assert!(!result.contains(&target));
    assert!(!result.contains(&conflicting));
    assert!(result.contains(&leader));
}

#[test]
fn xsort_returns_empty_for_empty_block_set() {
    let ordered = xsort(&HashSet::new());
    assert!(ordered.is_empty());
}

#[test]
fn xsort_respects_predecessor_order() {
    let genesis = block(1, 1, HashSet::new());
    let child_a = block(2, 2, HashSet::from([genesis.identity.clone()]));
    let child_b = block(3, 3, HashSet::from([child_a.identity.clone()]));

    let blocks = HashSet::from([child_b.clone(), genesis.clone(), child_a.clone()]);
    let ordered = xsort(&blocks);

    assert_eq!(
        ordered,
        vec![
            genesis.identity.clone(),
            child_a.identity.clone(),
            child_b.identity.clone(),
        ]
    );
}

#[test]
fn xsort_breaks_ties_by_block_identity() {
    let earlier = block(1, 1, HashSet::new());
    let later = block(1, 2, HashSet::new());

    let blocks = HashSet::from([later.clone(), earlier.clone()]);
    let ordered = xsort(&blocks);

    assert_eq!(ordered, vec![earlier.identity.clone(), later.identity.clone()]);
}

#[test]
fn xsort_ignores_predecessors_outside_selected_block_set() {
    let external_parent = block(1, 1, HashSet::new());
    let child = block(2, 2, HashSet::from([external_parent.identity.clone()]));
    let sibling = block(3, 3, HashSet::new());

    let blocks = HashSet::from([child.clone(), sibling.clone()]);
    let ordered = xsort(&blocks);

    assert_eq!(ordered, vec![child.identity.clone(), sibling.identity.clone()]);
}

#[test]
fn previous_final_leader_returns_none_for_first_wave_leader() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    let leader = block(1, 1, HashSet::new());
    insert(&mut blocklace, &leader);

    let round1_v2 = block(2, 2, HashSet::from([leader.identity.clone()]));
    let round1_v3 = block(3, 3, HashSet::from([leader.identity.clone()]));
    let round1_v4 = block(4, 4, HashSet::from([leader.identity.clone()]));
    insert(&mut blocklace, &round1_v2);
    insert(&mut blocklace, &round1_v3);
    insert(&mut blocklace, &round1_v4);

    let round2_v2 = block(
        2,
        5,
        HashSet::from([
            round1_v2.identity.clone(),
            round1_v3.identity.clone(),
            round1_v4.identity.clone(),
        ]),
    );
    let round2_v3 = block(
        3,
        6,
        HashSet::from([
            round1_v2.identity.clone(),
            round1_v3.identity.clone(),
            round1_v4.identity.clone(),
        ]),
    );
    let round2_v4 = block(
        4,
        7,
        HashSet::from([
            round1_v2.identity.clone(),
            round1_v3.identity.clone(),
            round1_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &round2_v2);
    insert(&mut blocklace, &round2_v3);
    insert(&mut blocklace, &round2_v4);

    let result = previous_final_leader(
        &blocklace,
        &leader.identity,
        wavelength,
        n,
        f,
        leader_node1,
    );

    assert!(result.is_none());
}

#[test]
fn previous_final_leader_returns_latest_earlier_final_leader_ratified_by_current() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    let wave0_leader = block(1, 1, HashSet::new());
    insert(&mut blocklace, &wave0_leader);

    let w0_r1_v2 = block(2, 2, HashSet::from([wave0_leader.identity.clone()]));
    let w0_r1_v3 = block(3, 3, HashSet::from([wave0_leader.identity.clone()]));
    let w0_r1_v4 = block(4, 4, HashSet::from([wave0_leader.identity.clone()]));
    insert(&mut blocklace, &w0_r1_v2);
    insert(&mut blocklace, &w0_r1_v3);
    insert(&mut blocklace, &w0_r1_v4);

    let w0_r2_v2 = block(
        2,
        5,
        HashSet::from([
            w0_r1_v2.identity.clone(),
            w0_r1_v3.identity.clone(),
            w0_r1_v4.identity.clone(),
        ]),
    );
    let w0_r2_v3 = block(
        3,
        6,
        HashSet::from([
            w0_r1_v2.identity.clone(),
            w0_r1_v3.identity.clone(),
            w0_r1_v4.identity.clone(),
        ]),
    );
    let w0_r2_v4 = block(
        4,
        7,
        HashSet::from([
            w0_r1_v2.identity.clone(),
            w0_r1_v3.identity.clone(),
            w0_r1_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &w0_r2_v2);
    insert(&mut blocklace, &w0_r2_v3);
    insert(&mut blocklace, &w0_r2_v4);

    let wave1_leader = block(
        1,
        8,
        HashSet::from([
            w0_r2_v2.identity.clone(),
            w0_r2_v3.identity.clone(),
            w0_r2_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &wave1_leader);

    let w1_r1_v2 = block(2, 9, HashSet::from([wave1_leader.identity.clone()]));
    let w1_r1_v3 = block(3, 10, HashSet::from([wave1_leader.identity.clone()]));
    let w1_r1_v4 = block(4, 11, HashSet::from([wave1_leader.identity.clone()]));
    insert(&mut blocklace, &w1_r1_v2);
    insert(&mut blocklace, &w1_r1_v3);
    insert(&mut blocklace, &w1_r1_v4);

    let w1_r2_v2 = block(
        2,
        12,
        HashSet::from([
            w1_r1_v2.identity.clone(),
            w1_r1_v3.identity.clone(),
            w1_r1_v4.identity.clone(),
        ]),
    );
    let w1_r2_v3 = block(
        3,
        13,
        HashSet::from([
            w1_r1_v2.identity.clone(),
            w1_r1_v3.identity.clone(),
            w1_r1_v4.identity.clone(),
        ]),
    );
    let w1_r2_v4 = block(
        4,
        14,
        HashSet::from([
            w1_r1_v2.identity.clone(),
            w1_r1_v3.identity.clone(),
            w1_r1_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &w1_r2_v2);
    insert(&mut blocklace, &w1_r2_v3);
    insert(&mut blocklace, &w1_r2_v4);

    let result = previous_final_leader(
        &blocklace,
        &wave1_leader.identity,
        wavelength,
        n,
        f,
        leader_node1,
    );

    assert_eq!(result, Some(wave0_leader.identity.clone()));
}

#[test]
fn tau_returns_empty_when_no_final_leader_exists() {
    let blocklace = Blocklace::new();
    let ordered = tau(&blocklace, 3, 4, 1, leader_node1);
    assert!(ordered.is_empty());
}

#[test]
fn tau_returns_xsort_of_approved_blocks_for_single_final_leader() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    let leader = block(1, 1, HashSet::new());
    insert(&mut blocklace, &leader);

    let round1_v2 = block(2, 2, HashSet::from([leader.identity.clone()]));
    let round1_v3 = block(3, 3, HashSet::from([leader.identity.clone()]));
    let round1_v4 = block(4, 4, HashSet::from([leader.identity.clone()]));
    insert(&mut blocklace, &round1_v2);
    insert(&mut blocklace, &round1_v3);
    insert(&mut blocklace, &round1_v4);

    let round2_v2 = block(
        2,
        5,
        HashSet::from([
            round1_v2.identity.clone(),
            round1_v3.identity.clone(),
            round1_v4.identity.clone(),
        ]),
    );
    let round2_v3 = block(
        3,
        6,
        HashSet::from([
            round1_v2.identity.clone(),
            round1_v3.identity.clone(),
            round1_v4.identity.clone(),
        ]),
    );
    let round2_v4 = block(
        4,
        7,
        HashSet::from([
            round1_v2.identity.clone(),
            round1_v3.identity.clone(),
            round1_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &round2_v2);
    insert(&mut blocklace, &round2_v3);
    insert(&mut blocklace, &round2_v4);

    let approved = approved_blocks_for_leader(&blocklace, &leader.identity);
    let ordered = tau(&blocklace, wavelength, n, f, leader_node1);

    assert_eq!(ordered, xsort(&approved));
}

#[test]
fn tau_grows_monotonically_across_final_leaders_without_duplicates() {
    let mut blocklace = Blocklace::new();
    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    let wave0_leader = block(1, 1, HashSet::new());
    insert(&mut blocklace, &wave0_leader);

    let w0_r1_v2 = block(2, 2, HashSet::from([wave0_leader.identity.clone()]));
    let w0_r1_v3 = block(3, 3, HashSet::from([wave0_leader.identity.clone()]));
    let w0_r1_v4 = block(4, 4, HashSet::from([wave0_leader.identity.clone()]));
    insert(&mut blocklace, &w0_r1_v2);
    insert(&mut blocklace, &w0_r1_v3);
    insert(&mut blocklace, &w0_r1_v4);

    let w0_r2_v2 = block(
        2,
        5,
        HashSet::from([
            w0_r1_v2.identity.clone(),
            w0_r1_v3.identity.clone(),
            w0_r1_v4.identity.clone(),
        ]),
    );
    let w0_r2_v3 = block(
        3,
        6,
        HashSet::from([
            w0_r1_v2.identity.clone(),
            w0_r1_v3.identity.clone(),
            w0_r1_v4.identity.clone(),
        ]),
    );
    let w0_r2_v4 = block(
        4,
        7,
        HashSet::from([
            w0_r1_v2.identity.clone(),
            w0_r1_v3.identity.clone(),
            w0_r1_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &w0_r2_v2);
    insert(&mut blocklace, &w0_r2_v3);
    insert(&mut blocklace, &w0_r2_v4);

    let first = tau(&blocklace, wavelength, n, f, leader_node1);

    let wave1_leader = block(
        1,
        8,
        HashSet::from([
            w0_r2_v2.identity.clone(),
            w0_r2_v3.identity.clone(),
            w0_r2_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &wave1_leader);

    let w1_r1_v2 = block(2, 9, HashSet::from([wave1_leader.identity.clone()]));
    let w1_r1_v3 = block(3, 10, HashSet::from([wave1_leader.identity.clone()]));
    let w1_r1_v4 = block(4, 11, HashSet::from([wave1_leader.identity.clone()]));
    insert(&mut blocklace, &w1_r1_v2);
    insert(&mut blocklace, &w1_r1_v3);
    insert(&mut blocklace, &w1_r1_v4);

    let w1_r2_v2 = block(
        2,
        12,
        HashSet::from([
            w1_r1_v2.identity.clone(),
            w1_r1_v3.identity.clone(),
            w1_r1_v4.identity.clone(),
        ]),
    );
    let w1_r2_v3 = block(
        3,
        13,
        HashSet::from([
            w1_r1_v2.identity.clone(),
            w1_r1_v3.identity.clone(),
            w1_r1_v4.identity.clone(),
        ]),
    );
    let w1_r2_v4 = block(
        4,
        14,
        HashSet::from([
            w1_r1_v2.identity.clone(),
            w1_r1_v3.identity.clone(),
            w1_r1_v4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &w1_r2_v2);
    insert(&mut blocklace, &w1_r2_v3);
    insert(&mut blocklace, &w1_r2_v4);

    let second = tau(&blocklace, wavelength, n, f, leader_node1);

    assert!(second.starts_with(&first));
    assert_eq!(
        second.iter().collect::<HashSet<_>>().len(),
        second.len()
    );
    assert!(second.len() >= first.len());
}
