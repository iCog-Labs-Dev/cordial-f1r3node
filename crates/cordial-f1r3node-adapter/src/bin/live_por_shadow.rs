//! One-node PoR observer. Emits JSON status and can stage local signed ratings.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::Parser;
use cordial_f1r3node_adapter::por::{
    DEFAULT_POR_LEADER_THRESHOLD_DENOMINATOR, DEFAULT_POR_LEADER_THRESHOLD_NUMERATOR,
    ReputationThreshold,
    shadow::{ShadowConfig, ShadowLeaderComparisonPolicy, run_shadow},
};

#[derive(Parser)]
#[command(name = "live-por-shadow")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:51401")]
    grpc_url: String,
    #[arg(long)]
    bonds_file: PathBuf,
    #[arg(long)]
    validator_rating_key_file: Option<PathBuf>,
    #[arg(long)]
    stage_local_ratings: bool,
    #[arg(long)]
    rating_exchange_dir: Option<PathBuf>,
    #[arg(long)]
    commit_complete_ratings: bool,
    /// Use finalized-prefix checkpoint rounds and v2 rating envelopes.
    #[arg(long)]
    checkpoint_ratings: bool,
    /// Activate committed checkpoint weights at the fixed future wave.
    #[arg(long)]
    checkpoint_epoch_delay_waves: Option<u64>,
    /// Report round-robin and PoR weighted-fair leaders without changing consensus.
    #[arg(long)]
    leader_comparison_schedule_length: Option<usize>,
    /// Strict reputation fraction numerator for the PoR leader-candidate set.
    #[arg(long, default_value_t = DEFAULT_POR_LEADER_THRESHOLD_NUMERATOR)]
    leader_candidate_threshold_numerator: u64,
    /// Strict reputation fraction denominator for the PoR leader-candidate set.
    #[arg(long, default_value_t = DEFAULT_POR_LEADER_THRESHOLD_DENOMINATOR)]
    leader_candidate_threshold_denominator: u64,
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
    let leader_comparison_policy =
        if let Some(schedule_length) = args.leader_comparison_schedule_length {
            Some(ShadowLeaderComparisonPolicy {
                schedule_length,
                candidate_threshold: ReputationThreshold::new(
                    args.leader_candidate_threshold_numerator,
                    args.leader_candidate_threshold_denominator,
                )?,
            })
        } else {
            None
        };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    run_shadow(ShadowConfig {
        grpc_url: args.grpc_url,
        bonds_file: args.bonds_file,
        validator_rating_key_file: args.validator_rating_key_file,
        stage_local_ratings: args.stage_local_ratings,
        rating_exchange_dir: args.rating_exchange_dir,
        commit_complete_ratings: args.commit_complete_ratings,
        checkpoint_ratings: args.checkpoint_ratings,
        checkpoint_epoch_delay_waves: args.checkpoint_epoch_delay_waves,
        leader_comparison_policy,
        data_dir: args.data_dir,
        shard_id: args.shard_id,
        height_batch_size: args.height_batch_size,
        poll_interval: Duration::from_millis(args.poll_interval_ms),
    })
    .await
}
