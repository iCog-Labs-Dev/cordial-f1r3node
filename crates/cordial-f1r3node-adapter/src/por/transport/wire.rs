//! Deterministic wire encoding for signed block-production ratings.
//!
//! The envelope is intentionally transport-independent: gRPC, HTTP, or peer
//! gossip can carry the same bounded bytes. Decoding performs structural and
//! size checks only. Signature and finalized-evidence validation remain the
//! responsibility of the PoR rating collector.

use std::fmt;

use cordial_por::RatingRecord;

/// Fixed domain identifying a block-production PoR rating envelope.
pub const BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN: &[u8] =
    b"cordial-por:block-production-rating-envelope";

/// Current block-production rating envelope version.
pub const BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION: u16 = 1;

const COMPRESSED_SECP256K1_PUBLIC_KEY_LEN: usize = 33;
const UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN: usize = 65;
const BLOCK_HASH_LEN: usize = 32;
const MAX_SECP256K1_DER_SIGNATURE_LEN: usize = 72;

/// Maximum byte length of a valid v1 envelope.
pub const MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN: usize = BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN
    .len()
    + 2
    + 8
    + 8
    + 2
    + UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN
    + 2
    + UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN
    + 8
    + BLOCK_HASH_LEN
    + 2
    + MAX_SECP256K1_DER_SIGNATURE_LEN;

/// Structural failures while encoding or decoding a rating envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingWireError {
    EnvelopeTooLong { actual: usize, maximum: usize },
    UnexpectedEnd,
    TrailingBytes,
    InvalidDomain,
    UnsupportedVersion(u16),
    InvalidRaterKeyLength(usize),
    InvalidRecipientKeyLength(usize),
    MissingInteractionReference,
    InvalidInteractionReferenceLength(usize),
    MissingSignature,
    SignatureTooLong { actual: usize, maximum: usize },
    FinalizedWaveRoundMismatch,
}

impl fmt::Display for PorRatingWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvelopeTooLong { actual, maximum } => {
                write!(
                    f,
                    "PoR rating envelope is {actual} bytes; maximum is {maximum}"
                )
            }
            Self::UnexpectedEnd => write!(f, "PoR rating envelope ended unexpectedly"),
            Self::TrailingBytes => write!(f, "PoR rating envelope contains trailing bytes"),
            Self::InvalidDomain => write!(f, "invalid PoR rating envelope domain"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported PoR rating envelope version {version}")
            }
            Self::InvalidRaterKeyLength(length) => {
                write!(f, "invalid PoR rating rater key length {length}")
            }
            Self::InvalidRecipientKeyLength(length) => {
                write!(f, "invalid PoR rating recipient key length {length}")
            }
            Self::MissingInteractionReference => {
                write!(
                    f,
                    "PoR block-production rating has no interaction reference"
                )
            }
            Self::InvalidInteractionReferenceLength(length) => write!(
                f,
                "invalid PoR block-production interaction reference length {length}"
            ),
            Self::MissingSignature => write!(f, "PoR rating envelope has no signature"),
            Self::SignatureTooLong { actual, maximum } => write!(
                f,
                "PoR rating signature is {actual} bytes; maximum is {maximum}"
            ),
            Self::FinalizedWaveRoundMismatch => write!(
                f,
                "PoR rating round does not immediately follow the finalized wave"
            ),
        }
    }
}

impl std::error::Error for PorRatingWireError {}

/// Version 1 transport envelope for one signed block-production rating.
///
/// Fields are private so an envelope that passed construction or decoding
/// cannot later be mutated into a structurally invalid value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockProductionRatingEnvelopeV1 {
    finalized_wave: u64,
    rating: RatingRecord,
}

impl BlockProductionRatingEnvelopeV1 {
    pub fn new(finalized_wave: u64, rating: RatingRecord) -> Result<Self, PorRatingWireError> {
        validate_rating_shape(finalized_wave, &rating)?;
        Ok(Self {
            finalized_wave,
            rating,
        })
    }

    pub fn finalized_wave(&self) -> u64 {
        self.finalized_wave
    }

    pub fn rating(&self) -> &RatingRecord {
        &self.rating
    }

    pub fn into_rating(self) -> RatingRecord {
        self.rating
    }

    /// Encode the envelope using the canonical v1 binary layout.
    pub fn encode(&self) -> Vec<u8> {
        let interaction_ref = self
            .rating
            .interaction_ref
            .as_deref()
            .expect("validated envelope always has an interaction reference");
        let mut output = Vec::with_capacity(
            BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN.len()
                + 2
                + 8
                + 8
                + 2
                + self.rating.rater.0.len()
                + 2
                + self.rating.recipient.0.len()
                + 8
                + interaction_ref.len()
                + 2
                + self.rating.signature.len(),
        );

        output.extend_from_slice(BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN);
        output.extend_from_slice(&BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION.to_be_bytes());
        output.extend_from_slice(&self.finalized_wave.to_be_bytes());
        output.extend_from_slice(&self.rating.round.to_be_bytes());
        put_u16_bytes(&mut output, &self.rating.rater.0);
        put_u16_bytes(&mut output, &self.rating.recipient.0);
        output.extend_from_slice(&self.rating.score.to_be_bytes());
        output.extend_from_slice(interaction_ref);
        put_u16_bytes(&mut output, &self.rating.signature);
        output
    }

    /// Decode one canonical v1 block-production rating envelope.
    pub fn decode(input: &[u8]) -> Result<Self, PorRatingWireError> {
        if input.len() > MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN {
            return Err(PorRatingWireError::EnvelopeTooLong {
                actual: input.len(),
                maximum: MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
            });
        }

        let mut decoder = Decoder::new(input);
        if decoder.read_exact(BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN.len())?
            != BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN
        {
            return Err(PorRatingWireError::InvalidDomain);
        }

        let version = decoder.read_u16()?;
        if version != BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION {
            return Err(PorRatingWireError::UnsupportedVersion(version));
        }

        let finalized_wave = decoder.read_u64()?;
        let round = decoder.read_u64()?;
        let rater = decoder.read_u16_bytes()?;
        let recipient = decoder.read_u16_bytes()?;
        let score = decoder.read_u64()?;
        let interaction_ref = decoder.read_exact(BLOCK_HASH_LEN)?.to_vec();
        let signature = decoder.read_u16_bytes()?;

        if !decoder.is_finished() {
            return Err(PorRatingWireError::TrailingBytes);
        }

        Self::new(
            finalized_wave,
            RatingRecord {
                round,
                rater: cordial_miners_core::NodeId(rater),
                recipient: cordial_miners_core::NodeId(recipient),
                score,
                signature,
                interaction_ref: Some(interaction_ref),
            },
        )
    }
}

fn validate_rating_shape(
    finalized_wave: u64,
    rating: &RatingRecord,
) -> Result<(), PorRatingWireError> {
    if finalized_wave.checked_add(1) != Some(rating.round) {
        return Err(PorRatingWireError::FinalizedWaveRoundMismatch);
    }

    validate_public_key_length(&rating.rater.0)
        .map_err(PorRatingWireError::InvalidRaterKeyLength)?;
    validate_public_key_length(&rating.recipient.0)
        .map_err(PorRatingWireError::InvalidRecipientKeyLength)?;

    let interaction_ref = rating
        .interaction_ref
        .as_ref()
        .ok_or(PorRatingWireError::MissingInteractionReference)?;
    if interaction_ref.len() != BLOCK_HASH_LEN {
        return Err(PorRatingWireError::InvalidInteractionReferenceLength(
            interaction_ref.len(),
        ));
    }

    if rating.signature.is_empty() {
        return Err(PorRatingWireError::MissingSignature);
    }
    if rating.signature.len() > MAX_SECP256K1_DER_SIGNATURE_LEN {
        return Err(PorRatingWireError::SignatureTooLong {
            actual: rating.signature.len(),
            maximum: MAX_SECP256K1_DER_SIGNATURE_LEN,
        });
    }

    Ok(())
}

fn validate_public_key_length(bytes: &[u8]) -> Result<(), usize> {
    match bytes.len() {
        COMPRESSED_SECP256K1_PUBLIC_KEY_LEN | UNCOMPRESSED_SECP256K1_PUBLIC_KEY_LEN => Ok(()),
        length => Err(length),
    }
}

fn put_u16_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    let length = u16::try_from(bytes.len()).expect("validated envelope field fits in u16");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(bytes);
}

struct Decoder<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], PorRatingWireError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(PorRatingWireError::UnexpectedEnd)?;
        let bytes = self
            .input
            .get(self.position..end)
            .ok_or(PorRatingWireError::UnexpectedEnd)?;
        self.position = end;
        Ok(bytes)
    }

    fn read_u16(&mut self) -> Result<u16, PorRatingWireError> {
        let bytes: [u8; 2] = self
            .read_exact(2)?
            .try_into()
            .expect("read_exact returned two bytes");
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, PorRatingWireError> {
        let bytes: [u8; 8] = self
            .read_exact(8)?
            .try_into()
            .expect("read_exact returned eight bytes");
        Ok(u64::from_be_bytes(bytes))
    }

    fn read_u16_bytes(&mut self) -> Result<Vec<u8>, PorRatingWireError> {
        let length = usize::from(self.read_u16()?);
        Ok(self.read_exact(length)?.to_vec())
    }

    fn is_finished(&self) -> bool {
        self.position == self.input.len()
    }
}
