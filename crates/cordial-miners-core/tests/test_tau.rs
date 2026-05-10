mod common;

use common::{block_on, genesis, make_identity, node};
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::finality::{approves, approved_causal_history, tau, xsort};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use std::collections::{HashMap, HashSet};

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

#[test]
fn tau_empty_blocklace_returns_empty() {
    let bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    assert_eq!(tau(&bl, &b), vec![]);
}

#[test]
fn tau_is_deterministic() {
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    let build = || {
        let mut bl = Blocklace::new();
        let v1 = node(1);
        let v2 = node(2);
        let v3 = node(3);

        let g1 = genesis(v1.clone(), 1);
        let g2 = genesis(v2.clone(), 2);
        let g3 = genesis(v3.clone(), 3);
        insert(&mut bl, &g1);
        insert(&mut bl, &g2);
        insert(&mut bl, &g3);

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

#[test]
fn tau_finalized_prefix_is_immutable() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    let g3 = genesis(v3.clone(), 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    let r2_v1 = block_on(v1.clone(), 10, vec![&g1, &g2, &g3]);
    let r2_v2 = block_on(v2.clone(), 11, vec![&g1, &g2, &g3]);
    let r2_v3 = block_on(v3.clone(), 12, vec![&g1, &g2, &g3]);
    insert(&mut bl, &r2_v1);
    insert(&mut bl, &r2_v2);
    insert(&mut bl, &r2_v3);

    let prefix = tau(&bl, &b);
    assert!(!prefix.is_empty(), "should have finalized blocks after cordial round");

    let r3_v1 = block_on(v1.clone(), 20, vec![&r2_v1, &r2_v2, &r2_v3]);
    let r3_v2 = block_on(v2.clone(), 21, vec![&r2_v1, &r2_v2, &r2_v3]);
    let r3_v3 = block_on(v3.clone(), 22, vec![&r2_v1, &r2_v2, &r2_v3]);
    insert(&mut bl, &r3_v1);
    insert(&mut bl, &r3_v2);
    insert(&mut bl, &r3_v3);

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

#[test]
fn tau_excludes_equivocator_blocks() {
    let mut bl = Blocklace::new();
    let b = bonds(&[(1, 100), (2, 100), (3, 500)]);

    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    let g3a = genesis(v3.clone(), 3);
    let g3b = genesis(v3.clone(), 4);
    insert(&mut bl, &g3a);
    insert(&mut bl, &g3b);

    let r2_v1 = block_on(v1.clone(), 10, vec![&g1, &g2]);
    let r2_v2 = block_on(v2.clone(), 11, vec![&g1, &g2]);
    insert(&mut bl, &r2_v1);
    insert(&mut bl, &r2_v2);

    assert!(bl.find_equivacators().contains(&v3));

    let output = tau(&bl, &b);

    for id in &output {
        assert_ne!(
            id.creator, v3,
            "equivocator v3's block {:?} must not appear in tau output",
            id
        );
    }
}

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

    assert!(!output.is_empty());

    let pos = |id: &BlockIdentity| output.iter().position(|x| x == id).unwrap();
    assert!(pos(&g.identity) < pos(&b2.identity));
    assert!(pos(&b2.identity) < pos(&b3.identity));
}

#[test]
fn approves_honest_block_in_ancestry() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g = genesis(v1.clone(), 1);
    let b = block_on(v2.clone(), 2, vec![&g]);
    insert(&mut bl, &g);
    insert(&mut bl, &b);

    assert!(approves(&bl, &b.identity, &g.identity));
    assert!(approves(&bl, &b.identity, &b.identity));
}

#[test]
fn approves_rejects_when_equivocating_sibling_observed() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1a = genesis(v1.clone(), 1);
    let g1b = genesis(v1.clone(), 2);
    insert(&mut bl, &g1a);
    insert(&mut bl, &g1b);

    let b = block_on(v2.clone(), 3, vec![&g1a, &g1b]);
    insert(&mut bl, &b);

    assert!(!approves(&bl, &b.identity, &g1a.identity));
    assert!(!approves(&bl, &b.identity, &g1b.identity));
}

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

    let g1 = genesis(v1.clone(), 1);
    let g2 = genesis(v2.clone(), 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    let ids: HashSet<BlockIdentity> = [g1.identity.clone(), g2.identity.clone()]
        .into_iter()
        .collect();

    let sorted1 = xsort(ids.clone(), &bl);
    let sorted2 = xsort(ids, &bl);
    assert_eq!(sorted1, sorted2, "xsort must be deterministic");
    assert_eq!(sorted1.len(), 2);
}

#[test]
fn approved_causal_history_excludes_equivocating_blocks() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1a = genesis(v1.clone(), 1);
    let g1b = genesis(v1.clone(), 2);
    insert(&mut bl, &g1a);
    insert(&mut bl, &g1b);

    let b = block_on(v2.clone(), 3, vec![&g1a, &g1b]);
    insert(&mut bl, &b);

    let ach = approved_causal_history(&bl, &b.identity);

    assert!(!ach.contains(&g1a.identity));
    assert!(!ach.contains(&g1b.identity));
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

    assert!(ach.contains(&g1.identity));
    assert!(ach.contains(&g2.identity));
    assert!(ach.contains(&b.identity));
}
