//! Trace fixture generator for Issue 188.
//!
//! Runs three canonical scenarios and writes their JSON traces to the
//! `lean/traces/` directory:
//!
//! | File                  | Scenario                                      |
//! |-----------------------|-----------------------------------------------|
//! | `normal.json`         | 7-node happy-path consensus                   |
//! | `equivocation.json`   | A non-leader equivocates; finality stays safe |
//! | `low_stake.json`      | Count quorum passes but weighted quorum fails |
//!
//! Run with:
//! ```sh
//! cargo test --features trace --test generate_trace_fixtures \
//!   generate_all_fixtures -- --nocapture
//! ```
//!
//! The tests set `CORDIAL_TRACE_FILE` before exercising the consensus
//! functions so the feature-gated `trace::emit` calls land in the right file.

#![cfg(feature = "trace")]

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::cordiality::all_equivocations_for_observer;
use cordial_miners_core::consensus::ordering::weighted_tau;
use cordial_miners_core::consensus::validation::{ValidationConfig, validate_block};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::trace::TraceEvent;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

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

/// Unique 1-byte node id shorthand.
fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

/// Construct a deterministic mock block.
fn make_block(creator_id: u8, seq: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = seq;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![creator_id, seq],
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace
        .insert(block.clone(), &MockVerifier)
        .expect("insert failed");
}

/// Return the path for a trace fixture, creating parent dirs as needed.
fn fixture_path(name: &str) -> PathBuf {
    let dir = std::env::var_os("CORDIAL_TRACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent() // crates/
                .unwrap()
                .parent() // workspace root
                .unwrap()
                .join("lean")
                .join("traces")
        });
    std::fs::create_dir_all(&dir).expect("failed to create traces/ dir");
    dir.join(name)
}

#[derive(Serialize)]
struct WeightEntry {
    node_id: String,
    weight: u64,
}

#[derive(Serialize)]
struct LeaderEntry {
    wave: u64,
    node_id: String,
}

#[derive(Serialize)]
struct ReplayConfig {
    hash_algorithm: &'static str,
    weight_table_hash: String,
    wavelength: u64,
    validators: Vec<WeightEntry>,
    leaders: Vec<LeaderEntry>,
}

fn prepare_fixture(name: &str, bonds: &HashMap<NodeId, u64>, wavelength: u64) -> PathBuf {
    let path = fixture_path(name);
    std::fs::write(&path, "").expect("failed to truncate trace fixture");
    let mut validators: Vec<_> = bonds
        .iter()
        .map(|(node, weight)| WeightEntry {
            node_id: cordial_miners_core::trace::hex(&node.0),
            weight: *weight,
        })
        .collect();
    validators.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    let leaders = (0..8)
        .filter_map(|wave| {
            leader(wave).map(|node| LeaderEntry {
                wave,
                node_id: cordial_miners_core::trace::hex(&node.0),
            })
        })
        .collect();
    let config = ReplayConfig {
        hash_algorithm: "fnv1a64-v1",
        weight_table_hash: cordial_miners_core::trace::weight_table_hash(bonds),
        wavelength,
        validators,
        leaders,
    };
    let weights_path = path.with_extension("weights.json");
    std::fs::write(weights_path, serde_json::to_vec_pretty(&config).unwrap())
        .expect("failed to write replay config");
    path
}

fn read_trace_events(name: &str) -> Vec<TraceEvent> {
    std::fs::read_to_string(fixture_path(name))
        .expect("read generated trace")
        .lines()
        .map(|line| serde_json::from_str(line).expect("generated trace event is valid JSON"))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Leader selection
// ─────────────────────────────────────────────────────────────────────────────

/// Leader selection: node (wave % 7) + 1 for a 7-node committee.
fn leader(wave: u64) -> Option<NodeId> {
    Some(node(((wave % 7) + 1) as u8))
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1: normal 7-node consensus
// ─────────────────────────────────────────────────────────────────────────────

/// Build a 7-node, one-wave happy-path blocklace.
///
/// Parameters: n=7, f=2, wavelength=3.
/// - Wave 0: rounds 0, 1, 2  (leader = node 1, at round 0)
///
/// Topology: each round's blocks reference ALL blocks from the previous
/// round (fully-connected cross-round DAG). This guarantees every block
/// at round r observes the leader at round 0 and can approve it without
/// any equivocation conflict.
fn build_normal_blocklace() -> (Blocklace, HashMap<NodeId, u64>) {
    let mut b = Blocklace::new();
    let bonds: HashMap<NodeId, u64> = (1u8..=7).map(|i| (node(i), 100u64)).collect();

    let mut prev_ids: HashSet<BlockIdentity> = HashSet::new();

    // Three rounds are sufficient for approval, ratification, and
    // super-ratification of the wave-0 leader.
    for round in 0u8..3u8 {
        let round_blocks: Vec<Block> = (1u8..=7)
            .map(|creator| make_block(creator, round * 10 + creator, prev_ids.clone()))
            .collect();
        for blk in &round_blocks {
            insert(&mut b, blk);
        }
        prev_ids = round_blocks
            .iter()
            .map(|blk| blk.identity.clone())
            .collect();
    }

    (b, bonds)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2: equivocation
// ─────────────────────────────────────────────────────────────────────────────

/// Build a blocklace where non-leader node 7 equivocates at round 0 while
/// the other six validators still safely finalize node 1's wave-0 leader.
fn build_equivocation_blocklace() -> (Blocklace, HashMap<NodeId, u64>) {
    let mut b = Blocklace::new();
    let bonds: HashMap<NodeId, u64> = (1u8..=7).map(|i| (node(i), 100u64)).collect();

    // Node 7 creates TWO incomparable genesis blocks → equivocation at round 0.
    let e1a = make_block(7, 7, HashSet::new());
    let e1b = make_block(7, 8, HashSet::new());
    let r0_others: Vec<Block> = (1u8..=6)
        .map(|id| make_block(id, id, HashSet::new()))
        .collect();

    insert(&mut b, &e1a);
    insert(&mut b, &e1b);
    for blk in &r0_others {
        insert(&mut b, blk);
    }

    // Validate node 2's genesis — this is a real validation execution point.
    let config = ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        ..ValidationConfig::default()
    };
    let _res = validate_block(&r0_others[0], &b, &bonds, &config);

    // Rounds 1 and 2: honest nodes acknowledge both branches.  Node 7 is
    // excluded, but the six honest validators retain >2/3 of the stake.
    let mut r0_all: HashSet<BlockIdentity> = [e1a.identity.clone(), e1b.identity.clone()].into();
    for blk in &r0_others {
        r0_all.insert(blk.identity.clone());
    }

    let r1: Vec<Block> = (1u8..=6)
        .map(|id| make_block(id, id + 10, r0_all.clone()))
        .collect();
    for blk in &r1 {
        insert(&mut b, blk);
    }

    let r1_ids: HashSet<BlockIdentity> = r1.iter().map(|block| block.identity.clone()).collect();
    let r2: Vec<Block> = (1u8..=6)
        .map(|id| make_block(id, id + 20, r1_ids.clone()))
        .collect();
    for blk in &r2 {
        insert(&mut b, blk);
    }

    (b, bonds)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3: low-stake ratifier (weighted mode)
// ─────────────────────────────────────────────────────────────────────────────

/// Build a 7-node blocklace in which five validators form a count quorum but
/// carry only about one third of the stake.  This is the regression scenario
/// that distinguishes weighted finality from validator counting.
fn build_low_stake_blocklace() -> (Blocklace, HashMap<NodeId, u64>) {
    let mut b = Blocklace::new();
    // Nodes 1–3 carry almost all stake; nodes 4–7 have one unit each.
    let bonds: HashMap<NodeId, u64> = (1u8..=3)
        .map(|i| (node(i), 1000u64))
        .chain((4u8..=7).map(|i| (node(i), 1u64)))
        .collect();

    let r0: Vec<Block> = (1u8..=7)
        .map(|id| make_block(id, id, HashSet::new()))
        .collect();
    for block in &r0 {
        insert(&mut b, block);
    }

    // Five validators vote by count, but only node 1 among the high-stake
    // validators participates: support=1004 of total=3004.
    let voters = [1u8, 4, 5, 6, 7];
    let r0_ids: HashSet<BlockIdentity> = r0.iter().map(|block| block.identity.clone()).collect();
    let r1: Vec<Block> = voters
        .iter()
        .map(|id| make_block(*id, *id + 10, r0_ids.clone()))
        .collect();
    for block in &r1 {
        insert(&mut b, block);
    }
    let r1_ids: HashSet<BlockIdentity> = r1.iter().map(|block| block.identity.clone()).collect();
    let r2: Vec<Block> = voters
        .iter()
        .map(|id| make_block(*id, *id + 20, r1_ids.clone()))
        .collect();
    for block in &r2 {
        insert(&mut b, block);
    }

    (b, bonds)
}

/// Four of seven equally weighted validators participate through four rounds.
/// A simple majority accepts both ratification levels, while the protocol's
/// strict two-thirds rule rejects them. This fixture is generated only by the
/// deliberately mutated Rust build.
#[cfg(feature = "trace-threshold-mutation")]
fn build_weakened_threshold_blocklace() -> (Blocklace, HashMap<NodeId, u64>) {
    let mut b = Blocklace::new();
    let bonds: HashMap<NodeId, u64> = (1u8..=7).map(|id| (node(id), 100u64)).collect();
    let voters = [1u8, 2, 3, 4];

    let leader_block = make_block(1, 1, HashSet::new());
    insert(&mut b, &leader_block);
    let mut previous: HashSet<BlockIdentity> = [leader_block.identity].into();

    for round in 1u8..=3 {
        let blocks: Vec<Block> = voters
            .iter()
            .map(|creator| make_block(*creator, round * 10 + *creator, previous.clone()))
            .collect();
        for block in &blocks {
            insert(&mut b, block);
        }
        previous = blocks.iter().map(|block| block.identity.clone()).collect();
    }

    (b, bonds)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test entry points
// ─────────────────────────────────────────────────────────────────────────────

fn generate_normal_fixture() {
    let bonds: HashMap<NodeId, u64> = (1u8..=7).map(|i| (node(i), 100u64)).collect();
    let path = prepare_fixture("normal.json", &bonds, 3);
    // SAFETY: tests are single-threaded here; set_var is safe in this context.
    unsafe {
        std::env::set_var("CORDIAL_TRACE_FILE", path.to_str().unwrap());
    }
    let (blocklace, _) = build_normal_blocklace();

    let wavelength = 3;

    // weighted_tau exercises finality, certificates, approvals, ordering, and
    // output from actual consensus functions.
    let output = weighted_tau(&blocklace, wavelength, &bonds, leader).unwrap_or_default();

    println!("[normal] trace written to {}", path.display());
    println!(
        "[normal] final leader: {:?}",
        output
            .first()
            .map(|id| cordial_miners_core::trace::hex(&id.content_hash))
    );
    println!("[normal] output prefix length: {}", output.len());

    assert!(
        !output.is_empty(),
        "Expected finalized output in the normal scenario"
    );
}

fn generate_equivocation_fixture() {
    let bonds: HashMap<NodeId, u64> = (1u8..=7).map(|i| (node(i), 100u64)).collect();
    let path = prepare_fixture("equivocation.json", &bonds, 3);
    // SAFETY: single-threaded test context.
    unsafe {
        std::env::set_var("CORDIAL_TRACE_FILE", path.to_str().unwrap());
    }
    let (blocklace, _) = build_equivocation_blocklace();

    // Trigger equivocation detection — emits DetectEquivocation events.
    // Model node 01's local view explicitly: node_id is the observer and
    // equivocation.creator is the validator that produced the conflicting blocks.
    let equivocations = all_equivocations_for_observer(&blocklace, &node(1));
    use cordial_miners_core::consensus::finality::latest_weighted_final_leader;
    let final_leader = latest_weighted_final_leader(&blocklace, 3, &bonds, leader);

    println!("[equivocation] trace written to {}", path.display());
    println!(
        "[equivocation] detected {} equivocation(s)",
        equivocations.len()
    );

    // Sanity: node 7 is excluded and the honest leader remains finalizable.
    assert!(
        equivocations.iter().any(|e| e.creator == node(7)),
        "Expected node 7 to be detected as an equivocator"
    );
    assert_eq!(
        equivocations[0].round, 0,
        "Equivocation should be at round 0"
    );
    assert_eq!(
        final_leader.as_ref().map(|id| id.creator.clone()),
        Some(node(1)),
        "Equivocation must not prevent the unique honest leader from finalizing"
    );
}

fn generate_low_stake_fixture() {
    let bonds: HashMap<NodeId, u64> = (1u8..=3)
        .map(|i| (node(i), 1000u64))
        .chain((4u8..=7).map(|i| (node(i), 1u64)))
        .collect();
    let path = prepare_fixture("low_stake.json", &bonds, 3);
    // SAFETY: single-threaded test context.
    unsafe {
        std::env::set_var("CORDIAL_TRACE_FILE", path.to_str().unwrap());
    }
    let (blocklace, _) = build_low_stake_blocklace();

    let wavelength = 3;

    // Exercise the weighted finality path — emits ComputeFinality events.
    use cordial_miners_core::consensus::finality::latest_weighted_final_leader;
    let final_leader = latest_weighted_final_leader(&blocklace, wavelength, &bonds, leader);
    use cordial_miners_core::consensus::finality::latest_final_leader;
    let count_final_leader = latest_final_leader(&blocklace, wavelength, 7, 2, leader);

    println!("[low_stake] trace written to {}", path.display());
    println!(
        "[low_stake] weighted final leader: {}",
        final_leader
            .as_ref()
            .map(|id| cordial_miners_core::trace::hex(&id.content_hash))
            .unwrap_or_else(|| "none".into())
    );

    // Independent scenario oracle: validator counting says final, stake says no.
    assert!(
        count_final_leader.is_some(),
        "Expected five voters to satisfy the count quorum"
    );
    assert!(
        final_leader.is_none(),
        "A low-stake count quorum must not satisfy weighted finality"
    );
}

/// Convenience test that generates all three fixtures in one run.
#[test]
fn generate_all_fixtures() {
    // SAFETY: this target's canonical generator runs on one test thread.
    // Every runtime emission must succeed, not merely enough to satisfy the
    // fixture's event-count assertions.
    unsafe { std::env::set_var("CORDIAL_TRACE_STRICT", "1") };
    generate_normal_fixture();
    generate_equivocation_fixture();
    generate_low_stake_fixture();

    // These are the safety-relevant events produced by the actual consensus
    // calls above. The remaining transport/scheduler/lifecycle boundaries are
    // asserted by `trace_runtime_instrumentation`.
    let events: Vec<TraceEvent> = ["normal.json", "equivocation.json", "low_stake.json"]
        .into_iter()
        .flat_map(read_trace_events)
        .collect();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::InsertBlock(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::ValidateBlock(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::DetectEquivocation(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::AcceptApproval(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::BuildThresholdCertificate(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::ComputeFinality(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::RunTauOrder(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::EmitOutput(_)))
    );

    let names = [
        "normal.json",
        "normal.weights.json",
        "equivocation.json",
        "equivocation.weights.json",
        "low_stake.json",
        "low_stake.weights.json",
    ];
    let first: Vec<_> = names
        .iter()
        .map(|name| std::fs::read(fixture_path(name)).expect("read first fixture generation"))
        .collect();

    // Generate the identical executions again in the same process. This
    // catches randomized HashSet traversal in event order as part of the
    // ordinary fixture command instead of relying on a visual diff in CI.
    generate_normal_fixture();
    generate_equivocation_fixture();
    generate_low_stake_fixture();
    for (name, expected) in names.iter().zip(first) {
        let actual = std::fs::read(fixture_path(name)).expect("read second fixture generation");
        assert_eq!(actual, expected, "fixture {name} is not deterministic");
    }

    unsafe { std::env::remove_var("CORDIAL_TRACE_FILE") };
    unsafe { std::env::remove_var("CORDIAL_TRACE_STRICT") };
    println!("\n✓ All fixture traces are deterministic and written to lean/traces/");
}

/// Generate a trace using the actual Rust finality pipeline compiled with the
/// deliberately weakened `2 * support > total` predicate. The surrounding
/// shell harness feeds this trace to the unchanged Lean CMRef and requires a
/// first-mismatch rejection.
#[cfg(feature = "trace-threshold-mutation")]
#[test]
fn generate_weakened_threshold_fixture() {
    let bonds: HashMap<NodeId, u64> = (1u8..=7).map(|id| (node(id), 100u64)).collect();
    let path = prepare_fixture("weakened_threshold.json", &bonds, 4);
    // SAFETY: this integration-test target is run with one test thread.
    unsafe {
        std::env::set_var("CORDIAL_TRACE_FILE", path.to_str().unwrap());
        std::env::set_var("CORDIAL_TRACE_STRICT", "1");
    }

    let (blocklace, _) = build_weakened_threshold_blocklace();
    use cordial_miners_core::consensus::finality::latest_weighted_final_leader;
    let mutated_result = latest_weighted_final_leader(&blocklace, 4, &bonds, leader);

    unsafe { std::env::remove_var("CORDIAL_TRACE_FILE") };
    unsafe { std::env::remove_var("CORDIAL_TRACE_STRICT") };
    assert!(
        mutated_result.is_some(),
        "the deliberately weakened Rust threshold must finalize four of seven validators"
    );
    println!(
        "[mutation/weakened-threshold] actual Rust execution wrote {}",
        path.display()
    );
}
