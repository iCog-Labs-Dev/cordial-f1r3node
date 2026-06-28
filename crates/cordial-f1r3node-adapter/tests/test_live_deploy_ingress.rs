use std::collections::HashMap;

use cordial_f1r3node_adapter::block_translation::{DeployData, SignedDeployData};
use cordial_f1r3node_adapter::casper_adapter::{
    CordialCasperAdapter, CordialMultiParentCasper, DeployError,
};
use cordial_f1r3node_adapter::live_deploy_ingress::{
    DeployIngressSource, HttpDeployRequest, HttpDeployConversionError, LiveDeployIngress,
    http_request_to_signed_deploy,
};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::execution::DeployPoolConfig;
use cordial_miners_core::types::{BlockContent, NodeId};
use either::Either;
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

fn sample_deploy(sig_byte: u8) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: format!("tx-{sig_byte}"),
            time_stamp: 1000 + sig_byte as i64,
            phlo_price: 1,
            phlo_limit: 10_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
            expiration_timestamp: None,
        },
        pk: vec![sig_byte; 32],
        sig: vec![sig_byte; 64],
        sig_algorithm: "ed25519".to_string(),
    }
}

fn sample_http_request(sig_byte: u8) -> HttpDeployRequest {
    let deploy = sample_deploy(sig_byte);
    HttpDeployRequest {
        data: deploy.data,
        deployer: encode(deploy.pk),
        signature: encode(deploy.sig),
        sig_algorithm: "ed25519".to_string(),
    }
}

#[test]
fn observe_grpc_deploy_records_metadata() {
    let mut ingress = LiveDeployIngress::new();
    let deploy = sample_deploy(7);

    let observed = ingress.observe_grpc_deploy(&deploy);

    assert_eq!(ingress.len(), 1);
    assert!(ingress.contains_signature(&deploy.sig));
    assert_eq!(observed.signature, deploy.sig);
    assert_eq!(observed.deployer, deploy.pk);
    assert_eq!(observed.shard_id, "root");
    assert_eq!(observed.term_len, deploy.data.term.len());
    assert_eq!(observed.observation_count, 1);
    assert!(observed.sources.contains(&DeployIngressSource::Grpc));
}

#[test]
fn observing_same_deploy_from_http_and_grpc_merges_sources() {
    let mut ingress = LiveDeployIngress::new();
    let deploy = sample_deploy(3);

    ingress.observe_http_deploy(&deploy);
    let observed = ingress.observe_grpc_deploy(&deploy);

    assert_eq!(ingress.len(), 1);
    assert_eq!(observed.observation_count, 2);
    assert!(observed.sources.contains(&DeployIngressSource::Http));
    assert!(observed.sources.contains(&DeployIngressSource::Grpc));
    assert_eq!(ingress.staged_deploys().len(), 1);
    assert_eq!(ingress.observed_signatures().len(), 1);
}

#[test]
fn http_request_converts_into_signed_deploy_data() {
    let request = sample_http_request(4);

    let deploy = http_request_to_signed_deploy(&request).unwrap();

    assert_eq!(deploy.pk, vec![4; 32]);
    assert_eq!(deploy.sig, vec![4; 64]);
    assert_eq!(deploy.sig_algorithm, "ed25519");
}

#[test]
fn invalid_http_request_hex_is_rejected() {
    let mut request = sample_http_request(6);
    request.signature = "not-hex".to_string();

    let err = http_request_to_signed_deploy(&request).unwrap_err();

    assert!(matches!(
        err,
        HttpDeployConversionError::InvalidSignatureHex(_)
    ));
}

#[tokio::test]
async fn observe_and_admit_keeps_adapter_behavior_unchanged() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let mut ingress = LiveDeployIngress::new();
    let deploy = sample_deploy(9);

    let result = ingress
        .observe_and_admit(DeployIngressSource::Grpc, deploy.clone(), &adapter)
        .unwrap();

    assert!(matches!(result.admission, Either::Right(_)));
    assert!(ingress.contains_signature(&deploy.sig));
    assert!(adapter.has_pending_deploys_in_storage().await.unwrap());
}

#[tokio::test]
async fn grpc_admission_entrypoint_routes_through_observer_and_adapter() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let mut ingress = LiveDeployIngress::new();
    let deploy = sample_deploy(11);

    let result = ingress.admit_grpc_deploy(deploy.clone(), &adapter).unwrap();

    assert!(matches!(result.admission, Either::Right(_)));
    assert!(result
        .observed
        .sources
        .contains(&DeployIngressSource::Grpc));
    assert!(ingress.contains_signature(&deploy.sig));
}

#[tokio::test]
async fn http_admission_entrypoint_routes_through_observer_and_adapter() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let mut ingress = LiveDeployIngress::new();
    let deploy = sample_deploy(12);

    let result = ingress.admit_http_deploy(deploy.clone(), &adapter).unwrap();

    assert!(matches!(result.admission, Either::Right(_)));
    assert!(result
        .observed
        .sources
        .contains(&DeployIngressSource::Http));
    assert!(ingress.contains_signature(&deploy.sig));
}

#[tokio::test]
async fn http_request_admission_entrypoint_decodes_and_routes_through_adapter() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let mut ingress = LiveDeployIngress::new();
    let request = sample_http_request(13);

    let result = ingress
        .admit_http_request_deploy(request.clone(), &adapter)
        .unwrap();

    assert!(matches!(result.admission, Either::Right(_)));
    assert!(result
        .observed
        .sources
        .contains(&DeployIngressSource::Http));
    assert_eq!(ingress.observed_signatures().len(), 1);
}

#[tokio::test]
async fn rejected_deploy_is_still_observed_for_debugging() {
    let adapter = CordialCasperAdapter::new_with_verifier(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    );
    let mut ingress = LiveDeployIngress::new();
    let mut deploy = sample_deploy(5);
    deploy.sig.clear();

    let result = ingress
        .observe_and_admit(DeployIngressSource::Http, deploy.clone(), &adapter)
        .unwrap();

    assert!(matches!(
        result.admission,
        Either::Left(DeployError::SignatureVerificationFailed)
    ));
    assert!(ingress.contains_signature(&deploy.sig));
    assert_eq!(
        ingress
            .staged_deploy(&deploy.sig)
            .unwrap()
            .observation_count,
        1
    );
}
