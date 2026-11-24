//! Durable checkpoint state and atomic markers for RAM-only block removal.

use std::collections::BTreeSet;

use cordial_miners_core::{BlockIdentity, CheckpointOrigin};
use serde::{Deserialize, Serialize};

use super::RSpaceBlocklaceRepository;
use crate::error::RepoError;

pub const CHECKPOINT_RECORD_KEY: &[u8] = b"checkpoint_record";
pub const PRUNED_PREFIX: &[u8] = b"pruned/";
pub const CHECKPOINT_RECORD_VERSION: u32 = 1;

/// Trusted checkpoint state; archived block bodies are never deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub version: u32,
    pub checkpoint: BlockIdentity,
    pub origin: CheckpointOrigin,
    pub depth: u64,
    pub order_prefix: Vec<BlockIdentity>,
    pub weighted_order_prefix: Vec<BlockIdentity>,
}

fn validate_version(record: &CheckpointRecord) -> Result<(), RepoError> {
    if record.version != CHECKPOINT_RECORD_VERSION {
        return Err(RepoError::Checkpoint(format!(
            "unsupported version {}",
            record.version
        )));
    }
    Ok(())
}

fn decode(bytes: &[u8]) -> Result<CheckpointRecord, RepoError> {
    let record = bincode::deserialize(bytes)
        .map_err(|error| RepoError::Checkpoint(format!("corrupt stored record: {error}")))?;
    validate_version(&record)?;
    Ok(record)
}

pub(crate) fn pruned_key(encoded_identity: &[u8]) -> Vec<u8> {
    let mut key = PRUNED_PREFIX.to_vec();
    key.extend_from_slice(encoded_identity);
    key
}

impl RSpaceBlocklaceRepository {
    /// Commit a monotonic checkpoint and all removed identities together.
    /// Aborted writes leave the previous record and markers intact.
    pub fn put_checkpoint(
        &self,
        record: &CheckpointRecord,
        removed: &BTreeSet<BlockIdentity>,
    ) -> Result<(), RepoError> {
        validate_version(record)?;
        let value = bincode::serialize(record)?;
        let meta = self.meta_db.lock()?;
        let mut txn = self.env.write_txn()?;
        if let Some(bytes) = meta.get(&txn, CHECKPOINT_RECORD_KEY)? {
            let previous = decode(bytes)?;
            if record.depth < previous.depth {
                return Err(RepoError::Checkpoint("depth cannot move backward".into()));
            }
            if record.depth == previous.depth && record.checkpoint != previous.checkpoint {
                return Err(RepoError::Checkpoint(
                    "same depth has a different checkpoint identity".into(),
                ));
            }
        }
        meta.put(&mut txn, CHECKPOINT_RECORD_KEY, &value)?;
        for id in removed {
            let key = pruned_key(&bincode::serialize(id)?);
            meta.put(&mut txn, &key, &[])?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn checkpoint_record(&self) -> Result<Option<CheckpointRecord>, RepoError> {
        let meta = self.meta_db.lock()?;
        let txn = self.env.read_txn()?;
        meta.get(&txn, CHECKPOINT_RECORD_KEY)?
            .map(decode)
            .transpose()
    }

    pub fn is_pruned(&self, id: &BlockIdentity) -> Result<bool, RepoError> {
        let key = pruned_key(&bincode::serialize(id)?);
        let meta = self.meta_db.lock()?;
        let txn = self.env.read_txn()?;
        Ok(meta.get(&txn, &key)?.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cordial_miners_core::NodeId;

    fn id(tag: u8) -> BlockIdentity {
        BlockIdentity {
            content_hash: [tag; 32],
            creator: NodeId(vec![1]),
            signature: vec![1],
        }
    }

    fn record() -> CheckpointRecord {
        CheckpointRecord {
            version: CHECKPOINT_RECORD_VERSION,
            checkpoint: id(3),
            origin: CheckpointOrigin::WeightedFinal,
            depth: 3,
            order_prefix: vec![],
            weighted_order_prefix: vec![id(1), id(2), id(3)],
        }
    }

    #[test]
    fn atomic_round_trip_and_monotonic_marker_accumulation() {
        let dir = tempfile::tempdir().unwrap();
        let repo = RSpaceBlocklaceRepository::open(dir.path(), 10 * 1024 * 1024).unwrap();
        let record = record();
        assert_eq!(repo.checkpoint_record().unwrap(), None);
        assert!(!repo.is_pruned(&id(1)).unwrap());
        repo.put_checkpoint(&record, &BTreeSet::from([id(1)]))
            .unwrap();
        // Read the record and marker in one snapshot of their write transaction.
        {
            let meta = repo.meta_db.lock().unwrap();
            let txn = repo.env.read_txn().unwrap();
            assert_eq!(
                decode(meta.get(&txn, CHECKPOINT_RECORD_KEY).unwrap().unwrap()).unwrap(),
                record
            );
            assert_eq!(
                meta.get(&txn, &pruned_key(&bincode::serialize(&id(1)).unwrap()))
                    .unwrap(),
                Some(&[][..])
            );
        }
        for rejected in [
            CheckpointRecord {
                depth: 2,
                ..record.clone()
            },
            CheckpointRecord {
                checkpoint: id(4),
                ..record.clone()
            },
        ] {
            assert!(matches!(
                repo.put_checkpoint(&rejected, &BTreeSet::from([id(2)])),
                Err(RepoError::Checkpoint(_))
            ));
            assert_eq!(repo.checkpoint_record().unwrap(), Some(record.clone()));
            assert!(!repo.is_pruned(&id(2)).unwrap());
        }
        repo.put_checkpoint(&record, &BTreeSet::from([id(2)]))
            .unwrap();
        assert!(repo.is_pruned(&id(1)).unwrap());
        assert!(repo.is_pruned(&id(2)).unwrap());
        let advanced = CheckpointRecord {
            depth: 6,
            checkpoint: id(6),
            ..record
        };
        repo.put_checkpoint(&advanced, &BTreeSet::new()).unwrap();
        assert_eq!(repo.checkpoint_record().unwrap(), Some(advanced));
        repo.close();
    }

    #[test]
    fn corrupt_and_unknown_version_records_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let repo = RSpaceBlocklaceRepository::open(dir.path(), 10 * 1024 * 1024).unwrap();
        let unknown = CheckpointRecord {
            version: 2,
            ..record()
        };
        assert!(matches!(
            repo.put_checkpoint(&unknown, &BTreeSet::new()),
            Err(RepoError::Checkpoint(_))
        ));
        assert_eq!(repo.checkpoint_record().unwrap(), None);
        for bytes in [vec![255; 4], bincode::serialize(&unknown).unwrap()] {
            {
                let meta = repo.meta_db.lock().unwrap();
                let mut txn = repo.env.write_txn().unwrap();
                meta.put(&mut txn, CHECKPOINT_RECORD_KEY, &bytes).unwrap();
                txn.commit().unwrap();
            }
            assert!(matches!(
                repo.checkpoint_record(),
                Err(RepoError::Checkpoint(_))
            ));
            assert!(matches!(
                repo.put_checkpoint(&record(), &BTreeSet::from([id(1)])),
                Err(RepoError::Checkpoint(_))
            ));
            assert!(!repo.is_pruned(&id(1)).unwrap());
        }
        repo.close();
    }
}
