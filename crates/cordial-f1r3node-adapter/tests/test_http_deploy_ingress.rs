use std::collections::HashMap;

use cordial_f1r3node_adapter::block_translation::DeployData;
use cordial_f1r3node_adapter::casper_adapter::{CordialCasperAdapter, CordialMultiParentCasper};
use cordial_f1r3node_adapter::http_deploy_ingress::{
    HttpDeployIngressState, route_http_deploy, route_http_deploy_with_admission,
};
use cordial_f1r3node_adapter::live_deploy_ingress::{HttpDeployRequest, LiveDeployIngress};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::execution::DeployPoolConfig;
use cordial_miners_core::types::{BlockContent, NodeId};
use hex::encode;

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(b: u8) -> NodeId {
    NodeId(vec![b])
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries.iter().map(|(b, s)| (node(*b), *s)).collect()
}

fn default_shard_conf() -> CasperShardConf {
    CasperShardConf::from_cordial(&DeployPoolConfig::default(), "root")
}

fn sample_deploy(sig_byte: u8) -> DeployData {
    DeployData {
        term: format!("tx-{sig_byte}"),
        time_stamp: 1000 + sig_byte as i64,
        phlo_price: 1,
        phlo_limit: 10_000,
        valid_after_block_number: 0,
        shard_id: "root".to_string(),
        expiration_timestamp: None,
    }
}

fn sample_http_request(sig_byte: u8) -> HttpDeployRequest {
    HttpDeployRequest {
        data: sample_deploy(sig_byte),
        deployer: encode(vec![sig_byte; 32]),
        signature: encode(vec![sig_byte; 64]),
        sig_algorithm: "ed25519".to_string(),
    }
}

#[tokio::test]
async fn http_deploy_handler_observes_deploy() {
    let ingress = LiveDeployIngress::new();
    let state = HttpDeployIngressState::new(ingress);
    let request = sample_http_request(1);

    let response = route_http_deploy(&state, request).await.unwrap();

    assert!(response.success);
    assert!(response.signature_hex.is_some());
    assert_eq!(response.observation_count, Some(1));

    let observed = state.ingress().lock().await;
    assert_eq!(observed.len(), 1);
}

#[tokio::test]
async fn http_deploy_handler_observes_multiple_deploys() {
    let ingress = LiveDeployIngress::new();
    let state = HttpDeployIngressState::new(ingress);

    let r1 = route_http_deploy(&state, sample_http_request(1))
        .await
        .unwrap();
    let r2 = route_http_deploy(&state, sample_http_request(2))
        .await
        .unwrap();

    assert!(r1.success);
    assert!(r2.success);

    let observed = state.ingress().lock().await;
    assert_eq!(observed.len(), 2);
}

#[tokio::test]
async fn http_deploy_handler_merges_duplicate_by_signature() {
    let ingress = LiveDeployIngress::new();
    let state = HttpDeployIngressState::new(ingress);
    let request = sample_http_request(5);

    let _r1 = route_http_deploy(&state, request.clone()).await.unwrap();
    let r2 = route_http_deploy(&state, request).await.unwrap();

    // Same-source duplicate reuses the existing observation
    assert_eq!(r2.observation_count, Some(1));

    let observed = state.ingress().lock().await;
    assert_eq!(observed.len(), 1);
    assert_eq!(observed.staged_deploys().len(), 1);
}

#[tokio::test]
async fn http_deploy_handler_rejects_invalid_hex() {
    let ingress = LiveDeployIngress::new();
    let state = HttpDeployIngressState::new(ingress);
    let mut request = sample_http_request(3);
    request.signature = "not-hex".to_string();

    let err = route_http_deploy(&state, request).await.unwrap_err();

    assert!(format!("{err}").contains("invalid signature hex"));
}

#[tokio::test]
async fn http_deploy_with_admission_observes_and_admits() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let state = HttpDeployIngressState::new(LiveDeployIngress::new());
    let request = sample_http_request(7);

    let response = route_http_deploy_with_admission(&state, request, &adapter)
        .await
        .unwrap();

    assert!(response.success);
    assert_eq!(response.observation_count, Some(1));

    let observed = state.ingress().lock().await;
    assert_eq!(observed.len(), 1);
    assert!(adapter.has_pending_deploys_in_storage().await.unwrap());
}

#[tokio::test]
async fn http_deploy_with_admission_preserves_adapter_rejection_path() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let state = HttpDeployIngressState::new(LiveDeployIngress::new());
    // Empty signature triggers PoolError::InvalidSignature
    let mut request = sample_http_request(8);
    request.signature = String::new();

    let response = route_http_deploy_with_admission(&state, request, &adapter)
        .await
        .unwrap();

    assert!(!response.success);
    assert_eq!(response.observation_count, Some(1));

    let observed = state.ingress().lock().await;
    assert_eq!(observed.len(), 1);
}

#[tokio::test]
async fn http_deploy_server_endpoint_responds() {
    let ingress = LiveDeployIngress::new();
    let state = HttpDeployIngressState::new(ingress);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = cordial_f1r3node_adapter::http_deploy_ingress::deploy_router(state);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "data": {
            "term": "@0!(\"hello\")",
            "time_stamp": 1000,
            "phlo_price": 1,
            "phlo_limit": 10000,
            "valid_after_block_number": 0,
            "shard_id": "root"
        },
        "deployer": "010101010101010101010101010101010101010101010101010101010101010101",
        "signature": "02020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202",
        "sigAlgorithm": "ed25519"
    });

    let resp = client
        .post(format!("http://{addr}/api/deploy"))
        .json(&request_body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert!(body["signature_hex"].is_string());
}
