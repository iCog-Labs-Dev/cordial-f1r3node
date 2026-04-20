use std::collections::HashMap;
use blocklace::blocklace::Blocklace;
use blocklace::consensus::{check_finality, find_last_finalized, can_be_finalized, FinalityStatus};
use blocklace::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

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
    bl.insert(block.clone()).expect("insert failed");
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries.iter().map(|(id, stake)| (node(*id), *stake)).collect()
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
        FinalityStatus::Finalized { supporting_stake, total_honest_stake } => {
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
