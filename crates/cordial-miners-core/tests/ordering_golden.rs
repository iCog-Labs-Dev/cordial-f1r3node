use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{OrderingCache, weighted_tau, weighted_tau_with_cache};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use sha2::{Digest, Sha256};

const WAVELENGTH: u64 = 3;
const VALIDATORS: usize = 4;
/// Fill in from the first run (the failure message prints it).
const EXPECTED_DIGEST: &str = "fce4334b17a234a758e10509d2fedb109eeab73c4569324275b73c1e29e30c9f";

struct MockVerifier;
impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(&self, _c: &BlockContent, _s: &[u8], _n: &NodeId) -> Result<(), String> {
        Ok(())
    }
}

fn node(id: u16) -> NodeId {
    NodeId(id.to_le_bytes().to_vec())
}

fn make_block(creator: &NodeId, tag: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    hash[8] = creator.0[0];
    hash[9] = creator.0[1];
    Block {
        identity: BlockIdentity {
            content_hash: hash,
            creator: creator.clone(),
            signature: tag.to_le_bytes().to_vec(),
        },
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors,
        },
    }
}

fn build_blocklace(waves: u64) -> Blocklace {
    let validators: Vec<NodeId> = (0..VALIDATORS as u16).map(node).collect();
    let mut blocklace = Blocklace::new();
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag = 1u64;
    for _ in 0..=(waves * WAVELENGTH) {
        let preds: HashSet<BlockIdentity> = prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, preds.clone());
            tag += 1;
            blocklace
                .insert(block.clone(), &MockVerifier)
                .expect("insert");
            this_round.push(block);
        }
        prev_round = this_round;
    }
    blocklace
}

#[test]
fn cached_weighted_tau_matches_uncached_and_pinned_digest() {
    let bonds: HashMap<NodeId, u64> = (0..VALIDATORS as u16).map(|i| (node(i), 100)).collect();
    let leader = |wave: u64| Some(node((wave % VALIDATORS as u64) as u16));
    let mut hasher = Sha256::new();

    for waves in [3u64, 6, 12] {
        let blocklace = build_blocklace(waves);
        let uncached = weighted_tau(&blocklace, WAVELENGTH, &bonds, leader).expect("tau");
        let mut cache = OrderingCache::default();
        let cached = weighted_tau_with_cache(&blocklace, WAVELENGTH, &bonds, 0, leader, &mut cache)
            .expect("cached tau");
        assert!(!uncached.is_empty());
        assert_eq!(uncached, cached, "cached output diverged at {waves} waves");
        for id in &uncached {
            hasher.update(id.content_hash);
        }
    }

    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest, EXPECTED_DIGEST,
        "pin this digest in EXPECTED_DIGEST"
    );
}
