use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_grpc::LiveGrpcBlockClient;
use cordial_f1r3node_adapter::live_ingress::{AlreadyValidatedVerifier, LiveIngress};
use cordial_f1r3node_adapter::native_block_source::LocalNativeBlockSource;
use cordial_f1r3node_adapter::native_live_runner::{
    apply_native_height_window, replay_native_height_window,
};
use cordial_f1r3node_adapter::repository::{RSpaceBlocklaceRepository, open_block_store};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::types::BlockIdentity;

#[derive(Parser)]
#[command(about = "Mirror full signed blocks from a local f1r3node into durable Cordial ingress")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:40401")]
    grpc_url: String,
    #[arg(long)]
    node_data_dir: PathBuf,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long, default_value = "root")]
    shard_id: String,
    #[arg(long, default_value_t = 32)]
    batch_heights: i64,
    #[arg(long, default_value_t = 2048)]
    max_backfill_blocks: usize,
    #[arg(long, default_value_t = 16)]
    replay_heights: i64,
    #[arg(long, default_value_t = 5000)]
    poll_ms: u64,
    #[arg(long)]
    once: bool,
}

struct PassthroughAdapter;

impl BlocklaceAdapter<BlockIdentity> for PassthroughAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.batch_heights <= 0 || args.replay_heights <= 0 || args.poll_ms == 0 {
        bail!("batch-heights, replay-heights, and poll-ms must be positive");
    }
    let source = LocalNativeBlockSource::open(&args.node_data_dir).await?;
    let repo = open_block_store(&args.data_dir)?;
    let (mut ingress, _) = LiveIngress::with_persistent_store(
        PassthroughAdapter,
        HashMap::new(),
        CasperShardConf::default(),
        args.shard_id.clone(),
        &repo,
        &AlreadyValidatedVerifier,
    )?;
    let mut grpc = LiveGrpcBlockClient::connect(args.grpc_url.clone()).await?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            result = poll_to_head(&mut grpc, &source, &repo, &mut ingress, &args) => result?,
        }
        if args.once {
            break;
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_millis(args.poll_ms)) => {},
        }
    }
    println!(
        "stopped at native height {:?}",
        repo.native_height_cursor()?
    );
    Ok(())
}

async fn poll_to_head(
    grpc: &mut LiveGrpcBlockClient,
    source: &LocalNativeBlockSource,
    repo: &RSpaceBlocklaceRepository,
    ingress: &mut LiveIngress<PassthroughAdapter>,
    args: &Args,
) -> Result<()> {
    let head = grpc
        .recent_light_blocks(1)
        .await?
        .into_iter()
        .map(|block| block.block_number)
        .max();
    let Some(head) = head else {
        return Ok(());
    };
    if let Some(cursor) = repo.native_height_cursor()? {
        if head < cursor {
            bail!("host head {head} is behind durable native cursor {cursor}");
        }
        let replay_start = cursor.saturating_sub(args.replay_heights - 1).max(0);
        let mut replay_from = replay_start;
        while replay_from <= cursor {
            let replay_end = replay_from
                .saturating_add(args.batch_heights - 1)
                .min(cursor);
            let blocks = grpc
                .native_blocks_by_heights(source, replay_from, replay_end)
                .await?;
            replay_native_height_window(
                ingress,
                repo,
                source,
                blocks,
                replay_from,
                replay_end,
                args.max_backfill_blocks,
            )
            .await?;
            replay_from = replay_end + 1;
        }
    }
    let mut start = repo.native_height_cursor()?.map_or(0, |height| height + 1);
    while start <= head {
        let end = start.saturating_add(args.batch_heights - 1).min(head);
        let blocks = grpc.native_blocks_by_heights(source, start, end).await?;
        let applied = apply_native_height_window(
            ingress,
            repo,
            source,
            blocks,
            start,
            end,
            args.max_backfill_blocks,
        )
        .await?;
        println!("native heights {start}..={end}: applied {applied} blocks");
        start = end + 1;
    }
    Ok(())
}
