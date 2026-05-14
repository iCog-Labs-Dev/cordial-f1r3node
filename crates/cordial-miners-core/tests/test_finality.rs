use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    FinalityStatus, can_be_finalized, check_finality, find_last_finalized, is_final_leader,
    leader_block_for_wave,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashMap;
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

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, stake)| (node(*id), *stake))
        .collect()
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

// ── check_finality tests ──

#[test]
fn unknown_block_returns_unknown() {
    let bl = Blocklace::new();
    let fake_id = make_id(&node(1), 99);
    let b = bonds(&[(1, 100)]);
    assert_eq!(check_finality(&bl, &fake_id, &b), FinalityStatus::Unknown);
}

#[test]
fn single_validator_finalizes_own_genesis() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // Single validator with 100% stake — trivially > 2/3
    let b = bonds(&[(1, 100)]);
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_finalized());
}

#[test]
fn block_not_in_supermajority_ancestry_is_pending() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // Each creates their own genesis
    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    let g3 = genesis(&v3, 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    // Equal stake — g1 is only supported by v1 (1/3), not > 2/3
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let status = check_finality(&bl, &g1.identity, &b);
    assert!(status.is_pending());
}

#[test]
fn supermajority_support_finalizes_block() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // v1 creates genesis
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // v2 and v3 build on g — all three have g in their ancestry
    let b2 = child(&v2, 2, &[&g]);
    let b3 = child(&v3, 3, &[&g]);
    insert(&mut bl, &b2);
    insert(&mut bl, &b3);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let status = check_finality(&bl, &g.identity, &b);

    // 3/3 validators support g — finalized
    assert!(status.is_finalized());
    match status {
        FinalityStatus::Finalized {
            supporting_stake,
            total_honest_stake,
        } => {
            assert_eq!(supporting_stake, 300);
            assert_eq!(total_honest_stake, 300);
        }
        _ => panic!("expected Finalized"),
    }
}

#[test]
fn two_thirds_plus_one_is_enough() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // v1 creates genesis
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // Only v2 builds on g. v3 creates its own genesis.
    let b2 = child(&v2, 2, &[&g]);
    let g3 = genesis(&v3, 3);
    insert(&mut bl, &b2);
    insert(&mut bl, &g3);

    // v1=100, v2=100, v3=100: g is supported by v1+v2 = 200/300 = 66.7% — NOT > 2/3
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_pending());

    // v1=100, v2=100, v3=99: g is supported by 200/299 = 66.9% — > 2/3
    let b2 = bonds(&[(1, 100), (2, 100), (3, 99)]);
    let status2 = check_finality(&bl, &g.identity, &b2);
    assert!(status2.is_finalized());
}

#[test]
fn equivocator_stake_excluded_from_total() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // v1 creates genesis
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // v2 builds on g
    let b2 = child(&v2, 2, &[&g]);
    insert(&mut bl, &b2);

    // v3 equivocates (two genesis blocks)
    let g3a = genesis(&v3, 3);
    let g3b = genesis(&v3, 4);
    insert(&mut bl, &g3a);
    insert(&mut bl, &g3b);

    // v1=100, v2=100, v3=1000 (equivocator)
    // Honest stake = 200. Supporting = 200 (v1+v2). 200 > 2/3 * 200 — finalized
    let b = bonds(&[(1, 100), (2, 100), (3, 1000)]);
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_finalized());
}

// ── find_last_finalized tests ──

#[test]
fn no_finalized_block_in_empty_blocklace() {
    let bl = Blocklace::new();
    let b = bonds(&[(1, 100)]);
    assert!(find_last_finalized(&bl, &b).is_none());
}

#[test]
fn find_last_finalized_returns_highest_finalized() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // Linear chain: g -> b2 -> b3, all three validators build on it
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b2 = child(&v2, 2, &[&g]);
    insert(&mut bl, &b2);

    let b3 = child(&v3, 3, &[&b2]);
    insert(&mut bl, &b3);

    // v1 also extends to see b3
    let b4 = child(&v1, 4, &[&b3]);
    insert(&mut bl, &b4);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let lfb = find_last_finalized(&bl, &b);

    // g is in everyone's ancestry — finalized.
    // b2 is in v2, v3, v1's ancestry (via b3->b4) — finalized.
    // The "highest" finalized block should be the most recent one
    // that all validators have built upon.
    assert!(lfb.is_some());
    let lfb = lfb.unwrap();
    // Both g and b2 are finalized. b2 is higher.
    assert!(check_finality(&bl, &lfb, &b).is_finalized());
}

#[test]
fn single_validator_last_finalized_is_tip() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    let b2 = child(&v1, 2, &[&g]);
    insert(&mut bl, &g);
    insert(&mut bl, &b2);

    let b = bonds(&[(1, 100)]);
    let lfb = find_last_finalized(&bl, &b).unwrap();

    // With a single validator, the tip is always finalized
    assert_eq!(lfb, b2.identity);
}

// ── can_be_finalized tests ──

#[test]
fn unknown_block_cannot_be_finalized() {
    let bl = Blocklace::new();
    let fake_id = make_id(&node(1), 99);
    let b = bonds(&[(1, 100)]);
    assert!(!can_be_finalized(&bl, &fake_id, &b));
}

#[test]
fn block_with_full_support_can_be_finalized() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b = bonds(&[(1, 100), (2, 100)]);
    // v2 has no tip yet — its stake counts as "undecided"
    // supporting=100, undecided=100, total=200. (100+100)*3 > 200*2 — yes
    assert!(can_be_finalized(&bl, &g.identity, &b));
}

#[test]
fn orphaned_block_cannot_be_finalized() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // v1 creates a genesis, but v2 and v3 create different ones
    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    let g3 = genesis(&v3, 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    // g1 supported only by v1 (100). v2 and v3 have their own tips.
    // supporting=100, undecided=0, total=300. 100*3 = 300, not > 200. Cannot be finalized.
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    assert!(!can_be_finalized(&bl, &g1.identity, &b));
}

#[test]
fn finality_status_helpers() {
    let finalized = FinalityStatus::Finalized {
        supporting_stake: 200,
        total_honest_stake: 300,
    };
    let pending = FinalityStatus::Pending {
        supporting_stake: 100,
        total_honest_stake: 300,
    };
    let unknown = FinalityStatus::Unknown;

    assert!(finalized.is_finalized());
    assert!(!finalized.is_pending());

    assert!(!pending.is_finalized());
    assert!(pending.is_pending());

    assert!(!unknown.is_finalized());
    assert!(!unknown.is_pending());
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
