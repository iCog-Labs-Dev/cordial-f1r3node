//! End-to-end Rholang execution tests via `F1r3RspaceRuntime`.
//!
//! These tests exercise the full adapter stack:
//!
//!   `ExecutionRequest`
//!     → `F1r3RspaceRuntime::execute_block`
//!     → f1r3node `RuntimeManager::compute_state` (real LMDB + Rholang)
//!     → `ExecutionResult`
//!
//! ## Bootstrap
//!
//! Each test calls `setup()` to build a genesis block in temporary LMDB
//! storage and return its state hash with a connected `RuntimeManager`.
//!
//! ## Running
//!
//! ```sh
//! just e2e-rholang
//! ```
//!
//! or directly:
//!
//! ```sh
//! cargo +nightly-2025-06-15 test -p cordial-f1r3space-adapter \
//!     --test test_e2e_execution -- --ignored --nocapture
//! ```
//!
//! ## Why `#[ignore]`
//!
//! Genesis bootstrapping takes 10–30 seconds and requires a Tokio runtime.
//! These tests are skipped in `cargo test` / `just test` to keep the normal
//! CI cycle fast. They must be run explicitly with `-- --ignored`.
//!
//! ## What these tests prove
//!
//! | Test | Invariant |
//! |------|-----------|
//! | `execute_block_changes_state_hash` | A valid deploy mutates the tuplespace (post-hash ≠ pre-hash) |
//! | `deploy_appears_in_processed_list` | The submitted deploy is visible in `ExecutionResult.processed_deploys` |
//! | `failed_deploy_surfaces_as_is_failed` | A runtime-failing deploy appears with `is_failed: true` — not silently dropped |
//! | `close_block_system_deploy_executes` | `SystemDeployRequest::CloseBlock` runs without error |
//! | `multiple_deploys_all_appear_in_result` | Multi-deploy blocks return all deploys in the result |
//! | `slash_updates_post_state_bonds` | A successful slash is reflected in `new_bonds` |
//!
//! ## Note on `rejected_deploys`
//!
//! `ExecutionResult.rejected_deploys` is always empty. This is correct:
//! f1r3node's `compute_state` does not expose a separate rejected list —
//! failed deploys appear in `processed_deploys` with `is_failed: true`.
//! See `lib.rs:161`.

use casper::rust::genesis::contracts::proof_of_stake::ProofOfStake;
use casper::rust::genesis::contracts::validator::Validator;
use casper::rust::genesis::contracts::vault::Vault;
use casper::rust::genesis::genesis::Genesis;
use casper::rust::storage::rnode_key_value_store_manager::new_key_value_store_manager;
use casper::rust::util::construct_deploy::{DEFAULT_PUB, DEFAULT_PUB2};
use casper::rust::util::rholang::runtime_manager::RuntimeManager as F1r3RuntimeManager;
use cordial_f1r3space_adapter::F1r3RspaceRuntime;
use cordial_miners_core::execution::{
    Bond, Deploy, DeploySignatureAlgorithm, ExecutionRequest, ProcessedSystemDeploy, RuntimeError,
    RuntimeManager as CoreRuntimeManager, SignedDeploy, SystemDeployRequest,
};
use cordial_miners_core::types::NodeId;
use rholang::rust::interpreter::external_services::ExternalServices;
use rholang::rust::interpreter::util::vault_address::VaultAddress;
use rspace_plus_plus::rspace::shared::key_value_store_manager::KeyValueStoreManager;
use shared::rust::store::key_value_typed_store_impl::KeyValueTypedStoreImpl;
use tempfile::TempDir;

// ── Bootstrap ─────────────────────────────────────────────────────────────────

/// Bootstrap result: a ready `RuntimeManager` and the genesis state hash.
struct Setup {
    rt: F1r3RuntimeManager,
    genesis_hash: Vec<u8>,
    _storage: TempDir,
    genesis_bonds: Vec<Bond>,
}

/// Build a `RuntimeManager` connected to a bootstrapped genesis state.
///
/// Uses the host's public storage and genesis APIs. The temporary directory
/// stays alive for the entire test so LMDB can read the genesis state.
async fn setup() -> Setup {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let storage = tempfile::tempdir().expect("temporary LMDB directory");
    let mut kvm = new_key_value_store_manager(storage.path().to_path_buf(), None);
    let rspace = kvm.r_space_stores().await.expect("RSpace stores");
    let mergeable = kvm
        .store("mergeable-channel-cache".to_string())
        .await
        .expect("mergeable channel store");
    let genesis_rt = F1r3RuntimeManager::create_with_store(
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
                pk: DEFAULT_PUB2.clone(),
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
                vault_address: VaultAddress::from_public_key(&DEFAULT_PUB)
                    .expect("default deployer vault address"),
                initial_balance: 9_000_000,
            },
            Vault {
                vault_address: VaultAddress::from_public_key(&DEFAULT_PUB2)
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
    let (rt, _history_repo) = F1r3RuntimeManager::create_with_history(
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

    Setup {
        rt,
        genesis_hash: block.body.state.post_state_hash.to_vec(),
        _storage: storage,
        genesis_bonds,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a `SignedDeploy` for a Rholang term string with a unique timestamp.
///
/// `ts_offset` is added to the base timestamp so that multiple deploys in the
/// same block have distinct (deployer, timestamp) identities. f1r3node deduplicates
/// deploys by this pair — two deploys with the same deployer and timestamp in one
/// block cause a `GasRefundFailure`.
///
/// Uses `DEFAULT_PUB` from f1r3node's `construct_deploy` module — this key has a
/// genesis vault with `initial_balance: 9_000_000` REV, enough to cover phlo costs.
fn signed_deploy_at(term: &str, ts_offset: u64) -> SignedDeploy {
    let timestamp = 1_700_000_000_000 + i64::try_from(ts_offset).unwrap();
    let host = casper::rust::util::construct_deploy::source_deploy(
        term.to_owned(),
        timestamp,
        Some(100_000),
        Some(1),
        None,
        Some(0),
        Some("root".to_owned()),
    )
    .expect("fixture deploy should sign");
    SignedDeploy {
        deploy: Deploy {
            term: host.data.term.as_bytes().to_vec(),
            timestamp: u64::try_from(host.data.time_stamp).unwrap(),
            phlo_price: u64::try_from(host.data.phlo_price).unwrap(),
            phlo_limit: u64::try_from(host.data.phlo_limit).unwrap(),
            valid_after_block_number: u64::try_from(host.data.valid_after_block_number).unwrap(),
            shard_id: host.data.shard_id,
            expiration_timestamp: host
                .data
                .expiration_timestamp
                .map(u64::try_from)
                .transpose()
                .unwrap(),
        },
        deployer: host.pk.bytes.to_vec(),
        signature: host.sig.to_vec(),
        signature_algorithm: DeploySignatureAlgorithm::Secp256k1,
    }
}

/// Convenience wrapper: single deploy with offset 0.
fn signed_deploy(term: &str) -> SignedDeploy {
    signed_deploy_at(term, 0)
}

/// A minimal bond set matching the test genesis validator.
fn test_bonds() -> Vec<Bond> {
    vec![Bond {
        validator: NodeId(DEFAULT_PUB2.bytes.to_vec()),
        stake: 1,
    }]
}

/// Wrap deploys + system deploys into an `ExecutionRequest` at genesis + offset.
fn request(
    genesis_hash: Vec<u8>,
    deploys: Vec<SignedDeploy>,
    system_deploys: Vec<SystemDeployRequest>,
    block_number: u64,
) -> ExecutionRequest {
    ExecutionRequest {
        pre_state_hash: genesis_hash,
        deploys,
        system_deploys,
        bonds: test_bonds(),
        block_number,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A valid Rholang deploy against the genesis state must produce a post-state
/// hash that differs from the pre-state hash.
///
/// This is the foundational invariant: `compute_state` must mutate the
/// tuplespace. If post == pre, the deploy had no effect.
///
/// ## Why `multi_thread` + `block_in_place`
///
/// `F1r3RspaceRuntime::execute_block` is a sync function that internally calls
/// `tokio::runtime::Handle::current().block_on(compute_state(...))`. Calling
/// `block_on` from inside a Tokio runtime panics ("cannot start runtime from
/// within a runtime"). The fix:
///
/// - `flavor = "multi_thread"` — spawns a thread pool so workers can be parked
/// - `tokio::task::block_in_place` — parks the current async task and runs the
///   closure on the thread pool without blocking the runtime's event loop
///
/// This pattern is used by all five tests in this file.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn execute_block_changes_state_hash() {
    let Setup {
        mut rt,
        genesis_hash,
        _storage,
        ..
    } = setup().await;
    let mut adapter = F1r3RspaceRuntime::new(&mut rt);

    let req = request(
        genesis_hash.clone(),
        vec![signed_deploy(r#"@0!("hello")"#)],
        vec![],
        1,
    );
    let result = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect("execute_block must not fail for a valid Rholang deploy");

    assert_ne!(
        result.post_state_hash, genesis_hash,
        "post-state hash must differ from pre-state hash after a non-trivial deploy"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn deploy_appears_in_processed_list() {
    let Setup {
        mut rt,
        genesis_hash,
        _storage,
        ..
    } = setup().await;
    let mut adapter = F1r3RspaceRuntime::new(&mut rt);

    let deploy = signed_deploy(r#"@0!("hello")"#);
    let term_bytes = deploy.deploy.term.clone();

    let req = request(genesis_hash, vec![deploy], vec![], 1);
    let result = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect("execute_block must not fail");

    assert_eq!(
        result.processed_deploys.len(),
        1,
        "expected exactly one processed deploy"
    );
    assert_eq!(
        result.processed_deploys[0].deploy.deploy.term, term_bytes,
        "processed deploy term must match the submitted deploy"
    );
    assert!(
        result.rejected_deploys.is_empty(),
        "rejected_deploys is always empty — failures surface via is_failed in processed_deploys"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn failed_deploy_surfaces_as_is_failed() {
    let Setup {
        mut rt,
        genesis_hash,
        _storage,
        ..
    } = setup().await;
    let mut adapter = F1r3RspaceRuntime::new(&mut rt);

    let failing_deploy = signed_deploy(r#"@"result"!(1 + "not_a_number")"#);
    let req = request(genesis_hash, vec![failing_deploy], vec![], 1);
    let result = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect("execute_block must succeed even when a deploy fails at runtime");

    assert_eq!(
        result.processed_deploys.len(),
        1,
        "the failed deploy must appear in processed_deploys (not be dropped)"
    );
    assert!(
        result.processed_deploys[0].is_failed,
        "a phlo-exhausted deploy must have is_failed: true; got is_failed=false"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn close_block_system_deploy_executes() {
    let Setup {
        mut rt,
        genesis_hash,
        _storage,
        genesis_bonds,
    } = setup().await;
    let mut adapter = F1r3RspaceRuntime::new(&mut rt);

    let req = request(
        genesis_hash,
        vec![],
        vec![SystemDeployRequest::CloseBlock],
        1,
    );
    let result = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect("execute_block with CloseBlock must not return an error");

    assert!(
        !result.system_deploys.is_empty(),
        "CloseBlock must produce at least one ProcessedSystemDeploy"
    );
    assert!(
        result
            .system_deploys
            .iter()
            .any(|sd| matches!(sd, ProcessedSystemDeploy::CloseBlock { succeeded: true })),
        "expected a succeeded CloseBlock in system_deploys; got: {:?}",
        result.system_deploys
    );
    assert_eq!(
        result.new_bonds, genesis_bonds,
        "a close-block transition must report the unchanged logical post-state bonds"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn slash_without_verified_epoch_is_rejected() {
    let Setup {
        mut rt,
        genesis_hash,
        genesis_bonds,
        _storage,
    } = setup().await;
    let target = genesis_bonds
        .iter()
        .find(|bond| bond.stake > 0)
        .expect("genesis must contain a bonded validator")
        .clone();
    let mut adapter = F1r3RspaceRuntime::new(&mut rt);

    let req = ExecutionRequest {
        pre_state_hash: genesis_hash,
        deploys: vec![],
        system_deploys: vec![SystemDeployRequest::Slash {
            validator: target.validator.clone(),
            invalid_block_hash: vec![0x5a; 32],
        }],
        bonds: genesis_bonds,
        block_number: 1,
    };
    let error = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect_err("slash without a verified host epoch must be rejected");
    assert!(matches!(
        error,
        RuntimeError::InternalError(message)
            if message.contains("verified target activation epoch")
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn multiple_deploys_all_appear_in_result() {
    let Setup {
        mut rt,
        genesis_hash,
        _storage,
        ..
    } = setup().await;
    let mut adapter = F1r3RspaceRuntime::new(&mut rt);

    let req = request(
        genesis_hash,
        vec![
            signed_deploy_at(r#"@0!("first")"#, 0),
            signed_deploy_at(r#"@1!("second")"#, 1),
            signed_deploy_at(r#"@2!("third")"#, 2),
        ],
        vec![],
        1,
    );
    let result = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect("execute_block must not fail");

    assert_eq!(
        result.processed_deploys.len(),
        3,
        "all three submitted deploys must appear in processed_deploys"
    );
}
