//! HTTP server binary for ordered finalized output.
//!
//! This binary connects to a running f1r3node over gRPC, mirrors recent blocks
//! into a **single long-lived** local Cordial blocklace, computes the finalized
//! ordered output, and serves it over a lightweight HTTP API:
//!
//! | Endpoint                        | Description                             |
//! |---------------------------------|-----------------------------------------|
//! | `GET /ordered-output/latest`    | Full `OrderedFinalizedOutput` as JSON   |
//! | `GET /ordered-output/status`    | Lightweight status (no block list)      |
//!
//! ## Usage
//!
//! ```bash
//! cargo run --bin ordered_output_server -- \
//!   --grpc-url http://127.0.0.1:40401 \
//!   --addr     127.0.0.1:7080
//! ```
//!
//! Then inspect:
//!
//! ```bash
//! curl http://127.0.0.1:7080/ordered-output/latest
//! curl http://127.0.0.1:7080/ordered-output/status
//! ```
//!
//! ## Startup sequence
//!
//! 1. Connect to f1r3node gRPC endpoint.
//! 2. Fetch `--depth` recent blocks and mirror them into a [`LiveIngress`].
//!    Bonds are derived once from this bootstrap window and only ever grow.
//! 3. Compute initial [`OrderedFinalizedOutput`], store in [`SharedOrderedOutput`].
//! 4. Spawn background task: poll gRPC every `--poll-interval-ms` ms, append
//!    new blocks into the **same** long-lived `LiveIngress` (duplicates are
//!    automatically discarded by the blocklace), update the bond map with any
//!    newly observed sender, recompute ordered output, update shared state.
//! 5. Start axum listener on `--addr`.
//!
//! The server never panics on startup if no finalized output exists yet — the
//! handlers return `503` until the first finalized leader is observed.
//!
//! ## Bonds
//!
//! Real stake weights are preferred. Pass `--bonds-file` (the same
//! `<public_key> <stake>` file the node reads with `--bonds-file`) and the
//! server seeds its bond map from it via `BondsParser`, exactly matching the
//! node's consensus view. Without it, the server falls back to a **uniform
//! weight of 100 per observed sender** so a read-only mirror can still compute
//! finality without knowing the genesis state.
//!
//! The bond map is bootstrapped once and **never shrinks**. Each poll cycle
//! merges newly observed senders into the existing bond map (existing entries,
//! including any seeded from `--bonds-file`, keep their weight). This means
//! validators that temporarily stop appearing in the recent window do not cause
//! bond_count to drop, preventing spurious finality-threshold changes.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use casper::rust::util::bonds_parser::BondsParser;
use clap::Parser;
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_grpc::{
    LiveGrpcBlockClient, reconcile_trusted_predecessor_creators,
    trusted_block_from_light_block_info_with_options,
};
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::ordered_output_server::{PollOutcome, poll_once, serve};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_f1r3node_adapter::shared_ordered_output::SharedOrderedOutput;
use cordial_miners_core::Block;
use cordial_miners_core::types::{BlockIdentity, NodeId};
use models::rust::string_ops::StringOps;
use tokio::sync::Mutex;

// ── CLI args ──────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "ordered-output-server")]
#[command(
    about = "Serve the latest Cordial finalized ordered output over HTTP (GET /ordered-output/latest and /ordered-output/status)"
)]
struct Args {
    /// f1r3node gRPC endpoint to mirror.
    #[arg(long, default_value = "http://127.0.0.1:40401")]
    grpc_url: String,

    /// HTTP bind address for the ordered-output API.
    #[arg(long, default_value = "127.0.0.1:7080")]
    addr: SocketAddr,

    /// Block history depth to fetch on startup and on each poll cycle.
    #[arg(long, default_value_t = 100)]
    depth: i32,

    /// Consensus wavelength for tau ordering.
    #[arg(long, default_value_t = 3)]
    wave_length: u64,

    /// How often (in milliseconds) the background task polls gRPC for new blocks.
    #[arg(long, default_value_t = 2000)]
    poll_interval_ms: u64,

    /// Shard to observe.
    #[arg(long, default_value = "root")]
    shard_id: String,

    /// Age in seconds before `/ordered-output/status` reports `is_stale: true`.
    #[arg(long, default_value_t = 30)]
    stale_threshold_secs: u64,

    /// Path to a bonds file (same `<public_key> <stake>` format used by the
    /// node's `--bonds-file`). Seeded into the bond map with real stake
    /// weights; when omitted, every observed sender is weighted uniformly at
    /// 100 so a read-only mirror can still compute finality.
    #[arg(long)]
    bonds_file: Option<PathBuf>,
}

// ── Passthrough adapter ───────────────────────────────────────────────────────

/// Minimal adapter that discards block-level side-effects.
///
/// The ordered-output server only reads consensus state; it does not need to
/// write to any external store on each block ingestion.
struct PassthroughAdapter;

impl BlocklaceAdapter<BlockIdentity> for PassthroughAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

// ── Startup helpers ───────────────────────────────────────────────────────────

/// Load a bond map from a `<public_key> <stake>` bonds file, mirroring the
/// node's `--bonds-file` handling (non-positive stakes are skipped).
fn load_bonds_from_file(path: &Path) -> Result<HashMap<NodeId, u64>> {
    let parsed = BondsParser::parse(path)
        .with_context(|| format!("failed to parse bonds file {}", path.display()))?;
    let mut bonds = HashMap::new();
    for (public_key, stake) in parsed {
        if stake > 0 {
            bonds.insert(NodeId(public_key.bytes.to_vec()), stake as u64);
        }
    }
    Ok(bonds)
}

/// Merge newly observed block senders into an existing bond map.
///
/// Senders already present keep their weight (real stakes seeded from
/// `--bonds-file`, or a previously inserted sender). New senders are assigned
/// the uniform fallback weight of 100. The map only grows — validators that
/// drop out of the recent window keep their bond entry so that finality
/// thresholds remain stable across poll cycles.
fn merge_bonds(bonds: &mut HashMap<NodeId, u64>, blocks: &[models::casper::LightBlockInfo]) {
    for block in blocks {
        if let Some(sender) = StringOps::decode_hex(block.sender.clone())
            && !sender.is_empty()
        {
            bonds.entry(NodeId(sender)).or_insert(100);
        }
    }
}

/// Mirror a slice of `LightBlockInfo` into `ingress`.
///
/// Blocks already known to the blocklace are silently skipped (the underlying
/// mirror returns `MirrorDisposition::Duplicate`). Returns the number of blocks
/// that were *newly* applied (not duplicates).
fn mirror_blocks(
    ingress: &mut LiveIngress<PassthroughAdapter>,
    blocks: &[models::casper::LightBlockInfo],
    known_creators: &mut HashMap<[u8; 32], NodeId>,
) -> Result<usize> {
    let mut applied = 0;
    let mut translated = Vec::with_capacity(blocks.len());
    for info in blocks {
        let block = trusted_block_from_light_block_info_with_options(info, true)
            .with_context(|| format!("failed to reconstruct trusted block {}", info.block_hash))?;
        if let Some(creator) = known_creators.get(&block.identity.content_hash) {
            if creator != &block.identity.creator {
                bail!("live block {} changed creator", info.block_hash);
            }
        } else {
            known_creators.insert(block.identity.content_hash, block.identity.creator.clone());
        }
        translated.push(block);
    }
    for (info, mut block) in blocks.iter().zip(translated) {
        reconcile_trusted_predecessor_creators(&mut block, known_creators);
        let update = ingress
            .ingest_trusted_block(block)
            .with_context(|| format!("failed to mirror live block {}", info.block_hash))?;
        use cordial_f1r3node_adapter::live_ingress::MirrorDisposition;
        if update.disposition != MirrorDisposition::Duplicate {
            applied += 1;
        }
    }
    Ok(applied)
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt::init();

    let shard_conf = CasperShardConf {
        shard_name: args.shard_id.clone(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let ingress = LiveIngress::with_consensus_view(
        PassthroughAdapter,
        HashMap::new(),
        shard_conf,
        &args.shard_id,
    );

    let shared = Arc::new(Mutex::new(SharedOrderedOutput::new()));
    let ingress = Arc::new(Mutex::new(ingress));
    let initial_bonds = match &args.bonds_file {
        Some(path) => {
            let bonds = load_bonds_from_file(path)
                .with_context(|| format!("failed to load bonds from {}", path.display()))?;
            tracing::info!(path = %path.display(), count = bonds.len(), "loaded bonds from file");
            bonds
        }
        None => {
            tracing::info!("no --bonds-file provided; uniform weight 100 per observed sender");
            HashMap::new()
        }
    };
    let bonds = Arc::new(Mutex::new(initial_bonds));

    // ── Background polling task ───────────────────────────────────────────────
    {
        let shared = Arc::clone(&shared);
        let ingress = Arc::clone(&ingress);
        let bonds = Arc::clone(&bonds);
        let grpc_url = args.grpc_url.clone();
        let wave_length = args.wave_length;
        let poll_interval = Duration::from_millis(args.poll_interval_ms);
        let depth = args.depth;

        tokio::spawn(async move {
            let mut client: Option<LiveGrpcBlockClient> = None;
            let mut bootstrapped = false;
            let mut known_creators: HashMap<[u8; 32], NodeId> = HashMap::new();

            loop {
                // Ensure gRPC client is connected
                if client.is_none() {
                    match LiveGrpcBlockClient::connect(grpc_url.clone()).await {
                        Ok(c) => {
                            tracing::info!(grpc_url = %grpc_url, "connected to f1r3node gRPC");
                            client = Some(c);
                        }
                        Err(err) => {
                            tracing::debug!(error = %err, "waiting to connect to f1r3node gRPC...");
                            tokio::time::sleep(poll_interval).await;
                            continue;
                        }
                    }
                }

                let c = client.as_mut().unwrap();
                let blocks_res = c.recent_light_blocks(depth).await;

                let blocks = match blocks_res {
                    Ok(b) => b,
                    Err(err) => {
                        tracing::debug!(error = %err, "failed to query recent blocks (retrying next cycle)");
                        client = None;
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                };

                // Merge any newly observed senders into the bond map.
                {
                    let mut bonds_guard = bonds.lock().await;
                    merge_bonds(&mut bonds_guard, &blocks);
                    let mut ingress_guard = ingress.lock().await;
                    ingress_guard.set_bonds(bonds_guard.clone());
                }

                // Append new blocks into the long-lived ingress.
                let new_blocks_res = {
                    let mut ingress_guard = ingress.lock().await;
                    mirror_blocks(&mut ingress_guard, &blocks, &mut known_creators)
                };

                let new_blocks = match new_blocks_res {
                    Ok(n) => n,
                    Err(err) => {
                        tracing::warn!(error = %err, "mirroring blocks failed");
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                };

                if !bootstrapped && !blocks.is_empty() {
                    bootstrapped = true;
                    tracing::info!(count = blocks.len(), "initial bootstrap blocks mirrored");
                } else if new_blocks > 0 {
                    tracing::debug!(new_blocks, "poll cycle: new blocks appended");
                }

                // Recompute ordered output over the accumulated blocklace.
                let output_res = {
                    let mut ingress_guard = ingress.lock().await;
                    ingress_guard.latest_finalized_ordered_output(wave_length)
                };

                match output_res {
                    Ok(output) => {
                        let mut guard = shared.lock().await;
                        match poll_once(&mut guard, output) {
                            PollOutcome::Updated { finalized_len } => {
                                tracing::debug!(
                                    finalized_blocks = finalized_len,
                                    "ordered output updated"
                                );
                            }
                            PollOutcome::PrefixRejected => {
                                tracing::warn!(
                                    "ordered output update rejected (prefix violation) — keeping previous"
                                );
                            }
                            PollOutcome::NoLeader => {
                                tracing::debug!("no finalized leader yet");
                            }
                        }
                    }
                    Err(err) => {
                        tracing::debug!(error = ?err, "ordering calculation not ready yet");
                    }
                }

                tokio::time::sleep(poll_interval).await;
            }
        });
    }

    // ── Step 2: Start HTTP server ─────────────────────────────────────────────
    let stale_threshold_ns = (args.stale_threshold_secs as u128) * 1_000_000_000;
    tracing::info!(addr = %args.addr, "starting ordered-output HTTP server");

    serve(shared, args.addr, stale_threshold_ns)
        .await
        .with_context(|| format!("HTTP server on {} failed", args.addr))?;

    Ok(())
}
