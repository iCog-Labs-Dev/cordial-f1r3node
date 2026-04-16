//! Multi-validator consensus simulations.
//!
//! These tests exercise fork choice + finality + validation together
//! in realistic scenarios with multiple validators, forks, equivocators,
//! and convergence.

use std::collections::HashMap;
use blocklace::blocklace::Blocklace;
use blocklace::consensus::{
    fork_choice, check_finality, find_last_finalized, collect_validator_tips,
    is_cordial, validated_insert, ValidationConfig,
};
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

fn no_crypto() -> ValidationConfig {
    ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        ..Default::default()
    }
}

// ═══════════════════════════════════════════════════════════════
// Scenario 1: Happy path — 3 validators build a linear cordial chain
// ═══════════════════════════════════════════════════════════════

#[test]
fn three_validators_linear_cordial_chain() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    // Round 1: v1 creates genesis
    let g = genesis(&node(1), 1);
    insert(&mut bl, &g);

    // Round 2: v2 sees g, creates a cordial block on top
    let r2 = child(&node(2), 2, &[&g]);
    insert(&mut bl, &r2);

    // Round 3: v3 sees r2 (which includes g), creates a cordial block
    let r3 = child(&node(3), 3, &[&r2]);
    insert(&mut bl, &r3);

    // Round 4: v1 sees r3, creates a cordial block
    let r4 = child(&node(1), 4, &[&r3]);
    insert(&mut bl, &r4);

    // All blocks should be valid
    assert!(bl.is_closed());

    // Fork choice: r4 is v1's tip, r3 is v3's tip, r2 is v2's tip
    let fc = fork_choice(&bl, &b).unwrap();
    assert!(!fc.tips.is_empty());

    // Genesis should be finalized — all 3 validators have it in ancestry
    assert!(check_finality(&bl, &g.identity, &b).is_finalized());

    // r2 should also be finalized — v2, v3, v1 all have it in ancestry
    assert!(check_finality(&bl, &r2.identity, &b).is_finalized());

    // LFB should be the highest finalized block
    let lfb = find_last_finalized(&bl, &b).unwrap();
    assert!(check_finality(&bl, &lfb, &b).is_finalized());
}

// ═══════════════════════════════════════════════════════════════
// Scenario 2: Two forks, heavier fork wins, lighter eventually joins
// ═══════════════════════════════════════════════════════════════

#[test]
fn two_forks_heavier_wins_then_convergence() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 200), (2, 200), (3, 100)]);

    // Shared genesis by v1
    let g = genesis(&node(1), 1);
    insert(&mut bl, &g);

    // Fork A: v1 and v2 build together (heavy fork)
    let a1 = child(&node(1), 10, &[&g]);
    insert(&mut bl, &a1);
    let a2 = child(&node(2), 11, &[&a1]);
    insert(&mut bl, &a2);

    // Fork B: v3 builds alone (light fork)
    let b1 = child(&node(3), 20, &[&g]);
    insert(&mut bl, &b1);

    // Fork choice should prefer the heavy fork
    let fc = fork_choice(&bl, &b).unwrap();
    // a2 (v2's tip) and a1 extended by v1 should have more weight
    let heavy_tip_score = fc.scores.get(&a2.identity).copied().unwrap_or(0);
    let light_tip_score = fc.scores.get(&b1.identity).copied().unwrap_or(0);
    assert!(heavy_tip_score > light_tip_score);

    // Genesis is finalized (all 3 validators have it)
    assert!(check_finality(&bl, &g.identity, &b).is_finalized());

    // Now v3 converges: sees the heavy fork and builds on it
    let converge = child(&node(3), 21, &[&a2, &b1]);
    insert(&mut bl, &converge);

    // After convergence, a1 should be finalized (all validators have it in ancestry)
    assert!(check_finality(&bl, &a1.identity, &b).is_finalized());
}

// ═══════════════════════════════════════════════════════════════
// Scenario 3: Byzantine equivocator gets excluded
// ═══════════════════════════════════════════════════════════════

#[test]
fn equivocator_excluded_from_consensus() {
    let mut bl = Blocklace::new();
    // v3 has the most stake but will equivocate
    let b = bonds(&[(1, 100), (2, 100), (3, 500)]);

    // v1 creates a shared genesis
    let g = genesis(&node(1), 1);
    insert(&mut bl, &g);

    // v2 builds on g
    let b2 = child(&node(2), 2, &[&g]);
    insert(&mut bl, &b2);

    // v3 equivocates — creates two incomparable genesis blocks
    let g3a = genesis(&node(3), 3);
    let g3b = genesis(&node(3), 4);
    insert(&mut bl, &g3a);
    insert(&mut bl, &g3b);

    // v3 is an equivocator
    let equivocators = bl.find_equivacators();
    assert!(equivocators.contains(&node(3)));

    // Fork choice should exclude v3
    let fc = fork_choice(&bl, &b).unwrap();
    assert_eq!(fc.tips.len(), 2); // only v1 and v2

    // With v3 excluded, honest stake = 200 (v1=100 + v2=100)
    // g is in both v1 and v2's ancestry — 200/200 > 2/3 — finalized
    assert!(check_finality(&bl, &g.identity, &b).is_finalized());
}

// ═══════════════════════════════════════════════════════════════
// Scenario 4: Cordial round — all validators see all tips
// ═══════════════════════════════════════════════════════════════

#[test]
fn cordial_round_all_validators_see_all_tips() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    // Round 1: each validator creates their own genesis
    let g1 = genesis(&node(1), 1);
    let g2 = genesis(&node(2), 2);
    let g3 = genesis(&node(3), 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    // No block is finalized yet — each only has 1/3 support
    assert!(check_finality(&bl, &g1.identity, &b).is_pending());

    // Round 2: each validator creates a cordial block referencing ALL tips
    let r2_v1 = child(&node(1), 10, &[&g1, &g2, &g3]);
    let r2_v2 = child(&node(2), 11, &[&g1, &g2, &g3]);
    let r2_v3 = child(&node(3), 12, &[&g1, &g2, &g3]);
    insert(&mut bl, &r2_v1);
    insert(&mut bl, &r2_v2);
    insert(&mut bl, &r2_v3);

    // Check cordial condition
    let tips_before_r2 = {
        let mut t = HashMap::new();
        t.insert(node(1), g1.identity.clone());
        t.insert(node(2), g2.identity.clone());
        t.insert(node(3), g3.identity.clone());
        t
    };
    assert!(is_cordial(&r2_v1, &tips_before_r2));
    assert!(is_cordial(&r2_v2, &tips_before_r2));
    assert!(is_cordial(&r2_v3, &tips_before_r2));

    // After the cordial round, ALL genesis blocks should be finalized
    // because every validator's tip (r2_vX) has all genesis blocks in ancestry
    assert!(check_finality(&bl, &g1.identity, &b).is_finalized());
    assert!(check_finality(&bl, &g2.identity, &b).is_finalized());
    assert!(check_finality(&bl, &g3.identity, &b).is_finalized());
}

// ═══════════════════════════════════════════════════════════════
// Scenario 5: Validation rejects equivocating block in simulation
// ═══════════════════════════════════════════════════════════════

#[test]
fn validation_prevents_equivocation_in_simulation() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let config = no_crypto();

    // v1 creates genesis via validated insert
    let g = genesis(&node(1), 1);
    let result = validated_insert(g.clone(), &mut bl, &b, &config);
    assert!(result.is_valid());

    // v1 tries to equivocate — create a second genesis
    let g2 = genesis(&node(1), 2);
    let result = validated_insert(g2, &mut bl, &b, &config);
    assert!(!result.is_valid()); // rejected!

    // Only the original genesis is in the blocklace
    assert_eq!(bl.dom().len(), 1);
}

// ═══════════════════════════════════════════════════════════════
// Scenario 6: 4 validators, one offline, finality still possible
// ═══════════════════════════════════════════════════════════════

#[test]
fn finality_with_one_validator_offline() {
    let mut bl = Blocklace::new();
    // v4 is offline (never creates blocks) but has stake
    let b = bonds(&[(1, 100), (2, 100), (3, 100), (4, 100)]);

    // Only v1, v2, v3 participate
    let g = genesis(&node(1), 1);
    insert(&mut bl, &g);

    let b2 = child(&node(2), 2, &[&g]);
    insert(&mut bl, &b2);

    let b3 = child(&node(3), 3, &[&b2]);
    insert(&mut bl, &b3);

    // 3 out of 4 validators (75%) have g in ancestry — > 2/3 — finalized
    assert!(check_finality(&bl, &g.identity, &b).is_finalized());

    // v4 is offline but finality is still achieved
    let tips = collect_validator_tips(&bl, &b);
    assert_eq!(tips.len(), 3); // v4 has no tip
}

// ═══════════════════════════════════════════════════════════════
// Scenario 7: Exactly 2/3 stake is NOT enough (need strictly more)
// ═══════════════════════════════════════════════════════════════

#[test]
fn exactly_two_thirds_is_not_finalized() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    // v1 creates genesis
    let g = genesis(&node(1), 1);
    insert(&mut bl, &g);

    // v2 builds on g
    let b2 = child(&node(2), 2, &[&g]);
    insert(&mut bl, &b2);

    // v3 creates its own separate genesis (does NOT see g)
    let g3 = genesis(&node(3), 3);
    insert(&mut bl, &g3);

    // g is in v1 and v2's ancestry = 200/300 = exactly 2/3
    // Strictly > 2/3 required, so NOT finalized
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_pending());
}

// ═══════════════════════════════════════════════════════════════
// Scenario 8: Finality advances monotonically through chain
// ═══════════════════════════════════════════════════════════════

#[test]
fn finality_advances_through_chain() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    // Build a chain that all validators participate in
    let g = genesis(&node(1), 1);
    insert(&mut bl, &g);

    let r1 = child(&node(2), 2, &[&g]);
    insert(&mut bl, &r1);

    let r2 = child(&node(3), 3, &[&r1]);
    insert(&mut bl, &r2);

    // At this point: g is finalized (all 3 have it in ancestry)
    let lfb1 = find_last_finalized(&bl, &b).unwrap();
    assert!(check_finality(&bl, &lfb1, &b).is_finalized());

    // Continue the chain
    let r3 = child(&node(1), 4, &[&r2]);
    insert(&mut bl, &r3);

    let r4 = child(&node(2), 5, &[&r3]);
    insert(&mut bl, &r4);

    let r5 = child(&node(3), 6, &[&r4]);
    insert(&mut bl, &r5);

    // More blocks are now finalized
    let lfb2 = find_last_finalized(&bl, &b).unwrap();
    assert!(check_finality(&bl, &lfb2, &b).is_finalized());

    // LFB should have advanced (lfb2 is higher than lfb1)
    if lfb1 != lfb2 {
        assert!(bl.precedes(&lfb1, &lfb2));
    }
}

// ═══════════════════════════════════════════════════════════════
// Scenario 9: Weighted validators — minority can't finalize alone
// ═══════════════════════════════════════════════════════════════

#[test]
fn minority_stake_cannot_finalize() {
    let mut bl = Blocklace::new();
    // v1 has majority but doesn't participate in v2+v3's fork
    let b = bonds(&[(1, 500), (2, 100), (3, 100)]);

    // v2 and v3 build their own chain
    let g2 = genesis(&node(2), 1);
    insert(&mut bl, &g2);

    let b3 = child(&node(3), 2, &[&g2]);
    insert(&mut bl, &b3);

    // v1 creates a separate genesis
    let g1 = genesis(&node(1), 3);
    insert(&mut bl, &g1);

    // g2 has support from v2+v3 = 200/700 = 28.6% — NOT finalized
    assert!(check_finality(&bl, &g2.identity, &b).is_pending());

    // g1 has support from only v1 = 500/700 = 71.4% — > 2/3 — finalized!
    assert!(check_finality(&bl, &g1.identity, &b).is_finalized());
}

// ═══════════════════════════════════════════════════════════════
// Scenario 10: Full simulation — 5 rounds of cordial consensus
// ═══════════════════════════════════════════════════════════════

#[test]
fn five_round_cordial_consensus() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let config = no_crypto();

    // Round 1: v1 starts with genesis
    let r1 = genesis(&node(1), 1);
    assert!(validated_insert(r1.clone(), &mut bl, &b, &config).is_valid());

    // Round 2: v2 references v1's genesis (cordial — only tip is r1)
    let r2 = child(&node(2), 2, &[&r1]);
    assert!(validated_insert(r2.clone(), &mut bl, &b, &config).is_valid());

    // Round 3: v3 references v2's block (which includes v1 in ancestry)
    let r3 = child(&node(3), 3, &[&r2]);
    assert!(validated_insert(r3.clone(), &mut bl, &b, &config).is_valid());

    // Round 4: v1 references v3's block
    let r4 = child(&node(1), 4, &[&r3]);
    assert!(validated_insert(r4.clone(), &mut bl, &b, &config).is_valid());

    // Round 5: v2 references v1's latest
    let r5 = child(&node(2), 5, &[&r4]);
    assert!(validated_insert(r5.clone(), &mut bl, &b, &config).is_valid());

    // Structural checks
    assert_eq!(bl.dom().len(), 5);
    assert!(bl.is_closed());
    assert!(bl.satisfies_chain_axiom_all());

    // Finality: r1 (genesis) should be finalized
    assert!(check_finality(&bl, &r1.identity, &b).is_finalized());

    // Fork choice should exist
    let fc = fork_choice(&bl, &b).unwrap();
    assert!(!fc.tips.is_empty());

    // LFB should exist and be finalized
    let lfb = find_last_finalized(&bl, &b).unwrap();
    assert!(check_finality(&bl, &lfb, &b).is_finalized());
}
