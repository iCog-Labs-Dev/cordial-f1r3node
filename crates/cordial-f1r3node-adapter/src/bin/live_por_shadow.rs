//! One-node read-only PoR observer. Emits JSON status and writes data_dir/por/shadow-status.json.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::Parser;
use cordial_f1r3node_adapter::por::shadow::{ShadowConfig, run_shadow};

#[derive(Parser)]
#[command(name = "live-por-shadow")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:51401")]
    grpc_url: String,
    #[arg(long)]
    bonds_file: PathBuf,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long, default_value = "root")]
    shard_id: String,
    #[arg(long, default_value_t = 64)]
    height_batch_size: i64,
    #[arg(long, default_value_t = 2000)]
    poll_interval_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    run_shadow(ShadowConfig {
        grpc_url: args.grpc_url,
        bonds_file: args.bonds_file,
        data_dir: args.data_dir,
        shard_id: args.shard_id,
        height_batch_size: args.height_batch_size,
        poll_interval: Duration::from_millis(args.poll_interval_ms),
    })
    .await
}
