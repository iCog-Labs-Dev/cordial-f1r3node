//! Compose app-event block scanning with finalized-output extraction.
//!
//! This module is the ergonomic adapter entry point for callers that already
//! have finalized order and the corresponding block messages.

use cordial_app_runtime::AppEvent;

use crate::app_event_block_scan::{
    AppEventBlockScanError, AppEventEnvelopeScanError, scan_app_deploys_by_block_hash,
};
use crate::app_event_extractor::{
    AppEventExtractionError, AppEventExtractionInput, extract_app_events,
};
use crate::block_translation::BlockMessage;
use crate::ordered_output::OrderedFinalizedOutput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEventExtraction {
    pub events: Vec<AppEvent>,
    pub envelope_errors: Vec<AppEventEnvelopeScanError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEventExtractionPipelineError {
    BlockScan(AppEventBlockScanError),
    Extraction(AppEventExtractionError),
}

impl std::fmt::Display for AppEventExtractionPipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlockScan(source) => write!(f, "app event block scan failed: {source}"),
            Self::Extraction(source) => write!(f, "app event extraction failed: {source}"),
        }
    }
}

impl std::error::Error for AppEventExtractionPipelineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BlockScan(source) => Some(source),
            Self::Extraction(source) => Some(source),
        }
    }
}

impl From<AppEventBlockScanError> for AppEventExtractionPipelineError {
    fn from(source: AppEventBlockScanError) -> Self {
        Self::BlockScan(source)
    }
}

impl From<AppEventExtractionError> for AppEventExtractionPipelineError {
    fn from(source: AppEventExtractionError) -> Self {
        Self::Extraction(source)
    }
}

/// Extract app events from finalized output and its corresponding block bodies.
///
/// Final app-event order comes from `ordered_output`; `blocks` only provide the
/// deploy bodies needed to find app-event envelopes.
pub fn extract_app_events_from_blocks(
    ordered_output: OrderedFinalizedOutput,
    blocks: &[BlockMessage],
    starting_ordered_index: u64,
) -> Result<AppEventExtraction, AppEventExtractionPipelineError> {
    let scan = scan_app_deploys_by_block_hash(blocks)?;
    let events = extract_app_events(AppEventExtractionInput {
        ordered_output,
        deploys_by_block_hash: scan.deploys_by_block_hash,
        starting_ordered_index,
    })?;

    Ok(AppEventExtraction {
        events,
        envelope_errors: scan.envelope_errors,
    })
}
