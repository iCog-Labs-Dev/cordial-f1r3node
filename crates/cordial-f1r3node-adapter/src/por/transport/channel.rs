//! Bounded process-local Tokio transport for PoR rating envelopes.
//!
//! This implementation exercises the complete asynchronous send/receive
//! lifecycle without choosing a network protocol. It uses synchronous
//! `try_send` at the broadcaster boundary so backpressure is explicit rather
//! than blocking a consensus task.

use std::fmt;

use tokio::sync::mpsc;

use super::{PorRatingTransportError, RatingEnvelopeBroadcaster, receive_rating_envelope};
use crate::por::collector::BlockProductionRatingCollector;

use super::wire::MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN;

/// Failures specific to the bounded channel implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingChannelError {
    ZeroCapacity,
    EnvelopeTooLong { actual: usize, maximum: usize },
    Full,
    Closed,
}

impl fmt::Display for PorRatingChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => write!(f, "PoR rating channel capacity must be non-zero"),
            Self::EnvelopeTooLong { actual, maximum } => write!(
                f,
                "PoR rating channel envelope is {actual} bytes; maximum is {maximum}"
            ),
            Self::Full => write!(f, "PoR rating channel is full"),
            Self::Closed => write!(f, "PoR rating channel is closed"),
        }
    }
}

impl std::error::Error for PorRatingChannelError {}

/// Outcome of awaiting the next channel item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingEnvelopeReceiveOutcome {
    Accepted,
    Closed,
}

/// Cloneable bounded-channel sender implementing the transport-neutral
/// broadcaster interface.
#[derive(Debug, Clone)]
pub struct ChannelRatingEnvelopeBroadcaster {
    sender: mpsc::Sender<Vec<u8>>,
}

impl ChannelRatingEnvelopeBroadcaster {
    pub fn try_broadcast(&self, envelope: &[u8]) -> Result<(), PorRatingChannelError> {
        if envelope.len() > MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN {
            return Err(PorRatingChannelError::EnvelopeTooLong {
                actual: envelope.len(),
                maximum: MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
            });
        }

        self.sender
            .try_send(envelope.to_vec())
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PorRatingChannelError::Full,
                mpsc::error::TrySendError::Closed(_) => PorRatingChannelError::Closed,
            })
    }
}

impl RatingEnvelopeBroadcaster for ChannelRatingEnvelopeBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.try_broadcast(envelope)
            .map_err(|error| error.to_string())
    }
}

/// Receiving half of the bounded rating-envelope channel.
pub struct ChannelRatingEnvelopeReceiver {
    receiver: mpsc::Receiver<Vec<u8>>,
}

impl ChannelRatingEnvelopeReceiver {
    /// Await, decode, and collect the next envelope.
    pub async fn receive_next(
        &mut self,
        collector: &mut BlockProductionRatingCollector<'_>,
    ) -> Result<RatingEnvelopeReceiveOutcome, PorRatingTransportError> {
        let Some(encoded) = self.receiver.recv().await else {
            return Ok(RatingEnvelopeReceiveOutcome::Closed);
        };

        receive_rating_envelope(collector, &encoded)?;
        Ok(RatingEnvelopeReceiveOutcome::Accepted)
    }
}

/// Construct a bounded process-local rating-envelope channel.
pub fn bounded_rating_envelope_channel(
    capacity: usize,
) -> Result<
    (
        ChannelRatingEnvelopeBroadcaster,
        ChannelRatingEnvelopeReceiver,
    ),
    PorRatingChannelError,
> {
    if capacity == 0 {
        return Err(PorRatingChannelError::ZeroCapacity);
    }

    let (sender, receiver) = mpsc::channel(capacity);
    Ok((
        ChannelRatingEnvelopeBroadcaster { sender },
        ChannelRatingEnvelopeReceiver { receiver },
    ))
}
