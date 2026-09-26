//! Bounded process-local handoff for signed reputation-block publications.
//!
//! A concrete peer stack can feed and drain this boundary without giving an
//! unbounded network queue direct access to consensus orchestration.

use tokio::sync::mpsc;

use super::reputation_block::{
    MAX_REPUTATION_BLOCK_PUBLICATION_LEN, PorReputationBlockTransportError,
    ReputationBlockEnvelopeBroadcaster, ReputationBlockPublicationV1,
    receive_reputation_block_envelope,
};
use thiserror::Error;

/// Failures specific to the bounded reputation-block channel.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PorReputationBlockChannelError {
    #[error("PoR reputation-block channel capacity must be non-zero")]
    ZeroCapacity,

    #[error("PoR reputation-block channel envelope is {actual} bytes; maximum is {maximum}")]
    EnvelopeTooLong { actual: usize, maximum: usize },

    #[error("PoR reputation-block channel is full")]
    Full,

    #[error("PoR reputation-block channel is closed")]
    Closed,
}

/// Outcome of awaiting the next signed block publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReputationBlockReceiveOutcome {
    Received(Box<ReputationBlockPublicationV1>),
    Closed,
}

/// Cloneable bounded-channel sender for signed block publications.
#[derive(Debug, Clone)]
pub struct ChannelReputationBlockEnvelopeBroadcaster {
    sender: mpsc::Sender<Vec<u8>>,
}

impl ChannelReputationBlockEnvelopeBroadcaster {
    pub fn try_broadcast(&self, envelope: &[u8]) -> Result<(), PorReputationBlockChannelError> {
        if envelope.len() > MAX_REPUTATION_BLOCK_PUBLICATION_LEN {
            return Err(PorReputationBlockChannelError::EnvelopeTooLong {
                actual: envelope.len(),
                maximum: MAX_REPUTATION_BLOCK_PUBLICATION_LEN,
            });
        }

        self.sender
            .try_send(envelope.to_vec())
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PorReputationBlockChannelError::Full,
                mpsc::error::TrySendError::Closed(_) => PorReputationBlockChannelError::Closed,
            })
    }
}

impl ReputationBlockEnvelopeBroadcaster for ChannelReputationBlockEnvelopeBroadcaster {
    fn broadcast_reputation_block_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.try_broadcast(envelope)
            .map_err(|error| error.to_string())
    }
}

/// Receiving half of the bounded signed-block channel.
pub struct ChannelReputationBlockEnvelopeReceiver {
    receiver: mpsc::Receiver<Vec<u8>>,
}

impl ChannelReputationBlockEnvelopeReceiver {
    /// Await, decode, and authenticate the next publication.
    pub async fn receive_next(
        &mut self,
    ) -> Result<ReputationBlockReceiveOutcome, PorReputationBlockTransportError> {
        let Some(encoded) = self.receiver.recv().await else {
            return Ok(ReputationBlockReceiveOutcome::Closed);
        };

        let publication = receive_reputation_block_envelope(&encoded)?;
        Ok(ReputationBlockReceiveOutcome::Received(Box::new(
            publication,
        )))
    }
}

/// Construct a bounded process-local signed reputation-block channel.
pub fn bounded_reputation_block_envelope_channel(
    capacity: usize,
) -> Result<
    (
        ChannelReputationBlockEnvelopeBroadcaster,
        ChannelReputationBlockEnvelopeReceiver,
    ),
    PorReputationBlockChannelError,
> {
    if capacity == 0 {
        return Err(PorReputationBlockChannelError::ZeroCapacity);
    }

    let (sender, receiver) = mpsc::channel(capacity);
    Ok((
        ChannelReputationBlockEnvelopeBroadcaster { sender },
        ChannelReputationBlockEnvelopeReceiver { receiver },
    ))
}
