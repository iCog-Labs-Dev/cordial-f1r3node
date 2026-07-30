//! Ordering-cache invalidation against the blocklace generation counter.
//!
//! `OrderingCache` used to decide freshness from `dom().len()`. Size is a weak
//! generation: two structurally different blocklaces of equal size are
//! indistinguishable to it, so a prune paired with an insert could leave the
//! cache serving ordering output computed against a DAG that no longer exists.
//!
//! These tests pin the counter that replaced it — that it advances on every
//! structural mutation, and that an equal-size change still invalidates.

use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    OrderingCache, checkpoint_after_finality, tau, tau_with_cache,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

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

const WAVELENGTH: u64 = 3;
const N: usize = 4;
const F: usize = 1;
const SELECTION_ID: u64 = 0;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn leader_node1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

fn make_id(creator: &NodeId, tag: u64) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    hash[8] = creator.0[0];
    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: tag.to_le_bytes().to_vec(),
    }
}

fn genesis(creator: &NodeId, tag: u64) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors: HashSet::new(),
        },
    }
}

fn child(creator: &NodeId, tag: u64, parents: &[&Block]) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors: parents.iter().map(|b| b.identity.clone()).collect(),
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace
        .insert(block.clone(), &MockVerifier)
        .expect("test block should insert");
}

/// Two finalised waves over four validators, wide enough that pruning below the
/// wave-1 checkpoint removes several blocks.
struct Graph {
    round2: Vec<Block>,
}

fn two_wave_graph(blocklace: &mut Blocklace) -> Graph {
    let (v1, v2, v3, v4) = (node(1), node(2), node(3), node(4));

    let w0_leader = genesis(&v1, 1);
    insert(blocklace, &w0_leader);

    let w0_r1: Vec<Block> = [(&v2, 2u64), (&v3, 3), (&v4, 4)]
        .iter()
        .map(|(v, t)| child(v, *t, &[&w0_leader]))
        .collect();
    for b in &w0_r1 {
        insert(blocklace, b);
    }

    let w0_r1_refs: Vec<&Block> = w0_r1.iter().collect();
    let w0_r2: Vec<Block> = [(&v2, 5u64), (&v3, 6), (&v4, 7)]
        .iter()
        .map(|(v, t)| child(v, *t, &w0_r1_refs))
        .collect();
    for b in &w0_r2 {
        insert(blocklace, b);
    }

    let w0_r2_refs: Vec<&Block> = w0_r2.iter().collect();
    let w1_leader = child(&v1, 8, &w0_r2_refs);
    insert(blocklace, &w1_leader);

    let w1_r1: Vec<Block> = [(&v2, 9u64), (&v3, 10), (&v4, 11)]
        .iter()
        .map(|(v, t)| child(v, *t, &[&w1_leader]))
        .collect();
    for b in &w1_r1 {
        insert(blocklace, b);
    }

    let w1_r1_refs: Vec<&Block> = w1_r1.iter().collect();
    let w1_r2: Vec<Block> = [(&v2, 12u64), (&v3, 13), (&v4, 14)]
        .iter()
        .map(|(v, t)| child(v, *t, &w1_r1_refs))
        .collect();
    for b in &w1_r2 {
        insert(blocklace, b);
    }

    Graph { round2: w1_r2 }
}

#[test]
fn generation_advances_on_every_insert() {
    let mut blocklace = Blocklace::new();
    assert_eq!(blocklace.generation(), 0);

    let g = genesis(&node(1), 1);
    insert(&mut blocklace, &g);
    assert_eq!(blocklace.generation(), 1);

    let c = child(&node(2), 2, &[&g]);
    insert(&mut blocklace, &c);
    assert_eq!(blocklace.generation(), 2);
}

#[test]
fn generation_advances_on_prune_and_checkpoint() {
    let mut blocklace = Blocklace::new();
    two_wave_graph(&mut blocklace);

    let before = blocklace.generation();
    let report = checkpoint_after_finality(&mut blocklace, WAVELENGTH, N, F, leader_node1)
        .expect("prune should succeed");

    assert!(
        report.is_some(),
        "a finalised wave should yield a checkpoint"
    );
    assert!(
        blocklace.generation() > before,
        "pruning and installing a checkpoint are structural mutations"
    );
}

/// The regression: a prune paired with enough inserts to restore the original
/// size must still invalidate the cache.
///
/// Under size-based invalidation the cache considers itself fresh here, because
/// `dom().len()` is unchanged.
///
/// Worth being precise about what this does and does not demonstrate. With
/// size-based invalidation this test fails on the generation assertion — the
/// cache genuinely believes stale entries are current — but `cached == fresh`
/// still holds in this particular scenario. The reason is that the inner caches
/// are keyed by *leader identity*, and pruning moves the checkpoint, so the
/// latest final leader changes and the lookup misses anyway.
///
/// So the counter is defence-in-depth against a latent hazard rather than a fix
/// for a demonstrated wrong answer: reaching observably wrong output would need
/// an equal-size mutation that leaves the latest final leader unchanged while
/// altering that leader's approved set. Both assertions are kept because the
/// per-leader keying is not a property anyone has committed to — if a future
/// change coarsens those keys, freshness becomes the only thing standing
/// between the cache and a wrong answer.
#[test]
fn cache_does_not_serve_stale_output_after_equal_size_prune_and_insert() {
    let mut blocklace = Blocklace::new();
    let graph = two_wave_graph(&mut blocklace);
    let mut cache = OrderingCache::default();

    // Populate the cache against the pre-mutation state.
    let before = tau_with_cache(
        &blocklace,
        WAVELENGTH,
        N,
        F,
        SELECTION_ID,
        leader_node1,
        &mut cache,
    )
    .expect("tau should compute");
    assert!(
        !before.is_empty(),
        "two finalised waves should order blocks"
    );
    assert!(
        cache.cached_entries() > 0,
        "the cache must actually be populated, otherwise this proves nothing"
    );

    let size_before = blocklace.dom().len();
    let generation_before = blocklace.generation();

    // Prune below the finalised checkpoint.
    let report = checkpoint_after_finality(&mut blocklace, WAVELENGTH, N, F, leader_node1)
        .expect("prune should succeed")
        .expect("a checkpoint should exist");
    let removed = size_before - blocklace.dom().len();
    assert!(
        removed > 0,
        "the scenario needs the prune to actually remove blocks, report={report:?}"
    );

    // Insert exactly as many new blocks as were pruned, restoring the size.
    let round2_refs: Vec<&Block> = graph.round2.iter().collect();
    let mut parents: Vec<Block> = Vec::new();
    for i in 0..removed {
        let tag = 100 + i as u64;
        let block = if parents.is_empty() {
            child(&node(2 + (i as u8 % 3)), tag, &round2_refs)
        } else {
            let refs: Vec<&Block> = parents.iter().collect();
            child(&node(2 + (i as u8 % 3)), tag, &refs)
        };
        insert(&mut blocklace, &block);
        parents = vec![block];
    }

    assert_eq!(
        blocklace.dom().len(),
        size_before,
        "the point of this test is an equal-size, different-content state"
    );
    assert!(
        blocklace.generation() > generation_before,
        "the generation must have advanced even though the size did not"
    );

    // The cached path must agree with a fresh computation.
    let cached = tau_with_cache(
        &blocklace,
        WAVELENGTH,
        N,
        F,
        SELECTION_ID,
        leader_node1,
        &mut cache,
    )
    .expect("tau should compute");
    let fresh = tau(&blocklace, WAVELENGTH, N, F, leader_node1).expect("tau should compute");

    assert_eq!(
        cached, fresh,
        "the cache served output that disagrees with a fresh computation"
    );
    assert_eq!(
        cache.generation(),
        blocklace.generation(),
        "the cache should be tracking the current generation"
    );
}

/// Caching must still work: with no mutation, entries survive across calls.
#[test]
fn cache_is_retained_when_nothing_changed() {
    let mut blocklace = Blocklace::new();
    two_wave_graph(&mut blocklace);
    let mut cache = OrderingCache::default();

    let first = tau_with_cache(
        &blocklace,
        WAVELENGTH,
        N,
        F,
        SELECTION_ID,
        leader_node1,
        &mut cache,
    )
    .expect("tau should compute");
    let entries_after_first = cache.cached_entries();
    let generation = cache.generation();
    assert!(entries_after_first > 0);

    let second = tau_with_cache(
        &blocklace,
        WAVELENGTH,
        N,
        F,
        SELECTION_ID,
        leader_node1,
        &mut cache,
    )
    .expect("tau should compute");

    assert_eq!(first, second);
    assert_eq!(
        cache.generation(),
        generation,
        "no mutation means no generation change"
    );
    assert!(
        cache.cached_entries() >= entries_after_first,
        "an unchanged blocklace must not clear the cache"
    );
}

/// A no-op prune attempt must not advance the generation, or every call would
/// needlessly invalidate the cache.
#[test]
fn generation_is_stable_when_nothing_is_removed() {
    let mut blocklace = Blocklace::new();
    two_wave_graph(&mut blocklace);

    checkpoint_after_finality(&mut blocklace, WAVELENGTH, N, F, leader_node1)
        .expect("first prune should succeed");
    let after_first = blocklace.generation();

    // Pruning again at the same checkpoint removes nothing new.
    let _ = checkpoint_after_finality(&mut blocklace, WAVELENGTH, N, F, leader_node1);
    let after_second = blocklace.generation();

    assert!(
        after_second - after_first <= 1,
        "a repeat prune should not churn the generation (checkpoint reinstall at \
         most), got {after_first} -> {after_second}"
    );
}
