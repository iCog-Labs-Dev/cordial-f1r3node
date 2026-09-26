//! Crash-safe append-only storage for canonical reputation blocks.
//!
//! Each retained round is stored as one canonical `cordial-por` wire envelope.
//! Recovery decodes every retained file and verifies filename, round, shard,
//! and previous-block continuity before exposing the history tip.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use cordial_por::{
    MAX_REPUTATION_BLOCK_WIRE_LEN, PorError, ReputationBlock, decode_reputation_block,
    encode_reputation_block, reputation_block_hash,
};
use thiserror::Error;

use super::persistence::POR_STATE_DIRECTORY;

/// Directory below `<data_dir>/por` containing immutable reputation blocks.
pub const POR_REPUTATION_BLOCK_HISTORY_DIRECTORY: &str = "reputation-blocks";

const REPUTATION_BLOCK_FILE_PREFIX: &str = "reputation-block-";
const REPUTATION_BLOCK_FILE_SUFFIX: &str = ".bin";
const REPUTATION_BLOCK_ROUND_DIGITS: usize = 20;
const REPUTATION_BLOCK_TEMP_FILE_NAME: &str = ".reputation-block.bin.tmp";

/// Outcome of an idempotent history append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorReputationBlockAppendOutcome {
    Appended,
    AlreadyPresent,
}

/// Failures while recovering or appending canonical reputation-block history.
#[derive(Debug, Error)]
pub enum PorReputationBlockHistoryError {
    #[error("PoR reputation-block history I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid PoR reputation block: {0}")]
    InvalidBlock(#[from] PorError),

    #[error("invalid PoR reputation-block history file name: {0}")]
    InvalidFileName(PathBuf),

    #[error(
        "PoR reputation-block history file names round {file_round}, but the block contains round {block_round}"
    )]
    FileRoundMismatch { file_round: u64, block_round: u64 },

    #[error(
        "non-consecutive PoR reputation-block history: expected round {expected}, found {actual}"
    )]
    NonConsecutiveRound { expected: u64, actual: u64 },

    #[error("PoR reputation-block history cannot advance past round {0}")]
    RoundOverflow(u64),

    #[error("PoR reputation-block history changes shard")]
    ShardMismatch,

    #[error("PoR reputation-block history has an invalid previous-block hash")]
    PreviousHashMismatch,

    #[error("the first appended PoR reputation block references unavailable history")]
    FirstBlockHasPredecessor,

    #[error("PoR reputation-block round {0} is already committed with different contents")]
    ConflictingRound(u64),

    #[error("PoR reputation-block history writer lock is poisoned")]
    WriterLockPoisoned,

    #[error("PoR reputation-block history requires startup recovery")]
    RecoveryRequired,

    #[error(
        "PoR reputation-block history ends at round {history_round}, but durable state has no latest block"
    )]
    HistoryAheadOfState { history_round: u64 },

    #[error(
        "durable PoR state round {state_round} does not reconcile with history round {history_round}"
    )]
    StateHistoryTipMismatch {
        state_round: u64,
        history_round: u64,
    },
}

#[derive(Debug, Default)]
struct RecoveredHistory {
    tip: Option<ReputationBlock>,
    len: usize,
    recovery_required: bool,
}

/// Filesystem-backed, append-only canonical reputation-block history.
///
/// One store instance serializes appenders. Final round files are created with
/// a hard link from a fully synced temporary file, so an existing committed
/// round is never replaced. A failed operation after that link is created
/// fail-closes the instance until it is reopened and recovered.
#[derive(Debug)]
pub struct PorReputationBlockHistory {
    directory: PathBuf,
    temporary_path: PathBuf,
    recovered: Mutex<RecoveredHistory>,
}

impl PorReputationBlockHistory {
    /// Open and fully validate retained history below `data_dir`.
    pub fn open(data_dir: &Path) -> Result<Self, PorReputationBlockHistoryError> {
        let por_directory = data_dir.join(POR_STATE_DIRECTORY);
        let directory = por_directory.join(POR_REPUTATION_BLOCK_HISTORY_DIRECTORY);
        fs::create_dir_all(&directory)?;
        File::open(data_dir)?.sync_all()?;
        File::open(&por_directory)?.sync_all()?;

        let recovered = recover_history(&directory)?;
        Ok(Self {
            temporary_path: directory.join(REPUTATION_BLOCK_TEMP_FILE_NAME),
            directory,
            recovered: Mutex::new(recovered),
        })
    }

    /// Return the history directory for diagnostics and backup tooling.
    pub fn directory_path(&self) -> &Path {
        &self.directory
    }

    /// Return the canonical path for one retained round.
    pub fn block_path(&self, round: u64) -> PathBuf {
        self.directory.join(reputation_block_file_name(round))
    }

    /// Return the number of validated retained blocks.
    pub fn len(&self) -> Result<usize, PorReputationBlockHistoryError> {
        let recovered = self.lock_recovered()?;
        ensure_healthy(&recovered)?;
        Ok(recovered.len)
    }

    /// Whether this recovered history contains no blocks.
    pub fn is_empty(&self) -> Result<bool, PorReputationBlockHistoryError> {
        self.len().map(|len| len == 0)
    }

    /// Return the validated history tip.
    pub fn latest(&self) -> Result<Option<ReputationBlock>, PorReputationBlockHistoryError> {
        let recovered = self.lock_recovered()?;
        ensure_healthy(&recovered)?;
        Ok(recovered.tip.clone())
    }

    /// Read and validate one retained round.
    pub fn load(
        &self,
        round: u64,
    ) -> Result<Option<ReputationBlock>, PorReputationBlockHistoryError> {
        {
            let recovered = self.lock_recovered()?;
            ensure_healthy(&recovered)?;
        }

        let path = self.block_path(round);
        match read_block_file(&path, round) {
            Ok(block) => Ok(Some(block)),
            Err(PorReputationBlockHistoryError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Append the next canonical block, or report an idempotent retry.
    ///
    /// A new history must begin with an unlinked block. Startup reconciliation
    /// has a separate checkpoint path for upgrading a snapshot that predates
    /// this history store.
    pub fn append(
        &self,
        block: &ReputationBlock,
    ) -> Result<PorReputationBlockAppendOutcome, PorReputationBlockHistoryError> {
        self.append_inner(block, false)
    }

    /// Reconcile recovered history with the latest block in durable state.
    ///
    /// The snapshot is committed before history during live advancement. If a
    /// process stops between those writes, the snapshot can therefore be one
    /// valid block ahead and this method completes the append. An empty history
    /// may also be initialized from the latest snapshot as an upgrade
    /// checkpoint, even when older blocks are not locally available.
    pub(crate) fn reconcile_state_tip(
        &self,
        state_tip: Option<&ReputationBlock>,
    ) -> Result<(), PorReputationBlockHistoryError> {
        let history_tip = self.latest()?;
        match (history_tip.as_ref(), state_tip) {
            (None, None) => Ok(()),
            (None, Some(block)) => self.append_inner(block, true).map(|_| ()),
            (Some(history), None) => Err(PorReputationBlockHistoryError::HistoryAheadOfState {
                history_round: history.header.round,
            }),
            (Some(history), Some(state)) if history == state => Ok(()),
            (Some(history), Some(state))
                if history.header.round.checked_add(1) == Some(state.header.round) =>
            {
                self.append_inner(state, false).map(|_| ())
            }
            (Some(history), Some(state)) => {
                Err(PorReputationBlockHistoryError::StateHistoryTipMismatch {
                    state_round: state.header.round,
                    history_round: history.header.round,
                })
            }
        }
    }

    fn append_inner(
        &self,
        block: &ReputationBlock,
        allow_checkpoint: bool,
    ) -> Result<PorReputationBlockAppendOutcome, PorReputationBlockHistoryError> {
        let encoded = encode_reputation_block(block)?;
        let mut recovered = self.lock_recovered()?;
        ensure_healthy(&recovered)?;

        if let Some(tip) = &recovered.tip {
            if block.header.round <= tip.header.round {
                return self.existing_outcome(block);
            }
            validate_successor(tip, block)?;
        } else if block.header.previous_reputation_hash.is_some() && !allow_checkpoint {
            return Err(PorReputationBlockHistoryError::FirstBlockHasPredecessor);
        }

        let committed_path = self.block_path(block.header.round);
        if committed_path.try_exists()? {
            return self.existing_outcome(block);
        }

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

        fs::hard_link(&self.temporary_path, &committed_path)?;
        if let Err(error) = fs::remove_file(&self.temporary_path) {
            recovered.recovery_required = true;
            return Err(error.into());
        }
        if let Err(error) = File::open(&self.directory).and_then(|directory| directory.sync_all()) {
            recovered.recovery_required = true;
            return Err(error.into());
        }

        recovered.tip = Some(block.clone());
        recovered.len = recovered
            .len
            .checked_add(1)
            .expect("history length cannot exceed addressable files");
        Ok(PorReputationBlockAppendOutcome::Appended)
    }

    fn existing_outcome(
        &self,
        block: &ReputationBlock,
    ) -> Result<PorReputationBlockAppendOutcome, PorReputationBlockHistoryError> {
        let existing = read_block_file(&self.block_path(block.header.round), block.header.round)?;
        if existing == *block {
            Ok(PorReputationBlockAppendOutcome::AlreadyPresent)
        } else {
            Err(PorReputationBlockHistoryError::ConflictingRound(
                block.header.round,
            ))
        }
    }

    fn lock_recovered(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RecoveredHistory>, PorReputationBlockHistoryError> {
        self.recovered
            .lock()
            .map_err(|_| PorReputationBlockHistoryError::WriterLockPoisoned)
    }
}

fn recover_history(directory: &Path) -> Result<RecoveredHistory, PorReputationBlockHistoryError> {
    let mut retained = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if name == REPUTATION_BLOCK_TEMP_FILE_NAME {
            continue;
        }

        let Some(round) = parse_reputation_block_file_name(name) else {
            if name.starts_with(REPUTATION_BLOCK_FILE_PREFIX) {
                return Err(PorReputationBlockHistoryError::InvalidFileName(path));
            }
            continue;
        };
        if !entry.file_type()?.is_file() || retained.insert(round, path.clone()).is_some() {
            return Err(PorReputationBlockHistoryError::InvalidFileName(path));
        }
    }

    let mut recovered = RecoveredHistory::default();
    for (round, path) in retained {
        let block = read_block_file(&path, round)?;
        if let Some(previous) = &recovered.tip {
            validate_successor(previous, &block)?;
        }
        recovered.tip = Some(block);
        recovered.len += 1;
    }
    Ok(recovered)
}

fn read_block_file(
    path: &Path,
    file_round: u64,
) -> Result<ReputationBlock, PorReputationBlockHistoryError> {
    let file = File::open(path)?;
    let read_limit = u64::try_from(MAX_REPUTATION_BLOCK_WIRE_LEN)
        .expect("reputation-block wire limit fits in u64")
        + 1;
    let mut encoded = Vec::new();
    file.take(read_limit).read_to_end(&mut encoded)?;
    if encoded.len() > MAX_REPUTATION_BLOCK_WIRE_LEN {
        return Err(PorError::ReputationBlockWireTooLarge.into());
    }

    let block = decode_reputation_block(&encoded)?;
    if block.header.round != file_round {
        return Err(PorReputationBlockHistoryError::FileRoundMismatch {
            file_round,
            block_round: block.header.round,
        });
    }
    Ok(block)
}

fn validate_successor(
    previous: &ReputationBlock,
    next: &ReputationBlock,
) -> Result<(), PorReputationBlockHistoryError> {
    let expected_round = previous.header.round.checked_add(1).ok_or(
        PorReputationBlockHistoryError::RoundOverflow(previous.header.round),
    )?;
    if next.header.round != expected_round {
        return Err(PorReputationBlockHistoryError::NonConsecutiveRound {
            expected: expected_round,
            actual: next.header.round,
        });
    }
    if next.header.shard_id != previous.header.shard_id {
        return Err(PorReputationBlockHistoryError::ShardMismatch);
    }
    if next.header.previous_reputation_hash != Some(reputation_block_hash(previous)?) {
        return Err(PorReputationBlockHistoryError::PreviousHashMismatch);
    }
    Ok(())
}

fn ensure_healthy(recovered: &RecoveredHistory) -> Result<(), PorReputationBlockHistoryError> {
    if recovered.recovery_required {
        Err(PorReputationBlockHistoryError::RecoveryRequired)
    } else {
        Ok(())
    }
}

fn reputation_block_file_name(round: u64) -> String {
    format!("{REPUTATION_BLOCK_FILE_PREFIX}{round:020}{REPUTATION_BLOCK_FILE_SUFFIX}")
}

fn parse_reputation_block_file_name(name: &str) -> Option<u64> {
    let digits = name
        .strip_prefix(REPUTATION_BLOCK_FILE_PREFIX)?
        .strip_suffix(REPUTATION_BLOCK_FILE_SUFFIX)?;
    if digits.len() != REPUTATION_BLOCK_ROUND_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}
