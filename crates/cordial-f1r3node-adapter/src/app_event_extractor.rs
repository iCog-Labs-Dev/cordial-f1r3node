//! Adapter-side extraction boundary from finalized ordered output to app events.
//!
//! This module is the first bridge from `OrderedFinalizedOutput` into the
//! app-neutral `cordial-app-runtime` crate. It does not parse Rholang or own
//! app business rules; callers provide already-decoded app deploy metadata.

use std::collections::BTreeMap;

use cordial_app_runtime::{AppEvent, AppId};

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
}

/// Extract app events from an ordered finalized output fragment.
///
/// This first slice only establishes the boundary and empty-output behavior.
/// Event emission for non-empty finalized outputs is added in the next slice.
pub fn extract_app_events(input: AppEventExtractionInput) -> Vec<AppEvent> {
    if input.ordered_output.is_empty() {
        return Vec::new();
    }

    Vec::new()
}
