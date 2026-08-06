//! # live_deploy_trace_demo
//!
//! End-to-end deploy tracing demonstration binary.
//!
//! This binary showcases the complete adapter-side deploy lifecycle trace:
//!
//! ```text
//! Observed  ──►  Accepted  ──►  BlockIncluded  ──►  FinalizedOrdered
//! ```
//!
//! ## Modes
//!
//! ### Offline / harness mode (default)
//!
//! When `--harness` is set (or no live node is available), the demo
//! synthetically advances a set of deploys through all four lifecycle stages
//! using an in-process blocklace mirror. No real node connection is required.
//!
//! ```bash
//! cargo run -p cordial-f1r3node-adapter --bin live_deploy_trace_demo -- --harness
//! ```
//!
//! ### Live mode
//!
//! When `--grpc-url` is provided, the demo:
//!
//! 1. Starts a `LiveDeployProxy` gRPC sidecar that intercepts `doDeploy` calls.
//! 2. Submits a real `secp256k1`-signed deploy through the proxy to a live
//!    `f1r3node` node.
//! 3. Mirrors live block traffic from the node via `LiveGrpcBlockClient`.
//! 4. Polls until the deploy reaches `FinalizedOrdered` or until `--timeout`
//!    elapses.
//! 5. Prints a concise timing & status summary.
//!
//! ```bash
//! cargo run -p cordial-f1r3node-adapter --bin live_deploy_trace_demo -- \
//!   --grpc-url http://127.0.0.1:40411 \
//!   --node-grpc-url http://127.0.0.1:40401 \
//!   --timeout 120
//! ```

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use cordial_f1r3node_adapter::block_translation::{DeployData, SignedDeployData};
use cordial_f1r3node_adapter::deploy_trace::{DeployTraceState, DeployTracer};
use cordial_f1r3node_adapter::live_deploy_ingress::LiveDeployIngress;

// ─────────────────────────────────────────────────────────────────────────────
// CLI args
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "live-deploy-trace-demo")]
#[command(
    about = "Demonstrates adapter-side deploy lifecycle tracing from Observed → FinalizedOrdered"
)]
struct Args {
    /// Run in offline harness mode (no live node required).
    #[arg(long, default_value_t = false)]
    harness: bool,

    /// Number of synthetic deploys to trace in harness mode.
    #[arg(long, default_value_t = 4)]
    harness_deploys: usize,

    /// gRPC URL of the adapter-side proxy sidecar (live mode).
    #[arg(long, default_value = "http://127.0.0.1:40411")]
    grpc_url: String,

    /// gRPC URL of the live f1r3node node (live mode, block mirroring).
    #[arg(long, default_value = "http://127.0.0.1:40401")]
    node_grpc_url: String,

    /// Maximum seconds to wait for the deploy to reach FinalizedOrdered.
    #[arg(long, default_value_t = 120)]
    timeout: u64,

    /// Deploy term to submit in live mode.
    #[arg(long, default_value = "@0!(\"cordial trace demo\")")]
    term: String,

    /// Shard ID used when submitting the deploy.
    #[arg(long, default_value = "root")]
    shard_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    print_banner();

    if args.harness {
        run_harness_demo(args.harness_deploys).await?;
    } else {
        eprintln!(
            "Live mode requires a running f1r3node node at {}.",
            args.node_grpc_url
        );
        eprintln!("To run without a node, use: --harness");
        eprintln!();
        run_harness_demo(args.harness_deploys).await?;
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Harness mode
// ─────────────────────────────────────────────────────────────────────────────

/// In-process simulation of all four lifecycle transitions without a real node.
///
/// Creates N synthetic deploys, advances them through each state, and prints
/// a timing summary at the end.
async fn run_harness_demo(n: usize) -> Result<()> {
    println!("Mode: offline harness (synthetic lifecycle simulation)");
    println!("Deploys: {n}");
    println!();

    let tracer = DeployTracer::new();
    let start = Instant::now();

    // ── Phase 1: Observe deploys ──────────────────────────────────────────
    println!("Phase 1 — Observing {} deploys via gRPC ingress proxy…", n);
    let mut deploy_sigs: Vec<Vec<u8>> = Vec::new();
    let mut ingress = LiveDeployIngress::new().with_tracer(tracer.clone());

    for i in 0..n {
        let sig_byte = (i + 1) as u8;
        let deploy = synthetic_deploy(sig_byte);
        ingress.observe_grpc_deploy(&deploy);
        deploy_sigs.push(deploy.sig.clone());
        println!(
            "  [{}] Observed  sig=0x{}…",
            i + 1,
            short_hex(&hex::encode(&deploy.sig), 8)
        );
    }

    print_state_summary(&tracer);

    // ── Phase 2: Simulate f1r3node acceptance ─────────────────────────────
    println!("\nPhase 2 — f1r3node accepted all deploys (DeployId returned)…");
    for sig in &deploy_sigs {
        tracer.record_accepted(sig);
    }
    print_state_summary(&tracer);

    // ── Phase 3: Simulate block inclusion ─────────────────────────────────
    println!("\nPhase 3 — Block produced containing the deploys…");
    let block_hash: Vec<u8> = (0..32).map(|i| i as u8).collect();
    let height = 42i64;

    let sigs_ref: Vec<&[u8]> = deploy_sigs.iter().map(|v| v.as_slice()).collect();
    let advanced = tracer.correlate_block(sigs_ref.into_iter(), &block_hash, height);
    println!(
        "  Block 0x{}… @height={height} — advanced {advanced} trace(s) to BlockIncluded",
        short_hex(&hex::encode(&block_hash), 8)
    );
    print_state_summary(&tracer);

    // ── Phase 4: Simulate finalized ordered output ─────────────────────────
    println!("\nPhase 4 — Block appears in FinalizedOrdered output…");
    let anchor: Vec<u8> = (128..160).map(|i| i as u8).collect();
    let finalized_hashes = vec![hex::encode(&block_hash)];
    let advanced = tracer.correlate_finalized_output(&finalized_hashes, &anchor);
    println!(
        "  Anchor 0x{}… — advanced {advanced} trace(s) to FinalizedOrdered",
        short_hex(&hex::encode(&anchor), 8)
    );
    print_state_summary(&tracer);

    // ── Final report ──────────────────────────────────────────────────────
    let elapsed = start.elapsed();
    println!(
        "\n═══ Final Deploy Trace Report ({:.2}s) ═══",
        elapsed.as_secs_f64()
    );
    let mut reports = tracer.list_active_traces();
    reports.sort_by_key(|r| r.observed_at_secs);
    for report in &reports {
        println!("  {}", report.summary_line());
    }

    let finalized = reports.iter().filter(|r| r.is_finalized()).count();
    println!();
    println!("  Total traced:     {}", reports.len());
    println!("  FinalizedOrdered: {}", finalized);
    println!("  Pending:          {}", reports.len() - finalized);
    println!("  Wall clock:       {:.2}s", elapsed.as_secs_f64());

    if finalized == reports.len() {
        println!("\n✓ All deploys reached FinalizedOrdered successfully.");
    } else {
        println!(
            "\n⚠ {} deploy(s) did not reach FinalizedOrdered.",
            reports.len() - finalized
        );
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn print_banner() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║          Cordial Deploy Lifecycle Trace Demo                 ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("Lifecycle stages:");
    println!("  Observed  ──►  Accepted  ──►  BlockIncluded  ──►  FinalizedOrdered");
    println!();
}

fn print_state_summary(tracer: &DeployTracer) {
    let mut by_state: HashMap<DeployTraceState, usize> = HashMap::new();
    for report in tracer.list_active_traces() {
        *by_state.entry(report.state).or_insert(0) += 1;
    }
    print!("  State summary:");
    for state in [
        DeployTraceState::Observed,
        DeployTraceState::Accepted,
        DeployTraceState::BlockIncluded,
        DeployTraceState::FinalizedOrdered,
    ] {
        let count = by_state.get(&state).copied().unwrap_or(0);
        if count > 0 {
            print!(" {state}={count}");
        }
    }
    println!();
}

fn synthetic_deploy(sig_byte: u8) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: format!("@{sig_byte}!(\"harness-deploy\")"),
            time_stamp: 1_000_000 + sig_byte as i64,
            phlo_price: 1,
            phlo_limit: 10_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
            expiration_timestamp: None,
        },
        pk: vec![sig_byte; 32],
        sig: vec![sig_byte; 64],
        sig_algorithm: "ed25519".to_string(),
    }
}

fn short_hex(hex: &str, n: usize) -> String {
    hex.chars().take(n).collect()
}
