//! Durable local staging for signed rating envelopes before peer delivery.
//!
//! A batch is written as one atomically replaced file. Repeating the same
//! batch is idempotent; a different batch for the same wave and rater fails
//! closed so retry cannot silently change already staged ratings.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail, ensure};
use cordial_miners_core::NodeId;
use cordial_por::RatingBatch;

use super::{encode_rating_batch, wire::BlockProductionRatingEnvelopeV1};
use crate::por::ratings::verify_rating_signature;

const MAGIC: &[u8] = b"cordial-por-rating-outbox-v1";
const MAX_OUTBOX_BYTES: usize = 8 * 1024 * 1024;
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// One validator's immutable signed rating batches below its own data root.
pub struct RatingOutbox {
    directory: PathBuf,
}

impl RatingOutbox {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("por").join("rating-outbox");
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating PoR rating outbox {}", directory.display()))?;
        File::open(directory.parent().context("outbox has no parent")?)?.sync_all()?;
        Ok(Self { directory })
    }

    /// Persist a complete local batch before attempting any outbound delivery.
    pub fn persist_local_batch(
        &self,
        finalized_wave: u64,
        rater: &NodeId,
        batch: &RatingBatch,
    ) -> Result<()> {
        ensure!(
            batch.ratings.iter().all(|rating| &rating.rater == rater),
            "PoR rating outbox batch contains another rater's rating"
        );
        let envelopes = encode_rating_batch(finalized_wave, batch)?;
        let bytes = encode_file(finalized_wave, rater, &envelopes)?;
        decode_file(&bytes, finalized_wave, rater)?;
        let path = self.batch_path(finalized_wave, rater);

        match read_bounded(&path)? {
            Some(existing) => {
                ensure!(existing == bytes, "conflicting PoR rating outbox batch");
                return Ok(());
            }
            None => {}
        }

        let (temporary, mut file) = loop {
            let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), nonce));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => break (temporary, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", temporary.display()));
                }
            }
        };
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        let linked = fs::hard_link(&temporary, &path);
        fs::remove_file(&temporary)?;
        match linked {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = read_bounded(&path)?.context("rating outbox batch disappeared")?;
                ensure!(existing == bytes, "conflicting PoR rating outbox batch");
            }
            Err(error) => {
                return Err(error).with_context(|| format!("publishing {}", path.display()));
            }
        }
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    /// Restore staged envelopes. Wire shape, wave, rater, and signatures are checked.
    pub fn load(&self, finalized_wave: u64, rater: &NodeId) -> Result<Option<Vec<Vec<u8>>>> {
        let path = self.batch_path(finalized_wave, rater);
        let bytes = match read_bounded(&path)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        Ok(Some(decode_file(&bytes, finalized_wave, rater)?))
    }

    fn batch_path(&self, finalized_wave: u64, rater: &NodeId) -> PathBuf {
        self.directory.join(format!(
            "{finalized_wave:020}-{}.bin",
            hex::encode(&rater.0)
        ))
    }
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("opening {}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_OUTBOX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        bytes.len() <= MAX_OUTBOX_BYTES,
        "PoR rating outbox batch is too large"
    );
    Ok(Some(bytes))
}

fn encode_file(finalized_wave: u64, rater: &NodeId, envelopes: &[Vec<u8>]) -> Result<Vec<u8>> {
    ensure!(
        matches!(rater.0.len(), 33 | 65),
        "invalid PoR rating outbox rater key length"
    );
    let rater_len = u16::try_from(rater.0.len()).context("PoR rater key is too long")?;
    let count = u32::try_from(envelopes.len()).context("too many PoR rating envelopes")?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&finalized_wave.to_be_bytes());
    bytes.extend_from_slice(&rater_len.to_be_bytes());
    bytes.extend_from_slice(&rater.0);
    bytes.extend_from_slice(&count.to_be_bytes());
    ensure!(
        bytes.len() <= MAX_OUTBOX_BYTES,
        "PoR rating outbox batch is too large"
    );
    for envelope in envelopes {
        let length = u32::try_from(envelope.len()).context("PoR rating envelope is too long")?;
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(envelope);
        ensure!(
            bytes.len() <= MAX_OUTBOX_BYTES,
            "PoR rating outbox batch is too large"
        );
    }
    Ok(bytes)
}

fn decode_file(bytes: &[u8], expected_wave: u64, expected_rater: &NodeId) -> Result<Vec<Vec<u8>>> {
    ensure!(
        bytes.len() <= MAX_OUTBOX_BYTES,
        "PoR rating outbox batch is too large"
    );
    let mut cursor = bytes;
    ensure!(
        take(&mut cursor, MAGIC.len())? == MAGIC,
        "invalid PoR rating outbox format"
    );
    let wave = u64::from_be_bytes(take(&mut cursor, 8)?.try_into()?);
    ensure!(wave == expected_wave, "PoR rating outbox wave mismatch");
    let rater_len = u16::from_be_bytes(take(&mut cursor, 2)?.try_into()?) as usize;
    ensure!(
        take(&mut cursor, rater_len)? == expected_rater.0,
        "PoR rating outbox rater mismatch"
    );
    let count = u32::from_be_bytes(take(&mut cursor, 4)?.try_into()?) as usize;
    ensure!(
        count <= cursor.len() / 4,
        "invalid PoR rating outbox envelope count"
    );
    let mut envelopes = Vec::with_capacity(count);
    for _ in 0..count {
        let length = u32::from_be_bytes(take(&mut cursor, 4)?.try_into()?) as usize;
        let envelope = take(&mut cursor, length)?.to_vec();
        let decoded = BlockProductionRatingEnvelopeV1::decode(&envelope)?;
        ensure!(
            decoded.finalized_wave() == wave,
            "PoR rating outbox envelope wave mismatch"
        );
        ensure!(
            decoded.rating().rater == *expected_rater,
            "PoR rating outbox envelope rater mismatch"
        );
        verify_rating_signature(decoded.rating())?;
        envelopes.push(envelope);
    }
    ensure!(cursor.is_empty(), "PoR rating outbox has trailing bytes");
    Ok(envelopes)
}

fn take<'a>(cursor: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    if cursor.len() < length {
        bail!("truncated PoR rating outbox batch");
    }
    let (prefix, rest) = cursor.split_at(length);
    *cursor = rest;
    Ok(prefix)
}
