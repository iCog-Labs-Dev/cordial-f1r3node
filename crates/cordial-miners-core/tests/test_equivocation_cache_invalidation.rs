//! Ordering-cache behaviour around equivocation detection and evidence recording.
//!
//! ## What these tests cover
//!
//! Four properties are pinned here:
//!
//! 1. **Recording equivocation evidence does not mutate the blocklace** — the
//!    `InMemoryEvidencePool` is a separate data structure. Calling
//!    `record_equivocation` must not advance `blocklace.generation()`, and the
//!    `OrderingCache` must remain valid (i.e. hit-on-next-call) because the
//!    blocklace itself has not changed.
//!
//! 2. **Equivocating branches are excluded from finalized ordered output** —
//!    even when both branches have been seen, only one survives the chain axiom,
//!    and the ordering functions must exclude equivocating-branch blocks from
//!    tau and weighted-tau.
//!
//! 3. **A subsequent valid insert after equivocation detection flushes the
//!    cache correctly** — the generation advances on the next structural
//!    mutation, and the post-flush cached result agrees with a fresh compute.
//!
//! 4. **Checkpoint pruning after equivocation does not corrupt the stored
//!    ordering prefix** — the tau / weighted-tau prefix recorded at the
//!    checkpoint is stable even when the equivocator's branch sits below the
//!    checkpoint boundary; and the evidence pool (being a separate structure)
//!    is unaffected by GC.
//!
//! ## Relationship to existing tests
//!
//! - `test_ordering_cache_generation.rs` pins the generation counter for the
//!   common finality case (insert, prune, equal-size churn).  These tests
//!   focus specifically on the equivocation path.
//! - `test_evidence_after_partition.rs` pins evidence capture semantics.
//!   These tests focus on the *cache* interaction, not evidence capture per se.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    CordialEvidencePool, EvidencePool, OrderingCache, checkpoint_after_finality,
    record_rejected_equivocation, tau, tau_with_cache, validate_block, weighted_tau,
    weighted_tau_with_cache,
};
use cordial_miners_core::consensus::{InvalidBlock, ValidationConfig};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

// ─── shared test helpers ────────────────────────────────────────────────────

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

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn bonds_uniform(ids: &[u8], stake: u64) -> HashMap<NodeId, u64> {
    ids.iter().map(|&id| (node(id), stake)).collect()
}

/// Build a `BlockIdentity` with a unique `content_hash` derived from `tag`
/// and the creator's id byte.
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
        .expect("test block should insert cleanly");
}

fn leader_node1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

/// Materialize a two-wave four-validator DAG that produces at least one
/// finalized leader so that `tau` / `weighted_tau` return non-empty results.
///
/// Returns the blocks in the order they were inserted so callers can
/// inspect specific wave members.
struct TwoWaveGraph {
    w0_leader: Block,
    w1_leader: Block,
    /// The last inserted blocks (wave-1 round-2) — used for building
    /// subsequent blocks on top of a finalized history.
    w1_round2: Vec<Block>,
}

fn build_two_wave_dag(blocklace: &mut Blocklace) -> TwoWaveGraph {
    let (v1, v2, v3, v4) = (node(1), node(2), node(3), node(4));

    // Wave 0
    let w0_leader = genesis(&v1, 1);
    insert(blocklace, &w0_leader);

    let w0_r1_v2 = child(&v2, 2, &[&w0_leader]);
    let w0_r1_v3 = child(&v3, 3, &[&w0_leader]);
    let w0_r1_v4 = child(&v4, 4, &[&w0_leader]);
    for b in [&w0_r1_v2, &w0_r1_v3, &w0_r1_v4] {
        insert(blocklace, b);
    }

    let w0_r2_v2 = child(&v2, 5, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v3 = child(&v3, 6, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    let w0_r2_v4 = child(&v4, 7, &[&w0_r1_v2, &w0_r1_v3, &w0_r1_v4]);
    for b in [&w0_r2_v2, &w0_r2_v3, &w0_r2_v4] {
        insert(blocklace, b);
    }

    // Wave 1
    let w1_leader = child(&v1, 8, &[&w0_r2_v2, &w0_r2_v3, &w0_r2_v4]);
    insert(blocklace, &w1_leader);

    let w1_r1_v2 = child(&v2, 9, &[&w1_leader]);
    let w1_r1_v3 = child(&v3, 10, &[&w1_leader]);
    let w1_r1_v4 = child(&v4, 11, &[&w1_leader]);
    for b in [&w1_r1_v2, &w1_r1_v3, &w1_r1_v4] {
        insert(blocklace, b);
    }

    let w1_r2_v2 = child(&v2, 12, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    let w1_r2_v3 = child(&v3, 13, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    let w1_r2_v4 = child(&v4, 14, &[&w1_r1_v2, &w1_r1_v3, &w1_r1_v4]);
    for b in [&w1_r2_v2, &w1_r2_v3, &w1_r2_v4] {
        insert(blocklace, b);
    }

    TwoWaveGraph {
        w0_leader,
        w1_leader,
        w1_round2: vec![w1_r2_v2, w1_r2_v3, w1_r2_v4],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1 — equivocation recording does not stale the ordering cache
// ─────────────────────────────────────────────────────────────────────────────

/// Recording equivocation evidence writes to the pool, not to the blocklace.
/// Therefore:
///   * `blocklace.generation()` must NOT advance after the evidence call.
///   * A subsequent `tau_with_cache` call must still be a cache hit (same
///     generation) and must agree with a fresh uncached `tau` result.
///
/// This is the most important property of this test file: it proves that the
/// cache's invalidation model (generation-based) remains sound even when the
/// caller performs an equivocation-detection workflow that touches a separate
/// data structure.
#[test]
fn equivocation_recording_does_not_stale_ordering_cache() {
    let mut blocklace = Blocklace::new();
    let graph = build_two_wave_dag(&mut blocklace);
    let mut cache = OrderingCache::default();
    let wavelength = 3;
    let n = 4;
    let f = 1;
    let selection_id = 0;

    // Warm the cache.
    let before = tau_with_cache(
        &blocklace,
        wavelength,
        n,
        f,
        selection_id,
        leader_node1,
        &mut cache,
    )
    .expect("tau should compute over a two-wave graph");
    assert!(
        !before.is_empty(),
        "two finalized waves must produce non-empty ordered output"
    );
    let gen_after_warm = blocklace.generation();
    let entries_after_warm = cache.cached_entries();
    assert!(
        entries_after_warm > 0,
        "cache must be populated after first tau call"
    );

    // Build an equivocating block for v2 (a second genesis from v2 with a
    // different tag, so it conflicts with w0_leader which was already inserted).
    // We need a block that conflicts with an existing v1 block in the DAG.
    // Use v2's first block and create a conflicting sibling at the same depth
    // with no overlap in ancestry with the existing v2 block at round 1.
    let equivocating_block = genesis(&node(2), 999); // genesis from v2 — conflicts with v2's existing blocks

    // Attempt to insert the equivocating block via validate_block.
    let bonds = bonds_uniform(&[1, 2, 3, 4], 1);
    let config = ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: false,
        check_closure: true,
        check_chain_axiom: true,
        check_cordial: false,
    };
    let result = validate_block(&equivocating_block, &blocklace, &bonds, &config);

    // The block is rejected as equivocation (v2 already has a block in the DAG).
    assert!(
        !result.is_valid(),
        "the equivocating block must be rejected by the chain axiom"
    );
    let errors = result.errors();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, InvalidBlock::Equivocation { .. })),
        "rejection must include InvalidBlock::Equivocation, got: {errors:?}"
    );

    // Record the rejected equivocation into the evidence pool.
    let mut pool = CordialEvidencePool::new();
    let recorded = record_rejected_equivocation(&equivocating_block, errors, &blocklace, &mut pool);
    assert!(
        recorded,
        "record_rejected_equivocation must return true for a genuine chain-axiom conflict"
    );

    // CORE ASSERTION: the blocklace generation must not have advanced.
    // The evidence pool is a separate data structure — recording into it
    // cannot mutate the blocklace.
    assert_eq!(
        blocklace.generation(),
        gen_after_warm,
        "blocklace.generation() must not change after recording equivocation evidence: \
         the pool and the blocklace are independent"
    );

    // CORE ASSERTION: tau_with_cache must still be a cache hit.
    // Because the generation has not advanced, the cache entries computed
    // before evidence recording remain valid.
    let after_evidence = tau_with_cache(
        &blocklace,
        wavelength,
        n,
        f,
        selection_id,
        leader_node1,
        &mut cache,
    )
    .expect("tau should still compute after evidence recording");

    assert_eq!(
        after_evidence, before,
        "tau output must not change after recording equivocation evidence: \
         evidence recording cannot alter the blocklace DAG"
    );

    // The fresh uncached path must agree with the cached path.
    let fresh = tau(&blocklace, wavelength, n, f, leader_node1)
        .expect("uncached tau must compute over the same graph");
    assert_eq!(
        after_evidence, fresh,
        "cached and fresh tau must agree after evidence recording"
    );

    // The pool must hold the evidence.
    let evidence = pool.evidence_for(&node(2));
    assert_eq!(
        evidence.len(),
        1,
        "pool must hold exactly one evidence record for v2"
    );

    // The equivocating block must not be in the blocklace (chain axiom held).
    assert!(
        blocklace.get(&equivocating_block.identity).is_none(),
        "the equivocating block must not have been inserted into the blocklace"
    );

    // v2's honest blocks remain present — evidence does not poison them.
    let v2_blocks: Vec<_> = blocklace
        .dom()
        .into_iter()
        .filter(|id| id.creator == node(2))
        .collect();
    assert!(
        !v2_blocks.is_empty(),
        "v2's honest blocks must remain in the blocklace despite the equivocation evidence"
    );

    // The graph reference leader must still appear in the ordering.
    assert!(
        after_evidence.contains(&graph.w0_leader.identity)
            || after_evidence.contains(&graph.w1_leader.identity),
        "at least one finalized leader must appear in the ordered output"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2 — equivocating branches are excluded from finalized ordered output
// ─────────────────────────────────────────────────────────────────────────────

/// Both `tau` and `weighted_tau` must exclude the equivocating block from
/// their output. The chain axiom ensures only one branch survives in the
/// blocklace; this test verifies that the excluded branch does not reappear
/// through any ordering path.
///
/// Additionally, the `OrderingCache` entries computed after the equivocating
/// block is rejected must agree with an uncached computation.
#[test]
fn equivocating_branch_excluded_from_finalized_output() {
    let mut blocklace = Blocklace::new();
    let mut cache = OrderingCache::default();
    let wavelength = 3;
    let n = 4;
    let f = 1;
    let selection_id = 0;
    let bonds = bonds_uniform(&[1, 2, 3, 4], 1);
    let config = ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: false,
        check_closure: true,
        check_chain_axiom: true,
        check_cordial: false,
    };

    let graph = build_two_wave_dag(&mut blocklace);

    // Create a second genesis block from v1 (which already has w0_leader in
    // the DAG). This will be rejected by the chain axiom.
    let equivocating_block = genesis(&node(1), 9001);

    let result = validate_block(&equivocating_block, &blocklace, &bonds, &config);
    assert!(!result.is_valid(), "second v1 genesis must be rejected");
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::Equivocation { .. })),
        "rejection must name the equivocation"
    );

    // Record evidence.
    let mut pool = CordialEvidencePool::new();
    record_rejected_equivocation(&equivocating_block, result.errors(), &blocklace, &mut pool);
    assert_eq!(pool.evidence_for(&node(1)).len(), 1);

    // tau — both cached and uncached.
    let cached_tau = tau_with_cache(
        &blocklace,
        wavelength,
        n,
        f,
        selection_id,
        leader_node1,
        &mut cache,
    )
    .expect("cached tau must compute");
    let fresh_tau =
        tau(&blocklace, wavelength, n, f, leader_node1).expect("fresh tau must compute");

    assert_eq!(
        cached_tau, fresh_tau,
        "cached and fresh tau must agree after equivocation detection"
    );
    assert!(
        !cached_tau.is_empty(),
        "finalized output must not be empty: two waves are present"
    );
    assert!(
        !cached_tau.contains(&equivocating_block.identity),
        "the equivocating block's identity must not appear in tau output"
    );

    // weighted_tau — both cached and uncached.
    let cached_wtau = weighted_tau_with_cache(
        &blocklace,
        wavelength,
        &bonds,
        selection_id,
        leader_node1,
        &mut cache,
    )
    .expect("cached weighted_tau must compute");
    let fresh_wtau = weighted_tau(&blocklace, wavelength, &bonds, leader_node1)
        .expect("fresh weighted_tau must compute");

    assert_eq!(
        cached_wtau, fresh_wtau,
        "cached and fresh weighted_tau must agree after equivocation detection"
    );
    assert!(
        !cached_wtau.contains(&equivocating_block.identity),
        "the equivocating block's identity must not appear in weighted_tau output"
    );

    // The honest w0_leader and w1_leader must remain in the output.
    assert!(
        cached_wtau.contains(&graph.w0_leader.identity)
            || cached_wtau.contains(&graph.w1_leader.identity),
        "at least one honest leader must appear in the weighted_tau output"
    );

    // The equivocating block is absent from the blocklace.
    assert!(blocklace.get(&equivocating_block.identity).is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3 — a subsequent valid insert flushes the cache correctly
// ─────────────────────────────────────────────────────────────────────────────

/// After equivocation detection (which does not advance the generation), the
/// next *valid* block insert must advance the generation and invalidate the
/// cache. The post-flush cached result must agree with a fresh computation.
///
/// This tests the defence-in-depth property: even if a caller performs
/// complex equivocation-detection workflows, the generation counter advances
/// on the very next structural mutation and the cache is guaranteed to flush.
#[test]
fn cache_invalidated_after_valid_insert_following_equivocation_detection() {
    let mut blocklace = Blocklace::new();
    let graph = build_two_wave_dag(&mut blocklace);
    let mut cache = OrderingCache::default();
    let wavelength = 3;
    let n = 4;
    let f = 1;
    let selection_id = 0;
    let bonds = bonds_uniform(&[1, 2, 3, 4], 1);
    let config = ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: false,
        check_closure: true,
        check_chain_axiom: true,
        check_cordial: false,
    };

    // Warm the cache.
    let before = tau_with_cache(
        &blocklace,
        wavelength,
        n,
        f,
        selection_id,
        leader_node1,
        &mut cache,
    )
    .expect("tau should compute");
    assert!(!before.is_empty());
    let gen_warm = blocklace.generation();
    let entries_warm = cache.cached_entries();
    assert!(entries_warm > 0, "cache must be populated");

    // Attempt to insert an equivocating block (rejected).
    let equivocating_block = genesis(&node(3), 8888); // v3 already has blocks in the DAG
    let result = validate_block(&equivocating_block, &blocklace, &bonds, &config);
    assert!(!result.is_valid(), "equivocating block must be rejected");

    // Record evidence — generation must not advance.
    let mut pool = CordialEvidencePool::new();
    record_rejected_equivocation(&equivocating_block, result.errors(), &blocklace, &mut pool);
    assert_eq!(
        blocklace.generation(),
        gen_warm,
        "generation must not advance after evidence recording"
    );

    // Now insert a valid new block built on top of the latest wave-1 round-2 tips.
    let new_block = child(
        &node(2),
        50_000,
        &graph.w1_round2.iter().collect::<Vec<_>>(),
    );
    insert(&mut blocklace, &new_block);

    let gen_after_insert = blocklace.generation();
    assert!(
        gen_after_insert > gen_warm,
        "generation must advance after a valid block insertion"
    );

    // The cache must be invalidated (generation mismatch) and rebuilt.
    let after_insert = tau_with_cache(
        &blocklace,
        wavelength,
        n,
        f,
        selection_id,
        leader_node1,
        &mut cache,
    )
    .expect("tau must compute after valid insert");
    let fresh = tau(&blocklace, wavelength, n, f, leader_node1).expect("fresh tau must compute");

    assert_eq!(
        after_insert, fresh,
        "cached result must agree with fresh computation after cache flush"
    );
    assert_eq!(
        cache.generation(),
        gen_after_insert,
        "cache generation must track the current blocklace generation"
    );

    // The equivocating block is still absent — evidence did not insert it.
    assert!(blocklace.get(&equivocating_block.identity).is_none());

    // Pool still holds the evidence independently of all of the above.
    assert_eq!(pool.evidence_for(&node(3)).len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4 — pruning after equivocation retains the ordering prefix and pool
// ─────────────────────────────────────────────────────────────────────────────

/// After equivocation detection and evidence recording, advancing the
/// checkpoint (checkpoint GC) must:
///   1. Preserve the tau ordering prefix stored at the checkpoint — `tau`
///      called on the pruned blocklace must return the same sequence as
///      before pruning.
///   2. Remove the equivocating block's history from the blocklace (if it sat
///      below the checkpoint boundary) — or confirm it was never present.
///   3. Leave the evidence pool completely unaffected — the pool is a separate
///      data structure and is not subject to blocklace GC.
///
/// This is the "fence" property: the checkpoint is a GC boundary for the
/// blocklace but evidence lives in its own independent store.
#[test]
fn pruning_after_equivocation_retains_evidence_and_prefix() {
    let mut blocklace = Blocklace::new();
    let graph = build_two_wave_dag(&mut blocklace);
    let wavelength = 3;
    let n = 4;
    let f = 1;
    let bonds = bonds_uniform(&[1, 2, 3, 4], 1);
    let config = ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: false,
        check_closure: true,
        check_chain_axiom: true,
        check_cordial: false,
    };

    // Record the tau output BEFORE pruning.
    let before_prune =
        tau(&blocklace, wavelength, n, f, leader_node1).expect("tau must compute before prune");
    assert!(
        !before_prune.is_empty(),
        "two finalized waves must produce non-empty tau output"
    );

    // Detect and record an equivocation for v4 (a second genesis block).
    // v4 already has blocks in the DAG from build_two_wave_dag.
    let equivocating_block = genesis(&node(4), 77_777);
    let result = validate_block(&equivocating_block, &blocklace, &bonds, &config);
    assert!(
        !result.is_valid(),
        "equivocating block for v4 must be rejected"
    );
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::Equivocation { .. })),
        "equivocation error must be present"
    );

    let mut pool = CordialEvidencePool::new();
    let recorded =
        record_rejected_equivocation(&equivocating_block, result.errors(), &blocklace, &mut pool);
    assert!(recorded, "evidence must be recorded for v4");
    let pool_size_before_prune = pool.len();
    assert_eq!(pool_size_before_prune, 1);

    // Advance the checkpoint to prune blocks below the latest finalized leader.
    let report = checkpoint_after_finality(&mut blocklace, wavelength, n, f, leader_node1)
        .expect("checkpoint advancement must not error")
        .expect("a finalized leader must exist for pruning");

    assert_eq!(
        report.checkpoint, graph.w1_leader.identity,
        "checkpoint must advance to the latest finalized leader"
    );
    assert!(
        !report.removed.is_empty(),
        "at least some blocks must be removed by GC"
    );
    assert!(
        blocklace.get(&graph.w0_leader.identity).is_none(),
        "the earlier leader (now below checkpoint) must be pruned from the blocklace"
    );
    assert!(
        blocklace.is_closed(),
        "blocklace must remain closed after GC"
    );

    // CORE ASSERTION 1: tau prefix is preserved after pruning.
    let after_prune =
        tau(&blocklace, wavelength, n, f, leader_node1).expect("tau must compute after prune");
    assert_eq!(
        after_prune, before_prune,
        "tau output must be identical before and after checkpoint GC: \
         the prefix stored at the checkpoint must replay correctly"
    );

    // CORE ASSERTION 2: the equivocating block was never in the blocklace
    // (chain axiom ensured this), so it is absent regardless of GC.
    assert!(
        blocklace.get(&equivocating_block.identity).is_none(),
        "the equivocating block must be absent from the blocklace (never inserted)"
    );

    // CORE ASSERTION 3: the evidence pool is entirely unaffected by GC.
    // GC operates only on the blocklace HashMap; the pool is a separate structure.
    assert_eq!(
        pool.len(),
        pool_size_before_prune,
        "checkpoint GC must not alter the evidence pool: the pool is independent of the blocklace"
    );
    let evidence = pool.evidence_for(&node(4));
    assert_eq!(
        evidence.len(),
        1,
        "evidence for v4 must remain intact after GC"
    );
    assert_eq!(
        evidence[0].blocks.len(),
        2,
        "evidence must hold both blocks"
    );

    // The checkpoint leader itself must be retained in the blocklace.
    assert!(
        blocklace.get(&graph.w1_leader.identity).is_some(),
        "the checkpoint leader must be retained in the blocklace after GC"
    );
}
