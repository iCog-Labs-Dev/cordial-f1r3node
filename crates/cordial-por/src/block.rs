use cordial_miners_core::{
    NodeId,
    crypto::{Blake2b256Hasher, Hasher},
};

use crate::{
    commitments::{
        config_commitment, rating_batch_commitment, reputation_block_hash,
        reputation_list_commitment, validate_reputation_entries,
    },
    config::PorConfig,
    error::PorError,
    ratings::rating_round_from_finalized_wave,
    types::{RatingBatch, ReputationBlock, ReputationBlockHeader, ReputationEntry, ReputationList},
};

/// Canonical reputation-block format emitted by this crate.
pub const REPUTATION_BLOCK_VERSION: u16 = 1;

/// Fixed prefix identifying a canonical reputation-block wire envelope.
pub const POR_REPUTATION_BLOCK_MAGIC: &[u8; 17] = b"cordial-por-block";

/// Canonical reputation-block wire format emitted by this crate.
pub const POR_REPUTATION_BLOCK_WIRE_VERSION: u16 = 1;

/// Maximum accepted encoded reputation-block envelope size (64 MiB).
pub const MAX_REPUTATION_BLOCK_WIRE_LEN: usize = 64 * 1024 * 1024;

/// Maximum number of entries in one encoded reputation block.
pub const MAX_REPUTATION_BLOCK_ENTRIES: usize = 1_000_000;

/// Allocation bound for a reputation-block node identifier.
pub const MAX_REPUTATION_BLOCK_NODE_ID_LEN: usize = 4 * 1024;

const REPUTATION_BLOCK_CHECKSUM_DOMAIN: &[u8] = b"cordial-por:reputation-block-envelope:v1";
const CHECKSUM_LEN: usize = 32;
const FIXED_ENVELOPE_LEN: usize = POR_REPUTATION_BLOCK_MAGIC.len() + 2 + 8 + CHECKSUM_LEN;
const MAX_REPUTATION_BLOCK_PAYLOAD_LEN: usize = MAX_REPUTATION_BLOCK_WIRE_LEN - FIXED_ENVELOPE_LEN;

/// Bound chain-context input before it is copied into every block header.
pub const MAX_REPUTATION_BLOCK_SHARD_ID_LEN: usize = 256;

/// External finalized context required to build or audit a reputation block.
#[derive(Debug, Clone, Copy)]
pub struct ReputationBlockContext<'a> {
    pub shard_id: &'a [u8],
    pub source_finalized_wave: u64,
    pub previous_block: Option<&'a ReputationBlock>,
}

/// Build a fully committed reputation block from finalized protocol inputs.
///
/// The caller supplies protocol data, never precomputed commitment bytes. This
/// function derives the previous-block hash, configuration commitment, rating
/// batch commitment, and reputation-list root using the canonical v1 formats.
pub fn build_reputation_block(
    context: ReputationBlockContext<'_>,
    ratings: &RatingBatch,
    reputation_list: ReputationList,
    config: &PorConfig,
) -> Result<ReputationBlock, PorError> {
    validate_shard_id(context.shard_id)?;
    let round = rating_round_from_finalized_wave(context.source_finalized_wave)?;
    if ratings.round != round {
        return Err(PorError::InvalidRatingRound);
    }
    if reputation_list.round != round {
        return Err(PorError::InvalidReputationBlockRound);
    }

    let previous_reputation_hash = match context.previous_block {
        Some(previous) => {
            if previous.header.shard_id != context.shard_id {
                return Err(PorError::PreviousReputationBlockShardMismatch);
            }
            if previous.header.round.checked_add(1) != Some(round) {
                return Err(PorError::InvalidPreviousReputationBlockRound);
            }
            Some(reputation_block_hash(previous)?)
        }
        None => None,
    };
    let config_hash = config_commitment(config);
    let ratings_hash = rating_batch_commitment(ratings, config)?;
    let reputation_root = reputation_list_commitment(&reputation_list)?;
    let block = ReputationBlock {
        header: ReputationBlockHeader {
            version: REPUTATION_BLOCK_VERSION,
            shard_id: context.shard_id.to_vec(),
            source_finalized_wave: context.source_finalized_wave,
            round,
            previous_reputation_hash,
            config_hash,
            ratings_hash,
            reputation_root,
        },
        reputation_list,
    };

    validate_reputation_block(&block)?;
    Ok(block)
}

/// Validate the self-contained structural commitments of a reputation block.
///
/// Full audit additionally requires the expected shard, finalized wave,
/// previous block, rating batch, previous reputation, and protocol config;
/// [`crate::verify_reputation_transition`] performs those checks.
pub fn validate_reputation_block(block: &ReputationBlock) -> Result<(), PorError> {
    let header = &block.header;
    if header.version != REPUTATION_BLOCK_VERSION {
        return Err(PorError::UnsupportedReputationBlockVersion(header.version));
    }
    validate_shard_id(&header.shard_id)?;

    if rating_round_from_finalized_wave(header.source_finalized_wave) != Ok(header.round) {
        return Err(PorError::InvalidReputationBlockSourceWave);
    }
    if header.round != block.reputation_list.round {
        return Err(PorError::InvalidReputationBlockRound);
    }

    let expected_root = reputation_list_commitment(&block.reputation_list)?;
    if header.reputation_root != expected_root {
        return Err(PorError::ReputationBlockRootMismatch);
    }

    Ok(())
}

fn validate_shard_id(shard_id: &[u8]) -> Result<(), PorError> {
    if shard_id.is_empty() {
        return Err(PorError::MissingReputationBlockShardId);
    }
    if shard_id.len() > MAX_REPUTATION_BLOCK_SHARD_ID_LEN {
        return Err(PorError::ReputationBlockShardIdTooLong);
    }
    Ok(())
}

/// Encode a reputation block into the canonical v1 publication envelope.
pub fn encode_reputation_block(block: &ReputationBlock) -> Result<Vec<u8>, PorError> {
    let payload = encode_reputation_block_payload(block)?;
    let payload_len =
        u64::try_from(payload.len()).map_err(|_| PorError::ReputationBlockWireTooLarge)?;
    let checksum =
        reputation_block_checksum(POR_REPUTATION_BLOCK_WIRE_VERSION, payload_len, &payload);

    let mut encoded = Vec::with_capacity(FIXED_ENVELOPE_LEN + payload.len());
    encoded.extend_from_slice(POR_REPUTATION_BLOCK_MAGIC);
    encoded.extend_from_slice(&POR_REPUTATION_BLOCK_WIRE_VERSION.to_be_bytes());
    encoded.extend_from_slice(&payload_len.to_be_bytes());
    encoded.extend_from_slice(&payload);
    encoded.extend_from_slice(&checksum);
    Ok(encoded)
}

/// Decode and structurally validate a canonical reputation-block envelope.
pub fn decode_reputation_block(bytes: &[u8]) -> Result<ReputationBlock, PorError> {
    if bytes.len() > MAX_REPUTATION_BLOCK_WIRE_LEN {
        return Err(PorError::ReputationBlockWireTooLarge);
    }
    if bytes.len() < FIXED_ENVELOPE_LEN {
        return Err(PorError::MalformedReputationBlockWire);
    }

    let mut envelope = ReputationBlockDecoder::new(bytes);
    if envelope.read_exact(POR_REPUTATION_BLOCK_MAGIC.len())? != POR_REPUTATION_BLOCK_MAGIC {
        return Err(PorError::MalformedReputationBlockWire);
    }
    let wire_version = envelope.read_u16()?;
    if wire_version != POR_REPUTATION_BLOCK_WIRE_VERSION {
        return Err(PorError::UnsupportedReputationBlockWireVersion(
            wire_version,
        ));
    }
    let payload_len_u64 = envelope.read_u64()?;
    let payload_len =
        usize::try_from(payload_len_u64).map_err(|_| PorError::ReputationBlockWireTooLarge)?;
    if payload_len > MAX_REPUTATION_BLOCK_PAYLOAD_LEN {
        return Err(PorError::ReputationBlockWireTooLarge);
    }
    if envelope.remaining() != payload_len + CHECKSUM_LEN {
        return Err(PorError::MalformedReputationBlockWire);
    }

    let payload = envelope.read_exact(payload_len)?;
    let checksum = envelope.read_array::<CHECKSUM_LEN>()?;
    let expected = reputation_block_checksum(wire_version, payload_len_u64, payload);
    if checksum != expected {
        return Err(PorError::ReputationBlockWireChecksumMismatch);
    }

    decode_reputation_block_payload(payload)
}

pub(crate) fn encode_reputation_block_payload(
    block: &ReputationBlock,
) -> Result<Vec<u8>, PorError> {
    validate_reputation_block(block)?;
    let header = &block.header;
    let mut output = Vec::new();
    put_u16(&mut output, header.version)?;
    put_bytes(&mut output, &header.shard_id)?;
    put_u64(&mut output, header.source_finalized_wave)?;
    put_u64(&mut output, header.round)?;
    match header.previous_reputation_hash {
        Some(hash) => {
            put_byte(&mut output, 1)?;
            put_fixed(&mut output, &hash)?;
        }
        None => put_byte(&mut output, 0)?,
    }
    put_fixed(&mut output, &header.config_hash)?;
    put_fixed(&mut output, &header.ratings_hash)?;
    put_fixed(&mut output, &header.reputation_root)?;
    encode_reputation_list(&mut output, &block.reputation_list)?;
    Ok(output)
}

pub(crate) fn decode_reputation_block_payload(bytes: &[u8]) -> Result<ReputationBlock, PorError> {
    if bytes.len() > MAX_REPUTATION_BLOCK_PAYLOAD_LEN {
        return Err(PorError::ReputationBlockWireTooLarge);
    }

    let mut decoder = ReputationBlockDecoder::new(bytes);
    let version = decoder.read_u16()?;
    let shard_id = decoder.read_bytes(MAX_REPUTATION_BLOCK_SHARD_ID_LEN)?;
    let source_finalized_wave = decoder.read_u64()?;
    let round = decoder.read_u64()?;
    let previous_reputation_hash = match decoder.read_discriminant()? {
        false => None,
        true => Some(decoder.read_array::<32>()?),
    };
    let block = ReputationBlock {
        header: ReputationBlockHeader {
            version,
            shard_id,
            source_finalized_wave,
            round,
            previous_reputation_hash,
            config_hash: decoder.read_array::<32>()?,
            ratings_hash: decoder.read_array::<32>()?,
            reputation_root: decoder.read_array::<32>()?,
        },
        reputation_list: decode_reputation_list(&mut decoder)?,
    };
    if decoder.remaining() != 0 {
        return Err(PorError::MalformedReputationBlockWire);
    }
    validate_reputation_block(&block)?;
    Ok(block)
}

fn encode_reputation_list(output: &mut Vec<u8>, list: &ReputationList) -> Result<(), PorError> {
    validate_reputation_entries(&list.entries)?;
    put_u64(output, list.round)?;
    put_count(output, list.entries.len())?;
    for entry in &list.entries {
        put_node_id(output, &entry.node_id)?;
        put_u64(output, entry.reputation)?;
        put_byte(output, u8::from(entry.is_excluded))?;
    }
    Ok(())
}

fn decode_reputation_list(
    decoder: &mut ReputationBlockDecoder<'_>,
) -> Result<ReputationList, PorError> {
    let round = decoder.read_u64()?;
    let count = decoder.read_count()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        entries.push(ReputationEntry {
            node_id: decoder.read_node_id()?,
            reputation: decoder.read_u64()?,
            is_excluded: decoder.read_discriminant()?,
        });
    }
    validate_reputation_entries(&entries)?;
    Ok(ReputationList { round, entries })
}

fn put_node_id(output: &mut Vec<u8>, node_id: &NodeId) -> Result<(), PorError> {
    if node_id.0.len() > MAX_REPUTATION_BLOCK_NODE_ID_LEN {
        return Err(PorError::ReputationBlockWireTooLarge);
    }
    put_bytes(output, &node_id.0)
}

fn put_count(output: &mut Vec<u8>, count: usize) -> Result<(), PorError> {
    if count > MAX_REPUTATION_BLOCK_ENTRIES {
        return Err(PorError::ReputationBlockWireTooLarge);
    }
    put_u64(
        output,
        u64::try_from(count).map_err(|_| PorError::ReputationBlockWireTooLarge)?,
    )
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PorError> {
    put_u64(
        output,
        u64::try_from(bytes.len()).map_err(|_| PorError::ReputationBlockWireTooLarge)?,
    )?;
    put_fixed(output, bytes)
}

fn put_u16(output: &mut Vec<u8>, value: u16) -> Result<(), PorError> {
    put_fixed(output, &value.to_be_bytes())
}

fn put_u64(output: &mut Vec<u8>, value: u64) -> Result<(), PorError> {
    put_fixed(output, &value.to_be_bytes())
}

fn put_byte(output: &mut Vec<u8>, value: u8) -> Result<(), PorError> {
    put_fixed(output, &[value])
}

fn put_fixed(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PorError> {
    let next_len = output
        .len()
        .checked_add(bytes.len())
        .ok_or(PorError::ReputationBlockWireTooLarge)?;
    if next_len > MAX_REPUTATION_BLOCK_PAYLOAD_LEN {
        return Err(PorError::ReputationBlockWireTooLarge);
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn reputation_block_checksum(version: u16, payload_len: u64, payload: &[u8]) -> [u8; 32] {
    let mut checksum_input = Vec::with_capacity(
        REPUTATION_BLOCK_CHECKSUM_DOMAIN.len()
            + POR_REPUTATION_BLOCK_MAGIC.len()
            + 2
            + 8
            + payload.len(),
    );
    checksum_input.extend_from_slice(REPUTATION_BLOCK_CHECKSUM_DOMAIN);
    checksum_input.extend_from_slice(POR_REPUTATION_BLOCK_MAGIC);
    checksum_input.extend_from_slice(&version.to_be_bytes());
    checksum_input.extend_from_slice(&payload_len.to_be_bytes());
    checksum_input.extend_from_slice(payload);
    Blake2b256Hasher.hash(&checksum_input)
}

struct ReputationBlockDecoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ReputationBlockDecoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], PorError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(PorError::MalformedReputationBlockWire)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(PorError::MalformedReputationBlockWire)?;
        self.offset = end;
        Ok(value)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], PorError> {
        self.read_exact(N)?
            .try_into()
            .map_err(|_| PorError::MalformedReputationBlockWire)
    }

    fn read_u16(&mut self) -> Result<u16, PorError> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    fn read_u64(&mut self) -> Result<u64, PorError> {
        Ok(u64::from_be_bytes(self.read_array()?))
    }

    fn read_count(&mut self) -> Result<usize, PorError> {
        let count =
            usize::try_from(self.read_u64()?).map_err(|_| PorError::ReputationBlockWireTooLarge)?;
        if count > MAX_REPUTATION_BLOCK_ENTRIES {
            return Err(PorError::ReputationBlockWireTooLarge);
        }
        Ok(count)
    }

    fn read_bytes(&mut self, max_len: usize) -> Result<Vec<u8>, PorError> {
        let len =
            usize::try_from(self.read_u64()?).map_err(|_| PorError::ReputationBlockWireTooLarge)?;
        if len > max_len {
            return Err(PorError::ReputationBlockWireTooLarge);
        }
        Ok(self.read_exact(len)?.to_vec())
    }

    fn read_node_id(&mut self) -> Result<NodeId, PorError> {
        Ok(NodeId(self.read_bytes(MAX_REPUTATION_BLOCK_NODE_ID_LEN)?))
    }

    fn read_discriminant(&mut self) -> Result<bool, PorError> {
        match self.read_exact(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(PorError::MalformedReputationBlockWire),
        }
    }
}
