//! Transport-neutral delivery of PoR block-production rating envelopes.
//!
//! This module defines the boundary used by future gRPC or peer-gossip
//! implementations. It does not choose a network protocol, retry policy,
//! quorum, or collection deadline.

use std::fmt;

use cordial_por::RatingBatch;

use crate::{
    por_rating_collector::{BlockProductionRatingCollector, PorRatingCollectorError},
    por_rating_wire::{BlockProductionRatingEnvelopeV1, PorRatingWireError},
};

/// Synchronous delivery boundary for one already-encoded rating envelope.
pub trait RatingEnvelopeBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String>;
}

/// Closure-backed broadcaster useful for wiring concrete transports.
pub struct FnRatingEnvelopeBroadcaster<F>
where
    F: Fn(&[u8]) -> Result<(), String>,
{
    broadcast: F,
}

impl<F> FnRatingEnvelopeBroadcaster<F>
where
    F: Fn(&[u8]) -> Result<(), String>,
{
    pub fn new(broadcast: F) -> Self {
        Self { broadcast }
    }
}

impl<F> RatingEnvelopeBroadcaster for FnRatingEnvelopeBroadcaster<F>
where
    F: Fn(&[u8]) -> Result<(), String>,
{
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        (self.broadcast)(envelope)
    }
}

/// Failures while preparing, sending, decoding, or collecting rating traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingTransportError {
    InvalidBatchRound,
    Wire(PorRatingWireError),
    Collector(PorRatingCollectorError),
    Broadcast { delivered: usize, message: String },
}

impl fmt::Display for PorRatingTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBatchRound => write!(
                f,
                "rating batch round does not immediately follow the finalized wave"
            ),
            Self::Wire(error) => error.fmt(f),
            Self::Collector(error) => error.fmt(f),
            Self::Broadcast { delivered, message } => write!(
                f,
                "PoR rating broadcast failed after {delivered} delivered envelopes: {message}"
            ),
        }
    }
}

impl std::error::Error for PorRatingTransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Wire(error) => Some(error),
            Self::Collector(error) => Some(error),
            Self::InvalidBatchRound | Self::Broadcast { .. } => None,
        }
    }
}

impl From<PorRatingWireError> for PorRatingTransportError {
    fn from(error: PorRatingWireError) -> Self {
        Self::Wire(error)
    }
}

impl From<PorRatingCollectorError> for PorRatingTransportError {
    fn from(error: PorRatingCollectorError) -> Self {
        Self::Collector(error)
    }
}

/// Encode every rating in a batch before any transport side effect occurs.
pub fn encode_rating_batch(
    finalized_wave: u64,
    batch: &RatingBatch,
) -> Result<Vec<Vec<u8>>, PorRatingTransportError> {
    if finalized_wave.checked_add(1) != Some(batch.round) {
        return Err(PorRatingTransportError::InvalidBatchRound);
    }

    batch
        .ratings
        .iter()
        .cloned()
        .map(|rating| Ok(BlockProductionRatingEnvelopeV1::new(finalized_wave, rating)?.encode()))
        .collect()
}

/// Pre-encode and broadcast all ratings in batch order.
///
/// Encoding is atomic: no envelope is sent unless the complete batch can be
/// encoded. A transport can still fail after earlier sends; that error reports
/// the exact delivered prefix so retry and deduplication policy can respond.
pub fn broadcast_rating_batch(
    broadcaster: &impl RatingEnvelopeBroadcaster,
    finalized_wave: u64,
    batch: &RatingBatch,
) -> Result<usize, PorRatingTransportError> {
    let envelopes = encode_rating_batch(finalized_wave, batch)?;

    for (delivered, envelope) in envelopes.iter().enumerate() {
        broadcaster
            .broadcast_rating_envelope(envelope)
            .map_err(|message| PorRatingTransportError::Broadcast { delivered, message })?;
    }

    Ok(envelopes.len())
}

/// Decode one inbound envelope and submit it to the evidence-backed collector.
pub fn receive_rating_envelope(
    collector: &mut BlockProductionRatingCollector<'_>,
    encoded: &[u8],
) -> Result<(), PorRatingTransportError> {
    let envelope = BlockProductionRatingEnvelopeV1::decode(encoded)?;
    collector.insert_envelope(envelope)?;
    Ok(())
}
