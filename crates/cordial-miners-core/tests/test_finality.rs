use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    final_leader_for_wave, is_final_leader, latest_final_leader, leader_block_for_wave,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(()) // Always allow in tests
    }
}

// ── Helpers ──

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_id(creator: &NodeId, tag: u8) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0] = creator.0[0];
    hash[1] = tag;
    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: vec![tag],
    }
}

fn genesis(creator: &NodeId, tag: u8) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    }
}

fn child(creator: &NodeId, tag: u8, parents: &[&Block]) -> Block {
    let preds = parents.iter().map(|b| b.identity.clone()).collect();
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: preds,
        },
    }
}

fn insert(bl: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    bl.insert(block.clone(), &verifier).expect("insert failed");
}

// Always elect node(1) as leader
fn leader_node1(wave: u64) -> Option<NodeId> {
    let _ = wave;
    Some(node(1))
}

// No leader ever elected
fn no_leader(_wave: u64) -> Option<NodeId> {
    None
}

// ── leader_block_for_wave tests ──

/// leader_block_for_wave returns the correct block when
/// the leader has exactly one block in the leader round.
#[test]
fn leader_block_for_wave_returns_correct_block() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let wave = 0u64; // leader round = 0

    // Leader (node 1) creates a genesis block at round 0
    let v1 = node(1);
    let leader_block = genesis(&v1, 10);
    insert(&mut bl, &leader_block);

    let result = leader_block_for_wave(&bl, wave, wavelength, leader_node1);
    assert_eq!(result, Some(leader_block.identity));
}

/// leader_block_for_wave returns None when no leader is elected.
#[test]
fn leader_block_for_wave_returns_none_when_no_leader() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let block = genesis(&v1, 1);
    insert(&mut bl, &block);

    let result = leader_block_for_wave(&bl, 0, 3, no_leader);
    assert!(result.is_none());
}

/// leader_block_for_wave returns None when the leader has
/// no block in the leader round.
#[test]
fn leader_block_for_wave_returns_none_when_leader_has_no_block() {
    let mut bl = Blocklace::new();

    // Only node 2 has a block — not the elected leader (node 1)
    let v2 = node(2);
    let block = genesis(&v2, 1);
    insert(&mut bl, &block);

    let result = leader_block_for_wave(&bl, 0, 3, leader_node1);
    assert!(result.is_none());
}

/// leader_block_for_wave returns None when the elected leader equivocated.
#[test]
fn leader_block_for_wave_returns_none_on_equivocation() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let wave = 0u64;

    // Leader equivocates — two blocks in the leader round.
    let v1 = node(1);
    let block_a = genesis(&v1, 20);
    let block_b = genesis(&v1, 10);
    insert(&mut bl, &block_a);
    insert(&mut bl, &block_b);

    let result = leader_block_for_wave(&bl, wave, wavelength, leader_node1);
    assert!(result.is_none());
}

// ── is_final_leader tests ──

/// is_final_leader returns true when the leader block is
/// super-ratified within its wave.
#[test]
fn is_final_leader_returns_true_when_super_ratified() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let n = 4;
    let f = 1;

    // Wave 0: rounds 0, 1, 2
    // Round 0: leader block by node 1
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);
    let v4 = node(4);
    let leader = genesis(&v1, 1);
    insert(&mut bl, &leader);

    // Round 1: nodes 2, 3, 4 all observe leader
    let b2r1 = child(&v2, 2, &[&leader]);
    let b3r1 = child(&v3, 3, &[&leader]);
    let b4r1 = child(&v4, 4, &[&leader]);
    insert(&mut bl, &b2r1);
    insert(&mut bl, &b3r1);
    insert(&mut bl, &b4r1);

    // Round 2: nodes 2, 3, 4 observe all of round 1
    let b2r2 = child(&v2, 5, &[&b2r1, &b3r1, &b4r1]);
    let b3r2 = child(&v3, 6, &[&b2r1, &b3r1, &b4r1]);
    let b4r2 = child(&v4, 7, &[&b2r1, &b3r1, &b4r1]);
    insert(&mut bl, &b2r2);
    insert(&mut bl, &b3r2);
    insert(&mut bl, &b4r2);

    let result = is_final_leader(&bl, &leader.identity, wavelength, n, f, leader_node1);
    assert!(result);
}

/// is_final_leader returns false when super-ratification is not achieved.
#[test]
fn is_final_leader_returns_false_when_not_super_ratified() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let n = 4;
    let f = 1;

    // Only the leader block exists — no ratifying blocks at all
    let v1 = node(1);
    let leader = genesis(&v1, 1);
    insert(&mut bl, &leader);

    let result = is_final_leader(&bl, &leader.identity, wavelength, n, f, leader_node1);
    assert!(!result);
}

/// is_final_leader returns false for a block that is not
/// the elected leader block for its wave.
#[test]
fn is_final_leader_returns_false_for_non_leader_block() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let n = 4;
    let f = 1;

    // node 2 is NOT the elected leader — node 1 is
    let v2 = node(2);
    let non_leader = genesis(&v2, 1);
    insert(&mut bl, &non_leader);

    let result = is_final_leader(&bl, &non_leader.identity, wavelength, n, f, leader_node1);
    assert!(!result);
}

/// An equivocating leader should not get an arbitrary leader branch selected
/// by the finality layer.
#[test]
fn is_final_leader_returns_false_for_equivocating_leader_branch() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let n = 4;
    let f = 1;

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);
    let v4 = node(4);

    let leader_a = genesis(&v1, 1);
    let leader_b = genesis(&v1, 2);
    insert(&mut bl, &leader_a);
    insert(&mut bl, &leader_b);

    // All later blocks observe both leader branches, so neither branch should
    // be approvable / super-ratifiable under the current approval semantics.
    let r1_v2 = child(&v2, 3, &[&leader_a, &leader_b]);
    let r1_v3 = child(&v3, 4, &[&leader_a, &leader_b]);
    let r1_v4 = child(&v4, 5, &[&leader_a, &leader_b]);
    insert(&mut bl, &r1_v2);
    insert(&mut bl, &r1_v3);
    insert(&mut bl, &r1_v4);

    let r2_v2 = child(&v2, 6, &[&r1_v2, &r1_v3, &r1_v4]);
    let r2_v3 = child(&v3, 7, &[&r1_v2, &r1_v3, &r1_v4]);
    let r2_v4 = child(&v4, 8, &[&r1_v2, &r1_v3, &r1_v4]);
    insert(&mut bl, &r2_v2);
    insert(&mut bl, &r2_v3);
    insert(&mut bl, &r2_v4);

    assert!(!is_final_leader(
        &bl,
        &leader_a.identity,
        wavelength,
        n,
        f,
        leader_node1
    ));
    assert!(!is_final_leader(
        &bl,
        &leader_b.identity,
        wavelength,
        n,
        f,
        leader_node1
    ));
}

#[test]
fn final_leader_for_wave_returns_unique_final_leader() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let wave = 0u64;
    let n = 4;
    let f = 1;

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);
    let v4 = node(4);
    let leader = genesis(&v1, 1);
    insert(&mut bl, &leader);

    let b2r1 = child(&v2, 2, &[&leader]);
    let b3r1 = child(&v3, 3, &[&leader]);
    let b4r1 = child(&v4, 4, &[&leader]);
    insert(&mut bl, &b2r1);
    insert(&mut bl, &b3r1);
    insert(&mut bl, &b4r1);

    let b2r2 = child(&v2, 5, &[&b2r1, &b3r1, &b4r1]);
    let b3r2 = child(&v3, 6, &[&b2r1, &b3r1, &b4r1]);
    let b4r2 = child(&v4, 7, &[&b2r1, &b3r1, &b4r1]);
    insert(&mut bl, &b2r2);
    insert(&mut bl, &b3r2);
    insert(&mut bl, &b4r2);

    assert_eq!(
        final_leader_for_wave(&bl, wave, wavelength, n, f, leader_node1),
        Some(leader.identity)
    );
}

#[test]
fn latest_final_leader_returns_most_recent_final_wave() {
    let mut bl = Blocklace::new();
    let wavelength = 3u64;
    let n = 4;
    let f = 1;

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);
    let v4 = node(4);

    let w0_leader = genesis(&v1, 1);
    insert(&mut bl, &w0_leader);

    let w0_r1_v2 = child(&v2, 2, &[&w0_leader]);
    let w0_r1_v3 = child(&v3, 3, &[&w0_leader]);
    let w0_r1_v4 = child(&v4, 4, &[&w0_leader]);
    insert(&mut bl, &w0_r1_v2);
    insert(&mut bl, &w0_r1_v3);
    insert(&mut bl, &w0_r1_v4);

    let w0_r2_v2 = child(&v2, 5, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v3 = child(&v3, 6, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v4 = child(&v4, 7, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    insert(&mut bl, &w0_r2_v2);
    insert(&mut bl, &w0_r2_v3);
    insert(&mut bl, &w0_r2_v4);

    let w1_leader = child(&v1, 8, &[&w0_r2_v2, &w0_r2_v3, &w0_r2_v4]);
    insert(&mut bl, &w1_leader);

    let w1_r1_v2 = child(&v2, 9, &[&w1_leader]);
    let w1_r1_v3 = child(&v3, 10, &[&w1_leader]);
    let w1_r1_v4 = child(&v4, 11, &[&w1_leader]);
    insert(&mut bl, &w1_r1_v2);
    insert(&mut bl, &w1_r1_v3);
    insert(&mut bl, &w1_r1_v4);

    let w1_r2_v2 = child(&v2, 12, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    let w1_r2_v3 = child(&v3, 13, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    let w1_r2_v4 = child(&v4, 14, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    insert(&mut bl, &w1_r2_v2);
    insert(&mut bl, &w1_r2_v3);
    insert(&mut bl, &w1_r2_v4);

    assert_eq!(
        latest_final_leader(&bl, wavelength, n, f, leader_node1),
        Some(w1_leader.identity)
    );
}
