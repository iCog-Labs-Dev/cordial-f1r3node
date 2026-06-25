use std::collections::{BTreeSet, HashMap};

use either::Either;

use crate::block_translation::SignedDeployData;
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

    pub fn observe_http_deploy(&mut self, deploy: &SignedDeployData) -> ObservedDeploy {
        self.observe_deploy(DeployIngressSource::Http, deploy)
    }

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
        if entry.observation_count == 1
            && entry.sources.len() == 1
            && entry.sources.contains(&source)
        {
            // already initialized with the first observation
        } else {
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
