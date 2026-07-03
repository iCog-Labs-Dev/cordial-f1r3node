use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::http_observer::HttpObserver;
use cordial_f1r3node_adapter::live_grpc::{
    LiveGrpcBlockClient, light_block_info_to_block_message,
    trusted_block_from_light_block_info_with_options,
};
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::consensus::{
    depth, is_weighted_final_leader, leader_block_for_wave, wave_of_round,
    weighted_final_leader_for_wave,
};
use cordial_miners_core::execution::CordialBlockPayload;
use cordial_miners_core::types::{BlockIdentity, NodeId};
use models::rust::casper::pretty_printer::PrettyPrinter;
use models::rust::string_ops::StringOps;

#[derive(Parser, Debug)]
#[command(name = "live-mirror-check")]
#[command(
    about = "Mirror live f1r3node blocks into Cordial state and compare against HTTP-visible node state"
)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:40401")]
    grpc_url: String,

    #[arg(long, default_value = "http://127.0.0.1:40403")]
    http_url: String,

    #[arg(long, default_value_t = 128)]
    depth: i32,

    #[arg(long, default_value = "root")]
    shard_id: String,

    #[arg(long, default_value_t = 8)]
    max_backfill_rounds: usize,

    #[arg(long, default_value_t = 256)]
    max_backfill_blocks: usize,

    #[arg(long, default_value_t = false)]
    parents_only_bootstrap: bool,

    #[arg(long, default_value_t = true)]
    height_bootstrap: bool,

    #[arg(long, default_value_t = 64)]
    height_batch_size: i64,

    #[arg(long, default_value_t = false)]
    skip_ordering: bool,

    #[arg(long, default_value_t = false)]
    skip_http_compare: bool,

    #[arg(long, default_value_t = 5)]
    ordering_preview: usize,

    #[arg(long, default_value_t = false)]
    ordering_fragment_only: bool,

    #[arg(long)]
    write_ordered_file: Option<PathBuf>,

    #[arg(long)]
    compare_ordered_file: Option<PathBuf>,
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

    println!("[phase] querying initial gRPC last finalized block");
    let initial_grpc_last_finalized = grpc
        .last_finalized_block_hash()
        .await
        .context("failed to query initial gRPC last finalized block")?
        .map(hex_string);

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

    let mut decoded_messages = 0usize;
    let mut height_bootstrapped = 0usize;
    let target_max_height = recent_blocks
        .iter()
        .map(|block| block.block_number)
        .max()
        .unwrap_or(0);

    if args.height_bootstrap {
        height_bootstrapped = bootstrap_by_heights(
            &mut grpc,
            &mut ingress,
            target_max_height,
            args.height_batch_size.max(1),
            args.parents_only_bootstrap,
            &mut decoded_messages,
        )
        .await?;
    } else {
        for info in &recent_blocks {
            let _ = light_block_info_to_block_message(info)
                .with_context(|| format!("failed to decode live block {}", info.block_hash))?;
            decoded_messages += 1;

            let block = trusted_block_from_light_block_info_with_options(
                info,
                !args.parents_only_bootstrap,
            )
            .with_context(|| format!("failed to reconstruct trusted block {}", info.block_hash))?;
            ingress
                .ingest_trusted_block(block)
                .with_context(|| format!("failed to mirror live block {}", info.block_hash))?;
        }
    }

    let backfilled = if ingress.blocklace().dom().is_empty() {
        backfill_missing_predecessors(
            &mut grpc,
            &mut ingress,
            args.max_backfill_rounds,
            args.max_backfill_blocks,
            args.parents_only_bootstrap,
        )
        .await?
    } else {
        0
    };

    println!("[phase] bootstrap complete");

    println!("[phase] computing mirror last finalized block");
    let mirror_last_finalized = ingress
        .last_finalized_block_hash()
        .map_err(|err| anyhow::anyhow!("failed to compute mirror last finalized block: {err:?}"))?
        .map(hex_string);

    println!("[phase] querying gRPC last finalized block");
    let grpc_last_finalized = grpc
        .last_finalized_block_hash()
        .await
        .context("failed to query gRPC last finalized block")?
        .map(hex_string);

    let mirror_lfb_meta = mirror_last_finalized
        .as_ref()
        .and_then(|hash| describe_mirror_block(&ingress, hash));

    let grpc_lfb_meta = match grpc_last_finalized.as_ref() {
        Some(hash) => Some(
            grpc.light_block_by_hash(hash.clone())
                .await
                .with_context(|| format!("failed to fetch gRPC LFB metadata for {hash}"))?,
        ),
        None => None,
    };

    let initial_grpc_lfb_meta =
        match initial_grpc_last_finalized.as_ref() {
            Some(hash) => Some(grpc.light_block_by_hash(hash.clone()).await.with_context(
                || format!("failed to fetch initial gRPC LFB metadata for {hash}"),
            )?),
            None => None,
        };

    let mut ordered_output: Option<OrderedFinalizedOutput> = None;
    let ordered_blocks = if args.skip_ordering {
        println!("[phase] skipping ordered finalized blocks");
        None
    } else {
        println!(
            "[phase] computing {}",
            if args.ordering_fragment_only {
                "latest finalized ordered output"
            } else {
                "ordered finalized blocks"
            }
        );
        Some(if args.ordering_fragment_only {
            // Use the stable ordered_output export seam instead of
            // recomputing ordering (approved_blocks_for_leader + xsort)
            // directly against the mirrored blocklace.
            let output = ingress.latest_finalized_ordered_output(3).map_err(|err| {
                anyhow::anyhow!("failed to compute latest ordered output: {err:?}")
            })?;
            let hashes = output.block_hashes();
            ordered_output = Some(output);
            hashes
        } else {
            ingress.ordered_finalized_blocks().map_err(|err| {
                anyhow::anyhow!("failed to compute ordered finalized blocks: {err:?}")
            })?
        })
    };
    let ordered_count = ordered_blocks.as_ref().map_or(0, Vec::len);

    let comparison = if args.skip_http_compare {
        println!("[phase] skipping HTTP comparison");
        None
    } else {
        println!("[phase] querying HTTP observer and comparing mirror");
        let observer = HttpObserver::new(args.http_url.clone());
        Some(
            observer
                .compare_live_ingress(&ingress)
                .await
                .with_context(|| {
                    format!(
                        "failed to compare mirror against HTTP endpoint {}",
                        args.http_url
                    )
                })?,
        )
    };

    println!("Cordial live mirror check");
    println!("=========================");
    println!("gRPC URL:          {}", args.grpc_url);
    println!("HTTP URL:          {}", args.http_url);
    println!("Depth:             {}", args.depth);
    println!("Max rounds:        {}", args.max_backfill_rounds);
    println!("Max backfill:      {}", args.max_backfill_blocks);
    println!("Parents-only:      {}", args.parents_only_bootstrap);
    println!("Height bootstrap:  {}", args.height_bootstrap);
    println!("Height batch:      {}", args.height_batch_size);
    println!("Skip ordering:     {}", args.skip_ordering);
    println!("Skip HTTP compare: {}", args.skip_http_compare);
    println!("Ordering preview:  {}", args.ordering_preview);
    println!("Fragment only:     {}", args.ordering_fragment_only);
    println!(
        "Write ordered:     {}",
        args.write_ordered_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!(
        "Compare ordered:   {}",
        args.compare_ordered_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!("Mirrored blocks:   {}", ingress.blocklace().dom().len());
    println!("Pending blocks:    {}", ingress.pending_blocks().len());
    println!("Decoded messages:  {}", decoded_messages);
    println!("Height bootstrapped: {}", height_bootstrapped);
    println!("Backfilled blocks: {}", backfilled);
    println!(
        "Unresolved preds:  {}",
        unresolved_predecessor_hashes(&ingress).len()
    );
    println!("Ordered blocks:    {}", ordered_count);
    if let Some(ordered) = ordered_blocks.as_ref() {
        print_ordering_preview(ordered, args.ordering_preview);
        if let Some(output) = ordered_output.as_ref() {
            print_ordered_output_summary(output, &ingress, args.ordering_preview);
        }
        if let Some(mirror_lfb) = mirror_last_finalized.as_ref() {
            println!(
                "LFB in ordered:    {}",
                if ordered
                    .iter()
                    .any(|hash| &hex_string(hash.clone()) == mirror_lfb)
                {
                    "yes"
                } else {
                    "no"
                }
            );
        }

        let ordered_hex: Vec<String> = ordered.iter().cloned().map(hex_string).collect();
        if let Some(path) = args.write_ordered_file.as_ref() {
            write_ordered_hashes(path, &ordered_hex)?;
            println!("Ordered file:      {}", path.display());
        }
        if let Some(path) = args.compare_ordered_file.as_ref() {
            let comparison = compare_ordered_hashes(path, &ordered_hex)?;
            println!("Ordered compare:   {}", comparison.status);
            println!("Ordered prefix:    {}", comparison.prefix_relation);
            if let Some(first_mismatch) = comparison.first_mismatch {
                println!("First mismatch:    {}", first_mismatch);
            }
        }
    }
    println!(
        "Initial gRPC LFB:  {}",
        initial_grpc_last_finalized.as_deref().unwrap_or("<none>")
    );
    if let Some(meta) = initial_grpc_lfb_meta.as_ref() {
        println!(
            "Initial gRPC Meta: creator={} block_number={}",
            meta.sender, meta.block_number
        );
    }
    println!(
        "Mirror LFB:        {}",
        mirror_last_finalized.as_deref().unwrap_or("<none>")
    );
    if let Some(meta) = mirror_lfb_meta.as_ref() {
        println!(
            "Mirror LFB Meta:   creator={} block_number={} round={} wave={}",
            meta.creator, meta.block_number, meta.round, meta.wave
        );
    }
    println!(
        "HTTP LFB:          {}",
        comparison
            .as_ref()
            .and_then(|c| c.http_last_finalized.as_deref())
            .unwrap_or("<none>")
    );
    println!(
        "gRPC LFB:          {}",
        grpc_last_finalized.as_deref().unwrap_or("<none>")
    );
    if let Some(meta) = grpc_lfb_meta.as_ref() {
        println!(
            "gRPC LFB Meta:     creator={} block_number={}",
            meta.sender, meta.block_number
        );
    }

    if args.skip_http_compare {
        println!("Finality neighborhood: skipped");
    } else {
        print_finality_neighborhood(&ingress, mirror_lfb_meta.as_ref(), grpc_lfb_meta.as_ref());
    }
    println!(
        "Comparison:        {}",
        match comparison.as_ref() {
            Some(report) if report.is_match() => "MATCH",
            Some(_) => "MISMATCH",
            None => "SKIPPED",
        }
    );

    if let Some(comparison) = comparison
        .as_ref()
        .filter(|c| !c.missing_from_mirror.is_empty())
    {
        println!("Missing from mirror:");
        for hash in &comparison.missing_from_mirror {
            println!("  - {}", hash);
        }
    }

    if let Some(comparison) = comparison
        .as_ref()
        .filter(|c| !c.missing_from_http.is_empty())
    {
        println!("Missing from HTTP:");
        for hash in &comparison.missing_from_http {
            println!("  - {}", hash);
        }
    }

    if comparison
        .as_ref()
        .is_some_and(|comparison| !comparison.last_finalized_matches)
    {
        println!("Last finalized block mismatch detected.");
    }

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

fn print_ordering_preview(ordered: &[Vec<u8>], preview: usize) {
    let preview = preview.min(ordered.len());
    println!("Ordered preview:   {}", preview);

    if preview == 0 {
        return;
    }

    println!("Ordered head:");
    for hash in ordered.iter().take(preview) {
        println!("  - {}", hex_string(hash.clone()));
    }

    if ordered.len() > preview {
        println!("Ordered tail:");
        for hash in ordered.iter().skip(ordered.len().saturating_sub(preview)) {
            println!("  - {}", hex_string(hash.clone()));
        }
    }
}

struct OrderedComparison {
    status: &'static str,
    prefix_relation: &'static str,
    first_mismatch: Option<String>,
}

fn write_ordered_hashes(path: &PathBuf, ordered: &[String]) -> Result<()> {
    let body = serde_json::to_string_pretty(ordered)
        .context("failed to serialize ordered hashes to JSON")?;
    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))
}

fn compare_ordered_hashes(path: &PathBuf, current: &[String]) -> Result<OrderedComparison> {
    let body =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let previous: Vec<String> = serde_json::from_str(&body)
        .with_context(|| format!("failed to parse ordered hashes from {}", path.display()))?;

    if previous == current {
        return Ok(OrderedComparison {
            status: "MATCH",
            prefix_relation: "equal",
            first_mismatch: None,
        });
    }

    let common_prefix_len = previous
        .iter()
        .zip(current.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let prefix_relation = if previous.len() == common_prefix_len {
        "previous-is-prefix"
    } else if current.len() == common_prefix_len {
        "current-is-prefix"
    } else {
        "diverged"
    };

    let first_mismatch = Some(
        match (
            previous.get(common_prefix_len),
            current.get(common_prefix_len),
        ) {
            (Some(prev), Some(curr)) => format!("prev={} current={}", prev, curr),
            (Some(prev), None) => format!("prev={} current=<end>", prev),
            (None, Some(curr)) => format!("prev=<end> current={}", curr),
            (None, None) => String::from("<none>"),
        },
    );

    Ok(OrderedComparison {
        status: "MISMATCH",
        prefix_relation,
        first_mismatch,
    })
}

/// Print the summary fields for an [`OrderedFinalizedOutput`] obtained from
/// the stable `ordered_output` export seam. Mirrors the head/tail preview
/// shape of `print_ordering_preview`, but surfaces the consensus metadata
/// (anchor, wavelength, bond count, mirror size) the seam attaches to the
/// fragment as a whole.
///
/// `BlockIdentity` (the per-block element of `OrderedFinalizedOutput`) only
/// carries content hash, creator, and signature — round/wave are not part
/// of the stable seam type. Since this debug tool has direct access to the
/// mirrored blocklace, it recomputes round/wave/block_number per block for
/// extra visibility, the same way `describe_mirror_block` does for the LFB.
fn print_ordered_output_summary(
    output: &OrderedFinalizedOutput,
    ingress: &LiveIngress<PassthroughAdapter>,
    preview: usize,
) {
    println!(
        "Ordered output anchor:   {}",
        output
            .anchor_hash()
            .map(hex_string)
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!("Wavelength:              {}", output.wavelength);
    println!("Bond count:              {}", output.bond_count);
    println!("Total mirrored blocks:   {}", output.total_mirrored_blocks);

    if output.is_empty() {
        println!("Ordered output blocks:   <none>");
        return;
    }

    let preview = preview.min(output.len());
    println!("Ordered output blocks:   {}", output.len());
    if preview == 0 {
        return;
    }

    println!("Ordered output head:");
    for block in output.blocks.iter().take(preview) {
        print_ordered_block_summary(block, ingress);
    }

    if output.len() > preview {
        println!("Ordered output tail:");
        for block in output
            .blocks
            .iter()
            .skip(output.len().saturating_sub(preview))
        {
            print_ordered_block_summary(block, ingress);
        }
    }
}

fn print_ordered_block_summary(block: &BlockIdentity, ingress: &LiveIngress<PassthroughAdapter>) {
    let round = depth(ingress.blocklace(), block);
    let wave = round.and_then(|r| wave_of_round(r, 3));
    let block_number = ingress
        .blocklace()
        .content(block)
        .and_then(|content| CordialBlockPayload::from_bytes(&content.payload).ok())
        .map(|payload| payload.state.block_number);

    println!(
        "  - hash={} creator={} block_number={} round={} wave={}",
        hex_string(block.content_hash.to_vec()),
        hex_string(block.creator.0.clone()),
        block_number
            .map(|n| n.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
        round
            .map(|r| r.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
        wave.map(|w| w.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
    );
}

#[derive(Debug, Clone)]
struct MirrorBlockMeta {
    creator: String,
    block_number: u64,
    round: u64,
    wave: u64,
}

fn describe_mirror_block(
    ingress: &LiveIngress<PassthroughAdapter>,
    hash_hex: &str,
) -> Option<MirrorBlockMeta> {
    let hash = StringOps::decode_hex(hash_hex.to_string())?;
    let id = ingress
        .blocklace()
        .dom()
        .iter()
        .find(|id| id.content_hash.as_slice() == hash.as_slice())?
        .to_owned();
    let round = depth(ingress.blocklace(), id)?;
    let wave = wave_of_round(round, 3)?;
    let content = ingress.blocklace().content(id)?;
    let payload = CordialBlockPayload::from_bytes(&content.payload).ok()?;

    Some(MirrorBlockMeta {
        creator: hex_string(id.creator.0.clone()),
        block_number: payload.state.block_number,
        round,
        wave,
    })
}

fn ordered_validators_for_debug(ingress: &LiveIngress<PassthroughAdapter>) -> Vec<NodeId> {
    let mut validators: Vec<NodeId> = ingress.bonds().keys().cloned().collect();
    validators.sort();
    validators
}

fn print_finality_neighborhood(
    ingress: &LiveIngress<PassthroughAdapter>,
    mirror_lfb_meta: Option<&MirrorBlockMeta>,
    grpc_lfb_meta: Option<&models::casper::LightBlockInfo>,
) {
    let validators = ordered_validators_for_debug(ingress);
    if validators.is_empty() {
        println!("Validator order:   <none>");
        return;
    }

    println!("Validator order:");
    for (idx, validator) in validators.iter().enumerate() {
        println!("  [{}] {}", idx, hex_string(validator.0.clone()));
    }

    let focus_wave = mirror_lfb_meta.map(|m| m.wave).unwrap_or(0);
    println!("Wave neighborhood:");
    for wave in focus_wave.saturating_sub(1)..=focus_wave + 1 {
        let selected = validators
            .get((wave as usize) % validators.len())
            .map(|id| hex_string(id.0.clone()))
            .unwrap_or_else(|| "<none>".to_string());
        let leader_block = leader_block_for_wave(ingress.blocklace(), wave, 3, |w| {
            validators.get((w as usize) % validators.len()).cloned()
        })
        .map(|id| hex_string(id.content_hash.to_vec()))
        .unwrap_or_else(|| "<none>".to_string());
        let final_block =
            weighted_final_leader_for_wave(ingress.blocklace(), wave, 3, ingress.bonds(), |w| {
                validators.get((w as usize) % validators.len()).cloned()
            })
            .map(|id| hex_string(id.content_hash.to_vec()))
            .unwrap_or_else(|| "<none>".to_string());

        println!(
            "  wave {}: selected_leader={} leader_block={} weighted_final={}",
            wave, selected, leader_block, final_block
        );
    }

    let mut interesting_heights = Vec::new();
    if let Some(meta) = mirror_lfb_meta {
        interesting_heights.push(meta.block_number);
    }
    if let Some(info) = grpc_lfb_meta {
        interesting_heights.push(info.block_number as u64);
    }
    interesting_heights.sort_unstable();
    interesting_heights.dedup();

    if let (Some(min), Some(max)) = (
        interesting_heights.first().copied(),
        interesting_heights.last().copied(),
    ) {
        println!("Block neighborhood:");
        for block in mirrored_blocks_in_height_range(ingress, min.saturating_sub(1), max + 1) {
            let hash = hex_string(block.identity.content_hash.to_vec());
            let creator = hex_string(block.identity.creator.0.clone());
            let payload = match ingress
                .blocklace()
                .content(&block.identity)
                .and_then(|content| CordialBlockPayload::from_bytes(&content.payload).ok())
            {
                Some(payload) => payload,
                None => continue,
            };
            let round = match depth(ingress.blocklace(), &block.identity) {
                Some(round) => round,
                None => continue,
            };
            let wave = match wave_of_round(round, 3) {
                Some(wave) => wave,
                None => continue,
            };
            let is_final = is_weighted_final_leader(
                ingress.blocklace(),
                &block.identity,
                3,
                ingress.bonds(),
                |w| validators.get((w as usize) % validators.len()).cloned(),
            );

            println!(
                "  block_number={} hash={} creator={} round={} wave={} weighted_final={}",
                payload.state.block_number, hash, creator, round, wave, is_final
            );
        }
    }
}

fn mirrored_blocks_in_height_range(
    ingress: &LiveIngress<PassthroughAdapter>,
    min_height: u64,
    max_height: u64,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for id in ingress.blocklace().dom() {
        let Some(content) = ingress.blocklace().content(id) else {
            continue;
        };
        let Ok(payload) = CordialBlockPayload::from_bytes(&content.payload) else {
            continue;
        };
        if (min_height..=max_height).contains(&payload.state.block_number)
            && let Some(block) = ingress.blocklace().get(id)
        {
            blocks.push(block);
        }
    }
    blocks.sort_by_key(|block| {
        ingress
            .blocklace()
            .content(&block.identity)
            .and_then(|content| CordialBlockPayload::from_bytes(&content.payload).ok())
            .map(|payload| payload.state.block_number)
            .unwrap_or(u64::MAX)
    });
    blocks
}

async fn bootstrap_by_heights(
    grpc: &mut LiveGrpcBlockClient,
    ingress: &mut LiveIngress<PassthroughAdapter>,
    target_max_height: i64,
    batch_size: i64,
    parents_only_bootstrap: bool,
    decoded_messages: &mut usize,
) -> Result<usize> {
    let mut start = 0i64;
    let mut bootstrapped = 0usize;

    println!(
        "[height-bootstrap] target_max_height={}, batch_size={}",
        target_max_height, batch_size
    );

    while start <= target_max_height {
        let end = (start + batch_size - 1).min(target_max_height);
        let blocks = grpc
            .light_blocks_by_heights(start, end)
            .await
            .with_context(|| {
                format!("failed to fetch blocks in height range {}..={}", start, end)
            })?;

        println!(
            "[height-bootstrap] range {}..={} returned {} blocks",
            start,
            end,
            blocks.len()
        );

        for info in &blocks {
            let _ = light_block_info_to_block_message(info)
                .with_context(|| format!("failed to decode live block {}", info.block_hash))?;
            *decoded_messages += 1;

            let block =
                trusted_block_from_light_block_info_with_options(info, !parents_only_bootstrap)
                    .with_context(|| {
                        format!("failed to reconstruct trusted block {}", info.block_hash)
                    })?;
            ingress
                .ingest_trusted_block(block)
                .with_context(|| format!("failed to mirror live block {}", info.block_hash))?;
            bootstrapped += 1;
        }

        println!(
            "[height-bootstrap] after range {}..={}: mirrored={}, pending={}",
            start,
            end,
            ingress.blocklace().dom().len(),
            ingress.pending_blocks().len()
        );

        start = end + 1;
    }

    Ok(bootstrapped)
}

async fn backfill_missing_predecessors(
    grpc: &mut LiveGrpcBlockClient,
    ingress: &mut LiveIngress<PassthroughAdapter>,
    max_rounds: usize,
    max_blocks: usize,
    parents_only_bootstrap: bool,
) -> Result<usize> {
    let mut attempted = HashSet::new();
    let mut backfilled = 0usize;
    let mut round = 0usize;

    loop {
        if round >= max_rounds {
            println!(
                "[backfill] stopping after {} rounds with {} blocks fetched",
                round, backfilled
            );
            break;
        }

        let missing = unresolved_predecessor_hashes(ingress);
        let frontier: Vec<String> = missing
            .into_iter()
            .filter(|hash| attempted.insert(hash.clone()))
            .collect();

        round += 1;
        println!(
            "[backfill] round {}: mirrored={}, pending={}, unresolved={}",
            round,
            ingress.blocklace().dom().len(),
            ingress.pending_blocks().len(),
            frontier.len()
        );
        if let Some(first) = frontier.first() {
            println!("[backfill] current unresolved head={}", first);
        }

        if frontier.is_empty() {
            println!("[backfill] no unresolved predecessor hashes remain");
            break;
        }

        let mut fetched_this_round = 0usize;
        for hash in frontier {
            if backfilled >= max_blocks {
                println!(
                    "[backfill] stopping after reaching max_backfill_blocks={}",
                    max_blocks
                );
                return Ok(backfilled);
            }

            let info = grpc
                .light_block_by_hash(hash.clone())
                .await
                .with_context(|| format!("failed to backfill missing predecessor {hash}"))?;

            let block =
                trusted_block_from_light_block_info_with_options(&info, !parents_only_bootstrap)
                    .with_context(|| {
                        format!("failed to reconstruct backfilled block {}", info.block_hash)
                    })?;
            ingress.ingest_trusted_block(block).with_context(|| {
                format!("failed to ingest backfilled block {}", info.block_hash)
            })?;
            backfilled += 1;
            fetched_this_round += 1;

            if fetched_this_round <= 5 || fetched_this_round.is_multiple_of(25) {
                println!(
                    "[backfill] fetched {} this round ({} total), latest={}, block_number={}, parents={}, justifications={}",
                    fetched_this_round,
                    backfilled,
                    hash,
                    info.block_number,
                    info.parents_hash_list.len(),
                    info.justifications.len()
                );
            }
        }

        if fetched_this_round == 0 {
            println!("[backfill] round {} made no progress", round);
            break;
        }

        println!(
            "[backfill] round {} complete: mirrored={}, pending={}",
            round,
            ingress.blocklace().dom().len(),
            ingress.pending_blocks().len()
        );
    }

    Ok(backfilled)
}

fn unresolved_predecessor_hashes(ingress: &LiveIngress<PassthroughAdapter>) -> BTreeSet<String> {
    let applied: HashSet<String> = ingress
        .blocklace()
        .dom()
        .iter()
        .map(|id| PrettyPrinter::build_string_no_limit(&id.content_hash))
        .collect();
    let pending: HashSet<String> = ingress
        .pending_blocks()
        .keys()
        .map(|id| PrettyPrinter::build_string_no_limit(&id.content_hash))
        .collect();

    let mut missing = BTreeSet::new();
    for block in ingress.pending_blocks().values() {
        for pred in &block.content.predecessors {
            let hash = PrettyPrinter::build_string_no_limit(&pred.content_hash);
            if !applied.contains(&hash) && !pending.contains(&hash) {
                missing.insert(hash);
            }
        }
    }

    missing
}
