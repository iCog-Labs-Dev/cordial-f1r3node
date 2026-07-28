use std::collections::{BTreeSet, HashMap};

use either::Either;
use models::casper::DeployDataProto;
use serde::{Deserialize, Serialize};

use crate::block_translation::{DeployData, SignedDeployData};
use crate::casper_adapter::{CasperError, CordialCasper, DeployError, DeployId};
use cordial_miners_core::crypto::CryptoVerifier;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeployIngressSource {
    Grpc,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedDeploy {
    pub signature: Vec<u8>,
    pub deployer: Vec<u8>,
    pub sig_algorithm: String,
    pub shard_id: String,
    pub term_len: usize,
    pub phlo_price: i64,
    pub phlo_limit: i64,
    pub valid_after_block_number: i64,
    pub expiration_timestamp: Option<i64>,
    pub sources: BTreeSet<DeployIngressSource>,
    pub observation_count: usize,
}

impl ObservedDeploy {
    fn from_signed(source: DeployIngressSource, deploy: &SignedDeployData) -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(source);
        Self {
            signature: deploy.sig.clone(),
            deployer: deploy.pk.clone(),
            sig_algorithm: deploy.sig_algorithm.clone(),
            shard_id: deploy.data.shard_id.clone(),
            term_len: deploy.data.term.len(),
            phlo_price: deploy.data.phlo_price,
            phlo_limit: deploy.data.phlo_limit,
            valid_after_block_number: deploy.data.valid_after_block_number,
            expiration_timestamp: deploy.data.expiration_timestamp,
            sources,
            observation_count: 1,
        }
    }

    fn observe_again(&mut self, source: DeployIngressSource) {
        self.sources.insert(source);
        self.observation_count += 1;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeployObservationResult {
    pub observed: ObservedDeploy,
    pub admission: Either<DeployError, DeployId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpDeployRequest {
    pub data: DeployData,
    pub deployer: String,
    pub signature: String,
    #[serde(rename = "sigAlgorithm")]
    pub sig_algorithm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpDeployConversionError {
    InvalidDeployerHex(String),
    InvalidSignatureHex(String),
}

impl std::fmt::Display for HttpDeployConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDeployerHex(err) => write!(f, "invalid deployer hex: {err}"),
            Self::InvalidSignatureHex(err) => write!(f, "invalid signature hex: {err}"),
        }
    }
}

impl std::error::Error for HttpDeployConversionError {}

#[derive(Debug)]
pub struct LiveDeployIngress {
    staged: HashMap<Vec<u8>, ObservedDeploy>,
    observed_order: Vec<Vec<u8>>,
}

impl Default for LiveDeployIngress {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveDeployIngress {
    pub fn new() -> Self {
        Self {
            staged: HashMap::new(),
            observed_order: Vec::new(),
        }
    }

    pub fn observe_grpc_deploy(&mut self, deploy: &SignedDeployData) -> ObservedDeploy {
        self.observe_deploy(DeployIngressSource::Grpc, deploy)
    }

    pub fn observe_grpc_proto_deploy(&mut self, proto: &DeployDataProto) -> ObservedDeploy {
        let deploy = grpc_proto_to_signed_deploy(proto);
        self.observe_grpc_deploy(&deploy)
    }

    pub fn observe_http_deploy(&mut self, deploy: &SignedDeployData) -> ObservedDeploy {
        self.observe_deploy(DeployIngressSource::Http, deploy)
    }

    pub fn observe_http_request_deploy(
        &mut self,
        request: &HttpDeployRequest,
    ) -> Result<ObservedDeploy, HttpDeployConversionError> {
        let deploy = http_request_to_signed_deploy(request)?;
        Ok(self.observe_http_deploy(&deploy))
    }

    /// Observe a deploy from the given source.
    ///
    /// **Duplicate semantics:** if the same signature is re-observed from the
    /// *same* source the observation count is intentionally not incremented
    /// (quiets retries / duplicate delivery). If it arrives via a *different*
    /// source (e.g. HTTP then gRPC) the count *is* bumped, reflecting a
    /// genuine multi-path sighting.
    pub fn observe_deploy(
        &mut self,
        source: DeployIngressSource,
        deploy: &SignedDeployData,
    ) -> ObservedDeploy {
        let signature = deploy.sig.clone();
        let entry = self.staged.entry(signature.clone()).or_insert_with(|| {
            self.observed_order.push(signature.clone());
            ObservedDeploy::from_signed(source, deploy)
        });
        if !entry.sources.contains(&source) {
            entry.observe_again(source);
        }
        entry.clone()
    }

    pub fn observe_and_admit<V>(
        &mut self,
        source: DeployIngressSource,
        deploy: SignedDeployData,
        adapter: &impl CordialCasper<V>,
    ) -> Result<DeployObservationResult, CasperError>
    where
        V: CryptoVerifier + Send + Sync,
    {
        let observed = self.observe_deploy(source, &deploy);
        let admission = adapter.deploy(deploy)?;
        Ok(DeployObservationResult {
            observed,
            admission,
        })
    }

    pub fn admit_grpc_deploy<V>(
        &mut self,
        deploy: SignedDeployData,
        adapter: &impl CordialCasper<V>,
    ) -> Result<DeployObservationResult, CasperError>
    where
        V: CryptoVerifier + Send + Sync,
    {
        self.observe_and_admit(DeployIngressSource::Grpc, deploy, adapter)
    }

    pub fn admit_grpc_proto_deploy<V>(
        &mut self,
        proto: DeployDataProto,
        adapter: &impl CordialCasper<V>,
    ) -> Result<DeployObservationResult, GrpcDeployIngressError>
    where
        V: CryptoVerifier + Send + Sync,
    {
        let deploy = grpc_proto_to_signed_deploy(&proto);
        let result = self.admit_grpc_deploy(deploy, adapter)?;
        Ok(result)
    }

    pub fn admit_http_deploy<V>(
        &mut self,
        deploy: SignedDeployData,
        adapter: &impl CordialCasper<V>,
    ) -> Result<DeployObservationResult, CasperError>
    where
        V: CryptoVerifier + Send + Sync,
    {
        self.observe_and_admit(DeployIngressSource::Http, deploy, adapter)
    }

    pub fn admit_http_request_deploy<V>(
        &mut self,
        request: HttpDeployRequest,
        adapter: &impl CordialCasper<V>,
    ) -> Result<DeployObservationResult, HttpDeployIngressError>
    where
        V: CryptoVerifier + Send + Sync,
    {
        let deploy = http_request_to_signed_deploy(&request)?;
        let result = self.admit_http_deploy(deploy, adapter)?;
        Ok(result)
    }

    pub fn len(&self) -> usize {
        self.staged.len()
    }

    pub fn is_empty(&self) -> bool {
        self.staged.is_empty()
    }

    pub fn contains_signature(&self, signature: &[u8]) -> bool {
        self.staged.contains_key(signature)
    }

    pub fn staged_deploy(&self, signature: &[u8]) -> Option<&ObservedDeploy> {
        self.staged.get(signature)
    }

    pub fn staged_deploys(&self) -> Vec<&ObservedDeploy> {
        self.observed_order
            .iter()
            .filter_map(|sig| self.staged.get(sig))
            .collect()
    }

    pub fn observed_signatures(&self) -> &[Vec<u8>] {
        &self.observed_order
    }
}

#[derive(Debug)]
pub enum HttpDeployIngressError {
    Conversion(HttpDeployConversionError),
    Admission(CasperError),
}

impl std::fmt::Display for HttpDeployIngressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conversion(err) => write!(f, "{err}"),
            Self::Admission(err) => write!(f, "{err:?}"),
        }
    }
}

impl std::error::Error for HttpDeployIngressError {}

impl From<HttpDeployConversionError> for HttpDeployIngressError {
    fn from(value: HttpDeployConversionError) -> Self {
        Self::Conversion(value)
    }
}

impl From<CasperError> for HttpDeployIngressError {
    fn from(value: CasperError) -> Self {
        Self::Admission(value)
    }
}

#[derive(Debug)]
pub enum GrpcDeployIngressError {
    Admission(CasperError),
}

impl std::fmt::Display for GrpcDeployIngressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admission(err) => write!(f, "{err:?}"),
        }
    }
}

impl std::error::Error for GrpcDeployIngressError {}

impl From<CasperError> for GrpcDeployIngressError {
    fn from(value: CasperError) -> Self {
        Self::Admission(value)
    }
}

pub fn grpc_proto_to_signed_deploy(proto: &DeployDataProto) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: proto.term.clone(),
            time_stamp: proto.timestamp,
            phlo_price: proto.phlo_price,
            phlo_limit: proto.phlo_limit,
            valid_after_block_number: proto.valid_after_block_number,
            shard_id: proto.shard_id.clone(),
            expiration_timestamp: if proto.expiration_timestamp == 0 {
                None
            } else {
                Some(proto.expiration_timestamp)
            },
        },
        pk: proto.deployer.to_vec(),
        sig: proto.sig.to_vec(),
        sig_algorithm: proto.sig_algorithm.clone(),
    }
}

pub fn http_request_to_signed_deploy(
    request: &HttpDeployRequest,
) -> Result<SignedDeployData, HttpDeployConversionError> {
    let pk = hex::decode(&request.deployer)
        .map_err(|err| HttpDeployConversionError::InvalidDeployerHex(err.to_string()))?;
    let sig = hex::decode(&request.signature)
        .map_err(|err| HttpDeployConversionError::InvalidSignatureHex(err.to_string()))?;

    Ok(SignedDeployData {
        data: request.data.clone(),
        pk,
        sig,
        sig_algorithm: request.sig_algorithm.clone(),
    })
}
