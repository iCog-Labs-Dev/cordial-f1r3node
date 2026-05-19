//! Multi-validator consensus simulations.
//!
//! These tests exercise fork choice + finality + validation together
//! in realistic scenarios with multiple validators, forks, equivocators,
//! and convergence.

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    ValidationConfig, collect_validator_tips, fork_choice, is_cordial, validated_insert,
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

    // Now v3 converges: sees the heavy fork and builds on it
    let converge = child(&node(3), 21, &[&a2, &b1]);
    insert(&mut bl, &converge);
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
}

// ═══════════════════════════════════════════════════════════════
// Scenario 4: Cordial round — all validators see all tips
// ═══════════════════════════════════════════════════════════════

#[test]
fn cordial_round_all_validators_see_all_tips() {
    let mut bl = Blocklace::new();

    // Round 1: each validator creates their own genesis
    let g1 = genesis(&node(1), 1);
    let g2 = genesis(&node(2), 2);
    let g3 = genesis(&node(3), 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

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
// Scenario 6: 4 validators, one offline, tip collection still works
// ═══════════════════════════════════════════════════════════════

#[test]
fn tip_collection_with_one_validator_offline() {
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

    // v4 is offline but validator tip collection still works
    let tips = collect_validator_tips(&bl, &b);
    assert_eq!(tips.len(), 3); // v4 has no tip
}
// ═══════════════════════════════════════════════════════════════
// Scenario 7: Full simulation — 5 rounds of cordial consensus
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

    // Fork choice should exist
    let fc = fork_choice(&bl, &b).unwrap();
    assert!(!fc.tips.is_empty());
}
