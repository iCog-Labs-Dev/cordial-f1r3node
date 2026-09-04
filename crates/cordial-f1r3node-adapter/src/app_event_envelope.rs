//! Minimal app-event envelope parser for f1r3node deploy terms.
//!
//! This module converts an app envelope embedded in a deploy term into
//! [`ExtractableAppDeploy`](crate::app_event_extractor::ExtractableAppDeploy).
//! It does not validate signatures or decode app-specific payload semantics.

use cordial_app_runtime::AppId;
use serde::Deserialize;

use crate::app_event_extractor::ExtractableAppDeploy;
use crate::block_translation::SignedDeployData;

const ENVELOPE_FIELD: &str = "cordial_app";
const SUPPORTED_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEventEnvelopeError {
    InvalidJson(String),
    InvalidEnvelope(String),
    UnsupportedVersion { version: u64 },
    InvalidPayloadHex(String),
}

impl std::fmt::Display for AppEventEnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(err) => write!(f, "invalid app event envelope JSON: {err}"),
            Self::InvalidEnvelope(err) => write!(f, "invalid app event envelope: {err}"),
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported app event envelope version: {version}")
            }
            Self::InvalidPayloadHex(err) => write!(f, "invalid app event payload hex: {err}"),
        }
    }
}

impl std::error::Error for AppEventEnvelopeError {}

#[derive(Debug, Deserialize)]
struct CordialAppEnvelope {
    version: u64,
    app_id: String,
    event_type: String,
    payload_hex: String,
}

/// Parse a deploy term into app-event metadata when it carries a Cordial app
/// envelope.
///
/// Returns:
///
/// - `Ok(Some(_))` when the deploy contains a valid supported app envelope
/// - `Ok(None)` when the deploy is not a Cordial app event
/// - `Err(_)` when the deploy declares a Cordial app envelope but it is invalid
pub fn parse_app_deploy_envelope(
    deploy: &SignedDeployData,
) -> Result<Option<ExtractableAppDeploy>, AppEventEnvelopeError> {
    let value = match serde_json::from_str::<serde_json::Value>(&deploy.data.term) {
        Ok(value) => value,
        Err(err) => {
            return if deploy.data.term.trim_start().starts_with('{') {
                Err(AppEventEnvelopeError::InvalidJson(err.to_string()))
            } else {
                Ok(None)
            };
        }
    };

    let Some(envelope_value) = value.get(ENVELOPE_FIELD) else {
        return Ok(None);
    };

    let envelope: CordialAppEnvelope = serde_json::from_value(envelope_value.clone())
        .map_err(|err| AppEventEnvelopeError::InvalidEnvelope(err.to_string()))?;

    if envelope.version != SUPPORTED_VERSION {
        return Err(AppEventEnvelopeError::UnsupportedVersion {
            version: envelope.version,
        });
    }

    let payload = hex::decode(envelope.payload_hex)
        .map_err(|err| AppEventEnvelopeError::InvalidPayloadHex(err.to_string()))?;

    Ok(Some(ExtractableAppDeploy {
        app_id: AppId(envelope.app_id),
        event_type: envelope.event_type,
        payload,
        submitter: deploy.pk.clone(),
        deploy_signature: Some(deploy.sig.clone()),
    }))
}
