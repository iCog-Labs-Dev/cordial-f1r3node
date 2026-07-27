//! HTTP-side counterpart to [`crate::live_deploy_proxy::LiveDeployProxy`].
//!
//! Same strategy as the gRPC proxy: we can't edit f1r3node's source, so we
//! can't insert an inline observation call into its `deploy(...)` handler.
//! Instead this sits in front of a real f1r3node HTTP API as a transparent
//! sidecar — it observes the deploy, then forwards the *original* request
//! body to the real upstream and relays the real response back untouched.
//!
//! This means admission is never run locally; it happens entirely on the
//! real node, exactly as it does today. Cordial only watches.
//!
//! ## Relationship to `http_deploy_ingress.rs`
//!
//! `handle_deploy` in `http_deploy_ingress.rs` is a standalone local-admission
//! path (or, currently, observation-only with no admission at all) — useful
//! for unit tests and for exercising the observer without a live node. This
//! module is the real integration path: run it in front of an actual
//! f1r3node HTTP endpoint and it behaves identically to talking to that node
//! directly, with deploy observation as a side effect.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::post};
use tokio::sync::Mutex;

use crate::live_deploy_ingress::{HttpDeployRequest, LiveDeployIngress, ObservedDeploy};

#[derive(Debug)]
pub enum LiveHttpDeployProxyError {
    Upstream(reqwest::Error),
    InvalidUpstreamUrl(String),
}

impl std::fmt::Display for LiveHttpDeployProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upstream(err) => write!(f, "upstream deploy request failed: {err}"),
            Self::InvalidUpstreamUrl(err) => write!(f, "invalid upstream base url: {err}"),
        }
    }
}

impl std::error::Error for LiveHttpDeployProxyError {}

#[derive(Clone)]
pub struct LiveHttpDeployProxy {
    client: reqwest::Client,
    upstream_base_url: String,
    ingress: Arc<Mutex<LiveDeployIngress>>,
}

impl LiveHttpDeployProxy {
    pub fn new(upstream_base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            upstream_base_url: upstream_base_url.into().trim_end_matches('/').to_string(),
            ingress: Arc::new(Mutex::new(LiveDeployIngress::new())),
        }
    }

    pub fn with_shared_ingress(
        upstream_base_url: impl Into<String>,
        ingress: Arc<Mutex<LiveDeployIngress>>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            upstream_base_url: upstream_base_url.into().trim_end_matches('/').to_string(),
            ingress,
        }
    }

    pub fn ingress(&self) -> &Arc<Mutex<LiveDeployIngress>> {
        &self.ingress
    }

    pub async fn observed_deploys(&self) -> Vec<ObservedDeploy> {
        self.ingress
            .lock()
            .await
            .staged_deploys()
            .into_iter()
            .cloned()
            .collect()
    }

    async fn observe_and_forward(
        &self,
        path: &str,
        request: &HttpDeployRequest,
        raw_body: &[u8],
    ) -> Result<reqwest::Response, LiveHttpDeployProxyError> {
        match crate::live_deploy_ingress::http_request_to_signed_deploy(request) {
            Ok(signed) => {
                let mut ingress = self.ingress.lock().await;
                ingress.observe_http_deploy(&signed);
            }
            Err(err) => {
                tracing::warn!("cordial http deploy observation failed: {err}");
            }
        }

        self.client
            .post(format!("{}{}", self.upstream_base_url, path))
            .header("content-type", "application/json")
            .body(raw_body.to_vec())
            .send()
            .await
            .map_err(LiveHttpDeployProxyError::Upstream)
    }
}

pub fn live_http_deploy_proxy_router(proxy: LiveHttpDeployProxy) -> Router {
    Router::new()
        .route("/api/deploy", post(proxy_deploy))
        .route("/api/v1/deploy", post(proxy_deploy))
        .with_state(proxy)
}

const BODY_LIMIT: usize = 10_000_000;

async fn proxy_deploy(State(proxy): State<LiveHttpDeployProxy>, uri: Uri, body: Body) -> Response {
    let path = uri.path().to_string();

    let bytes = match to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                format!("failed to read request body: {err}"),
            )
                .into_response();
        }
    };

    let request: HttpDeployRequest = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(err) => {
            return match proxy
                .client
                .post(format!("{}{path}", proxy.upstream_base_url))
                .header("content-type", "application/json")
                .body(bytes.to_vec())
                .send()
                .await
            {
                Ok(upstream_response) => relay(upstream_response).await,
                Err(send_err) => (
                    axum::http::StatusCode::BAD_GATEWAY,
                    format!(
                        "request body did not match expected deploy shape ({err}) and upstream forward also failed: {send_err}"
                    ),
                )
                    .into_response(),
            };
        }
    };

    match proxy.observe_and_forward(&path, &request, &bytes).await {
        Ok(upstream_response) => relay(upstream_response).await,
        Err(err) => (axum::http::StatusCode::BAD_GATEWAY, format!("{err}")).into_response(),
    }
}

async fn relay(upstream_response: reqwest::Response) -> Response {
    let status = upstream_response.status();
    let axum_status = axum::http::StatusCode::from_u16(status.as_u16())
        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
    match upstream_response.bytes().await {
        Ok(body) => (axum_status, body).into_response(),
        Err(err) => (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("failed to read upstream response body: {err}"),
        )
            .into_response(),
    }
}

pub async fn serve_live_http_deploy_proxy(
    proxy: LiveHttpDeployProxy,
    addr: std::net::SocketAddr,
) -> Result<(), std::io::Error> {
    let router = live_http_deploy_proxy_router(proxy);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .await
        .map_err(std::io::Error::other)
}
