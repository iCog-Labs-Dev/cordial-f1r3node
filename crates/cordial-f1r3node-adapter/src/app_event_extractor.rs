//! Adapter-side extraction boundary from finalized ordered output to app events.
//!
//! This module is the first bridge from `OrderedFinalizedOutput` into the
//! app-neutral `cordial-app-runtime` crate. It does not parse Rholang or own
//! app business rules; callers provide already-decoded app deploy metadata.

use std::collections::BTreeMap;

use cordial_app_runtime::{AppEvent, AppEventId, AppId};
use sha2::{Digest, Sha256};

use crate::ordered_output::OrderedFinalizedOutput;

/// App-level deploy metadata that can be projected into an [`AppEvent`].
///
/// This is intentionally app-neutral. A future envelope parser can produce
/// values of this type from deploy terms or deploy metadata without making the
/// generic runtime depend on f1r3node internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractableAppDeploy {
    pub app_id: AppId,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub submitter: Vec<u8>,
    pub deploy_signature: Option<Vec<u8>>,
}

/// Inputs needed to extract app events from finalized ordered output.
///
/// `deploys_by_block_hash` is keyed by block content hash. Deploy vectors are
/// already in the per-block order the caller wants to preserve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEventExtractionInput {
    pub ordered_output: OrderedFinalizedOutput,
    pub deploys_by_block_hash: BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>>,
    pub starting_ordered_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEventExtractionError {
    MissingFinalizedBlockDeploys {
        block_hash: Vec<u8>,
    },
    OrderedIndexOverflow {
        starting_ordered_index: u64,
        emitted_count: usize,
    },
}

impl std::fmt::Display for AppEventExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFinalizedBlockDeploys { block_hash } => write!(
                f,
                "missing scanned deploy data for finalized block {}",
                hex::encode(block_hash)
            ),
            Self::OrderedIndexOverflow {
                starting_ordered_index,
                emitted_count,
            } => write!(
                f,
                "app event ordered_index overflow for start {} and emitted count {}",
                starting_ordered_index, emitted_count
            ),
        }
    }
}

impl std::error::Error for AppEventExtractionError {}

/// Extract app events from an ordered finalized output fragment.
pub fn extract_app_events(
    input: AppEventExtractionInput,
) -> Result<Vec<AppEvent>, AppEventExtractionError> {
    let finalized_anchor = input.ordered_output.anchor_hash().unwrap_or_default();
    let mut events = Vec::new();

    for block in &input.ordered_output.blocks {
        let block_hash = block.content_hash.to_vec();
        let deploys = input
            .deploys_by_block_hash
            .get(&block_hash)
            .ok_or_else(|| AppEventExtractionError::MissingFinalizedBlockDeploys {
                block_hash: block_hash.clone(),
            })?;

        for (deploy_index, deploy) in deploys.iter().enumerate() {
            let ordered_index = next_ordered_index(input.starting_ordered_index, events.len())?;
            events.push(AppEvent {
                event_id: event_id_for(&block_hash, deploy_index, deploy),
                app_id: deploy.app_id.clone(),
                event_type: deploy.event_type.clone(),
                payload: deploy.payload.clone(),
                submitter: deploy.submitter.clone(),
                ordered_index,
                block_hash: block_hash.clone(),
                deploy_signature: deploy.deploy_signature.clone(),
                finalized_anchor: finalized_anchor.clone(),
            });
        }
    }

    Ok(events)
}

fn next_ordered_index(
    starting_ordered_index: u64,
    emitted_count: usize,
) -> Result<u64, AppEventExtractionError> {
    let emitted_count_u64 = u64::try_from(emitted_count).map_err(|_| {
        AppEventExtractionError::OrderedIndexOverflow {
            starting_ordered_index,
            emitted_count,
        }
    })?;

    starting_ordered_index.checked_add(emitted_count_u64).ok_or(
        AppEventExtractionError::OrderedIndexOverflow {
            starting_ordered_index,
            emitted_count,
        },
    )
}

fn event_id_for(
    block_hash: &[u8],
    deploy_index: usize,
    deploy: &ExtractableAppDeploy,
) -> AppEventId {
    let mut encoded = Vec::new();
    put_bytes(&mut encoded, b"cordial-app-event:v1");
    put_bytes(&mut encoded, block_hash);
    put_u64(&mut encoded, deploy_index as u64);
    put_bytes(&mut encoded, deploy.app_id.0.as_bytes());
    put_bytes(&mut encoded, deploy.event_type.as_bytes());
    put_bytes(&mut encoded, &deploy.payload);
    put_bytes(&mut encoded, &deploy.submitter);
    put_optional_bytes(&mut encoded, deploy.deploy_signature.as_deref());

    AppEventId(hex::encode(Sha256::digest(encoded)))
}

fn put_u64(encoded: &mut Vec<u8>, value: u64) {
    encoded.extend_from_slice(&value.to_le_bytes());
}

fn put_bytes(encoded: &mut Vec<u8>, value: &[u8]) {
    put_u64(encoded, value.len() as u64);
    encoded.extend_from_slice(value);
}

fn put_optional_bytes(encoded: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        Some(bytes) => {
            encoded.push(1);
            put_bytes(encoded, bytes);
        }
        None => encoded.push(0),
    }
}
