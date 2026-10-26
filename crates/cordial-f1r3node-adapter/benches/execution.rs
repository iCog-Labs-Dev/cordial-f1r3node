//! Opt-in production RSpace/Rholang execution and replay from a fixed genesis.
//! CORDIAL_BENCH_RSPACE=1 enables this expensive workload. Genesis construction,
//! deploy signing and request cloning are untimed; adapter translation, signature
//! verification, interpreter execution, persistence and bond lookup are timed.
//! Repeated execution uses the same pre-state; it is not a sustained-growth test.
use cordial_f1r3space_adapter::F1r3RspaceRuntime;
use cordial_miners_core::bench_support as support;
use cordial_miners_core::execution::{
    Deploy, DeploySignatureAlgorithm, ExecutionRequest, RuntimeManager, SignedDeploy,
};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::time::Duration;

// Untimed bootstrap, using public genesis/storage/runtime APIs.
use casper::rust::genesis::contracts::proof_of_stake::ProofOfStake;
use casper::rust::genesis::contracts::validator::Validator;
use casper::rust::genesis::contracts::vault::Vault;
use casper::rust::genesis::genesis::Genesis;
use casper::rust::storage::rnode_key_value_store_manager::new_key_value_store_manager;
use casper::rust::util::rholang::runtime_manager::RuntimeManager as HostRuntimeManager;
use cordial_miners_core::execution::Bond;
use cordial_miners_core::types::NodeId;
use crypto::rust::private_key::PrivateKey;
use crypto::rust::signatures::secp256k1::Secp256k1;
use crypto::rust::signatures::signatures_alg::SignaturesAlg;
use rholang::rust::interpreter::external_services::ExternalServices;
use rholang::rust::interpreter::util::vault_address::VaultAddress;
use rspace_plus_plus::rspace::shared::key_value_store_manager::KeyValueStoreManager;
use shared::rust::store::key_value_typed_store_impl::KeyValueTypedStoreImpl;

struct ExecutionFixture {
    runtime: HostRuntimeManager,
    genesis_hash: Vec<u8>,
    genesis_bonds: Vec<Bond>,
    // Keep LMDB's directory alive for the entire benchmark.
    _storage: tempfile::TempDir,
}

/// Explicit development-only payer, matching the host's funded test fixture.
fn deployer_key() -> PrivateKey {
    PrivateKey::from_bytes(
        &hex::decode("a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76")
            .expect("development key"),
    )
}

async fn setup_execution() -> ExecutionFixture {
    let deployer = Secp256k1.to_public(&deployer_key());
    let validator = Secp256k1.to_public(&PrivateKey::from_bytes(
        &hex::decode("5a0bde2f5857124b1379c78535b07a278e3b9cefbcacc02e62ab3294c02765a1")
            .expect("development validator key"),
    ));
    let storage = tempfile::tempdir().expect("temporary LMDB directory");
    let mut kvm = new_key_value_store_manager(storage.path().to_path_buf(), None);
    let rspace = kvm.r_space_stores().await.expect("RSpace stores");
    let mergeable = kvm
        .store("mergeable-channel-cache".to_string())
        .await
        .expect("mergeable channel store");
    let genesis_rt = HostRuntimeManager::create_with_store(
        rspace,
        KeyValueTypedStoreImpl::new(mergeable),
        Genesis::default_mergeable_tags_arc(),
        ExternalServices::noop(),
    );
    let genesis = Genesis {
        shard_id: "root".to_string(),
        timestamp: 0,
        block_number: 0,
        proof_of_stake: ProofOfStake {
            minimum_bond: 1,
            maximum_bond: i64::MAX,
            validators: vec![Validator {
                pk: validator.clone(),
                stake: 1,
            }],
            epoch_length: 1000,
            quarantine_length: 50_000,
            number_of_active_validators: 100,
            fault_tolerance_threshold_ppm: 0,
            max_parent_depth: 15,
            deploy_lifespan: 50,
            min_phlo_price: 0,
            pos_multi_sig_public_keys: vec![
                "04db91a53a2b72fcdcb201031772da86edad1e4979eb6742928d27731b1771e0bc40c9e9c9fa6554bdec041a87cee423d6f2e09e9dfb408b78e85a4aa611aad20c".to_string(),
                "042a736b30fffcc7d5a58bb9416f7e46180818c82b15542d0a7819d1a437aa7f4b6940c50db73a67bfc5f5ec5b5fa555d24ef8339b03edaa09c096de4ded6eae14".to_string(),
                "047f0f0f5bbe1d6d1a8dac4d88a3957851940f39a57cd89d55fe25b536ab67e6d76fd3f365c83e5bfe11fe7117e549b1ae3dd39bfc867d1c725a4177692c4e7754".to_string(),
            ],
            pos_multi_sig_quorum: 2,
        },
        vaults: vec![
            Vault {
                vault_address: VaultAddress::from_public_key(&deployer)
                    .expect("default deployer vault address"),
                initial_balance: 9_000_000,
            },
            Vault {
                vault_address: VaultAddress::from_public_key(&validator)
                    .expect("validator vault address"),
                initial_balance: 0,
            },
        ],
        supply: i64::MAX,
        version: 1,
        native_token_name: "F1R3CAP".to_string(),
        native_token_symbol: "F1R3".to_string(),
        native_token_decimals: 8,
        fs_bundle: vec![],
        consensus_fs_snapshot_cadence: None,
    };
    let block = Genesis::create_genesis_block(&genesis_rt, &genesis)
        .await
        .expect("create genesis block");
    let rspace = kvm.r_space_stores().await.expect("RSpace history stores");
    let mergeable = kvm
        .store("mergeable-channel-cache".to_string())
        .await
        .expect("mergeable history store");
    let (rt, _history_repo) = HostRuntimeManager::create_with_history(
        rspace,
        KeyValueTypedStoreImpl::new(mergeable),
        Genesis::default_mergeable_tags_arc(),
        ExternalServices::noop(),
    );

    let mut genesis_bonds: Vec<Bond> = block
        .body
        .state
        .bonds
        .iter()
        .map(|bond| Bond {
            validator: NodeId(bond.validator.to_vec()),
            stake: u64::try_from(bond.stake).expect("genesis stake must be non-negative"),
        })
        .collect();
    genesis_bonds.sort_by(|left, right| left.validator.0.cmp(&right.validator.0));

    ExecutionFixture {
        runtime: rt,
        genesis_hash: block.body.state.post_state_hash.to_vec(),
        _storage: storage,
        genesis_bonds,
    }
}

fn deploy(index: usize) -> SignedDeploy {
    let signed = casper::rust::util::construct_deploy::source_deploy(
        format!("@{index}!(\"criterion-{index}\")"),
        1_700_000_000_000 + index as i64,
        Some(100_000),
        Some(1),
        Some(deployer_key()),
        Some(0),
        Some("root".into()),
    )
    .expect("signed funded genesis deploy");
    SignedDeploy {
        deploy: Deploy {
            term: signed.data.term.as_bytes().to_vec(),
            timestamp: signed.data.time_stamp as u64,
            phlo_price: signed.data.phlo_price as u64,
            phlo_limit: signed.data.phlo_limit as u64,
            valid_after_block_number: signed.data.valid_after_block_number as u64,
            shard_id: signed.data.shard_id,
            expiration_timestamp: signed.data.expiration_timestamp.map(|x| x as u64),
        },
        deployer: signed.pk.bytes.to_vec(),
        signature: signed.sig.to_vec(),
        signature_algorithm: DeploySignatureAlgorithm::Secp256k1,
    }
}
fn bench_execution(c: &mut Criterion) {
    if std::env::var("CORDIAL_BENCH_RSPACE").as_deref() != Ok("1") {
        eprintln!("execution skipped; set CORDIAL_BENCH_RSPACE=1 to bootstrap real RSpace");
        return;
    }
    let tokio = tokio::runtime::Runtime::new().expect("tokio");
    let ExecutionFixture {
        runtime: mut host,
        genesis_hash,
        genesis_bonds: bonds,
        _storage,
    } = tokio.block_on(setup_execution());
    // Enter a multi-thread runtime on a synchronous thread, so the adapter can
    // call Handle::current().block_on without nesting a running async task.
    let _entered = tokio.enter();
    let mut adapter = F1r3RspaceRuntime::new(&mut host);
    let mut group = c.benchmark_group("execution/real_rspace");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    let sizes: &[usize] = if support::smoke() {
        &[1]
    } else if support::large() {
        &[1, 10, 32]
    } else {
        &[1, 10]
    };
    for &n in sizes {
        let request = ExecutionRequest {
            pre_state_hash: genesis_hash.clone(),
            deploys: (0..n).map(deploy).collect(),
            system_deploys: vec![],
            bonds: bonds.clone(),
            block_number: 1,
        };
        let expected = adapter
            .execute_block(request.clone())
            .expect("preflight execution");
        assert_eq!(expected.processed_deploys.len(), n);
        assert!(expected.rejected_deploys.is_empty());
        assert!(expected.processed_deploys.iter().all(|d| !d.is_failed));
        assert_ne!(expected.post_state_hash, genesis_hash);
        assert!(
            adapter
                .validate_post_state(request.clone(), &expected.post_state_hash)
                .expect("deterministic replay")
        );
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("execute_from_genesis", n), |b| {
            b.iter_batched(
                || request.clone(),
                |request| adapter.execute_block(request).expect("execute"),
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("validate_post_state_replay", n), |b| {
            b.iter_batched(
                || request.clone(),
                |request| {
                    adapter
                        .validate_post_state(request, &expected.post_state_hash)
                        .expect("replay")
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}
criterion_group!(benches, bench_execution);
criterion_main!(benches);
