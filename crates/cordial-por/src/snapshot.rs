//! Versioned durable encoding for finalized Proof-of-Reputation state.
//!
//! This module owns bytes and validation, not filesystem I/O. The v1 snapshot
//! stores the current reputation list, permanent ejection registry, and latest
//! audited block in one checksummed envelope. Pending ratings are deliberately
//! rejected because they have not crossed the finalized-state boundary.

use std::collections::BTreeSet;

use cordial_miners_core::{
    NodeId,
    crypto::{Blake2b256Hasher, Hasher},
};

use crate::{
    block::{
        MAX_REPUTATION_BLOCK_ENTRIES, MAX_REPUTATION_BLOCK_NODE_ID_LEN,
        decode_reputation_block_payload, encode_reputation_block_payload,
    },
    commitments::validate_reputation_entries,
    error::PorError,
    state::ReputationState,
    types::{ReputationEntry, ReputationList},
};

/// Fixed prefix identifying a durable PoR state snapshot.
pub const POR_STATE_SNAPSHOT_MAGIC: &[u8; 17] = b"cordial-por-state";

/// Durable PoR state format emitted by this crate.
pub const POR_STATE_SNAPSHOT_VERSION: u16 = 1;

/// Maximum accepted encoded snapshot size (64 MiB).
pub const MAX_REPUTATION_STATE_SNAPSHOT_LEN: usize = 64 * 1024 * 1024;

/// Maximum number of reputation or exclusion entries in one snapshot.
pub const MAX_REPUTATION_STATE_ENTRIES: usize = MAX_REPUTATION_BLOCK_ENTRIES;

/// Allocation bound for a persisted node identifier.
pub const MAX_REPUTATION_STATE_NODE_ID_LEN: usize = MAX_REPUTATION_BLOCK_NODE_ID_LEN;

const SNAPSHOT_CHECKSUM_DOMAIN: &[u8] = b"cordial-por:state-snapshot:v1";
const CHECKSUM_LEN: usize = 32;
const FIXED_ENVELOPE_LEN: usize = POR_STATE_SNAPSHOT_MAGIC.len() + 2 + 8 + CHECKSUM_LEN;

/// Encode a finalized reputation state into the canonical v1 snapshot format.
pub fn encode_reputation_state_snapshot(state: &ReputationState) -> Result<Vec<u8>, PorError> {
    state.validate_snapshot_invariants()?;

    let mut payload = Vec::new();
    put_u64(&mut payload, state.round());
    encode_reputation_list(&mut payload, state.reputation_list())?;

    put_count(&mut payload, state.excluded_keys().len())?;
    for node_id in state.excluded_keys() {
        put_node_id(&mut payload, node_id)?;
    }

    match state.latest_block() {
        Some(block) => {
            payload.push(1);
            let encoded =
                encode_reputation_block_payload(block).map_err(map_reputation_block_wire_error)?;
            let next_len = payload
                .len()
                .checked_add(encoded.len())
                .ok_or(PorError::ReputationStateSnapshotTooLarge)?;
            if next_len > MAX_REPUTATION_STATE_SNAPSHOT_LEN - FIXED_ENVELOPE_LEN {
                return Err(PorError::ReputationStateSnapshotTooLarge);
            }
            payload.extend_from_slice(&encoded);
        }
        None => payload.push(0),
    }

    let total_len = FIXED_ENVELOPE_LEN
        .checked_add(payload.len())
        .ok_or(PorError::ReputationStateSnapshotTooLarge)?;
    if total_len > MAX_REPUTATION_STATE_SNAPSHOT_LEN {
        return Err(PorError::ReputationStateSnapshotTooLarge);
    }

    let payload_len =
        u64::try_from(payload.len()).map_err(|_| PorError::ReputationStateSnapshotTooLarge)?;
    let checksum = snapshot_checksum(POR_STATE_SNAPSHOT_VERSION, payload_len, &payload);
    let mut encoded = Vec::with_capacity(total_len);
    encoded.extend_from_slice(POR_STATE_SNAPSHOT_MAGIC);
    encoded.extend_from_slice(&POR_STATE_SNAPSHOT_VERSION.to_be_bytes());
    encoded.extend_from_slice(&payload_len.to_be_bytes());
    encoded.extend_from_slice(&payload);
    encoded.extend_from_slice(&checksum);
    Ok(encoded)
}

/// Decode and fully validate a canonical v1 reputation state snapshot.
pub fn decode_reputation_state_snapshot(bytes: &[u8]) -> Result<ReputationState, PorError> {
    if bytes.len() > MAX_REPUTATION_STATE_SNAPSHOT_LEN {
        return Err(PorError::ReputationStateSnapshotTooLarge);
    }
    if bytes.len() < FIXED_ENVELOPE_LEN {
        return Err(PorError::MalformedReputationStateSnapshot);
    }

    let mut envelope = Decoder::new(bytes);
    if envelope.read_exact(POR_STATE_SNAPSHOT_MAGIC.len())? != POR_STATE_SNAPSHOT_MAGIC {
        return Err(PorError::MalformedReputationStateSnapshot);
    }
    let version = envelope.read_u16()?;
    if version != POR_STATE_SNAPSHOT_VERSION {
        return Err(PorError::UnsupportedReputationStateSnapshotVersion(version));
    }
    let payload_len_u64 = envelope.read_u64()?;
    let payload_len =
        usize::try_from(payload_len_u64).map_err(|_| PorError::ReputationStateSnapshotTooLarge)?;
    if payload_len > MAX_REPUTATION_STATE_SNAPSHOT_LEN - FIXED_ENVELOPE_LEN
        || envelope.remaining() != payload_len + CHECKSUM_LEN
    {
        return Err(PorError::MalformedReputationStateSnapshot);
    }

    let payload = envelope.read_exact(payload_len)?;
    let checksum = envelope.read_array::<CHECKSUM_LEN>()?;
    let expected = snapshot_checksum(version, payload_len_u64, payload);
    if checksum != expected {
        return Err(PorError::ReputationStateSnapshotChecksumMismatch);
    }

    let mut decoder = Decoder::new(payload);
    let current_round = decoder.read_u64()?;
    let reputation_list = decode_reputation_list(&mut decoder)?;

    let excluded_count = decoder.read_count()?;
    let mut excluded_keys = BTreeSet::new();
    let mut previous_excluded: Option<NodeId> = None;
    for _ in 0..excluded_count {
        let node_id = decoder.read_node_id()?;
        if previous_excluded
            .as_ref()
            .is_some_and(|previous| previous >= &node_id)
        {
            return Err(PorError::ReputationStateSnapshotExclusionMismatch);
        }
        previous_excluded = Some(node_id.clone());
        excluded_keys.insert(node_id);
    }

    let latest_block = match decoder.read_discriminant()? {
        false => None,
        true => {
            let remaining = decoder.remaining();
            let encoded = decoder.read_exact(remaining)?;
            Some(
                decode_reputation_block_payload(encoded)
                    .map_err(map_reputation_block_wire_error)?,
            )
        }
    };
    if decoder.remaining() != 0 {
        return Err(PorError::MalformedReputationStateSnapshot);
    }

    ReputationState::from_snapshot_parts(
        current_round,
        reputation_list,
        excluded_keys,
        latest_block,
    )
}

fn map_reputation_block_wire_error(error: PorError) -> PorError {
    match error {
        PorError::ReputationBlockWireTooLarge => PorError::ReputationStateSnapshotTooLarge,
        PorError::MalformedReputationBlockWire
        | PorError::UnsupportedReputationBlockWireVersion(_)
        | PorError::ReputationBlockWireChecksumMismatch => {
            PorError::MalformedReputationStateSnapshot
        }
        other => other,
    }
}

fn encode_reputation_list(output: &mut Vec<u8>, list: &ReputationList) -> Result<(), PorError> {
    validate_reputation_entries(&list.entries)?;
    put_u64(output, list.round);
    put_count(output, list.entries.len())?;
    for entry in &list.entries {
        put_node_id(output, &entry.node_id)?;
        put_u64(output, entry.reputation);
        output.push(u8::from(entry.is_excluded));
    }
    Ok(())
}

fn decode_reputation_list(decoder: &mut Decoder<'_>) -> Result<ReputationList, PorError> {
    let round = decoder.read_u64()?;
    let count = decoder.read_count()?;
    let mut entries = Vec::with_capacity(count);
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
    if node_id.0.len() > MAX_REPUTATION_STATE_NODE_ID_LEN {
        return Err(PorError::ReputationStateSnapshotTooLarge);
    }
    put_bytes(output, &node_id.0)
}

fn put_count(output: &mut Vec<u8>, count: usize) -> Result<(), PorError> {
    if count > MAX_REPUTATION_STATE_ENTRIES {
        return Err(PorError::ReputationStateSnapshotTooLarge);
    }
    let count = u64::try_from(count).map_err(|_| PorError::ReputationStateSnapshotTooLarge)?;
    put_u64(output, count);
    Ok(())
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PorError> {
    let len = u64::try_from(bytes.len()).map_err(|_| PorError::ReputationStateSnapshotTooLarge)?;
    put_u64(output, len);
    output.extend_from_slice(bytes);
    Ok(())
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn snapshot_checksum(version: u16, payload_len: u64, payload: &[u8]) -> [u8; 32] {
    let mut checksum_input = Vec::with_capacity(
        SNAPSHOT_CHECKSUM_DOMAIN.len() + POR_STATE_SNAPSHOT_MAGIC.len() + 2 + 8 + payload.len(),
    );
    checksum_input.extend_from_slice(SNAPSHOT_CHECKSUM_DOMAIN);
    checksum_input.extend_from_slice(POR_STATE_SNAPSHOT_MAGIC);
    checksum_input.extend_from_slice(&version.to_be_bytes());
    checksum_input.extend_from_slice(&payload_len.to_be_bytes());
    checksum_input.extend_from_slice(payload);
    Blake2b256Hasher.hash(&checksum_input)
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
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
            .ok_or(PorError::MalformedReputationStateSnapshot)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(PorError::MalformedReputationStateSnapshot)?;
        self.offset = end;
        Ok(value)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], PorError> {
        self.read_exact(N)?
            .try_into()
            .map_err(|_| PorError::MalformedReputationStateSnapshot)
    }

    fn read_u16(&mut self) -> Result<u16, PorError> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    fn read_u64(&mut self) -> Result<u64, PorError> {
        Ok(u64::from_be_bytes(self.read_array()?))
    }

    fn read_count(&mut self) -> Result<usize, PorError> {
        let count = usize::try_from(self.read_u64()?)
            .map_err(|_| PorError::ReputationStateSnapshotTooLarge)?;
        if count > MAX_REPUTATION_STATE_ENTRIES {
            return Err(PorError::ReputationStateSnapshotTooLarge);
        }
        Ok(count)
    }

    fn read_bytes(&mut self, max_len: usize) -> Result<Vec<u8>, PorError> {
        let len = usize::try_from(self.read_u64()?)
            .map_err(|_| PorError::ReputationStateSnapshotTooLarge)?;
        if len > max_len {
            return Err(PorError::ReputationStateSnapshotTooLarge);
        }
        Ok(self.read_exact(len)?.to_vec())
    }

    fn read_node_id(&mut self) -> Result<NodeId, PorError> {
        Ok(NodeId(self.read_bytes(MAX_REPUTATION_STATE_NODE_ID_LEN)?))
    }

    fn read_discriminant(&mut self) -> Result<bool, PorError> {
        match self.read_exact(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(PorError::MalformedReputationStateSnapshot),
        }
    }
}
