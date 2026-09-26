use cordial_f1r3node_adapter::por::{
    PorReputationBlockAppendOutcome, PorReputationBlockHistory, PorReputationBlockHistoryError,
};
use cordial_miners_core::NodeId;
use cordial_por::{
    PorError, REPUTATION_BLOCK_VERSION, ReputationBlock, ReputationBlockHeader, ReputationEntry,
    ReputationList, reputation_block_hash, reputation_list_commitment,
};
use tempfile::tempdir;

const SHARD_ID: &[u8] = b"root";

fn block(round: u64, previous: Option<&ReputationBlock>, shard_id: &[u8]) -> ReputationBlock {
    let reputation_list = ReputationList {
        round,
        entries: vec![
            ReputationEntry {
                node_id: NodeId(vec![1]),
                reputation: 40 + round,
                is_excluded: false,
            },
            ReputationEntry {
                node_id: NodeId(vec![2]),
                reputation: 60 + round,
                is_excluded: false,
            },
        ],
    };
    ReputationBlock {
        header: ReputationBlockHeader {
            version: REPUTATION_BLOCK_VERSION,
            shard_id: shard_id.to_vec(),
            source_finalized_wave: round - 1,
            round,
            previous_reputation_hash: previous.map(|block| reputation_block_hash(block).unwrap()),
            config_hash: [0x11; 32],
            ratings_hash: [u8::try_from(round).unwrap(); 32],
            reputation_root: reputation_list_commitment(&reputation_list).unwrap(),
        },
        reputation_list,
    }
}

#[test]
fn appends_loads_and_idempotently_recovers_canonical_history() {
    let directory = tempdir().unwrap();
    let history = PorReputationBlockHistory::open(directory.path()).unwrap();
    assert!(history.is_empty().unwrap());
    assert_eq!(history.latest().unwrap(), None);

    let first = block(1, None, SHARD_ID);
    let second = block(2, Some(&first), SHARD_ID);
    assert_eq!(
        history.append(&first).unwrap(),
        PorReputationBlockAppendOutcome::Appended
    );
    assert_eq!(
        history.append(&second).unwrap(),
        PorReputationBlockAppendOutcome::Appended
    );
    assert_eq!(
        history.append(&first).unwrap(),
        PorReputationBlockAppendOutcome::AlreadyPresent
    );
    assert_eq!(history.len().unwrap(), 2);
    assert_eq!(history.load(1).unwrap(), Some(first.clone()));
    assert_eq!(history.load(2).unwrap(), Some(second.clone()));
    assert_eq!(history.load(3).unwrap(), None);
    assert_eq!(history.latest().unwrap(), Some(second.clone()));
    assert!(history.block_path(1).is_file());

    drop(history);
    let reopened = PorReputationBlockHistory::open(directory.path()).unwrap();
    assert_eq!(reopened.len().unwrap(), 2);
    assert_eq!(reopened.latest().unwrap(), Some(second));
}

#[test]
fn append_rejects_missing_prefix_gaps_shard_changes_bad_links_and_conflicts() {
    let directory = tempdir().unwrap();
    let history = PorReputationBlockHistory::open(directory.path()).unwrap();
    let first = block(1, None, SHARD_ID);
    let linked_first = block(2, Some(&first), SHARD_ID);
    assert!(matches!(
        history.append(&linked_first),
        Err(PorReputationBlockHistoryError::FirstBlockHasPredecessor)
    ));

    history.append(&first).unwrap();

    let skipped = block(3, Some(&first), SHARD_ID);
    assert!(matches!(
        history.append(&skipped),
        Err(PorReputationBlockHistoryError::NonConsecutiveRound {
            expected: 2,
            actual: 3
        })
    ));

    let other_shard = block(2, Some(&first), b"other");
    assert!(matches!(
        history.append(&other_shard),
        Err(PorReputationBlockHistoryError::ShardMismatch)
    ));

    let mut bad_link = block(2, Some(&first), SHARD_ID);
    bad_link.header.previous_reputation_hash = Some([0x99; 32]);
    assert!(matches!(
        history.append(&bad_link),
        Err(PorReputationBlockHistoryError::PreviousHashMismatch)
    ));

    let mut conflict = first.clone();
    conflict.header.config_hash = [0x22; 32];
    assert!(matches!(
        history.append(&conflict),
        Err(PorReputationBlockHistoryError::ConflictingRound(1))
    ));
    assert_eq!(history.latest().unwrap(), Some(first));
}

#[test]
fn recovery_rejects_corruption_missing_rounds_and_file_round_mismatch() {
    let corrupt_directory = tempdir().unwrap();
    let corrupt_history = PorReputationBlockHistory::open(corrupt_directory.path()).unwrap();
    let first = block(1, None, SHARD_ID);
    corrupt_history.append(&first).unwrap();
    let path = corrupt_history.block_path(1);
    let mut bytes = std::fs::read(&path).unwrap();
    let midpoint = bytes.len() / 2;
    bytes[midpoint] ^= 1;
    std::fs::write(path, bytes).unwrap();
    drop(corrupt_history);
    assert!(matches!(
        PorReputationBlockHistory::open(corrupt_directory.path()),
        Err(PorReputationBlockHistoryError::InvalidBlock(
            PorError::ReputationBlockWireChecksumMismatch
        ))
    ));

    let gap_directory = tempdir().unwrap();
    let gap_history = PorReputationBlockHistory::open(gap_directory.path()).unwrap();
    let second = block(2, Some(&first), SHARD_ID);
    let third = block(3, Some(&second), SHARD_ID);
    gap_history.append(&first).unwrap();
    gap_history.append(&second).unwrap();
    gap_history.append(&third).unwrap();
    std::fs::remove_file(gap_history.block_path(2)).unwrap();
    drop(gap_history);
    assert!(matches!(
        PorReputationBlockHistory::open(gap_directory.path()),
        Err(PorReputationBlockHistoryError::NonConsecutiveRound {
            expected: 2,
            actual: 3
        })
    ));

    let mismatch_directory = tempdir().unwrap();
    let mismatch_history = PorReputationBlockHistory::open(mismatch_directory.path()).unwrap();
    mismatch_history.append(&first).unwrap();
    std::fs::rename(
        mismatch_history.block_path(1),
        mismatch_history.block_path(2),
    )
    .unwrap();
    drop(mismatch_history);
    assert!(matches!(
        PorReputationBlockHistory::open(mismatch_directory.path()),
        Err(PorReputationBlockHistoryError::FileRoundMismatch {
            file_round: 2,
            block_round: 1
        })
    ));
}

#[test]
fn recovery_ignores_interrupted_temporary_and_unrelated_files() {
    let directory = tempdir().unwrap();
    let history = PorReputationBlockHistory::open(directory.path()).unwrap();
    let first = block(1, None, SHARD_ID);
    history.append(&first).unwrap();

    std::fs::write(
        history.directory_path().join(".reputation-block.bin.tmp"),
        b"interrupted append",
    )
    .unwrap();
    std::fs::write(history.directory_path().join("README"), b"backup notes").unwrap();
    drop(history);

    let reopened = PorReputationBlockHistory::open(directory.path()).unwrap();
    assert_eq!(reopened.len().unwrap(), 1);
    assert_eq!(reopened.latest().unwrap(), Some(first));
}

#[test]
fn recovery_rejects_malformed_controlled_file_names() {
    let directory = tempdir().unwrap();
    let history = PorReputationBlockHistory::open(directory.path()).unwrap();
    let malformed = history.directory_path().join("reputation-block-1.bin");
    std::fs::write(&malformed, b"not canonical").unwrap();
    drop(history);

    assert!(matches!(
        PorReputationBlockHistory::open(directory.path()),
        Err(PorReputationBlockHistoryError::InvalidFileName(path)) if path == malformed
    ));
}
