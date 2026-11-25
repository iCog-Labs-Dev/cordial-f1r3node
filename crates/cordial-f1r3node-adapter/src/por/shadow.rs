//! Read-only live PoR host. Cordial remains the source of membership and finality.
//!
//! The entire blocklace is replayed on startup because ingress is process-local.
//! A retained finalized prefix is checked before a recovered view is published.

use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use casper::rust::util::bonds_parser::BondsParser;
use cordial_miners_core::{Block, NodeId, types::BlockIdentity};
use cordial_por::{PorConfig, ReputationState};
use serde::{Deserialize, Serialize};

use crate::{
    grpc_ingest::BlocklaceAdapter,
    live_grpc::{LiveGrpcBlockClient, trusted_block_from_light_block_info_with_options},
    live_ingress::LiveIngress,
    ordered_output::OrderedFinalizedOutput,
    shard_conf::CasperShardConf,
    snapshot::CORDIAL_WAVELENGTH,
};

use super::runtime::PorRuntime;

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
    pub active_por_round: u64,
    pub weight_commitment: String,
    pub weights: BTreeMap<String, u64>,
    pub finalized_anchor: Option<String>,
    pub finalized_hashes: Vec<String>,
    pub mirrored_blocks: usize,
    pub source_height: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusSummary {
    active_por_round: u64,
    weight_commitment: String,
    weight_count: usize,
    finalized_anchor: Option<String>,
    finalized_count: usize,
    mirrored_blocks: usize,
    source_height: i64,
}

impl From<&ShadowStatus> for StatusSummary {
    fn from(status: &ShadowStatus) -> Self {
        Self {
            active_por_round: status.active_por_round,
            weight_commitment: status.weight_commitment.clone(),
            weight_count: status.weights.len(),
            finalized_anchor: status.finalized_anchor.clone(),
            finalized_count: status.finalized_hashes.len(),
            mirrored_blocks: status.mirrored_blocks,
            source_height: status.source_height,
        }
    }
}

fn log_status_if_changed(status: &ShadowStatus, previous: &mut Option<StatusSummary>) {
    let summary = StatusSummary::from(status);
    if previous.as_ref() == Some(&summary) {
        return;
    }
    tracing::info!(
        active_por_round = summary.active_por_round,
        weight_commitment = %summary.weight_commitment,
        weight_count = summary.weight_count,
        finalized_count = summary.finalized_count,
        finalized_anchor = summary.finalized_anchor.as_deref().unwrap_or("none"),
        mirrored_blocks = summary.mirrored_blocks,
        source_height = summary.source_height,
        "PoR shadow status"
    );
    *previous = Some(summary);
}

impl ShadowStatus {
    fn from_runtime(
        runtime: &PorRuntime<PassthroughAdapter>,
        output: &OrderedFinalizedOutput,
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
            active_por_round: record.reputation_round(),
            weight_commitment: hex::encode(record.weights_commitment()),
            weights,
            finalized_anchor: output.anchor_hash().map(hex::encode),
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
        ensure!(
            recovered
                .finalized_hashes
                .starts_with(&self.finalized_hashes),
            "replayed Cordial finalized output changed or lost its durable prefix"
        );
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
    Ok(())
}

/// Run until stopped. Transport errors retry; invalid evidence and prefix drift fail closed.
pub async fn run_shadow(config: ShadowConfig) -> Result<()> {
    validate_config(&config)?;
    let bonds = load_bonds(&config.bonds_file)?;
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
    let mut scanned_height: Option<i64> = None;
    let mut client: Option<LiveGrpcBlockClient> = None;
    let mut last_logged_summary = None;

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
            tracing::debug!(start, end, block_count, "PoR shadow replayed height batch");
            if end == head {
                break;
            }
            start = end.checked_add(1).context("block height overflow")?;
        }
        if transport_failed {
            tokio::time::sleep(config.poll_interval).await;
            continue;
        }
        let output = runtime.publish_finalized_output()?;
        let status = ShadowStatus::from_runtime(&runtime, &output, head)?;
        if let Some(previous) = &previous {
            previous.validate_recovered(&status)?;
        }
        // Also prevent regression within this process, including late forks.
        if let Some(published) = read_status(&path)? {
            published.validate_recovered(&status)?;
        }
        write_status(&path, &status)?;
        log_status_if_changed(&status, &mut last_logged_summary);
        tokio::time::sleep(config.poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct TestWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn status(hashes: &[&str]) -> ShadowStatus {
        ShadowStatus {
            schema_version: 1,
            shadow_mode: true,
            active_por_round: 0,
            weight_commitment: "abc".into(),
            weights: BTreeMap::from([("key".into(), 1)]),
            finalized_anchor: hashes.last().map(|hash| hash.to_string()),
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
    fn status_summary_logs_first_and_changed_status_but_suppresses_repeats() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = Arc::clone(&output);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || TestWriter(Arc::clone(&writer_output)))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let first = status(&["a", "b"]);
            let mut last_logged = None;
            log_status_if_changed(&first, &mut last_logged);
            log_status_if_changed(&first, &mut last_logged);

            let mut changed = first;
            changed.source_height += 1;
            log_status_if_changed(&changed, &mut last_logged);
        });

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert_eq!(output.matches("PoR shadow status").count(), 2);
        assert!(output.contains("source_height=10"));
        assert!(output.contains("source_height=11"));
    }

    #[test]
    fn status_summary_stays_bounded_for_long_finalized_history() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = Arc::clone(&output);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || TestWriter(Arc::clone(&writer_output)))
            .finish();

        let mut long_history = status(&[]);
        long_history.finalized_hashes = (0..10_000).map(|index| format!("{index:064x}")).collect();
        long_history.finalized_anchor = long_history.finalized_hashes.last().cloned();
        long_history.mirrored_blocks = long_history.finalized_hashes.len();
        let first_hash = long_history.finalized_hashes[0].clone();

        tracing::subscriber::with_default(subscriber, || {
            log_status_if_changed(&long_history, &mut None);
        });

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(output.len() < 512, "summary was {} bytes", output.len());
        assert!(!output.contains(&first_hash));
        assert!(!output.contains("finalized_hashes"));
        assert!(output.contains("active_por_round=0"));
        assert!(output.contains("weight_commitment=abc"));
        assert!(output.contains("weight_count=1"));
        assert!(output.contains("finalized_count=10000"));
        assert!(output.contains("finalized_anchor="));
        assert!(
            output.contains("000000000000000000000000000000000000000000000000000000000000270f")
        );
        assert!(output.contains("mirrored_blocks=10000"));
        assert!(output.contains("source_height=10"));
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
            ShadowStatus::from_runtime(&first, &OrderedFinalizedOutput::default(), 0).unwrap();
        assert_eq!(first_status.active_por_round, 0);
        assert_eq!(first_status.weights.len(), 2);
        let path = status_path(directory.path());
        write_status(&path, &first_status).unwrap();
        drop(first);

        let restored = open();
        let restored_status =
            ShadowStatus::from_runtime(&restored, &OrderedFinalizedOutput::default(), 0).unwrap();
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
            data_dir: "data".into(),
            shard_id: "root".into(),
            height_batch_size: 64,
            poll_interval: Duration::from_secs(1),
        };
        config.height_batch_size = 0;
        assert!(validate_config(&config).is_err());
    }
}
