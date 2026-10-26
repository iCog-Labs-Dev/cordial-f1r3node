//! Reproducible benchmark fixtures; never production verification policy.
#![allow(dead_code)]
use crate::Block;
use crate::blocklace::Blocklace;
use crate::crypto::{CryptoVerifier, block_hash, sign};
use crate::types::{BlockContent, BlockIdentity, NodeId};
use k256::ecdsa::SigningKey;
use std::collections::{HashMap, HashSet};

pub fn large() -> bool {
    std::env::var("CORDIAL_BENCH_LARGE").is_ok_and(|value| value == "1")
}
/// Criterion test mode runs each selected routine once, but does not reduce fixtures.
pub fn smoke() -> bool {
    std::env::var("CORDIAL_BENCH_SMOKE").is_ok_and(|value| value == "1")
        || std::env::args().any(|arg| arg == "--test")
}
pub fn sizes(small: &[usize], expensive: &[usize]) -> Vec<usize> {
    if smoke() {
        // Retain each workload family, using its smallest routine fixture.
        small.iter().take(1).copied().collect()
    } else if large() {
        expensive.to_vec()
    } else {
        small.to_vec()
    }
}
pub fn node(index: usize) -> NodeId {
    NodeId((index as u64).to_be_bytes().to_vec())
}
pub fn bonds(validators: usize) -> HashMap<NodeId, u64> {
    (0..validators).map(|index| (node(index), 100)).collect()
}
pub fn leader(wave: u64) -> Option<NodeId> {
    Some(node(wave as usize % 4))
}

/// Explicitly excludes cryptography in graph-only workloads.
pub struct FixtureVerifier;
impl CryptoVerifier for FixtureVerifier {
    type Error = String;
    fn verify_block(&self, _: &BlockContent, _: &[u8], _: &NodeId) -> Result<(), String> {
        Ok(())
    }
}
pub fn block(
    creator: NodeId,
    tag: u64,
    predecessors: HashSet<BlockIdentity>,
    bytes: usize,
) -> Block {
    let mut payload = vec![0xAB; bytes.max(8)];
    payload[..8].copy_from_slice(&tag.to_le_bytes());
    let content = BlockContent {
        payload,
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: block_hash(&creator, &content),
            creator,
            signature: Vec::new(),
        },
        content,
    }
}
pub fn signed_block(key: &SigningKey, tag: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    let creator = NodeId(
        key.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    );
    let mut value = block(creator, tag, predecessors, 256);
    value.identity.signature = sign(&value.identity.content_hash, &key.to_bytes());
    value
}
pub fn chain(count: usize, bytes: usize) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::with_capacity(count);
    for index in 0..count {
        let predecessors = blocks
            .last()
            .map(|b| HashSet::from([b.identity.clone()]))
            .unwrap_or_default();
        blocks.push(block(node(index % 4), index as u64, predecessors, bytes));
    }
    blocks
}
/// The argument is the exact layer count, not the highest round index.
pub fn rounds(validators: usize, count: usize) -> Vec<Vec<Block>> {
    let mut layers: Vec<Vec<Block>> = Vec::with_capacity(count);
    for round in 0..count {
        let predecessors: HashSet<BlockIdentity> = layers
            .last()
            .map(|layer| layer.iter().map(|b| b.identity.clone()).collect())
            .unwrap_or_default();
        layers.push(
            (0..validators)
                .map(|index| {
                    block(
                        node(index),
                        (round * validators + index) as u64,
                        predecessors.clone(),
                        256,
                    )
                })
                .collect(),
        );
    }
    layers
}
/// Four bonded validators, with a single rotating leader at every wave boundary.
/// Both following rounds contain all four validators. All paths from a later
/// wave to earlier history pass through its leader, allowing checkpoint GC to
/// reclaim that history. Fully populated boundary rounds retain peer references
/// to old history and deliberately use rounds() instead.
pub fn checkpoint_rounds(count: usize) -> Vec<Vec<Block>> {
    let mut layers: Vec<Vec<Block>> = Vec::with_capacity(count);
    for round in 0..count {
        let predecessors: HashSet<BlockIdentity> = layers
            .last()
            .map(|layer| layer.iter().map(|b| b.identity.clone()).collect())
            .unwrap_or_default();
        let creators: Vec<usize> = if round % 3 == 0 {
            vec![(round / 3) % 4]
        } else {
            (0..4).collect()
        };
        layers.push(
            creators
                .into_iter()
                .map(|index| {
                    block(
                        node(index),
                        (round * 4 + index) as u64,
                        predecessors.clone(),
                        256,
                    )
                })
                .collect(),
        );
    }
    layers
}

pub fn insert_all(blocks: &[Block]) -> Blocklace {
    let mut dag = Blocklace::new();
    for block in blocks {
        dag.insert(block.clone(), &FixtureVerifier)
            .expect("fixture insertion");
    }
    assert_eq!(dag.dom().len(), blocks.len());
    assert!(dag.is_closed());
    dag
}
pub fn flatten(layers: &[Vec<Block>]) -> Vec<Block> {
    layers.iter().flatten().cloned().collect()
}
pub fn signed_dag(count: usize) -> (Blocklace, HashMap<NodeId, u64>, Block) {
    assert_eq!(count % 4, 0, "signed DAG uses complete rounds");
    let keys: Vec<_> = (1u8..=4)
        .map(|byte| SigningKey::from_slice(&[byte; 32]).expect("fixture key"))
        .collect();
    let weights = keys
        .iter()
        .map(|key| {
            (
                NodeId(
                    key.verifying_key()
                        .to_encoded_point(true)
                        .as_bytes()
                        .to_vec(),
                ),
                100,
            )
        })
        .collect();
    let mut dag = Blocklace::new();
    let mut predecessors = HashSet::new();
    for round in 0..count / 4 {
        let mut next = HashSet::new();
        for (index, key) in keys.iter().enumerate() {
            let value = signed_block(key, (round * 4 + index) as u64, predecessors.clone());
            next.insert(value.identity.clone());
            dag.insert(value, &crate::crypto::Secp256k1Scheme)
                .expect("real fixture verification");
        }
        predecessors = next;
    }
    let candidate = signed_block(&keys[0], count as u64, predecessors);
    assert_eq!(dag.dom().len(), count);
    (dag, weights, candidate)
}

/// Compare complete canonical content without requiring production PartialEq.
pub fn assert_content_eq(actual: &BlockContent, expected: &BlockContent) {
    assert_eq!(actual.payload, expected.payload, "payload is preserved");
    assert_eq!(
        actual.predecessors, expected.predecessors,
        "predecessors are preserved"
    );
}
