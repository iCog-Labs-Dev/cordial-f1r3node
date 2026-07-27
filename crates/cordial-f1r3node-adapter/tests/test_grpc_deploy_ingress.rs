use cordial_f1r3node_adapter::casper_adapter::DeployError;
use cordial_f1r3node_adapter::grpc_deploy_ingress::{AdmitFn, GrpcDeployIngressHandler};
use cordial_f1r3node_adapter::live_deploy_ingress::LiveDeployIngress;
use either::Either;
use models::casper::DeployDataProto;
use models::casper::v1::deploy_service_server::DeployService;

fn sample_proto(sig_byte: u8) -> DeployDataProto {
    DeployDataProto {
        deployer: vec![sig_byte; 32].into(),
        term: format!("tx-{sig_byte}"),
        timestamp: 1000 + sig_byte as i64,
        sig: vec![sig_byte; 64].into(),
        sig_algorithm: "ed25519".to_string(),
        phlo_price: 1,
        phlo_limit: 10_000,
        valid_after_block_number: 0,
        shard_id: "root".to_string(),
        language: "rholang".to_string(),
        expiration_timestamp: 0,
    }
}

fn accepting_admit_fn() -> AdmitFn {
    std::sync::Arc::new(|deploy| Ok(Either::Right(deploy.sig)))
}

fn rejecting_admit_fn() -> AdmitFn {
    std::sync::Arc::new(|_deploy| Ok(Either::Left(DeployError::SignatureVerificationFailed)))
}

#[tokio::test]
async fn grpc_direct_handler_observes_and_admits_deploy() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, accepting_admit_fn());

    let proto = sample_proto(1);
    let (observed, admission) = handler.handle_do_deploy_inner(proto).await.unwrap();

    assert!(matches!(admission, Either::Right(_)));
    assert_eq!(observed.observation_count, 1);

    let observed_list = handler.observed_deploys().await;
    assert_eq!(observed_list.len(), 1);
}

#[tokio::test]
async fn grpc_direct_handler_observes_deploy_even_when_admission_fails() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, rejecting_admit_fn());

    let proto = sample_proto(2);
    let (observed, admission) = handler.handle_do_deploy_inner(proto).await.unwrap();

    assert!(matches!(admission, Either::Left(_)));
    assert_eq!(observed.observation_count, 1);

    let observed_list = handler.observed_deploys().await;
    assert_eq!(observed_list.len(), 1);
}

#[tokio::test]
async fn grpc_direct_handler_merges_duplicate_deploys() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, accepting_admit_fn());

    let proto = sample_proto(3);

    let (_first, _) = handler.handle_do_deploy_inner(proto.clone()).await.unwrap();
    let (second, _) = handler.handle_do_deploy_inner(proto).await.unwrap();

    // Same-source duplicate does not increment observation_count,
    // but the deploy remains a single entry.
    assert_eq!(second.observation_count, 1);

    let observed_list = handler.observed_deploys().await;
    assert_eq!(observed_list.len(), 1);
}

#[tokio::test]
async fn grpc_direct_handler_tracks_multiple_deploys() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, accepting_admit_fn());

    for i in 0..5 {
        let proto = sample_proto(i);
        let (_observed, admission) = handler.handle_do_deploy_inner(proto).await.unwrap();
        assert!(matches!(admission, Either::Right(_)));
    }

    let observed_list = handler.observed_deploys().await;
    assert_eq!(observed_list.len(), 5);
}

#[tokio::test]
async fn grpc_direct_handler_deploy_service_accepts_deploy() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, accepting_admit_fn());

    let proto = sample_proto(10);
    let request = tonic::Request::new(proto.clone());

    let response = handler.do_deploy(request).await.unwrap().into_inner();

    match response.message {
        Some(models::casper::v1::deploy_response::Message::Result(msg)) => {
            assert!(msg.contains("Success"), "expected success, got: {msg}");
        }
        other => panic!("expected result, got: {other:?}"),
    }

    let observed = handler.observed_deploys().await;
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].signature, proto.sig);
}

#[tokio::test]
async fn grpc_direct_handler_deploy_service_rejects_with_status() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, rejecting_admit_fn());

    let proto = sample_proto(11);
    let request = tonic::Request::new(proto.clone());

    let result = handler.do_deploy(request).await;

    assert!(result.is_err(), "expected error status for rejected deploy");

    let observed = handler.observed_deploys().await;
    assert_eq!(
        observed.len(),
        1,
        "rejected deploy should still be observable after rejection"
    );
}

#[tokio::test]
async fn grpc_direct_handler_status_returns_node_info() {
    let ingress = LiveDeployIngress::new();
    let handler = GrpcDeployIngressHandler::new(ingress, accepting_admit_fn());

    let response = handler.status(tonic::Request::new(())).await.unwrap();

    assert!(response.into_inner().message.is_some());
}
