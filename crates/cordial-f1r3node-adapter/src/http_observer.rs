use std::collections::BTreeSet;

use cordial_miners_core::types::BlockIdentity;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::live_ingress::LiveIngress;

#[derive(Debug, thiserror::Error)]
pub enum HttpObserverError {
    #[error("failed to issue HTTP request: {0}")]
    Request(#[from] reqwest::Error),
    #[error("HTTP request to {path} returned status {status}")]
    Status {
        path: &'static str,
        status: StatusCode,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HttpLightBlockInfo {
    #[serde(rename = "blockHash")]
    pub block_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HttpBlockInfo {
    #[serde(rename = "blockInfo")]
    pub block_info: HttpLightBlockInfo,
    pub deploys: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorHttpComparison {
    pub mirror_block_hashes: BTreeSet<String>,
    pub http_block_hashes: BTreeSet<String>,
    pub mirror_last_finalized: Option<String>,
    pub http_last_finalized: Option<String>,
    pub missing_from_mirror: BTreeSet<String>,
    pub missing_from_http: BTreeSet<String>,
    pub last_finalized_matches: bool,
}

impl MirrorHttpComparison {
    pub fn is_match(&self) -> bool {
        self.missing_from_mirror.is_empty()
            && self.missing_from_http.is_empty()
            && self.last_finalized_matches
    }
}

pub struct HttpObserver {
    client: reqwest::Client,
    base_url: String,
}

impl HttpObserver {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn fetch_blocks(&self) -> Result<Vec<HttpLightBlockInfo>, HttpObserverError> {
        self.get_json("/api/blocks").await
    }

    pub async fn fetch_last_finalized_block(&self) -> Result<HttpBlockInfo, HttpObserverError> {
        self.get_json("/api/last-finalized-block").await
    }

    pub async fn compare_live_ingress<A>(
        &self,
        ingress: &LiveIngress<A>,
    ) -> Result<MirrorHttpComparison, HttpObserverError> {
        let blocks = self.fetch_blocks().await?;
        let last_finalized = self.fetch_last_finalized_block().await?;
        Ok(compare_mirror_against_http(
            ingress,
            &blocks,
            Some(&last_finalized),
        ))
    }

    async fn get_json<T>(&self, path: &'static str) -> Result<T, HttpObserverError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(HttpObserverError::Status { path, status });
        }

        response
            .json::<T>()
            .await
            .map_err(HttpObserverError::Request)
    }
}

pub fn compare_mirror_against_http<A>(
    ingress: &LiveIngress<A>,
    http_blocks: &[HttpLightBlockInfo],
    http_last_finalized: Option<&HttpBlockInfo>,
) -> MirrorHttpComparison {
    let mirror_block_hashes = ingress
        .blocklace()
        .dom()
        .iter()
        .map(|id| hex_string(&id.content_hash))
        .collect::<BTreeSet<_>>();

    let http_block_hashes = http_blocks
        .iter()
        .map(|block| normalize_hex(&block.block_hash))
        .collect::<BTreeSet<_>>();

    let mirror_last_finalized = ingress
        .last_finalized_block_hash()
        .ok()
        .flatten()
        .map(|hash| hex_string(&hash));
    let http_last_finalized =
        http_last_finalized.map(|block| normalize_hex(&block.block_info.block_hash));

    let missing_from_mirror = http_block_hashes
        .difference(&mirror_block_hashes)
        .cloned()
        .collect();
    let missing_from_http = mirror_block_hashes
        .difference(&http_block_hashes)
        .cloned()
        .collect();

    let last_finalized_matches = mirror_last_finalized == http_last_finalized;

    MirrorHttpComparison {
        mirror_block_hashes,
        http_block_hashes,
        mirror_last_finalized,
        http_last_finalized,
        missing_from_mirror,
        missing_from_http,
        last_finalized_matches,
    }
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_hex(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[allow(dead_code)]
fn _identity_hex(id: &BlockIdentity) -> String {
    hex_string(&id.content_hash)
}
