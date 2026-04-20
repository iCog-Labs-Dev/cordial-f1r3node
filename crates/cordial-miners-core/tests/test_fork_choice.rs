use std::collections::HashMap;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{fork_choice, collect_validator_tips, is_cordial};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
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

// ── Tests ──

#[test]
fn fork_choice_returns_none_on_empty_blocklace() {
    let bl = Blocklace::new();
    let b = bonds(&[(1, 100)]);
    assert!(fork_choice(&bl, &b).is_none());
}

#[test]
fn fork_choice_single_validator_single_block() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b = bonds(&[(1, 100)]);
    let fc = fork_choice(&bl, &b).unwrap();

    assert_eq!(fc.tips.len(), 1);
    assert_eq!(fc.tips[0], g.identity);
    assert_eq!(fc.lca, g.identity);
    assert_eq!(*fc.scores.get(&g.identity).unwrap(), 100);
}

#[test]
fn fork_choice_single_validator_chain() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    let b2 = child(&v1, 2, &[&g]);
    let b3 = child(&v1, 3, &[&b2]);
    insert(&mut bl, &g);
    insert(&mut bl, &b2);
    insert(&mut bl, &b3);

    let b = bonds(&[(1, 50)]);
    let fc = fork_choice(&bl, &b).unwrap();

    // Tip should be the latest block
    assert_eq!(fc.tips[0], b3.identity);
    // LCA is the same as the tip (single validator)
    assert_eq!(fc.lca, b3.identity);
}

#[test]
fn fork_choice_two_validators_agreeing() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // v1 creates genesis
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // v2 creates a block on top of g
    let b2 = child(&v2, 2, &[&g]);
    insert(&mut bl, &b2);

    // v1 creates a block on top of b2 (sees v2's block)
    let b3 = child(&v1, 3, &[&b2]);
    insert(&mut bl, &b3);

    let b = bonds(&[(1, 100), (2, 100)]);
    let fc = fork_choice(&bl, &b).unwrap();

    // Both validators agree on a linear chain
    // v1's tip is b3, v2's tip is b2
    // LCA is b2 (common ancestor of b3 and b2)
    assert_eq!(fc.lca, b2.identity);
    assert!(fc.tips.contains(&b3.identity));
    assert!(fc.tips.contains(&b2.identity));
}

#[test]
fn fork_choice_two_validators_diverging_heavier_wins() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // Shared genesis
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // v1 forks one way
    let b_v1 = child(&v1, 2, &[&g]);
    insert(&mut bl, &b_v1);

    // v2 forks another way
    let b_v2 = child(&v2, 3, &[&g]);
    insert(&mut bl, &b_v2);

    // v1 has more stake
    let b = bonds(&[(1, 200), (2, 100)]);
    let fc = fork_choice(&bl, &b).unwrap();

    // LCA should be genesis (the shared ancestor)
    assert_eq!(fc.lca, g.identity);

    // v1's tip should rank first (higher stake)
    assert_eq!(fc.tips[0], b_v1.identity);
    assert_eq!(*fc.scores.get(&b_v1.identity).unwrap(), 200);
    assert_eq!(*fc.scores.get(&b_v2.identity).unwrap(), 100);
}

#[test]
fn fork_choice_excludes_equivocators() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // v1 creates two genesis blocks (equivocation!)
    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v1, 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // v2 creates a normal genesis
    let g_v2 = genesis(&v2, 3);
    insert(&mut bl, &g_v2);

    let b = bonds(&[(1, 100), (2, 50)]);
    let fc = fork_choice(&bl, &b).unwrap();

    // v1 is an equivocator, so only v2's tip should appear
    assert_eq!(fc.tips.len(), 1);
    assert_eq!(fc.tips[0], g_v2.identity);
}

#[test]
fn fork_choice_unbonded_validators_ignored() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // Only v1 is bonded
    let b = bonds(&[(1, 100)]);
    let fc = fork_choice(&bl, &b).unwrap();

    assert_eq!(fc.tips.len(), 1);
    assert_eq!(fc.tips[0], g1.identity);
}

#[test]
fn fork_choice_lca_on_genesis_with_three_validators() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // Shared genesis by v1
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    // All three build on genesis
    let b2 = child(&v2, 2, &[&g]);
    let b3 = child(&v3, 3, &[&g]);
    let b4 = child(&v1, 4, &[&g]);
    insert(&mut bl, &b2);
    insert(&mut bl, &b3);
    insert(&mut bl, &b4);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let fc = fork_choice(&bl, &b).unwrap();

    // LCA should be genesis
    assert_eq!(fc.lca, g.identity);
    assert_eq!(fc.tips.len(), 3);
}

// ── Validator tips ──

#[test]
fn collect_validator_tips_returns_latest_per_validator() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1 = genesis(&v1, 1);
    insert(&mut bl, &g1);
    let b2 = child(&v1, 2, &[&g1]);
    insert(&mut bl, &b2);

    let g2 = genesis(&v2, 3);
    insert(&mut bl, &g2);

    let b = bonds(&[(1, 100), (2, 100)]);
    let tips = collect_validator_tips(&bl, &b);

    assert_eq!(tips.len(), 2);
    assert_eq!(tips[&v1], b2.identity); // v1's latest is b2, not g1
    assert_eq!(tips[&v2], g2.identity);
}

#[test]
fn collect_validator_tips_excludes_equivocators() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // v1 equivocates
    let g1a = genesis(&v1, 1);
    let g1b = genesis(&v1, 2);
    insert(&mut bl, &g1a);
    insert(&mut bl, &g1b);

    let g2 = genesis(&v2, 3);
    insert(&mut bl, &g2);

    let b = bonds(&[(1, 100), (2, 100)]);
    let tips = collect_validator_tips(&bl, &b);

    assert_eq!(tips.len(), 1);
    assert!(tips.contains_key(&v2));
    assert!(!tips.contains_key(&v1));
}

// ── Cordial condition ──

#[test]
fn cordial_block_references_all_tips() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // v1 creates a block referencing both tips — cordial
    let cordial_block = child(&v1, 3, &[&g1, &g2]);
    insert(&mut bl, &cordial_block);

    let b = bonds(&[(1, 100), (2, 100)]);
    let tips = collect_validator_tips(&bl, &b);
    // After inserting cordial_block, v1's tip is cordial_block, v2's tip is g2
    // cordial_block should reference g2 (which is v2's tip)
    assert!(is_cordial(&cordial_block, &tips));
}

#[test]
fn non_cordial_block_misses_a_tip() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // v1 creates a block referencing only its own genesis — NOT cordial
    let non_cordial = child(&v1, 3, &[&g1]);

    // Build tips map as it would be before this block is inserted
    let mut tips = HashMap::new();
    tips.insert(v1.clone(), g1.identity.clone());
    tips.insert(v2.clone(), g2.identity.clone());

    assert!(!is_cordial(&non_cordial, &tips));
}

#[test]
fn block_that_is_itself_a_tip_is_cordial() {
    let v1 = node(1);
    let g1 = genesis(&v1, 1);

    // Only one validator, and the block IS the tip
    let mut tips = HashMap::new();
    tips.insert(v1.clone(), g1.identity.clone());

    assert!(is_cordial(&g1, &tips));
}
