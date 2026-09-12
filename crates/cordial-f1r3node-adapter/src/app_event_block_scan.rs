//! Scan finalized block messages for Cordial app deploy envelopes.
//!
//! The scanner connects the single-deploy parser to the extractor input model:
//! it walks processed deploys from adapter `BlockMessage` values and groups
//! parsed app deploy metadata by containing block hash.

use std::collections::{BTreeMap, BTreeSet};

use crate::app_event_envelope::{AppEventEnvelopeError, parse_app_deploy_envelope};
use crate::app_event_extractor::ExtractableAppDeploy;
use crate::block_translation::BlockMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEventBlockScan {
    pub deploys_by_block_hash: BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>>,
    pub envelope_errors: Vec<AppEventEnvelopeScanError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEventEnvelopeScanError {
    pub block_hash: Vec<u8>,
    pub deploy_index: usize,
    pub source: AppEventEnvelopeError,
}

impl std::fmt::Display for AppEventEnvelopeScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid app event envelope in block {} deploy {}: {}",
            hex::encode(&self.block_hash),
            self.deploy_index,
            self.source
        )
    }
}

impl std::error::Error for AppEventEnvelopeScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEventBlockScanError {
    DuplicateBlockHash { block_hash: Vec<u8> },
}

impl std::fmt::Display for AppEventBlockScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateBlockHash { block_hash } => write!(
                f,
                "duplicate block hash while scanning app events: {}",
                hex::encode(block_hash)
            ),
        }
    }
}

impl std::error::Error for AppEventBlockScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// Scan block messages and group parsed app deploy metadata by block hash.
///
/// The input slice order does not determine final app-event order. Final order
/// still comes from `OrderedFinalizedOutput` when the map is later passed to
/// `extract_app_events`. This function only preserves deploy order inside each
/// block.
pub fn scan_app_deploys_by_block_hash(
    blocks: &[BlockMessage],
) -> Result<AppEventBlockScan, AppEventBlockScanError> {
    let mut seen_block_hashes = BTreeSet::new();
    let mut deploys_by_block_hash = BTreeMap::new();
    let mut envelope_errors = Vec::new();

    for block in blocks {
        if !seen_block_hashes.insert(block.block_hash.clone()) {
            return Err(AppEventBlockScanError::DuplicateBlockHash {
                block_hash: block.block_hash.clone(),
            });
        }

        let mut app_deploys = Vec::new();
        for (deploy_index, processed_deploy) in block.body.deploys.iter().enumerate() {
            match parse_app_deploy_envelope(&processed_deploy.deploy) {
                Ok(Some(app_deploy)) => app_deploys.push(app_deploy),
                Ok(None) => {}
                Err(source) => envelope_errors.push(AppEventEnvelopeScanError {
                    block_hash: block.block_hash.clone(),
                    deploy_index,
                    source,
                }),
            }
        }

        deploys_by_block_hash.insert(block.block_hash.clone(), app_deploys);
    }

    Ok(AppEventBlockScan {
        deploys_by_block_hash,
        envelope_errors,
    })
}
