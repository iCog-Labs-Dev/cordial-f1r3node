//! Durable record of the exact PoR projection activated in Cordial.
//!
//! The reputation-state snapshot and this record deliberately commit at
//! different times. State commits first. Cordial then accepts the projected
//! weights, and only then is the activation record replaced. The record retains
//! the canonical weight map so startup can restore the actual prior consensus
//! weights before evaluating a newer committed round.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use cordial_miners_core::{
    NodeId,
    crypto::{Blake2b256Hasher, Hasher},
};
use cordial_por::{
    MAX_REPUTATION_STATE_ENTRIES, MAX_REPUTATION_STATE_NODE_ID_LEN,
    MAX_REPUTATION_STATE_SNAPSHOT_LEN, PorError, ReputationCommitment, ReputationState,
    ReputationWeight, reputation_block_hash,
};
use thiserror::Error;

use super::persistence::POR_STATE_DIRECTORY;

/// Atomic activation-record target inside [`POR_STATE_DIRECTORY`].
pub const POR_WEIGHT_ACTIVATION_FILE_NAME: &str = "weight-activation.bin";

const POR_WEIGHT_ACTIVATION_TEMP_FILE_NAME: &str = ".weight-activation.bin.tmp";
const POR_WEIGHT_ACTIVATION_MAGIC: &[u8] = b"cordial-por-weight-activation";
const POR_WEIGHT_ACTIVATION_VERSION: u16 = 2;
const POR_WEIGHT_ACTIVATION_CHECKSUM_DOMAIN: &[u8] = b"cordial-por:weight-activation-record:v2";
const POR_WEIGHT_COMMITMENT_DOMAIN: &[u8] = b"cordial-por:authorized-weight-map:v1";
const MAX_POR_WEIGHT_ACTIVATION_RECORD_LEN: usize = MAX_REPUTATION_STATE_SNAPSHOT_LEN;
const CHECKSUM_LEN: usize = 32;

/// Recoverable identity of one PoR weight projection accepted by Cordial.
///
/// The checkpoint hash binds the committed reputation calculation. The weight
/// commitment additionally binds the exact Cordial-supplied validator set and
/// projected values, without transferring ownership of membership to PoR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorWeightActivationRecord {
    reputation_round: u64,
    checkpoint_hash: Option<ReputationCommitment>,
    source_finalized_wave: Option<u64>,
    weights: HashMap<NodeId, ReputationWeight>,
    weights_commitment: ReputationCommitment,
}

impl PorWeightActivationRecord {
    pub fn reputation_round(&self) -> u64 {
        self.reputation_round
    }

    pub fn checkpoint_hash(&self) -> Option<ReputationCommitment> {
        self.checkpoint_hash
    }

    pub fn source_finalized_wave(&self) -> Option<u64> {
        self.source_finalized_wave
    }

    pub fn weights_commitment(&self) -> ReputationCommitment {
        self.weights_commitment
    }

    /// Exact validator-weight map that was accepted by Cordial.
    pub fn weights(&self) -> &HashMap<NodeId, ReputationWeight> {
        &self.weights
    }

    pub(crate) fn from_state_and_weights(
        state: &ReputationState,
        weights: &HashMap<NodeId, ReputationWeight>,
    ) -> Result<Self, PorError> {
        let (checkpoint_hash, source_finalized_wave) = match state.latest_block() {
            Some(block) => (
                Some(reputation_block_hash(block)?),
                Some(block.header.source_finalized_wave),
            ),
            None => (None, None),
        };

        Ok(Self {
            reputation_round: state.round(),
            checkpoint_hash,
            weights: weights.clone(),
            source_finalized_wave,
            weights_commitment: authorized_weight_commitment(weights),
        })
    }

    pub(crate) fn matches_state_checkpoint(
        &self,
        state: &ReputationState,
    ) -> Result<bool, PorError> {
        if self.reputation_round != state.round() {
            return Ok(false);
        }

        let (checkpoint_hash, source_finalized_wave) = match state.latest_block() {
            Some(block) => (
                Some(reputation_block_hash(block)?),
                Some(block.header.source_finalized_wave),
            ),
            None => (None, None),
        };
        Ok(self.checkpoint_hash == checkpoint_hash
            && self.source_finalized_wave == source_finalized_wave)
    }
}

/// Result of synchronizing the latest committed PoR state into one ingress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorWeightActivationOutcome {
    /// A new round or validator-set projection was activated and recorded.
    Activated,
    /// An existing durable record was reapplied to fresh or drifted memory.
    Restored,
    /// This ingress already contains the exact recorded projection.
    AlreadyActive,
}

/// Errors reading or atomically replacing the durable activation record.
#[derive(Debug, Error)]
pub enum PorWeightActivationStoreError {
    #[error("PoR weight-activation storage I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("malformed PoR weight-activation record")]
    MalformedRecord,

    #[error("PoR weight-activation record is {actual} bytes, exceeding the {maximum}-byte limit")]
    RecordTooLarge { actual: usize, maximum: usize },

    #[error("PoR weight-activation projection commitment mismatch")]
    ProjectionCommitmentMismatch,

    #[error("unsupported PoR weight-activation record version {0}")]
    UnsupportedVersion(u16),

    #[error("PoR weight-activation record checksum mismatch")]
    ChecksumMismatch,

    #[error("PoR weight-activation writer lock is poisoned")]
    WriterLockPoisoned,
}

/// Crash-safe store for the last PoR projection accepted by Cordial.
#[derive(Debug)]
pub struct PorWeightActivationStore {
    directory: PathBuf,
    record_path: PathBuf,
    temporary_path: PathBuf,
    writer: Mutex<()>,
}

impl PorWeightActivationStore {
    pub fn open(data_dir: &Path) -> Result<Self, PorWeightActivationStoreError> {
        let directory = data_dir.join(POR_STATE_DIRECTORY);
        fs::create_dir_all(&directory)?;
        File::open(data_dir)?.sync_all()?;

        Ok(Self {
            record_path: directory.join(POR_WEIGHT_ACTIVATION_FILE_NAME),
            temporary_path: directory.join(POR_WEIGHT_ACTIVATION_TEMP_FILE_NAME),
            directory,
            writer: Mutex::new(()),
        })
    }

    pub fn record_path(&self) -> &Path {
        &self.record_path
    }

    pub fn restore(
        &self,
    ) -> Result<Option<PorWeightActivationRecord>, PorWeightActivationStoreError> {
        let file = match File::open(&self.record_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        let mut encoded = Vec::new();
        file.take((MAX_POR_WEIGHT_ACTIVATION_RECORD_LEN + 1) as u64)
            .read_to_end(&mut encoded)?;
        if encoded.len() > MAX_POR_WEIGHT_ACTIVATION_RECORD_LEN {
            return Err(PorWeightActivationStoreError::MalformedRecord);
        }

        decode_activation_record(&encoded).map(Some)
    }

    pub fn persist(
        &self,
        record: &PorWeightActivationRecord,
    ) -> Result<(), PorWeightActivationStoreError> {
        let encoded = encode_activation_record(record);
        if encoded.len() > MAX_POR_WEIGHT_ACTIVATION_RECORD_LEN {
            return Err(PorWeightActivationStoreError::RecordTooLarge {
                actual: encoded.len(),
                maximum: MAX_POR_WEIGHT_ACTIVATION_RECORD_LEN,
            });
        }
        let _writer = self
            .writer
            .lock()
            .map_err(|_| PorWeightActivationStoreError::WriterLockPoisoned)?;

        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut temporary = options.open(&self.temporary_path)?;
        temporary.write_all(&encoded)?;
        temporary.sync_all()?;
        drop(temporary);

        fs::rename(&self.temporary_path, &self.record_path)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

pub(crate) fn authorized_weight_commitment(
    weights: &HashMap<NodeId, ReputationWeight>,
) -> ReputationCommitment {
    let mut entries: Vec<_> = weights.iter().collect();
    entries.sort_by_key(|(node_id, _)| *node_id);

    let mut payload = Vec::new();
    payload.extend_from_slice(POR_WEIGHT_COMMITMENT_DOMAIN);
    put_len(&mut payload, entries.len());
    for (node_id, weight) in entries {
        put_len(&mut payload, node_id.0.len());
        payload.extend_from_slice(&node_id.0);
        payload.extend_from_slice(&weight.to_be_bytes());
    }
    Blake2b256Hasher.hash(&payload)
}

fn encode_activation_record(record: &PorWeightActivationRecord) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&record.reputation_round.to_be_bytes());
    match (record.checkpoint_hash, record.source_finalized_wave) {
        (Some(hash), Some(source_wave)) => {
            payload.push(1);
            payload.extend_from_slice(&hash);
            payload.extend_from_slice(&source_wave.to_be_bytes());
        }
        (None, None) => payload.push(0),
        _ => unreachable!("activation record checkpoint fields are constructed together"),
    }
    payload.extend_from_slice(&record.weights_commitment);
    let mut entries: Vec<_> = record.weights.iter().collect();
    entries.sort_by_key(|(node_id, _)| *node_id);
    put_len(&mut payload, entries.len());
    for (node_id, weight) in entries {
        put_len(&mut payload, node_id.0.len());
        payload.extend_from_slice(&node_id.0);
        payload.extend_from_slice(&weight.to_be_bytes());
    }

    let mut encoded = Vec::new();
    encoded.extend_from_slice(POR_WEIGHT_ACTIVATION_MAGIC);
    encoded.extend_from_slice(&POR_WEIGHT_ACTIVATION_VERSION.to_be_bytes());
    put_len(&mut encoded, payload.len());
    encoded.extend_from_slice(&payload);

    let mut checksum_payload = Vec::new();
    checksum_payload.extend_from_slice(POR_WEIGHT_ACTIVATION_CHECKSUM_DOMAIN);
    checksum_payload.extend_from_slice(&encoded);
    encoded.extend_from_slice(&Blake2b256Hasher.hash(&checksum_payload));
    encoded
}

fn decode_activation_record(
    encoded: &[u8],
) -> Result<PorWeightActivationRecord, PorWeightActivationStoreError> {
    let header_len = POR_WEIGHT_ACTIVATION_MAGIC.len() + 2 + 8;
    if encoded.len() < header_len + CHECKSUM_LEN
        || !encoded.starts_with(POR_WEIGHT_ACTIVATION_MAGIC)
    {
        return Err(PorWeightActivationStoreError::MalformedRecord);
    }

    let version_offset = POR_WEIGHT_ACTIVATION_MAGIC.len();
    let version = u16::from_be_bytes(
        encoded[version_offset..version_offset + 2]
            .try_into()
            .expect("checked activation-record header length"),
    );
    if version != POR_WEIGHT_ACTIVATION_VERSION {
        return Err(PorWeightActivationStoreError::UnsupportedVersion(version));
    }

    let payload_len_offset = version_offset + 2;
    let payload_len = u64::from_be_bytes(
        encoded[payload_len_offset..payload_len_offset + 8]
            .try_into()
            .expect("checked activation-record header length"),
    );
    let payload_len =
        usize::try_from(payload_len).map_err(|_| PorWeightActivationStoreError::MalformedRecord)?;
    let payload_offset = header_len;
    let checksum_offset = payload_offset
        .checked_add(payload_len)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    let expected_len = checksum_offset
        .checked_add(CHECKSUM_LEN)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    if encoded.len() != expected_len {
        return Err(PorWeightActivationStoreError::MalformedRecord);
    }

    let mut checksum_payload = Vec::new();
    checksum_payload.extend_from_slice(POR_WEIGHT_ACTIVATION_CHECKSUM_DOMAIN);
    checksum_payload.extend_from_slice(&encoded[..checksum_offset]);
    let expected_checksum = Blake2b256Hasher.hash(&checksum_payload);
    if encoded[checksum_offset..] != expected_checksum {
        return Err(PorWeightActivationStoreError::ChecksumMismatch);
    }

    let payload = &encoded[payload_offset..checksum_offset];
    let mut cursor = 0;
    let reputation_round = take_u64(payload, &mut cursor)?;
    let checkpoint_presence = take_byte(payload, &mut cursor)?;
    let (checkpoint_hash, source_finalized_wave) = match checkpoint_presence {
        0 => (None, None),
        1 => (
            Some(take_commitment(payload, &mut cursor)?),
            Some(take_u64(payload, &mut cursor)?),
        ),
        _ => return Err(PorWeightActivationStoreError::MalformedRecord),
    };
    let weights_commitment = take_commitment(payload, &mut cursor)?;
    let entry_count = take_len(payload, &mut cursor, MAX_REPUTATION_STATE_ENTRIES)?;
    if entry_count == 0 {
        return Err(PorWeightActivationStoreError::MalformedRecord);
    }
    let mut weights = HashMap::with_capacity(entry_count);
    let mut previous_node_id: Option<NodeId> = None;
    let mut total_weight = 0u128;
    for _ in 0..entry_count {
        let node_id_len = take_len(payload, &mut cursor, MAX_REPUTATION_STATE_NODE_ID_LEN)?;
        let node_id = NodeId(take_bytes(payload, &mut cursor, node_id_len)?.to_vec());
        if previous_node_id
            .as_ref()
            .is_some_and(|previous| previous >= &node_id)
        {
            return Err(PorWeightActivationStoreError::MalformedRecord);
        }
        let weight = take_u64(payload, &mut cursor)?;
        total_weight += u128::from(weight);
        weights.insert(node_id.clone(), weight);
        previous_node_id = Some(node_id);
    }
    if cursor != payload.len() || total_weight == 0 {
        return Err(PorWeightActivationStoreError::MalformedRecord);
    }
    if authorized_weight_commitment(&weights) != weights_commitment {
        return Err(PorWeightActivationStoreError::ProjectionCommitmentMismatch);
    }

    Ok(PorWeightActivationRecord {
        reputation_round,
        checkpoint_hash,
        source_finalized_wave,
        weights_commitment,
        weights,
    })
}

fn put_len(target: &mut Vec<u8>, len: usize) {
    let len = u64::try_from(len).expect("in-memory activation value length fits in u64");
    target.extend_from_slice(&len.to_be_bytes());
}

fn take_len(
    payload: &[u8],
    cursor: &mut usize,
    maximum: usize,
) -> Result<usize, PorWeightActivationStoreError> {
    let value = take_u64(payload, cursor)?;
    let value =
        usize::try_from(value).map_err(|_| PorWeightActivationStoreError::MalformedRecord)?;
    if value > maximum {
        return Err(PorWeightActivationStoreError::MalformedRecord);
    }
    Ok(value)
}

fn take_bytes<'a>(
    payload: &'a [u8],
    cursor: &mut usize,
    len: usize,
) -> Result<&'a [u8], PorWeightActivationStoreError> {
    let end = cursor
        .checked_add(len)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    let bytes = payload
        .get(*cursor..end)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    *cursor = end;
    Ok(bytes)
}

fn take_byte(payload: &[u8], cursor: &mut usize) -> Result<u8, PorWeightActivationStoreError> {
    let byte = payload
        .get(*cursor)
        .copied()
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    *cursor += 1;
    Ok(byte)
}

fn take_u64(payload: &[u8], cursor: &mut usize) -> Result<u64, PorWeightActivationStoreError> {
    let end = cursor
        .checked_add(8)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    let bytes = payload
        .get(*cursor..end)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    *cursor = end;
    Ok(u64::from_be_bytes(
        bytes
            .try_into()
            .expect("activation u64 slice has exact length"),
    ))
}

fn take_commitment(
    payload: &[u8],
    cursor: &mut usize,
) -> Result<ReputationCommitment, PorWeightActivationStoreError> {
    let end = cursor
        .checked_add(32)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    let bytes = payload
        .get(*cursor..end)
        .ok_or(PorWeightActivationStoreError::MalformedRecord)?;
    *cursor = end;
    Ok(bytes
        .try_into()
        .expect("activation commitment slice has exact length"))
}
