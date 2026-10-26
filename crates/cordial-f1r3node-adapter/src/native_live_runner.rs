use std::collections::{BTreeSet, HashSet};

use cordial_miners_core::types::BlockIdentity;
use models::rust::casper::protocol::casper_message::BlockMessage;

use crate::grpc_ingest::BlocklaceAdapter;
use crate::live_ingress::{LiveIngress, LiveIngressError, MirrorDisposition};
use crate::native_block_source::{LocalNativeBlockSource, NativeBlockSourceError};
use crate::repository::{RSpaceBlocklaceRepository, RepoError};

#[derive(Debug, thiserror::Error)]
pub enum NativeBackfillError {
    #[error("native block has an invalid {0} hash length")]
    InvalidHash(&'static str),
    #[error("native block ancestry contains a cycle at {0}")]
    Cycle(String),
    #[error("native ancestry needs more than {0} backfilled blocks")]
    BackfillLimit(usize),
    #[error("invalid native height window {start}..={end}")]
    InvalidWindow { start: i64, end: i64 },
    #[error("native height window starts at {start}, but durable cursor requires {expected}")]
    CursorStart { start: i64, expected: i64 },
    #[error("native height window includes block outside {start}..={end}: {actual}")]
    HeightOutsideWindow { start: i64, end: i64, actual: i64 },
    #[error("native height window is missing height {0}")]
    MissingHeight(i64),
    #[error(transparent)]
    Source(#[from] NativeBlockSourceError),
    #[error(transparent)]
    Ingress(#[from] LiveIngressError),
    #[error(transparent)]
    Repository(#[from] RepoError),
}

enum Work {
    Enter(BlockMessage),
    Apply(BlockMessage),
}

pub async fn apply_native_height_window<A>(
    ingress: &mut LiveIngress<A>,
    repo: &RSpaceBlocklaceRepository,
    source: &LocalNativeBlockSource,
    blocks: Vec<BlockMessage>,
    start: i64,
    end: i64,
    max_backfill_blocks: usize,
) -> Result<usize, NativeBackfillError>
where
    A: BlocklaceAdapter<BlockIdentity>,
{
    let expected = repo.native_height_cursor()?.map_or(0, |height| height + 1);
    if start != expected {
        return Err(NativeBackfillError::CursorStart { start, expected });
    }
    let applied = replay_native_height_window(
        ingress,
        repo,
        source,
        blocks,
        start,
        end,
        max_backfill_blocks,
    )
    .await?;
    repo.put_native_height_cursor(end)?;
    Ok(applied)
}

pub async fn replay_native_height_window<A>(
    ingress: &mut LiveIngress<A>,
    repo: &RSpaceBlocklaceRepository,
    source: &LocalNativeBlockSource,
    mut blocks: Vec<BlockMessage>,
    start: i64,
    end: i64,
    max_backfill_blocks: usize,
) -> Result<usize, NativeBackfillError>
where
    A: BlocklaceAdapter<BlockIdentity>,
{
    if start < 0 || end < start || end == i64::MAX {
        return Err(NativeBackfillError::InvalidWindow { start, end });
    }
    let mut present = BTreeSet::new();
    for block in &blocks {
        let height = block.body.state.block_number;
        if !(start..=end).contains(&height) {
            return Err(NativeBackfillError::HeightOutsideWindow {
                start,
                end,
                actual: height,
            });
        }
        present.insert(height);
    }
    for height in start..=end {
        if !present.contains(&height) {
            return Err(NativeBackfillError::MissingHeight(height));
        }
    }
    blocks.sort_by(|a, b| {
        a.body
            .state
            .block_number
            .cmp(&b.body.state.block_number)
            .then_with(|| a.block_hash.cmp(&b.block_hash))
    });
    let mut applied = 0usize;
    for block in blocks {
        applied +=
            ingest_with_native_backfill(ingress, repo, source, block, max_backfill_blocks).await?;
    }
    Ok(applied)
}

pub async fn ingest_with_native_backfill<A>(
    ingress: &mut LiveIngress<A>,
    repo: &RSpaceBlocklaceRepository,
    source: &LocalNativeBlockSource,
    target: BlockMessage,
    max_backfill_blocks: usize,
) -> Result<usize, NativeBackfillError>
where
    A: BlocklaceAdapter<BlockIdentity>,
{
    let mut stack = vec![Work::Enter(target)];
    let mut ancestry = HashSet::new();
    let mut fetched = 0usize;
    let mut applied = 0usize;

    while let Some(work) = stack.pop() {
        match work {
            Work::Enter(block) => {
                let hash = hash_32(&block.block_hash, "block")?;
                if ingress.native_identity_for_wire_hash(&hash).is_some() {
                    continue;
                }
                if !ancestry.insert(hash) {
                    return Err(NativeBackfillError::Cycle(hex::encode(hash)));
                }
                let mut dependencies = Vec::new();
                for parent in &block.header.parents_hash_list {
                    dependencies.push(hash_32(parent, "parent")?);
                }
                for justification in &block.justifications {
                    dependencies.push(hash_32(&justification.latest_block_hash, "justification")?);
                }
                dependencies.sort_unstable();
                dependencies.dedup();
                stack.push(Work::Apply(block));
                for dependency in dependencies.into_iter().rev() {
                    if ingress.native_identity_for_wire_hash(&dependency).is_some() {
                        continue;
                    }
                    if ancestry.contains(&dependency) {
                        return Err(NativeBackfillError::Cycle(hex::encode(dependency)));
                    }
                    if fetched >= max_backfill_blocks {
                        return Err(NativeBackfillError::BackfillLimit(max_backfill_blocks));
                    }
                    stack.push(Work::Enter(source.read(dependency).await?));
                    fetched += 1;
                }
            }
            Work::Apply(block) => {
                let hash = hash_32(&block.block_hash, "block")?;
                let update = if block.body.state.block_number == 0 && block.sig.is_empty() {
                    ingress.ingest_trusted_unsigned_native_genesis_and_persist(&block, repo)?
                } else {
                    ingress.ingest_native_block_message_and_persist(&block, repo)?
                };
                if update.disposition == MirrorDisposition::Applied {
                    applied += 1;
                }
                ancestry.remove(&hash);
            }
        }
    }
    Ok(applied)
}

fn hash_32(bytes: &[u8], field: &'static str) -> Result<[u8; 32], NativeBackfillError> {
    bytes
        .try_into()
        .map_err(|_| NativeBackfillError::InvalidHash(field))
}
