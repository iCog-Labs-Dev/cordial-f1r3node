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
use cordial_por::{MAX_REPUTATION_BLOCK_SHARD_ID_LEN, RatingSourceCheckpoint};

use super::{
    RatingEnvelopeBroadcaster,
    wire::{
        BlockProductionRatingEnvelopeV1, BlockProductionRatingEnvelopeV2,
        MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN, MAX_CHECKPOINT_RATING_ENVELOPE_LEN,
    },
};
use crate::por::ratings::{verify_checkpoint_rating_signature, verify_rating_signature};

const MAX_WAVE_ENVELOPES: usize = 8192;
const CHECKPOINT_SOURCE_MAGIC: &[u8] = b"cordial-por-exchange-source-v2";
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

    /// Publisher for one exact checkpoint source and rater.
    pub fn checkpoint_broadcaster(
        &self,
        source: RatingSourceCheckpoint,
        rater: NodeId,
    ) -> Result<FileCheckpointRatingBroadcaster> {
        source.validate()?;
        Ok(FileCheckpointRatingBroadcaster {
            directory: self.directory.clone(),
            source,
            rater,
        })
    }

    /// Read peer v2 ratings only when the round's durable source marker matches.
    pub fn read_checkpoint_peer_envelopes(
        &self,
        source: RatingSourceCheckpoint,
        local_rater: &NodeId,
        authorized: &HashSet<NodeId>,
    ) -> Result<Vec<Vec<u8>>> {
        self.read_checkpoint_peer_envelopes_for_eligible(
            source,
            local_rater,
            authorized,
            authorized,
        )
    }

    /// Membership stays intact; preceding-state ejections determine who must rate.
    pub fn read_checkpoint_peer_envelopes_for_eligible(
        &self,
        source: RatingSourceCheckpoint,
        local_rater: &NodeId,
        authorized: &HashSet<NodeId>,
        eligible: &HashSet<NodeId>,
    ) -> Result<Vec<Vec<u8>>> {
        source.validate()?;
        ensure!(
            eligible.is_subset(authorized),
            "eligible raters must be authorized"
        );
        ensure!(
            authorized.contains(local_rater),
            "local PoR exchange rater is not authorized"
        );
        let round_directory = checkpoint_round_directory(&self.directory, source.round);
        let raters = match fs::read_dir(&round_directory) {
            Ok(raters) => raters,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("reading PoR checkpoint exchange round"),
        };
        ensure!(
            read_source_marker(&round_directory)?.as_ref() == Some(&source),
            "PoR checkpoint exchange source mismatch or missing marker"
        );
        let mut received = Vec::new();
        for rater_entry in raters {
            let rater_entry = rater_entry?;
            let name = rater_entry.file_name();
            if name == "source.bin" || name.to_string_lossy().starts_with(".tmp-") {
                continue;
            }
            ensure!(
                rater_entry.file_type()?.is_dir(),
                "invalid PoR checkpoint exchange rater entry"
            );
            let rater = node_from_name(&rater_entry.file_name())?;
            ensure!(
                authorized.contains(&rater),
                "unauthorized PoR checkpoint exchange rater"
            );
            if &rater == local_rater || !eligible.contains(&rater) {
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
                    "invalid PoR checkpoint exchange envelope entry"
                );
                let recipient = recipient_from_name(&name)?;
                ensure!(
                    authorized.contains(&recipient),
                    "unauthorized PoR checkpoint exchange recipient"
                );
                ensure!(
                    received.len() < MAX_WAVE_ENVELOPES,
                    "too many PoR checkpoint exchange envelopes"
                );
                let bytes = read_optional_bounded(
                    &envelope_entry.path(),
                    MAX_CHECKPOINT_RATING_ENVELOPE_LEN,
                )?
                .context("PoR checkpoint exchange envelope disappeared")?;
                validate_checkpoint_envelope(&bytes, source, &rater, &recipient)?;
                received.push((rater.clone(), recipient, bytes));
            }
        }
        received.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        Ok(received.into_iter().map(|(_, _, bytes)| bytes).collect())
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

pub struct FileCheckpointRatingBroadcaster {
    directory: PathBuf,
    source: RatingSourceCheckpoint,
    rater: NodeId,
}

impl FileCheckpointRatingBroadcaster {
    pub fn publish(&self, encoded: &[u8]) -> Result<()> {
        let decoded = BlockProductionRatingEnvelopeV2::decode(encoded)?;
        let recipient = &decoded.rating().recipient;
        validate_checkpoint_envelope(encoded, self.source, &self.rater, recipient)?;
        let namespace = self.directory.join("checkpoint-v2");
        let round_directory = checkpoint_round_directory(&self.directory, self.source.round);
        fs::create_dir_all(&round_directory)?;
        File::open(&self.directory)?.sync_all()?;
        File::open(&namespace)?.sync_all()?;
        publish_immutable(
            &round_directory.join("source.bin"),
            &source_marker(self.source),
            128,
        )?;
        let rater_directory = round_directory.join(hex::encode(&self.rater.0));
        fs::create_dir_all(&rater_directory)?;
        File::open(&round_directory)?.sync_all()?;
        let path = rater_directory.join(format!("{}.bin", hex::encode(&recipient.0)));
        publish_immutable(&path, encoded, MAX_CHECKPOINT_RATING_ENVELOPE_LEN)
    }
}

impl RatingEnvelopeBroadcaster for FileCheckpointRatingBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.publish(envelope).map_err(|error| error.to_string())
    }
}

fn checkpoint_round_directory(directory: &Path, round: u64) -> PathBuf {
    directory.join("checkpoint-v2").join(format!("{round:020}"))
}

fn source_marker(source: RatingSourceCheckpoint) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CHECKPOINT_SOURCE_MAGIC.len() + 48);
    bytes.extend_from_slice(CHECKPOINT_SOURCE_MAGIC);
    bytes.extend_from_slice(&source.round.to_be_bytes());
    bytes.extend_from_slice(&source.finalized_prefix_len.to_be_bytes());
    bytes.extend_from_slice(&source.finalized_prefix_hash);
    bytes
}

fn read_source_marker(directory: &Path) -> Result<Option<RatingSourceCheckpoint>> {
    let Some(bytes) = read_optional_bounded(&directory.join("source.bin"), 128)? else {
        return Ok(None);
    };
    ensure!(
        bytes.len() == CHECKPOINT_SOURCE_MAGIC.len() + 48
            && bytes.starts_with(CHECKPOINT_SOURCE_MAGIC),
        "invalid PoR checkpoint exchange source marker"
    );
    let offset = CHECKPOINT_SOURCE_MAGIC.len();
    let source = RatingSourceCheckpoint {
        round: u64::from_be_bytes(bytes[offset..offset + 8].try_into()?),
        finalized_prefix_len: u64::from_be_bytes(bytes[offset + 8..offset + 16].try_into()?),
        finalized_prefix_hash: bytes[offset + 16..offset + 48].try_into()?,
    };
    source.validate()?;
    Ok(Some(source))
}

fn validate_checkpoint_envelope(
    encoded: &[u8],
    source: RatingSourceCheckpoint,
    rater: &NodeId,
    recipient: &NodeId,
) -> Result<()> {
    let decoded = BlockProductionRatingEnvelopeV2::decode(encoded)?;
    ensure!(
        decoded.source() == source,
        "PoR checkpoint exchange source mismatch"
    );
    ensure!(
        decoded.rating().rater == *rater,
        "PoR checkpoint exchange rater mismatch"
    );
    ensure!(
        decoded.rating().recipient == *recipient,
        "PoR checkpoint exchange recipient mismatch"
    );
    verify_checkpoint_rating_signature(decoded.rating(), &source)?;
    Ok(())
}

fn publish_immutable(path: &Path, bytes: &[u8], max_bytes: usize) -> Result<()> {
    if let Some(existing) = read_optional_bounded(path, max_bytes)? {
        ensure!(
            existing == bytes,
            "conflicting PoR checkpoint exchange file"
        );
        return Ok(());
    }
    let parent = path
        .parent()
        .context("PoR checkpoint exchange path has no parent")?;
    let (temporary, mut file) = loop {
        let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".tmp-{}-{nonce}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).context("creating PoR checkpoint exchange temporary file");
            }
        }
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let linked = fs::hard_link(&temporary, path);
    fs::remove_file(&temporary)?;
    match linked {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = read_optional_bounded(path, max_bytes)?
                .context("PoR checkpoint exchange file disappeared")?;
            ensure!(
                existing == bytes,
                "conflicting PoR checkpoint exchange file"
            );
        }
        Err(error) => return Err(error).context("publishing PoR checkpoint exchange file"),
    }
    File::open(parent)?.sync_all()?;
    Ok(())
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
    read_optional_bounded(path, MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN)
}

fn read_optional_bounded(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>> {
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
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= maximum, "PoR exchange envelope is too large");
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
