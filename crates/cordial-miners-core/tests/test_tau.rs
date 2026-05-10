//! Tests for the τ (tau) total ordering function.
//!
//! Acceptance criteria:
//!   1. Determinism  — two identical blocklaces yield the exact same τ output.
//!   2. Monotonicity — appending blocks only extends the un-finalized suffix;
//!                     the finalized prefix is strictly immutable.
//!   3. Empty        — τ on an empty blocklace returns [].
//!   4. Equivocator exclusion — Byzantine blocks never appear in τ output.

mod common;

use common::{block_on, genesis, make_identity, node};
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::finality::{approves, approved_causal_history, tau, xsort};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use std::collections::{HashMap, HashSet};

// ── Shared helpers ────────────────────────────────────────────────────────────

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

fn insert(bl: &mut Blocklace, block: &cordial_miners_core::Block) {
    bl.insert(block.clone(), &MockVerifier)
        .expect("insert failed");
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, stake)| (node(*id), *stake))
        .collect()
}

// ── Test 1: Empty blocklace ───────────────────────────────────────────────────

#[test]
fn tau_empty_blocklace_returns_empty() {
    let bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    assert_eq!(tau(&bl, &b), vec![]);
}

// ── Test 2: Determinism ───────────────────────────────────────────────────────

/// Build the same blocklace twice, independently. τ must return the same vector.
#[test]
fn tau_is_deterministic() {
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    // Helper that builds a standard 3-validator cordial chain.
    let build = || {
        let mut bl = Blocklace::new();
        let v1 = node(1);
        let v2 = node(2);
        let v3 = node(3);

        // Round 1: each validator creates a genesis
        let g1 = genesis(v1.clone(), 1);
        let g2 = genesis(v2.clone(), 2);
        let g3 = genesis(v3.clone(), 3);
        insert(&mut bl, &g1);
        insert(&mut bl, &g2);
        insert(&mut bl, &g3);

        // Round 2: each validator creates a cordial block referencing all three genesis blocks
        let r2_v1 = block_on(v1.clone(), 10, vec![&g1, &g2, &g3]);
        let r2_v2 = block_on(v2.clone(), 11, vec![&g1, &g2, &g3]);
        let r2_v3 = block_on(v3.clone(), 12, vec![&g1, &g2, &g3]);
        insert(&mut bl, &r2_v1);
        insert(&mut bl, &r2_v2);
        insert(&mut bl, &r2_v3);

        bl
    };

    let bl1 = build();
    let bl2 = build();

    let out1 = tau(&bl1, &b);
    let out2 = tau(&bl2, &b);

    assert!(!out1.is_empty(), "tau should produce output when blocks are finalized");
    assert_eq!(out1, out2, "tau must be deterministic: same blocklace → same output");
}

// ── Test 3: Monotonicity ──────────────────────────────────────────────────────

/// Appending new blocks to the blocklace must not change the finalized prefix.
#[test]
fn tau_finalized_prefix_is_immutable() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // Phase 1: build a small finalized chain.
    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    let g3 = genesis(v3.clone(), 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    // Cordial round — all three genesis blocks become finalized.
    let r2_v1 = block_on(v1.clone(), 10, vec![&g1, &g2, &g3]);
    let r2_v2 = block_on(v2.clone(), 11, vec![&g1, &g2, &g3]);
    let r2_v3 = block_on(v3.clone(), 12, vec![&g1, &g2, &g3]);
    insert(&mut bl, &r2_v1);
    insert(&mut bl, &r2_v2);
    insert(&mut bl, &r2_v3);

    // Record τ output after Phase 1.
    let prefix = tau(&bl, &b);
    assert!(!prefix.is_empty(), "should have finalized blocks after cordial round");

    // Phase 2: extend the blocklace with more blocks.
    let r3_v1 = block_on(v1.clone(), 20, vec![&r2_v1, &r2_v2, &r2_v3]);
    let r3_v2 = block_on(v2.clone(), 21, vec![&r2_v1, &r2_v2, &r2_v3]);
    let r3_v3 = block_on(v3.clone(), 22, vec![&r2_v1, &r2_v2, &r2_v3]);
    insert(&mut bl, &r3_v1);
    insert(&mut bl, &r3_v2);
    insert(&mut bl, &r3_v3);

    // τ after Phase 2 must start with the exact same prefix.
    let extended = tau(&bl, &b);
    assert!(
        extended.len() >= prefix.len(),
        "extended output must be at least as long as the original"
    );
    assert_eq!(
        &extended[..prefix.len()],
        prefix.as_slice(),
        "finalized prefix must be immutable after adding more blocks"
    );
}

// ── Test 4: Equivocator exclusion ─────────────────────────────────────────────

/// Blocks from a Byzantine equivocating validator must never appear in τ output.
#[test]
fn tau_excludes_equivocator_blocks() {
    let mut bl = Blocklace::new();
    // v3 has large stake but will equivocate.
    let b = bonds(&[(1, 100), (2, 100), (3, 500)]);

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // v1 and v2 build a finalized chain.
    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // v3 equivocates: two incomparable genesis blocks.
    let g3a = genesis(v3.clone(), 3);
    let g3b = genesis(v3.clone(), 4); // incomparable to g3a — equivocation
    insert(&mut bl, &g3a);
    insert(&mut bl, &g3b);

    // v1 and v2 build a cordial round on top of each other (ignoring v3).
    let r2_v1 = block_on(v1.clone(), 10, vec![&g1, &g2]);
    let r2_v2 = block_on(v2.clone(), 11, vec![&g1, &g2]);
    insert(&mut bl, &r2_v1);
    insert(&mut bl, &r2_v2);

    // v3 is detected as an equivocator.
    assert!(bl.find_equivacators().contains(&v3));

    let output = tau(&bl, &b);

    // v3's blocks must not appear in the output.
    for id in &output {
        assert_ne!(
            id.creator, v3,
            "equivocator v3's block {:?} must not appear in tau output",
            id
        );
    }
}

// ── Test 5: Single validator trivial case ─────────────────────────────────────

#[test]
fn tau_single_validator_linear_chain() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100)]);
    let v1 = node(1);

    let g = genesis(v1.clone(), 1);
    let b2 = block_on(v1.clone(), 2, vec![&g]);
    let b3 = block_on(v1.clone(), 3, vec![&b2]);
    insert(&mut bl, &g);
    insert(&mut bl, &b2);
    insert(&mut bl, &b3);

    let output = tau(&bl, &b);

    // All blocks should be finalized (single validator = 100% stake).
    assert!(!output.is_empty());

    // Output must be in topological order: g before b2 before b3.
    let pos = |id: &BlockIdentity| output.iter().position(|x| x == id).unwrap();
    assert!(pos(&g.identity) < pos(&b2.identity));
    assert!(pos(&b2.identity) < pos(&b3.identity));
}

// ── Test 6: approves() — basic approval ──────────────────────────────────────

#[test]
fn approves_honest_block_in_ancestry() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g = genesis(v1.clone(), 1);
    let b = block_on(v2.clone(), 2, vec![&g]);
    insert(&mut bl, &g);
    insert(&mut bl, &b);

    // b observes g and there are no equivocating siblings of g → b approves g
    assert!(approves(&bl, &b.identity, &g.identity));
    // b approves itself
    assert!(approves(&bl, &b.identity, &b.identity));
}

#[test]
fn approves_rejects_when_equivocating_sibling_observed() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // v1 equivocates: two incomparable genesis blocks.
    let g1a = genesis(v1.clone(), 1);
    let g1b = genesis(v1.clone(), 2); // equivocates with g1a
    insert(&mut bl, &g1a);
    insert(&mut bl, &g1b);

    // v2 sees both equivocating blocks.
    let b = block_on(v2.clone(), 3, vec![&g1a, &g1b]);
    insert(&mut bl, &b);

    // b observes g1a but also observes g1b (equivocating sibling) → does NOT approve g1a
    assert!(!approves(&bl, &b.identity, &g1a.identity));
    assert!(!approves(&bl, &b.identity, &g1b.identity));
}

// ── Test 7: xsort — deterministic topo sort ───────────────────────────────────

#[test]
fn xsort_respects_topological_order() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g = genesis(v1.clone(), 1);
    let b = block_on(v2.clone(), 2, vec![&g]);
    insert(&mut bl, &g);
    insert(&mut bl, &b);

    let ids: HashSet<BlockIdentity> = [g.identity.clone(), b.identity.clone()]
        .into_iter()
        .collect();

    let sorted = xsort(ids, &bl);
    assert_eq!(sorted.len(), 2);

    let pos_g = sorted.iter().position(|x| x == &g.identity).unwrap();
    let pos_b = sorted.iter().position(|x| x == &b.identity).unwrap();
    assert!(pos_g < pos_b, "ancestor must come before descendant");
}

#[test]
fn xsort_is_deterministic_for_incomparable_blocks() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // Two independent genesis blocks — incomparable in the DAG.
    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    let ids: HashSet<BlockIdentity> = [g1.identity.clone(), g2.identity.clone()]
        .into_iter()
        .collect();

    // Run xsort twice on the same input — must produce the same order.
    let sorted1 = xsort(ids.clone(), &bl);
    let sorted2 = xsort(ids, &bl);
    assert_eq!(sorted1, sorted2, "xsort must be deterministic");
    assert_eq!(sorted1.len(), 2);
}

// ── Test 8: approved_causal_history ──────────────────────────────────────────

#[test]
fn approved_causal_history_excludes_equivocating_blocks() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // v1 equivocates.
    let g1a = genesis(v1.clone(), 1);
    let g1b = genesis(v1.clone(), 2);
    insert(&mut bl, &g1a);
    insert(&mut bl, &g1b);

    // v2 sees both equivocating blocks.
    let b = block_on(v2.clone(), 3, vec![&g1a, &g1b]);
    insert(&mut bl, &b);

    let ach = approved_causal_history(&bl, &b.identity);

    // v1's equivocating blocks must not be in the approved causal history.
    assert!(!ach.contains(&g1a.identity));
    assert!(!ach.contains(&g1b.identity));

    // b itself should be in its own approved causal history.
    assert!(ach.contains(&b.identity));
}

#[test]
fn approved_causal_history_includes_honest_ancestors() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    let b = block_on(v3.clone(), 3, vec![&g1, &g2]);
    insert(&mut bl, &b);

    let ach = approved_causal_history(&bl, &b.identity);

    // All three honest blocks should be in the approved causal history.
    assert!(ach.contains(&g1.identity));
    assert!(ach.contains(&g2.identity));
    assert!(ach.contains(&b.identity));
}
