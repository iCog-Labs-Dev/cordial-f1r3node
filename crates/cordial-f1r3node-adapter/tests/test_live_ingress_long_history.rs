//! Long-history check for the finalized-output path used by the PoR shadow
//! observer (#263).
//!
//! `PorRuntime::publish_finalized_output()` calls
//! `LiveIngress::latest_finalized_ordered_output`, which is what the observer
//! runs after mirroring a node's blocks. This test feeds a synthetic history
//! through the same public `LiveIngress` API, with no gRPC node, and checks
//! that a finalized output with an anchor is produced.
//!
//! The small test runs in the normal suite. The large one is `#[ignore]` and
//! sized from `LONG_HISTORY_WAVES` (default 1067 waves, about 3,200 heights and
//! 12,800 blocks, the scale of the failed run):
//!
//! ```text
//! cargo test --release -p cordial-f1r3node-adapter \
//!     --test test_live_ingress_long_history -- --ignored --nocapture
//! ```

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter, live_ingress::LiveIngress,
    ordered_output::OrderedFinalizedOutput, shard_conf::CasperShardConf,
    snapshot::CORDIAL_WAVELENGTH,
};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

const VALIDATORS: u16 = 4;

struct AcceptingAdapter;

impl BlocklaceAdapter<BlockIdentity> for AcceptingAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn node(id: u16) -> NodeId {
    NodeId(id.to_le_bytes().to_vec())
}

fn make_block(creator: &NodeId, tag: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0..8].copy_from_slice(&tag.to_le_bytes());
    content_hash[8] = creator.0[0];
    content_hash[9] = creator.0[1];
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: creator.clone(),
            signature: tag.to_le_bytes().to_vec(),
        },
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors,
        },
    }
}

/// Mirror `waves` waves of a fully connected history, then compute the
/// finalized output. Returns the output, the number of blocks mirrored, the
/// time spent ingesting, and the time spent computing the output.
fn finalized_output_at(waves: u64) -> (OrderedFinalizedOutput, usize, Duration, Duration) {
    let validators: Vec<NodeId> = (0..VALIDATORS).map(node).collect();
    let bonds: HashMap<NodeId, u64> = validators.iter().map(|v| (v.clone(), 100)).collect();
    let mut ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        bonds,
        CasperShardConf::default(),
        "root",
    );

    let ingest_started = Instant::now();
    let mut previous_round: Vec<Block> = Vec::new();
    let mut tag = 1u64;
    let mut mirrored = 0usize;
    for _round in 0..=(waves * CORDIAL_WAVELENGTH) {
        let predecessors: HashSet<BlockIdentity> = previous_round
            .iter()
            .map(|block| block.identity.clone())
            .collect();
        let mut this_round = Vec::with_capacity(validators.len());
        for validator in &validators {
            let block = make_block(validator, tag, predecessors.clone());
            tag += 1;
            ingress
                .ingest_trusted_block(block.clone())
                .expect("synthetic block should mirror");
            mirrored += 1;
            this_round.push(block);
        }
        previous_round = this_round;
    }
    let ingest = ingest_started.elapsed();

    let publish_started = Instant::now();
    let output = ingress
        .latest_finalized_ordered_output(CORDIAL_WAVELENGTH)
        .expect("finalized output should be computed");
    (output, mirrored, ingest, publish_started.elapsed())
}

fn rss_mib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .map_or(0, |kib| kib / 1024)
}

#[test]
fn finalized_output_is_published_on_a_short_history() {
    let (output, mirrored, _, _) = finalized_output_at(20);

    assert!(output.anchor.is_some(), "a finalized anchor is expected");
    assert!(!output.is_empty(), "finalized blocks are expected");
    assert_eq!(output.total_mirrored_blocks, mirrored);
}

#[test]
#[ignore = "large history; run with --ignored --nocapture (about 15 minutes in release)"]
fn finalized_output_is_published_on_a_failed_run_sized_history() {
    let waves: u64 = std::env::var("LONG_HISTORY_WAVES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1067);

    let (output, mirrored, ingest, publish) = finalized_output_at(waves);

    println!(
        "waves={waves} heights={} mirrored_blocks={mirrored} finalized_blocks={} \
         ingest_ms={} publish_ms={} rss_hwm_mib={}",
        waves * CORDIAL_WAVELENGTH,
        output.len(),
        ingest.as_millis(),
        publish.as_millis(),
        rss_mib(),
    );

    assert!(output.anchor.is_some(), "a finalized anchor is expected");
    assert!(!output.is_empty(), "finalized blocks are expected");
    assert_eq!(output.total_mirrored_blocks, mirrored);
}
