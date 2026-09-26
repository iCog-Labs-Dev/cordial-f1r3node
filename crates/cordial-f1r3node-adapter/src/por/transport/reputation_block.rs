//! Signed, transport-neutral publication of canonical reputation blocks.
//!
//! The publication signature authenticates the validator that sent the exact
//! canonical block envelope. It does not prove publication quorum or that the
//! block is a valid transition from the receiver's local state.

use cordial_miners_core::{
    NodeId,
    crypto::{Blake2b256Hasher, Hasher, Secp256k1Scheme, SignatureScheme},
};
use cordial_por::{
    MAX_REPUTATION_BLOCK_WIRE_LEN, PorError, ReputationBlock, decode_reputation_block,
    encode_reputation_block,
};
use thiserror::Error;

/// Fixed domain identifying a signed reputation-block publication.
pub const REPUTATION_BLOCK_PUBLICATION_DOMAIN: &[u8] = b"cordial-por:reputation-block-publication";

/// Current signed reputation-block publication version.
pub const REPUTATION_BLOCK_PUBLICATION_VERSION: u16 = 1;

const COMPRESSED_SECP256K1_PUBLIC_KEY_LEN: usize = 33;
const UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN: usize = 65;
const MAX_SECP256K1_DER_SIGNATURE_LEN: usize = 72;
const PUBLICATION_FIXED_LEN: usize = REPUTATION_BLOCK_PUBLICATION_DOMAIN.len() + 2 + 2 + 8 + 2;

/// Maximum byte length of a signed v1 reputation-block publication.
pub const MAX_REPUTATION_BLOCK_PUBLICATION_LEN: usize = PUBLICATION_FIXED_LEN
    + UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN
    + MAX_REPUTATION_BLOCK_WIRE_LEN
    + MAX_SECP256K1_DER_SIGNATURE_LEN;

/// Failures while signing, encoding, decoding, or authenticating a publication.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PorReputationBlockPublicationError {
    #[error("PoR reputation-block publication is {actual} bytes; maximum is {maximum}")]
    PublicationTooLong { actual: usize, maximum: usize },

    #[error("PoR reputation-block publication ended unexpectedly")]
    UnexpectedEnd,

    #[error("PoR reputation-block publication contains trailing bytes")]
    TrailingBytes,

    #[error("invalid PoR reputation-block publication domain")]
    InvalidDomain,

    #[error("unsupported PoR reputation-block publication version {0}")]
    UnsupportedVersion(u16),

    #[error("invalid PoR reputation-block publisher key length {0}")]
    InvalidPublisherKeyLength(usize),

    #[error("PoR reputation-block envelope is {actual} bytes; maximum is {maximum}")]
    BlockEnvelopeTooLong { actual: usize, maximum: usize },

    #[error("PoR reputation-block publication has no signature")]
    MissingSignature,

    #[error("PoR reputation-block publication signature is {actual} bytes; maximum is {maximum}")]
    SignatureTooLong { actual: usize, maximum: usize },

    #[error("PoR reputation-block publication signing failed: {0}")]
    Signing(String),

    #[error("PoR reputation-block publication signature is invalid")]
    InvalidSignature,

    #[error("invalid canonical reputation block: {0}")]
    Block(#[from] PorError),
}

/// A structurally valid block publication with a verified publisher signature.
///
/// Construction and decoding both verify the signature. The fields are private,
/// so safe code cannot mutate an authenticated publication after verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationBlockPublicationV1 {
    publisher: NodeId,
    block: ReputationBlock,
    block_envelope: Vec<u8>,
    signature: Vec<u8>,
}

impl ReputationBlockPublicationV1 {
    /// Verify an externally supplied signature over a canonical block.
    pub fn new(
        publisher: NodeId,
        block: ReputationBlock,
        signature: Vec<u8>,
    ) -> Result<Self, PorReputationBlockPublicationError> {
        let block_envelope = encode_reputation_block(&block)?;
        Self::from_encoded_parts(publisher, block, block_envelope, signature)
    }

    /// Sign and self-verify one canonical block publication.
    pub fn sign(
        publisher: NodeId,
        block: ReputationBlock,
        private_key: &[u8],
    ) -> Result<Self, PorReputationBlockPublicationError> {
        validate_publisher(&publisher)?;
        let block_envelope = encode_reputation_block(&block)?;
        let signing_hash = publication_signing_hash(&publisher, &block_envelope);
        let signature = Secp256k1Scheme
            .sign(&signing_hash, private_key)
            .map_err(PorReputationBlockPublicationError::Signing)?;
        Self::from_encoded_parts(publisher, block, block_envelope, signature)
    }

    pub fn publisher(&self) -> &NodeId {
        &self.publisher
    }

    pub fn block(&self) -> &ReputationBlock {
        &self.block
    }

    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    pub fn into_block(self) -> ReputationBlock {
        self.block
    }

    /// Encode the signed publication using the canonical v1 layout.
    pub fn encode(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(
            PUBLICATION_FIXED_LEN
                + self.publisher.0.len()
                + self.block_envelope.len()
                + self.signature.len(),
        );
        output.extend_from_slice(REPUTATION_BLOCK_PUBLICATION_DOMAIN);
        output.extend_from_slice(&REPUTATION_BLOCK_PUBLICATION_VERSION.to_be_bytes());
        put_u16_bytes(&mut output, &self.publisher.0);
        put_u64_bytes(&mut output, &self.block_envelope);
        put_u16_bytes(&mut output, &self.signature);
        output
    }

    /// Decode, structurally validate, and authenticate one v1 publication.
    pub fn decode(input: &[u8]) -> Result<Self, PorReputationBlockPublicationError> {
        if input.len() > MAX_REPUTATION_BLOCK_PUBLICATION_LEN {
            return Err(PorReputationBlockPublicationError::PublicationTooLong {
                actual: input.len(),
                maximum: MAX_REPUTATION_BLOCK_PUBLICATION_LEN,
            });
        }

        let mut decoder = PublicationDecoder::new(input);
        if decoder.read_exact(REPUTATION_BLOCK_PUBLICATION_DOMAIN.len())?
            != REPUTATION_BLOCK_PUBLICATION_DOMAIN
        {
            return Err(PorReputationBlockPublicationError::InvalidDomain);
        }

        let version = decoder.read_u16()?;
        if version != REPUTATION_BLOCK_PUBLICATION_VERSION {
            return Err(PorReputationBlockPublicationError::UnsupportedVersion(
                version,
            ));
        }

        let publisher = NodeId(decoder.read_u16_bytes()?.to_vec());
        validate_publisher(&publisher)?;

        let block_len_u64 = decoder.read_u64()?;
        let block_len = usize::try_from(block_len_u64).map_err(|_| {
            PorReputationBlockPublicationError::BlockEnvelopeTooLong {
                actual: usize::MAX,
                maximum: MAX_REPUTATION_BLOCK_WIRE_LEN,
            }
        })?;
        if block_len > MAX_REPUTATION_BLOCK_WIRE_LEN {
            return Err(PorReputationBlockPublicationError::BlockEnvelopeTooLong {
                actual: block_len,
                maximum: MAX_REPUTATION_BLOCK_WIRE_LEN,
            });
        }
        let block_envelope = decoder.read_exact(block_len)?.to_vec();
        let block = decode_reputation_block(&block_envelope)?;

        let signature = decoder.read_u16_bytes()?.to_vec();
        validate_signature(&signature)?;
        if !decoder.is_finished() {
            return Err(PorReputationBlockPublicationError::TrailingBytes);
        }

        Self::from_encoded_parts(publisher, block, block_envelope, signature)
    }

    fn from_encoded_parts(
        publisher: NodeId,
        block: ReputationBlock,
        block_envelope: Vec<u8>,
        signature: Vec<u8>,
    ) -> Result<Self, PorReputationBlockPublicationError> {
        validate_publisher(&publisher)?;
        validate_signature(&signature)?;
        if block_envelope.len() > MAX_REPUTATION_BLOCK_WIRE_LEN {
            return Err(PorReputationBlockPublicationError::BlockEnvelopeTooLong {
                actual: block_envelope.len(),
                maximum: MAX_REPUTATION_BLOCK_WIRE_LEN,
            });
        }

        let signing_hash = publication_signing_hash(&publisher, &block_envelope);
        if !Secp256k1Scheme.verify(&signing_hash, &publisher.0, &signature) {
            return Err(PorReputationBlockPublicationError::InvalidSignature);
        }

        Ok(Self {
            publisher,
            block,
            block_envelope,
            signature,
        })
    }
}

/// Return the Blake2b-256 digest signed by a reputation-block publisher.
pub fn reputation_block_publication_signing_hash(
    publisher: &NodeId,
    block: &ReputationBlock,
) -> Result<[u8; 32], PorReputationBlockPublicationError> {
    validate_publisher(publisher)?;
    let block_envelope = encode_reputation_block(block)?;
    Ok(publication_signing_hash(publisher, &block_envelope))
}

fn publication_signing_hash(publisher: &NodeId, block_envelope: &[u8]) -> [u8; 32] {
    let mut payload = Vec::with_capacity(
        REPUTATION_BLOCK_PUBLICATION_DOMAIN.len()
            + 2
            + 2
            + publisher.0.len()
            + 8
            + block_envelope.len(),
    );
    payload.extend_from_slice(REPUTATION_BLOCK_PUBLICATION_DOMAIN);
    payload.extend_from_slice(&REPUTATION_BLOCK_PUBLICATION_VERSION.to_be_bytes());
    put_u16_bytes(&mut payload, &publisher.0);
    put_u64_bytes(&mut payload, block_envelope);
    Blake2b256Hasher.hash(&payload)
}

fn validate_publisher(publisher: &NodeId) -> Result<(), PorReputationBlockPublicationError> {
    match publisher.0.len() {
        COMPRESSED_SECP256K1_PUBLIC_KEY_LEN | UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN => Ok(()),
        length => Err(PorReputationBlockPublicationError::InvalidPublisherKeyLength(length)),
    }
}

fn validate_signature(signature: &[u8]) -> Result<(), PorReputationBlockPublicationError> {
    if signature.is_empty() {
        return Err(PorReputationBlockPublicationError::MissingSignature);
    }
    if signature.len() > MAX_SECP256K1_DER_SIGNATURE_LEN {
        return Err(PorReputationBlockPublicationError::SignatureTooLong {
            actual: signature.len(),
            maximum: MAX_SECP256K1_DER_SIGNATURE_LEN,
        });
    }
    Ok(())
}

fn put_u16_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    let length = u16::try_from(bytes.len()).expect("validated publication field fits in u16");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(bytes);
}

fn put_u64_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).expect("validated publication field fits in u64");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(bytes);
}

struct PublicationDecoder<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> PublicationDecoder<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn read_exact(
        &mut self,
        length: usize,
    ) -> Result<&'a [u8], PorReputationBlockPublicationError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(PorReputationBlockPublicationError::UnexpectedEnd)?;
        let bytes = self
            .input
            .get(self.position..end)
            .ok_or(PorReputationBlockPublicationError::UnexpectedEnd)?;
        self.position = end;
        Ok(bytes)
    }

    fn read_u16(&mut self) -> Result<u16, PorReputationBlockPublicationError> {
        let bytes: [u8; 2] = self
            .read_exact(2)?
            .try_into()
            .expect("read_exact returned two bytes");
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, PorReputationBlockPublicationError> {
        let bytes: [u8; 8] = self
            .read_exact(8)?
            .try_into()
            .expect("read_exact returned eight bytes");
        Ok(u64::from_be_bytes(bytes))
    }

    fn read_u16_bytes(&mut self) -> Result<&'a [u8], PorReputationBlockPublicationError> {
        let length = usize::from(self.read_u16()?);
        self.read_exact(length)
    }

    fn is_finished(&self) -> bool {
        self.position == self.input.len()
    }
}

/// Synchronous delivery boundary for one signed reputation-block publication.
pub trait ReputationBlockEnvelopeBroadcaster {
    fn broadcast_reputation_block_envelope(&self, envelope: &[u8]) -> Result<(), String>;
}

/// Closure-backed broadcaster useful for wiring concrete peer transports.
pub struct FnReputationBlockEnvelopeBroadcaster<F>
where
    F: Fn(&[u8]) -> Result<(), String>,
{
    broadcast: F,
}

impl<F> FnReputationBlockEnvelopeBroadcaster<F>
where
    F: Fn(&[u8]) -> Result<(), String>,
{
    pub fn new(broadcast: F) -> Self {
        Self { broadcast }
    }
}

impl<F> ReputationBlockEnvelopeBroadcaster for FnReputationBlockEnvelopeBroadcaster<F>
where
    F: Fn(&[u8]) -> Result<(), String>,
{
    fn broadcast_reputation_block_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        (self.broadcast)(envelope)
    }
}

/// Failures while authenticating, sending, or receiving block publications.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PorReputationBlockTransportError {
    #[error(transparent)]
    Publication(#[from] PorReputationBlockPublicationError),

    #[error("PoR reputation-block broadcast failed: {0}")]
    Broadcast(String),
}

/// Broadcast an already authenticated publication, preserving it for retries.
pub fn broadcast_reputation_block_publication(
    broadcaster: &impl ReputationBlockEnvelopeBroadcaster,
    publication: &ReputationBlockPublicationV1,
) -> Result<(), PorReputationBlockTransportError> {
    let encoded = publication.encode();
    broadcaster
        .broadcast_reputation_block_envelope(&encoded)
        .map_err(PorReputationBlockTransportError::Broadcast)
}

/// Sign and broadcast one block after all encoding work succeeds.
pub fn broadcast_reputation_block(
    broadcaster: &impl ReputationBlockEnvelopeBroadcaster,
    publisher: NodeId,
    block: ReputationBlock,
    private_key: &[u8],
) -> Result<ReputationBlockPublicationV1, PorReputationBlockTransportError> {
    let publication = ReputationBlockPublicationV1::sign(publisher, block, private_key)?;
    broadcast_reputation_block_publication(broadcaster, &publication)?;
    Ok(publication)
}

/// Decode and authenticate an inbound block without admitting its transition.
pub fn receive_reputation_block_envelope(
    encoded: &[u8],
) -> Result<ReputationBlockPublicationV1, PorReputationBlockTransportError> {
    Ok(ReputationBlockPublicationV1::decode(encoded)?)
}
