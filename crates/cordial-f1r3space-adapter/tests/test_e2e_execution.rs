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
//! Each test calls `setup()` which uses f1r3node's own `test_utils` to:
//! 1. Build a genesis block (in-memory + temp LMDB, ~10–30 s first run, cached after)
//! 2. Return the genesis state hash and a `RuntimeManager` connected to it
//!
//! This is the same mechanism used by f1r3node's own casper tests.
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

use casper::rust::test_utils::util::rholang::resources::{
    genesis_context, mk_runtime_manager_with_history_at, mk_test_rnode_store_manager_from_genesis,
};
use casper::rust::util::rholang::runtime_manager::RuntimeManager as F1r3RuntimeManager;
use cordial_f1r3space_adapter::F1r3RspaceRuntime;
use cordial_miners_core::execution::{
    Bond, Deploy, DeploySignatureAlgorithm, ExecutionRequest, ProcessedSystemDeploy,
    RuntimeManager as CoreRuntimeManager, SignedDeploy, SystemDeployRequest,
};
use cordial_miners_core::types::NodeId;

// ── Bootstrap ─────────────────────────────────────────────────────────────────

/// Bootstrap result: a ready `RuntimeManager` and the genesis state hash.
struct Setup {
    rt: F1r3RuntimeManager,
    genesis_hash: Vec<u8>,
    genesis_bonds: Vec<Bond>,
}

/// Build a `RuntimeManager` connected to a bootstrapped genesis state.
///
/// Uses f1r3node's own `test_utils::GenesisBuilder` + `resources::genesis_context()`,
/// which is the same mechanism f1r3node's casper integration tests use.
/// Genesis is cached in a process-wide `OnceLock`; the heavy work only runs once.
async fn setup() -> Setup {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let genesis_ctx = genesis_context()
        .await
        .expect("genesis_context() failed — check that f1r3node's test-utils are available");

    // `body` and `state` are plain structs (not Option) in f1r3node's BlockMessage.
    // `post_state_hash` is a `ByteString` (alias for `prost::bytes::Bytes`, 32 bytes).
    let genesis_hash = genesis_ctx
        .genesis_block
        .body
        .state
        .post_state_hash
        .to_vec();
    let mut genesis_bonds: Vec<Bond> = genesis_ctx
        .genesis_block
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

    let mut kvm = mk_test_rnode_store_manager_from_genesis(&genesis_ctx);
    let (rt, _history_repo) = mk_runtime_manager_with_history_at(&mut *kvm).await;

    Setup {
        rt,
        genesis_hash,
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

/// A minimal bond set using the default test public key.
fn test_bonds() -> Vec<Bond> {
    use casper::rust::util::construct_deploy::DEFAULT_PUB;
    vec![Bond {
        validator: NodeId(DEFAULT_PUB.bytes.to_vec()),
        stake: 100,
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
async fn slash_updates_post_state_bonds() {
    let Setup {
        mut rt,
        genesis_hash,
        genesis_bonds,
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
    let result = tokio::task::block_in_place(|| adapter.execute_block(req))
        .expect("a valid slash must execute and return post-state bonds");

    assert!(
        result.system_deploys.iter().any(|deploy| matches!(
            deploy,
            ProcessedSystemDeploy::Slash {
                validator,
                succeeded: true
            } if validator == &target.validator
        )),
        "expected a successful slash for the target validator: {:?}",
        result.system_deploys
    );
    assert!(
        result
            .new_bonds
            .iter()
            .find(|bond| bond.validator == target.validator)
            .is_none_or(|bond| bond.stake == 0),
        "slashed validator must be absent or have zero effective stake: {:?}",
        result.new_bonds
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs genesis bootstrapping (~10–30 s); run with: just e2e-rholang"]
async fn multiple_deploys_all_appear_in_result() {
    let Setup {
        mut rt,
        genesis_hash,
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
