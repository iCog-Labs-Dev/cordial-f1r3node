//! Immutable shared-directory exchange for signed validator rating envelopes.
//!
//! This is a concrete transport for colocated shadow observers. Each envelope
//! is published once under its wave, rater, and recipient. A partial batch is
//! visible as a partial prefix; the evidence-backed collector decides whether
//! the rater's complete deterministic batch has arrived.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, ensure};
use cordial_miners_core::NodeId;
use cordial_por::MAX_REPUTATION_BLOCK_SHARD_ID_LEN;

use super::{
    RatingEnvelopeBroadcaster,
    wire::{BlockProductionRatingEnvelopeV1, MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN},
};
use crate::por::ratings::verify_rating_signature;

const MAX_WAVE_ENVELOPES: usize = 8192;
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// A shared, shard-scoped directory containing immutable rating envelopes.
pub struct FileRatingExchange {
    directory: PathBuf,
}

impl FileRatingExchange {
    pub fn open(root: &Path, shard_id: &[u8]) -> Result<Self> {
        ensure!(
            !shard_id.is_empty() && shard_id.len() <= MAX_REPUTATION_BLOCK_SHARD_ID_LEN,
            "invalid PoR rating exchange shard ID"
        );
        let directory = root.join(hex::encode(shard_id));
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating PoR rating exchange {}", directory.display()))?;
        File::open(root)?.sync_all()?;
        Ok(Self { directory })
    }

    pub fn broadcaster(&self, finalized_wave: u64, rater: NodeId) -> FileRatingBroadcaster {
        FileRatingBroadcaster {
            directory: self.directory.clone(),
            finalized_wave,
            rater,
        }
    }

    /// Read signed envelopes from authorized peers in canonical key order.
    ///
    /// The caller still submits these bytes to the evidence-backed collector;
    /// a valid signature alone does not establish a valid interaction.
    pub fn read_peer_envelopes(
        &self,
        finalized_wave: u64,
        local_rater: &NodeId,
        authorized: &HashSet<NodeId>,
    ) -> Result<Vec<Vec<u8>>> {
        ensure!(
            authorized.contains(local_rater),
            "local PoR exchange rater is not authorized"
        );
        let wave_directory = self.directory.join(format!("{finalized_wave:020}"));
        let raters = match fs::read_dir(&wave_directory) {
            Ok(raters) => raters,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("reading PoR rating exchange wave"),
        };
        let mut received = Vec::new();
        for rater_entry in raters {
            let rater_entry = rater_entry?;
            ensure!(
                rater_entry.file_type()?.is_dir(),
                "invalid PoR exchange rater entry"
            );
            let rater = node_from_name(&rater_entry.file_name())?;
            ensure!(
                authorized.contains(&rater),
                "unauthorized PoR exchange rater"
            );
            if &rater == local_rater {
                continue;
            }
            for envelope_entry in fs::read_dir(rater_entry.path())? {
                let envelope_entry = envelope_entry?;
                let name = envelope_entry.file_name();
                if name.to_string_lossy().starts_with(".tmp-") {
                    continue;
                }
                ensure!(
                    envelope_entry.file_type()?.is_file(),
                    "invalid PoR exchange envelope entry"
                );
                let recipient = recipient_from_name(&name)?;
                ensure!(
                    authorized.contains(&recipient),
                    "unauthorized PoR exchange recipient"
                );
                ensure!(
                    received.len() < MAX_WAVE_ENVELOPES,
                    "too many PoR exchange envelopes in one wave"
                );
                let bytes = read_envelope(&envelope_entry.path())?;
                validate_envelope(&bytes, finalized_wave, &rater, &recipient)?;
                received.push((rater.clone(), recipient, bytes));
            }
        }
        received.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        Ok(received.into_iter().map(|(_, _, bytes)| bytes).collect())
    }
}

/// One validator's synchronous publisher for the transport-neutral boundary.
pub struct FileRatingBroadcaster {
    directory: PathBuf,
    finalized_wave: u64,
    rater: NodeId,
}

impl FileRatingBroadcaster {
    pub fn publish(&self, encoded: &[u8]) -> Result<()> {
        let decoded = BlockProductionRatingEnvelopeV1::decode(encoded)?;
        let recipient = &decoded.rating().recipient;
        validate_envelope(encoded, self.finalized_wave, &self.rater, recipient)?;
        let wave_directory = self.directory.join(format!("{:020}", self.finalized_wave));
        let rater_directory = wave_directory.join(hex::encode(&self.rater.0));
        fs::create_dir_all(&rater_directory)?;
        File::open(&self.directory)?.sync_all()?;
        File::open(&wave_directory)?.sync_all()?;
        let path = rater_directory.join(format!("{}.bin", hex::encode(&recipient.0)));
        if let Some(existing) = read_optional_envelope(&path)? {
            ensure!(
                existing == encoded,
                "conflicting PoR exchange rating envelope"
            );
            return Ok(());
        }

        let (temporary, mut file) = loop {
            let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let temporary = rater_directory.join(format!(".tmp-{}-{nonce}", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => break (temporary, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("creating PoR exchange temporary file"),
            }
        };
        file.write_all(encoded)?;
        file.sync_all()?;
        drop(file);
        let linked = fs::hard_link(&temporary, &path);
        fs::remove_file(&temporary)?;
        match linked {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = read_envelope(&path)?;
                ensure!(
                    existing == encoded,
                    "conflicting PoR exchange rating envelope"
                );
            }
            Err(error) => return Err(error).context("publishing PoR exchange envelope"),
        }
        File::open(&rater_directory)?.sync_all()?;
        Ok(())
    }
}

impl RatingEnvelopeBroadcaster for FileRatingBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.publish(envelope).map_err(|error| error.to_string())
    }
}

fn validate_envelope(encoded: &[u8], wave: u64, rater: &NodeId, recipient: &NodeId) -> Result<()> {
    let decoded = BlockProductionRatingEnvelopeV1::decode(encoded)?;
    ensure!(
        decoded.finalized_wave() == wave,
        "PoR exchange wave mismatch"
    );
    ensure!(
        decoded.rating().rater == *rater,
        "PoR exchange rater mismatch"
    );
    ensure!(
        decoded.rating().recipient == *recipient,
        "PoR exchange recipient mismatch"
    );
    verify_rating_signature(decoded.rating())?;
    Ok(())
}

fn read_optional_envelope(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("inspecting PoR exchange envelope"),
    };
    ensure!(
        metadata.file_type().is_file(),
        "invalid PoR exchange envelope file"
    );
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => return Err(error).context("opening PoR exchange envelope"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
        "PoR exchange envelope is too large"
    );
    Ok(Some(bytes))
}

fn read_envelope(path: &Path) -> Result<Vec<u8>> {
    read_optional_envelope(path)?.context("PoR exchange envelope disappeared")
}

fn node_from_name(name: &std::ffi::OsStr) -> Result<NodeId> {
    let bytes = hex::decode(name.to_str().context("invalid PoR exchange rater name")?)?;
    ensure!(
        matches!(bytes.len(), 33 | 65),
        "invalid PoR exchange rater key"
    );
    Ok(NodeId(bytes))
}

fn recipient_from_name(name: &std::ffi::OsStr) -> Result<NodeId> {
    let filename = name
        .to_str()
        .context("invalid PoR exchange envelope name")?;
    let key = filename
        .strip_suffix(".bin")
        .context("invalid PoR exchange envelope extension")?;
    let bytes = hex::decode(key)?;
    ensure!(
        matches!(bytes.len(), 33 | 65),
        "invalid PoR exchange recipient key"
    );
    Ok(NodeId(bytes))
}
