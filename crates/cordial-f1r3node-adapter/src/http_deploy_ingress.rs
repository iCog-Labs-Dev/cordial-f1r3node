use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::block_translation::SignedDeployData;
use crate::live_deploy_ingress::{
    HttpDeployIngressError, HttpDeployRequest, LiveDeployIngress, ObservedDeploy,
    http_request_to_signed_deploy,
};

#[derive(Debug, Clone, Serialize)]
pub struct DeployResponse {
    pub success: bool,
    pub message: String,
    pub signature_hex: Option<String>,
    pub deployer_hex: Option<String>,
    pub observation_count: Option<usize>,
}

impl DeployResponse {
    pub fn accepted(observed: &ObservedDeploy) -> Self {
        Self {
            success: true,
            message: "deploy accepted".into(),
            signature_hex: Some(hex::encode(&observed.signature)),
            deployer_hex: Some(hex::encode(&observed.deployer)),
            observation_count: Some(observed.observation_count),
        }
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            signature_hex: None,
            deployer_hex: None,
            observation_count: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpDeployIngressState {
    ingress: Arc<Mutex<LiveDeployIngress>>,
}

impl HttpDeployIngressState {
    pub fn new(ingress: LiveDeployIngress) -> Self {
        Self {
            ingress: Arc::new(Mutex::new(ingress)),
        }
    }

    pub fn from_arc(ingress: Arc<Mutex<LiveDeployIngress>>) -> Self {
        Self { ingress }
    }

    pub fn ingress(&self) -> &Arc<Mutex<LiveDeployIngress>> {
        &self.ingress
    }
}

pub fn deploy_router(state: HttpDeployIngressState) -> Router {
    Router::new()
        .route("/api/deploy", post(handle_deploy))
        .route("/api/v1/deploy", post(handle_deploy))
        .with_state(state)
}

pub async fn handle_deploy(
    State(state): State<HttpDeployIngressState>,
    Json(request): Json<HttpDeployRequest>,
) -> Response {
    match route_http_deploy(&state, request).await {
        Ok(response) => (axum::http::StatusCode::OK, Json(response)).into_response(),
        Err(err) => {
            let response = DeployResponse::rejected(format!("{err}"));
            (axum::http::StatusCode::BAD_REQUEST, Json(response)).into_response()
        }
    }
}

pub async fn route_http_deploy(
    state: &HttpDeployIngressState,
    request: HttpDeployRequest,
) -> Result<DeployResponse, HttpDeployIngressError> {
    let signed = http_request_to_signed_deploy(&request)?;
    let observed = {
        let mut ingress = state.ingress.lock().await;
        ingress.observe_http_deploy(&signed)
    };
    Ok(DeployResponse::accepted(&observed))
}

pub async fn route_http_deploy_with_admission<V>(
    state: &HttpDeployIngressState,
    request: HttpDeployRequest,
    adapter: &impl crate::casper_adapter::CordialCasper<V>,
) -> Result<DeployResponse, HttpDeployIngressError>
where
    V: cordial_miners_core::crypto::CryptoVerifier + Send + Sync,
{
    let signed: SignedDeployData = http_request_to_signed_deploy(&request)?;
    let mut ingress = state.ingress.lock().await;
    let result = ingress.admit_http_deploy(signed, adapter)?;
    let response = DeployResponse {
        success: matches!(result.admission, either::Either::Right(_)),
        message: match result.admission {
            either::Either::Right(id) => format!("deploy accepted: {}", hex::encode(&id)),
            either::Either::Left(err) => format!("deploy rejected: {err:?}"),
        },
        signature_hex: Some(hex::encode(&result.observed.signature)),
        deployer_hex: Some(hex::encode(&result.observed.deployer)),
        observation_count: Some(result.observed.observation_count),
    };
    Ok(response)
}

pub async fn serve_http_deploy_ingress(
    state: HttpDeployIngressState,
    addr: std::net::SocketAddr,
) -> Result<(), std::io::Error> {
    let router = deploy_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .await
        .map_err(std::io::Error::other)
}
