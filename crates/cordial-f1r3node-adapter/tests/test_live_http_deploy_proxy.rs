use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::routing::post;
use cordial_f1r3node_adapter::live_deploy_ingress::LiveDeployIngress;
use cordial_f1r3node_adapter::live_http_deploy_proxy::{
    LiveHttpDeployProxy, live_http_deploy_proxy_router,
};
use tokio::sync::Mutex;

const BODY_LIMIT: usize = 10_000_000;

#[derive(Debug, Clone)]
struct ReceivedRequest {
    path: String,
    body: Vec<u8>,
}

struct MockUpstream {
    received: Arc<Mutex<Vec<ReceivedRequest>>>,
}

impl MockUpstream {
    fn new() -> (Self, Arc<Mutex<Vec<ReceivedRequest>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                received: received.clone(),
            },
            received,
        )
    }

    fn router(self) -> Router {
        let received = self.received.clone();
        let handler = move |uri: Uri, body: Body| {
            let received = received.clone();
            async move {
                let bytes = to_bytes(body, BODY_LIMIT).await.unwrap();
                received.lock().await.push(ReceivedRequest {
                    path: uri.path().to_string(),
                    body: bytes.to_vec(),
                });
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "success": true,
                        "message": "upstream-processed"
                    })),
                )
                    .into_response()
            }
        };
        Router::new()
            .route("/api/deploy", post(handler.clone()))
            .route("/api/v1/deploy", post(handler))
    }
}

fn sample_http_body(sig_byte: u8) -> serde_json::Value {
    let sig = hex::encode(vec![sig_byte; 64]);
    let deployer = hex::encode(vec![sig_byte; 32]);
    serde_json::json!({
        "data": {
            "term": format!("@0!(\"tx-{sig_byte}\")"),
            "time_stamp": 1000 + sig_byte as i64,
            "phlo_price": 1,
            "phlo_limit": 10000,
            "valid_after_block_number": 0,
            "shard_id": "root"
        },
        "deployer": deployer,
        "signature": sig,
        "sigAlgorithm": "ed25519"
    })
}

async fn spawn_upstream(mock: MockUpstream) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock.router()).await.unwrap();
    });
    addr
}

async fn spawn_proxy(proxy: LiveHttpDeployProxy) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, live_http_deploy_proxy_router(proxy))
            .await
            .unwrap();
    });
    addr
}

#[tokio::test]
async fn http_proxy_observes_and_forwards_deploy() {
    let (mock, upstream_received) = MockUpstream::new();
    let upstream_addr = spawn_upstream(mock).await;

    let proxy = LiveHttpDeployProxy::new(format!("http://{upstream_addr}"));
    let proxy_addr = spawn_proxy(proxy.clone()).await;

    let client = reqwest::Client::new();
    let body = sample_http_body(1);
    let resp = client
        .post(format!("http://{proxy_addr}/api/deploy"))
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let resp_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(resp_json["success"], true);
    assert_eq!(resp_json["message"], "upstream-processed");

    let observed = proxy.observed_deploys().await;
    assert_eq!(observed.len(), 1);

    let received = upstream_received.lock().await;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].path, "/api/deploy");
    let forwarded_json: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(forwarded_json["signature"], body["signature"]);
}

#[tokio::test]
async fn http_proxy_observes_multiple_deploys() {
    let (mock, _) = MockUpstream::new();
    let upstream_addr = spawn_upstream(mock).await;
    let proxy = LiveHttpDeployProxy::new(format!("http://{upstream_addr}"));
    let proxy_addr = spawn_proxy(proxy.clone()).await;

    let client = reqwest::Client::new();
    for b in 1..=3 {
        let resp = client
            .post(format!("http://{proxy_addr}/api/deploy"))
            .json(&sample_http_body(b))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    let observed = proxy.observed_deploys().await;
    assert_eq!(observed.len(), 3);
}

#[tokio::test]
async fn http_proxy_handles_upstream_error() {
    let proxy = LiveHttpDeployProxy::new("http://127.0.0.1:1".to_string());
    let proxy_addr = spawn_proxy(proxy.clone()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{proxy_addr}/api/deploy"))
        .json(&sample_http_body(1))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn http_proxy_v1_endpoint_forwards_to_upstream_v1_path() {
    let (mock, upstream_received) = MockUpstream::new();
    let upstream_addr = spawn_upstream(mock).await;
    let proxy = LiveHttpDeployProxy::new(format!("http://{upstream_addr}"));
    let proxy_addr = spawn_proxy(proxy.clone()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{proxy_addr}/api/v1/deploy"))
        .json(&sample_http_body(1))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let observed = proxy.observed_deploys().await;
    assert_eq!(observed.len(), 1);

    // Verify upstream received the request on /api/v1/deploy, not /api/deploy
    let received = upstream_received.lock().await;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].path, "/api/v1/deploy");
}

#[tokio::test]
async fn http_proxy_shared_ingress() {
    let (mock, _received) = MockUpstream::new();
    let upstream_addr = spawn_upstream(mock).await;

    let shared = Arc::new(Mutex::new(LiveDeployIngress::new()));
    let proxy =
        LiveHttpDeployProxy::with_shared_ingress(format!("http://{upstream_addr}"), shared.clone());

    let body = sample_http_body(42);
    let sig_hex = body["signature"].as_str().unwrap().to_string();
    let signed = cordial_f1r3node_adapter::live_deploy_ingress::http_request_to_signed_deploy(
        &serde_json::from_value(body).unwrap(),
    )
    .unwrap();
    shared.lock().await.observe_http_deploy(&signed);

    let observed = proxy.observed_deploys().await;
    assert_eq!(observed.len(), 1);
    assert_eq!(hex::encode(&observed[0].signature), sig_hex);
}

#[tokio::test]
async fn http_proxy_forwards_on_observe_failure() {
    let (mock, upstream_received) = MockUpstream::new();
    let upstream_addr = spawn_upstream(mock).await;
    let proxy = LiveHttpDeployProxy::new(format!("http://{upstream_addr}"));
    let proxy_addr = spawn_proxy(proxy).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{proxy_addr}/api/deploy"))
        .json(&serde_json::json!({"garbage": true}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let received = upstream_received.lock().await;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].path, "/api/deploy");
}
