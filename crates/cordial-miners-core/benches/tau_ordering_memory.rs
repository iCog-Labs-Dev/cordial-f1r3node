//! Benchmark: peak memory and time by phase for weighted tau ordering.
//!
//! Peak RSS only grows within a process, so run one history length per
//! process:
//!
//! ```text
//! BENCH_WAVES=160 cargo bench -p cordial-miners-core --bench tau_ordering_memory
//! ```
//!
//! With 4 validators and wavelength 3, one wave is 12 blocks and 3 heights.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{OrderingCache, weighted_tau_with_cache};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

const WAVELENGTH: u64 = 3;
const VALIDATORS: usize = 4;

// ── peak-tracking allocator ──────────────────────────────────────────────────

struct PeakAlloc;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for PeakAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let current = CUR.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(current, Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CUR.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                let grown = new_size - layout.size();
                let current = CUR.fetch_add(grown, Relaxed) + grown;
                PEAK.fetch_max(current, Relaxed);
            } else {
                CUR.fetch_sub(layout.size() - new_size, Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: PeakAlloc = PeakAlloc;

fn vm_hwm_mib() -> u64 {
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

// ── history builder (same shape as benches/tau_ordering.rs) ──────────────────

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
    let mut tag: u64 = 1;

    for _ in 0..=(waves * WAVELENGTH) {
        let predecessors: HashSet<BlockIdentity> =
            prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, predecessors.clone());
            tag += 1;
            blocklace
                .insert(block.clone(), &MockVerifier)
                .expect("bench block should insert");
            this_round.push(block);
        }
        prev_round = this_round;
    }
    blocklace
}

fn main() {
    let waves: u64 = std::env::var("BENCH_WAVES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let bonds: HashMap<NodeId, u64> = (0..VALIDATORS as u16).map(|i| (node(i), 100)).collect();
    let leader = |wave: u64| Some(node((wave % VALIDATORS as u64) as u16));

    let started = Instant::now();
    let blocklace = build_blocklace(waves);
    let ingest = started.elapsed();
    let blocks = blocklace.dom().len();
    let heap_peak_ingest = PEAK.load(Relaxed);

    // Reset the peak to the live heap so the ordering peak excludes ingestion
    // garbage but still includes the blocklace itself.
    PEAK.store(CUR.load(Relaxed), Relaxed);
    let mut cache = OrderingCache::default();
    let started = Instant::now();
    let output = weighted_tau_with_cache(&blocklace, WAVELENGTH, &bonds, 0, leader, &mut cache)
        .expect("tau ordering");
    let order = started.elapsed();
    let heap_peak_order = PEAK.load(Relaxed);

    println!(
        "waves={waves} heights={} blocks={blocks} output={} ingest_ms={} order_ms={} \
         heap_peak_ingest_mib={} heap_peak_order_mib={} rss_hwm_mib={}",
        waves * WAVELENGTH,
        output.len(),
        ingest.as_millis(),
        order.as_millis(),
        heap_peak_ingest >> 20,
        heap_peak_order >> 20,
        vm_hwm_mib(),
    );
}
