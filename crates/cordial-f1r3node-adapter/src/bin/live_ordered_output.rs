//! Dedicated inspection tool for the ordered finalized output export seam.
//!
//! Unlike `live_mirror_check`, which does a full mirror-vs-HTTP comparison
//! run, this binary does exactly one thing: mirror live blocks from a
//! running f1r3node into a local Cordial blocklace, then print the latest
//! finalized ordered fragment via `cordial_f1r3node_adapter::ordered_output`.
//!
//! Out of scope (see the tracking issue): HTTP/gRPC serving of ordered
//! output, and node-side consumption. This is inspection tooling only.

use anyhow::{Context, Result, bail};
use clap::Parser;
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_grpc::{
    LiveGrpcBlockClient, light_block_info_to_block_message,
    trusted_block_from_light_block_info_with_options,
};
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::ordered_output::{OrderedBlockSummary, OrderedFragment};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::types::{BlockIdentity, NodeId};
use models::rust::casper::pretty_printer::PrettyPrinter;
use models::rust::string_ops::StringOps;
use std::collections::HashMap;

#[derive(Parser, Debug)]
#[command(name = "live-ordered-output")]
#[command(
    about = "Mirror live f1r3node blocks and print the latest finalized ordered fragment via the ordered_output export seam"
)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:40401")]
    grpc_url: String,

    #[arg(long, default_value_t = 128)]
    depth: i32,

    #[arg(long, default_value = "root")]
    shard_id: String,

    #[arg(long, default_value_t = 3)]
    wave_length: u64,

    #[arg(long, default_value_t = 5)]
    preview: usize,
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

    let mut grpc = LiveGrpcBlockClient::connect(args.grpc_url.clone())
        .await
        .with_context(|| format!("failed to connect to gRPC endpoint {}", args.grpc_url))?;

    let recent_blocks = grpc
        .recent_light_blocks(args.depth)
        .await
        .with_context(|| format!("failed to fetch recent blocks at depth {}", args.depth))?;

    if recent_blocks.is_empty() {
        bail!("no blocks were returned from the live node");
    }

    let bonds = derive_uniform_bonds(&recent_blocks);
    let shard_conf = CasperShardConf {
        shard_name: args.shard_id.clone(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let mut ingress =
        LiveIngress::with_consensus_view(PassthroughAdapter, bonds, shard_conf, &args.shard_id);

    for info in &recent_blocks {
        let _ = light_block_info_to_block_message(info)
            .with_context(|| format!("failed to decode live block {}", info.block_hash))?;

        let block = trusted_block_from_light_block_info_with_options(info, true)
            .with_context(|| format!("failed to reconstruct trusted block {}", info.block_hash))?;
        ingress
            .ingest_trusted_block(block)
            .with_context(|| format!("failed to mirror live block {}", info.block_hash))?;
    }

    println!("Mirrored blocks: {}", ingress.blocklace().dom().len());
    println!("Pending blocks:  {}", ingress.pending_blocks().len());

    // The whole point of this binary: go through the stable export seam
    // rather than recomputing ordering against the mirrored blocklace.
    let fragment = ingress
        .latest_finalized_ordered_fragment(args.wave_length)
        .map_err(|err| anyhow::anyhow!("failed to compute latest ordered fragment: {err}"))?;

    print_fragment(fragment.as_ref(), args.preview);

    Ok(())
}

fn derive_uniform_bonds(blocks: &[models::casper::LightBlockInfo]) -> HashMap<NodeId, u64> {
    let mut bonds = HashMap::new();
    for block in blocks {
        if let Some(sender) = StringOps::decode_hex(block.sender.clone()) {
            bonds.entry(NodeId(sender)).or_insert(100);
        }
    }
    bonds
}

fn hex_string(bytes: Vec<u8>) -> String {
    PrettyPrinter::build_string_no_limit(&bytes)
}

fn print_fragment(fragment: Option<&OrderedFragment>, preview: usize) {
    let Some(fragment) = fragment else {
        println!("Latest finalized ordered fragment: <none> (no finalized leader yet)");
        return;
    };

    println!(
        "Latest finalized leader: {}",
        hex_string(fragment.leader_hash.clone())
    );
    println!("Ordered fragment size:   {}", fragment.len());

    if fragment.is_empty() {
        return;
    }

    let preview = preview.min(fragment.len());
    println!("Ordered fragment head:");
    for block in fragment.blocks.iter().take(preview) {
        print_block(block);
    }
    if fragment.len() > preview {
        println!("Ordered fragment tail:");
        for block in fragment
            .blocks
            .iter()
            .skip(fragment.len().saturating_sub(preview))
        {
            print_block(block);
        }
    }
}

fn print_block(block: &OrderedBlockSummary) {
    println!(
        "  - hash={} creator={} block_number={} round={} wave={}",
        hex_string(block.content_hash.clone()),
        hex_string(block.creator.clone()),
        block.block_number,
        block
            .round
            .map(|r| r.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
        block
            .wave
            .map(|w| w.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
    );
}