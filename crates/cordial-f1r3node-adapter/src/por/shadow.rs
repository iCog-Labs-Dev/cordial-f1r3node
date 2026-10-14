//! Live PoR shadow host. Cordial remains the source of membership and finality.
//! Optional local rating exchange writes signed envelopes to a shared directory.
//!
//! The entire blocklace is replayed on startup because ingress is process-local.
//! A retained finalized prefix is checked before a recovered view is published.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use casper::rust::util::bonds_parser::BondsParser;
use cordial_miners_core::{Block, NodeId, types::BlockIdentity};
use cordial_por::{PorConfig, ReputationState, authorized_validator_weights};
use serde::{Deserialize, Serialize};

use crate::{
    grpc_ingest::BlocklaceAdapter,
    live_grpc::{LiveGrpcBlockClient, trusted_block_from_light_block_info_with_options},
    live_ingress::LiveIngress,
    ordered_output::OrderedFinalizedOutput,
    shard_conf::CasperShardConf,
    snapshot::CORDIAL_WAVELENGTH,
};

use super::{
    activation::PorWeightActivationRecord,
    finality::{finalized_output_for_wave, finalized_wave_from_output},
    local_ratings::{
        RatingExchangeProgress, close_rating_exchange_if_complete, observe_rating_exchange,
        stage_local_rating_batch,
    },
    rating_key::load_bonded_rating_key,
    runtime::PorRuntime,
    transition::apply_completed_reputation_round,
    transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox},
};

const STATUS_FILE: &str = "shadow-status.json";

struct PassthroughAdapter;

impl BlocklaceAdapter<BlockIdentity> for PassthroughAdapter {
    fn on_block(&mut self, _block: Block) -> Result<()> {
        Ok(())
    }
}

/// Configuration for one observer. The bonds file is the node's authorized set.
#[derive(Debug, Clone)]
pub struct ShadowConfig {
    pub grpc_url: String,
    pub bonds_file: PathBuf,
    pub validator_rating_key_file: Option<PathBuf>,
    pub stage_local_ratings: bool,
    pub rating_exchange_dir: Option<PathBuf>,
    pub commit_complete_ratings: bool,
    pub data_dir: PathBuf,
    pub shard_id: String,
    pub height_batch_size: i64,
    pub poll_interval: Duration,
}

/// Machine-readable local status, atomically replaced after every successful poll.
/// The finalized hashes are retained to prove the complete prefix after restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowStatus {
    pub schema_version: u16,
    pub shadow_mode: bool,
    /// Bonded validator identity validated for future local rating production.
    #[serde(default)]
    pub configured_rating_rater: Option<String>,
    pub active_por_round: u64,
    pub weight_commitment: String,
    pub weights: BTreeMap<String, u64>,
    pub finalized_anchor: Option<String>,
    /// Latest Cordial wave observed as finalized; this is not a committed PoR round.
    #[serde(default)]
    pub observed_finalized_wave: Option<u64>,
    pub finalized_hashes: Vec<String>,
    pub mirrored_blocks: usize,
    pub source_height: i64,
}

impl ShadowStatus {
    fn from_runtime(
        runtime: &PorRuntime<PassthroughAdapter>,
        output: &OrderedFinalizedOutput,
        rating_rater: Option<&str>,
        source_height: i64,
    ) -> Result<Self> {
        let record = runtime
            .state()
            .activated_weight_record()?
            .context("PoR genesis projection was not activated")?;
        let weights = record
            .weights()
            .iter()
            .map(|(node, weight)| (hex::encode(&node.0), *weight))
            .collect();
        Ok(Self {
            schema_version: 1,
            shadow_mode: true,
            configured_rating_rater: rating_rater.map(str::to_owned),
            active_por_round: record.reputation_round(),
            weight_commitment: hex::encode(record.weights_commitment()),
            weights,
            finalized_anchor: output.anchor_hash().map(hex::encode),
            observed_finalized_wave: finalized_wave_from_output(
                runtime.ingress().blocklace(),
                output,
            )?,
            finalized_hashes: output.block_hashes().into_iter().map(hex::encode).collect(),
            mirrored_blocks: output.total_mirrored_blocks,
            source_height,
        })
    }

    fn validate_recovered(&self, recovered: &Self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.shadow_mode,
            "unsupported shadow status"
        );
        self.validate_configured_rater(recovered.configured_rating_rater.as_deref())?;
        ensure!(
            recovered
                .finalized_hashes
                .starts_with(&self.finalized_hashes),
            "replayed Cordial finalized output changed or lost its durable prefix"
        );
        if let Some(previous_wave) = self.observed_finalized_wave {
            let recovered_wave = recovered
                .observed_finalized_wave
                .context("replayed Cordial final leader disappeared")?;
            ensure!(
                recovered_wave >= previous_wave,
                "replayed Cordial finalized wave regressed"
            );
            if recovered_wave == previous_wave {
                ensure!(
                    recovered.finalized_anchor == self.finalized_anchor,
                    "replayed Cordial final leader changed within the same wave"
                );
            }
        }
        ensure!(
            recovered.active_por_round >= self.active_por_round,
            "recovered PoR activation regressed"
        );
        if recovered.active_por_round == self.active_por_round {
            ensure!(
                recovered.weight_commitment == self.weight_commitment
                    && recovered.weights == self.weights,
                "recovered PoR projection differs from the previous publication"
            );
        }
        Ok(())
    }

    fn validate_configured_rater(&self, rater: Option<&str>) -> Result<()> {
        if let Some(previous_rater) = &self.configured_rating_rater {
            ensure!(
                rater == Some(previous_rater.as_str()),
                "configured PoR rating rater changed for this data directory"
            );
        }
        Ok(())
    }
}

fn load_bonds(path: &Path) -> Result<HashMap<NodeId, u64>> {
    let parsed = BondsParser::parse(path)
        .with_context(|| format!("failed to parse bonds file {}", path.display()))?;
    let mut bonds = HashMap::new();
    for (key, stake) in parsed {
        if stake > 0 {
            let previous = bonds.insert(NodeId(key.bytes.to_vec()), stake as u64);
            ensure!(
                previous.is_none(),
                "duplicate active validator in bonds file"
            );
        }
    }
    ensure!(!bonds.is_empty(), "bonds file has no active validators");
    Ok(bonds)
}

fn initial_state(bonds: &HashMap<NodeId, u64>, config: &PorConfig) -> ReputationState {
    let mut state = ReputationState::new(0);
    for node in bonds.keys() {
        state.set_reputation(node.clone(), config.initial_reputation);
    }
    state
}

fn status_path(data_dir: &Path) -> PathBuf {
    data_dir.join("por").join(STATUS_FILE)
}

fn read_status(path: &Path) -> Result<Option<ShadowStatus>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .context("invalid retained shadow status")
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn write_status(path: &Path, status: &ShadowStatus) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(status)?;
    let mut file = File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(path.parent().context("status path has no parent")?)?.sync_all()?;
    Ok(())
}

fn validate_config(config: &ShadowConfig) -> Result<()> {
    ensure!(!config.grpc_url.is_empty(), "gRPC URL must not be empty");
    ensure!(!config.shard_id.is_empty(), "shard ID must not be empty");
    ensure!(
        config.height_batch_size > 0,
        "height batch size must be positive"
    );
    ensure!(
        !config.poll_interval.is_zero(),
        "poll interval must be positive"
    );
    ensure!(
        !config.stage_local_ratings || config.validator_rating_key_file.is_some(),
        "staging local PoR ratings requires a validator rating key file"
    );
    ensure!(
        config.rating_exchange_dir.is_none() || config.stage_local_ratings,
        "PoR rating exchange requires local rating staging"
    );
    ensure!(
        !config.commit_complete_ratings || config.rating_exchange_dir.is_some(),
        "committing complete PoR ratings requires a rating exchange directory"
    );
    Ok(())
}

/// Run until stopped. Transport errors retry; invalid evidence and prefix drift fail closed.
pub async fn run_shadow(config: ShadowConfig) -> Result<()> {
    validate_config(&config)?;
    let bonds = load_bonds(&config.bonds_file)?;
    let rating_key = config
        .validator_rating_key_file
        .as_ref()
        .map(|path| load_bonded_rating_key(path, &bonds))
        .transpose()?;
    let rating_outbox = config
        .stage_local_ratings
        .then(|| RatingOutbox::open(&config.data_dir))
        .transpose()?;
    let rating_exchange = config
        .rating_exchange_dir
        .as_ref()
        .map(|root| FileRatingExchange::open(root, config.shard_id.as_bytes()))
        .transpose()?;
    let authorized_raters: HashSet<NodeId> = bonds.keys().cloned().collect();
    let configured_rating_rater = rating_key
        .as_ref()
        .map(|signer| hex::encode(&signer.rater().0));
    let por_config = PorConfig::default();
    let ingress = LiveIngress::with_consensus_view(
        PassthroughAdapter,
        bonds.clone(),
        CasperShardConf {
            shard_name: config.shard_id.clone(),
            max_number_of_parents: 16,
            fault_tolerance_threshold: 0.333,
            deploy_lifespan: 50,
            min_phlo_price: 1,
            ..CasperShardConf::default()
        },
        &config.shard_id,
    );
    // This restores the durable projection before any live block or finality work.
    let mut runtime = PorRuntime::open(
        &config.data_dir,
        initial_state(&bonds, &por_config),
        ingress,
        por_config,
        config.shard_id.as_bytes(),
        CORDIAL_WAVELENGTH,
    )?;
    let path = status_path(&config.data_dir);
    let previous = read_status(&path)?;
    if let Some(previous) = &previous {
        previous.validate_configured_rater(configured_rating_rater.as_deref())?;
    }
    let mut scanned_height: Option<i64> = None;
    let mut client: Option<LiveGrpcBlockClient> = None;
    let mut last_logged_rating_wave: Option<u64> = None;
    let mut last_logged_exchange_progress: Option<RatingExchangeProgress> = None;
    let mut last_logged_candidate_checkpoint: Option<[u8; 32]> = None;
    let mut last_logged_deferred_wave: Option<u64> = None;

    loop {
        if client.is_none() {
            match LiveGrpcBlockClient::connect(config.grpc_url.clone()).await {
                Ok(connected) => client = Some(connected),
                Err(error) => {
                    tracing::warn!(%error, "waiting for f1r3node gRPC");
                    tokio::time::sleep(config.poll_interval).await;
                    continue;
                }
            }
        }
        let grpc = client.as_mut().expect("client connected above");
        let head = match grpc.recent_light_blocks(1).await {
            Ok(blocks) => blocks.into_iter().map(|block| block.block_number).max(),
            Err(error) => {
                tracing::warn!(%error, "gRPC head query failed");
                client = None;
                tokio::time::sleep(config.poll_interval).await;
                continue;
            }
        };
        let Some(head) = head else {
            tokio::time::sleep(config.poll_interval).await;
            continue;
        };
        ensure!(head >= 0, "negative live block height");
        if let Some(previous_head) = scanned_height {
            ensure!(
                head >= previous_head,
                "live source height regressed from {previous_head} to {head}"
            );
        }
        // Revisit the last two heights to catch late forks at the chain tip.
        let mut start = scanned_height.map_or(0, |height| height.saturating_sub(2).max(0));
        let mut transport_failed = false;
        while start <= head {
            let end = start.saturating_add(config.height_batch_size - 1).min(head);
            let blocks = match grpc.light_blocks_by_heights(start, end).await {
                Ok(blocks) => blocks,
                Err(error) => {
                    tracing::warn!(%error, start, end, "gRPC height query failed");
                    client = None;
                    transport_failed = true;
                    break;
                }
            };
            let block_count = blocks.len();
            for info in blocks {
                let block = trusted_block_from_light_block_info_with_options(&info, true)
                    .with_context(|| format!("invalid live block {}", info.block_hash))?;
                runtime
                    .ingress_mut()
                    .ingest_trusted_block(block)
                    .with_context(|| format!("cannot mirror block {}", info.block_hash))?;
            }
            scanned_height = Some(end);
            tracing::info!(start, end, block_count, "PoR shadow replayed height batch");
            if end == head {
                break;
            }
            start = end.checked_add(1).context("block height overflow")?;
        }
        if transport_failed {
            tokio::time::sleep(config.poll_interval).await;
            continue;
        }
        let mut output = runtime.publish_finalized_output()?;
        if runtime.state().has_unactivated_committed_round()? {
            let activation = runtime.retry_weight_activation()?;
            tracing::info!(
                ?activation,
                "restored pending PoR weight activation after replay"
            );
            output = runtime.publish_finalized_output()?;
        }
        let mut status = ShadowStatus::from_runtime(
            &runtime,
            &output,
            configured_rating_rater.as_deref(),
            head,
        )?;
        if let Some(previous) = &previous {
            previous.validate_recovered(&status)?;
        }
        // Also prevent regression within this process, including late forks.
        if let Some(published) = read_status(&path)? {
            published.validate_recovered(&status)?;
        }
        if let (Some(key), Some(outbox)) = (&rating_key, &rating_outbox) {
            let state = runtime.state().state()?;
            let expected_wave = state.round();
            // This wave is replayed under the currently active weights. After
            // activation, only future waves may be committed by this host.
            if let Some(rating_output) = finalized_output_for_wave(
                runtime.ingress().blocklace(),
                &output,
                runtime.ingress().bonds(),
                expected_wave,
            )? {
                if let Some(staged) = stage_local_rating_batch(
                    runtime.ingress().blocklace(),
                    &rating_output,
                    state,
                    runtime.config(),
                    key,
                    outbox,
                )? {
                    if last_logged_rating_wave != Some(staged.finalized_wave) {
                        tracing::info!(
                            finalized_wave = staged.finalized_wave,
                            rating_round = staged.rating_round,
                            rating_count = staged.rating_count,
                            "staged local PoR ratings"
                        );
                        last_logged_rating_wave = Some(staged.finalized_wave);
                    }
                    if let Some(exchange) = &rating_exchange {
                        let progress = observe_rating_exchange(
                            runtime.ingress().blocklace(),
                            &rating_output,
                            state,
                            runtime.config(),
                            key,
                            &authorized_raters,
                            outbox,
                            exchange,
                        )?;
                        if last_logged_exchange_progress != Some(progress) {
                            tracing::info!(
                                finalized_wave = progress.finalized_wave,
                                rating_round = progress.rating_round,
                                local_envelopes = progress.local_envelopes,
                                peer_envelopes = progress.peer_envelopes,
                                completed_raters = progress.completed_raters,
                                eligible_raters = progress.eligible_raters,
                                quorum_reached = progress.quorum_reached,
                                all_raters_complete = progress.all_raters_complete,
                                "observed PoR rating exchange"
                            );
                            last_logged_exchange_progress = Some(progress);
                        }
                        if progress.all_raters_complete {
                            if let Some(completed) = close_rating_exchange_if_complete(
                                runtime.ingress().blocklace(),
                                &rating_output,
                                state,
                                runtime.config(),
                                key,
                                &authorized_raters,
                                outbox,
                                exchange,
                            )? {
                                let mut candidate_state = state.clone();
                                apply_completed_reputation_round(
                                    &completed,
                                    &mut candidate_state,
                                    runtime.config(),
                                    runtime.shard_id(),
                                )?;
                                let mut validators: Vec<_> =
                                    runtime.ingress().bonds().keys().cloned().collect();
                                validators.sort();
                                let weights =
                                    authorized_validator_weights(&candidate_state, &validators)?;
                                let candidate = PorWeightActivationRecord::from_state_and_weights(
                                    &candidate_state,
                                    &weights,
                                )?;
                                let commitment = candidate.weights_commitment();
                                let checkpoint = candidate
                                    .checkpoint_hash()
                                    .context("complete PoR candidate has no checkpoint")?;
                                if last_logged_candidate_checkpoint != Some(checkpoint) {
                                    tracing::info!(
                                        source_finalized_wave = completed.opened().finalized_wave,
                                        candidate_por_round = candidate.reputation_round(),
                                        candidate_checkpoint = %hex::encode(checkpoint),
                                        candidate_weight_commitment = %hex::encode(commitment),
                                        "derived complete PoR round candidate"
                                    );
                                    last_logged_candidate_checkpoint = Some(checkpoint);
                                }
                                if config.commit_complete_ratings {
                                    if status.observed_finalized_wave == Some(expected_wave) {
                                        runtime.ingress().validate_por_weights(&candidate_state)?;
                                        let before_commit = status.clone();
                                        let committed =
                                            runtime.commit_completed_round(&completed)?;
                                        match committed.activation {
                                            Ok(activation) => tracing::info!(
                                                committed_por_round = candidate.reputation_round(),
                                                ?activation,
                                                "committed and activated complete PoR round"
                                            ),
                                            Err(error) => anyhow::bail!(
                                                "PoR round {} committed but activation is pending: {error}",
                                                candidate.reputation_round()
                                            ),
                                        }
                                        output = runtime.publish_finalized_output()?;
                                        status = ShadowStatus::from_runtime(
                                            &runtime,
                                            &output,
                                            configured_rating_rater.as_deref(),
                                            head,
                                        )?;
                                        before_commit.validate_recovered(&status)?;
                                    } else if last_logged_deferred_wave != Some(expected_wave) {
                                        tracing::warn!(
                                            source_finalized_wave = expected_wave,
                                            observed_finalized_wave = ?status.observed_finalized_wave,
                                            "PoR commit waits for the source wave to be the current finalized wave"
                                        );
                                        last_logged_deferred_wave = Some(expected_wave);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        write_status(&path, &status)?;
        println!("{}", serde_json::to_string(&status)?);
        tokio::time::sleep(config.poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(hashes: &[&str]) -> ShadowStatus {
        ShadowStatus {
            schema_version: 1,
            shadow_mode: true,
            configured_rating_rater: None,
            active_por_round: 0,
            weight_commitment: "abc".into(),
            weights: BTreeMap::from([("key".into(), 1)]),
            finalized_anchor: hashes.last().map(|hash| hash.to_string()),
            observed_finalized_wave: None,
            finalized_hashes: hashes.iter().map(|hash| hash.to_string()).collect(),
            mirrored_blocks: hashes.len(),
            source_height: 10,
        }
    }

    #[test]
    fn recovered_prefix_and_projection_are_checked() {
        let old = status(&["a", "b"]);
        old.validate_recovered(&status(&["a", "b", "c"])).unwrap();
        assert!(old.validate_recovered(&status(&["a", "x"])).is_err());
        assert!(old.validate_recovered(&status(&["a"])).is_err());
        let mut drifted = status(&["a", "b"]);
        drifted.weights.insert("key".into(), 2);
        assert!(old.validate_recovered(&drifted).is_err());
    }

    #[test]
    fn recovered_finalized_wave_cannot_regress_or_change_leader() {
        let mut old = status(&["a", "b"]);
        old.observed_finalized_wave = Some(2);
        let mut recovered = status(&["a", "b", "c"]);
        recovered.observed_finalized_wave = Some(3);
        old.validate_recovered(&recovered).unwrap();

        recovered.observed_finalized_wave = Some(1);
        assert!(old.validate_recovered(&recovered).is_err());
        recovered.observed_finalized_wave = None;
        assert!(old.validate_recovered(&recovered).is_err());
        recovered.observed_finalized_wave = Some(2);
        assert!(old.validate_recovered(&recovered).is_err());
    }

    #[test]
    fn old_status_without_observed_wave_remains_readable() {
        let mut value = serde_json::to_value(status(&["a"])).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("observed_finalized_wave");
        value
            .as_object_mut()
            .unwrap()
            .remove("configured_rating_rater");
        let previous: ShadowStatus = serde_json::from_value(value).unwrap();
        assert_eq!(previous.observed_finalized_wave, None);
        assert_eq!(previous.configured_rating_rater, None);
        let mut recovered = status(&["a", "b"]);
        recovered.observed_finalized_wave = Some(1);
        previous.validate_recovered(&recovered).unwrap();
    }

    #[test]
    fn recovered_rating_identity_must_match_when_configured() {
        let mut old = status(&["a"]);
        old.configured_rating_rater = Some("validator-1".into());
        let mut recovered = status(&["a", "b"]);
        recovered.configured_rating_rater = Some("validator-1".into());
        old.validate_recovered(&recovered).unwrap();

        recovered.configured_rating_rater = Some("validator-2".into());
        assert!(old.validate_recovered(&recovered).is_err());
        recovered.configured_rating_rater = None;
        assert!(old.validate_recovered(&recovered).is_err());
    }

    #[test]
    fn genesis_projection_and_status_survive_runtime_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let bonds = HashMap::from([(NodeId(vec![1]), 10), (NodeId(vec![2]), 20)]);
        let config = PorConfig::default();
        let open = || {
            let ingress = LiveIngress::with_consensus_view(
                PassthroughAdapter,
                bonds.clone(),
                CasperShardConf::default(),
                "root",
            );
            PorRuntime::open(
                directory.path(),
                initial_state(&bonds, &config),
                ingress,
                config.clone(),
                b"root".to_vec(),
                CORDIAL_WAVELENGTH,
            )
            .unwrap()
        };
        let first = open();
        let first_status =
            ShadowStatus::from_runtime(&first, &OrderedFinalizedOutput::default(), None, 0)
                .unwrap();
        assert_eq!(first_status.active_por_round, 0);
        assert_eq!(first_status.weights.len(), 2);
        let path = status_path(directory.path());
        write_status(&path, &first_status).unwrap();
        drop(first);

        let restored = open();
        let restored_status =
            ShadowStatus::from_runtime(&restored, &OrderedFinalizedOutput::default(), None, 0)
                .unwrap();
        assert_eq!(restored_status, first_status);
        read_status(&path)
            .unwrap()
            .unwrap()
            .validate_recovered(&restored_status)
            .unwrap();
    }

    #[test]
    fn config_rejects_invalid_polling() {
        let mut config = ShadowConfig {
            grpc_url: "http://127.0.0.1:40401".into(),
            bonds_file: "bonds.txt".into(),
            validator_rating_key_file: None,
            stage_local_ratings: false,
            rating_exchange_dir: None,
            commit_complete_ratings: false,
            data_dir: "data".into(),
            shard_id: "root".into(),
            height_batch_size: 64,
            poll_interval: Duration::from_secs(1),
        };
        config.height_batch_size = 0;
        assert!(validate_config(&config).is_err());
        config.height_batch_size = 64;
        config.stage_local_ratings = true;
        assert!(validate_config(&config).is_err());
        config.stage_local_ratings = false;
        config.rating_exchange_dir = Some("exchange".into());
        assert!(validate_config(&config).is_err());
        config.stage_local_ratings = true;
        config.validator_rating_key_file = Some("validator.key".into());
        config.rating_exchange_dir = None;
        config.commit_complete_ratings = true;
        assert!(validate_config(&config).is_err());
    }
}
