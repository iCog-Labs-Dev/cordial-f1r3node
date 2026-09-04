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
pub enum AppEventBlockScanError {
    DuplicateBlockHash {
        block_hash: Vec<u8>,
    },
    Envelope {
        block_hash: Vec<u8>,
        deploy_index: usize,
        source: AppEventEnvelopeError,
    },
}

impl std::fmt::Display for AppEventBlockScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateBlockHash { block_hash } => write!(
                f,
                "duplicate block hash while scanning app events: {}",
                hex::encode(block_hash)
            ),
            Self::Envelope {
                block_hash,
                deploy_index,
                source,
            } => write!(
                f,
                "invalid app event envelope in block {} deploy {}: {}",
                hex::encode(block_hash),
                deploy_index,
                source
            ),
        }
    }
}

impl std::error::Error for AppEventBlockScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Envelope { source, .. } => Some(source),
            Self::DuplicateBlockHash { .. } => None,
        }
    }
}

/// Scan block messages and group parsed app deploy metadata by block hash.
///
/// The input slice order does not determine final app-event order. Final order
/// still comes from `OrderedFinalizedOutput` when the map is later passed to
/// `extract_app_events`. This function only preserves deploy order inside each
/// block.
pub fn collect_app_deploys_by_block_hash(
    blocks: &[BlockMessage],
) -> Result<BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>>, AppEventBlockScanError> {
    let mut seen_block_hashes = BTreeSet::new();
    let mut deploys_by_block_hash = BTreeMap::new();

    for block in blocks {
        if !seen_block_hashes.insert(block.block_hash.clone()) {
            return Err(AppEventBlockScanError::DuplicateBlockHash {
                block_hash: block.block_hash.clone(),
            });
        }

        let mut app_deploys = Vec::new();
        for (deploy_index, processed_deploy) in block.body.deploys.iter().enumerate() {
            let parsed = parse_app_deploy_envelope(&processed_deploy.deploy).map_err(|source| {
                AppEventBlockScanError::Envelope {
                    block_hash: block.block_hash.clone(),
                    deploy_index,
                    source,
                }
            })?;

            if let Some(app_deploy) = parsed {
                app_deploys.push(app_deploy);
            }
        }

        if !app_deploys.is_empty() {
            deploys_by_block_hash.insert(block.block_hash.clone(), app_deploys);
        }
    }

    Ok(deploys_by_block_hash)
}
