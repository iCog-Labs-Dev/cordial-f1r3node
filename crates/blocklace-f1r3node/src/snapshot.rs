//! `CasperSnapshot` construction (Phase 3.3).
//!
//! Builds a mirror of f1r3node's `CasperSnapshot` from the blocklace state.
//! The snapshot is the state bundle passed through f1r3node's consensus
//! operations (proposal, validation, block processing). Our job is to make
//! it reflect the current blocklace.
//!
//! ## What's mirrored vs simplified
//!
//! f1r3node's `CasperSnapshot` contains concurrent collections (`DashSet`,
//! `DashMap`, `imbl` persistent structures) and an LMDB-backed
//! `KeyValueDagRepresentation`. In the adapter we use plain
//! `HashMap`/`HashSet` and a simplified `DagRepresentation` mirror that
//! holds only the indexed fields a snapshot consumer reads. This keeps the
//! crate standalone-buildable; when the `models` / `block_storage` path
//! dependencies land, the mirrors get replaced with real types.
//!
//! ## Field sourcing
//!
//! | Snapshot field        | Source in blocklace                                   |
//! |-----------------------|-------------------------------------------------------|
//! | `dag.dag_set`         | `Blocklace::dom()`                                     |
//! | `dag.latest_messages_map` | `collect_validator_tips(blocklace, bonds)`        |
//! | `dag.child_map`       | Inverted predecessor relation                         |
//! | `dag.height_map`      | Indexed by each block's `CordialBlockPayload.state.block_number` |
//! | `dag.block_number_map`| Same source, inverse lookup                           |
//! | `last_finalized_block` | `find_last_finalized(blocklace, bonds)`              |
//! | `lca`                  | `fork_choice(blocklace, bonds).lca`                  |
//! | `tips`                 | `fork_choice(blocklace, bonds).tips`                 |
//! | `parents`              | Translated from tips via `block_to_message`           |
//! | `justifications`       | Built from each tip's `(creator, content_hash)`       |
//! | `invalid_blocks`       | Currently empty (no invalid-block tracking yet)       |
//! | `deploys_in_scope`     | `compute_deploys_in_scope()` over current tips        |
//! | `max_block_num`        | Max `block_number` across all payloads                |
//! | `max_seq_nums`         | Count of blocks per validator (sequence number stand-in)|
//! | `on_chain_state.bonds_map` | From bonds argument (passed in by caller)         |
//! | `on_chain_state.active_validators` | Keys of `bonds_map` minus equivocators    |
//!
//! ## Errors
//!
//! Construction returns [`SnapshotError`] if any block's payload fails to
//! decode. Partial state is not returned — snapshots are all-or-nothing.
//!
//! ## Known limitation: content-hash collisions
//!
//! The snapshot's `dag_set`, `block_number_map`, `height_map`, and
//! `child_map` are keyed by `content_hash: [u8; 32]`. In the blocklace,
//! `BlockIdentity` uses `(content_hash, creator, signature)` so two blocks
//! with identical content but different creators can coexist — but they
//! will collapse to one entry in the snapshot indices. This matters only
//! when two validators sign over byte-identical `BlockContent`, which is
//! rare in practice (payloads usually differ per block) but possible.
//!
//! The proper fix belongs to Phase 3.4 (crypto bridge): compute an
//! f1r3node-style `block_hash` that mixes the creator into the hash, so
//! distinct validators always produce distinct block hashes even over
//! equal content.

use std::collections::{HashMap, HashSet};

use blocklace::blocklace::Blocklace;
use blocklace::consensus::{
    collect_validator_tips, find_last_finalized, fork_choice,
};
use blocklace::execution::{compute_deploys_in_scope, CordialBlockPayload};
use blocklace::types::{BlockIdentity, NodeId};

use crate::block_translation::{block_to_message, BlockMessage, Justification, TranslationError};

/// Simplified mirror of f1r3node's `KeyValueDagRepresentation`.
///
/// Contains the indexed views that snapshot consumers read. Backed by
/// plain `HashMap` / `HashSet` since we construct the snapshot once and
/// don't need concurrent mutation.
#[derive(Debug, Clone, Default)]
pub struct DagRepresentation {
    /// All block hashes currently in the blocklace. Equivalent to f1r3node's
    /// `dag_set`.
    pub dag_set: HashSet<Vec<u8>>,

    /// Latest message per validator (excludes equivocators). Equivalent to
    /// f1r3node's `latest_messages_map`.
    pub latest_messages_map: HashMap<Vec<u8>, Vec<u8>>,

    /// Predecessor → set of direct successors. Equivalent to f1r3node's
    /// `child_map`. Built by inverting the blocklace's predecessor relation.
    pub child_map: HashMap<Vec<u8>, HashSet<Vec<u8>>>,

    /// Block number → set of hashes at that height. Equivalent to f1r3node's
    /// `height_map`. Uses `BTreeMap`-compatible ordering via i64.
    pub height_map: std::collections::BTreeMap<i64, HashSet<Vec<u8>>>,

    /// Block hash → block number. Equivalent to f1r3node's `block_number_map`.
    pub block_number_map: HashMap<Vec<u8>, i64>,

    /// Invalid block hashes. Currently always empty; validation rejects
    /// invalid blocks before they enter the blocklace, so there's nothing
    /// to track. Mapping kept for API compatibility with f1r3node.
    pub invalid_blocks_set: HashSet<Vec<u8>>,

    /// Last finalized block. Zero-length sentinel if no block is finalized yet.
    pub last_finalized_block_hash: Vec<u8>,

    /// All finalized block hashes (inclusive of last_finalized_block_hash).
    pub finalized_blocks_set: HashSet<Vec<u8>>,
}

/// Mirror of f1r3node's `CasperShardConf` subset needed by the snapshot.
/// Full `CasperShardConf` lives in [`super::shard_conf`] (Phase 3.6).
#[derive(Debug, Clone, Default)]
pub struct CasperShardConf {
    pub fault_tolerance_threshold: f32,
    pub shard_name: String,
    pub max_number_of_parents: i32,
    pub max_parent_depth: Option<i32>,
    pub deploy_lifespan: i64,
    pub min_phlo_price: i64,
}

/// Mirror of f1r3node's `OnChainCasperState`.
#[derive(Debug, Clone, Default)]
pub struct OnChainCasperState {
    pub shard_conf: CasperShardConf,
    pub bonds_map: HashMap<Vec<u8>, i64>,
    pub active_validators: Vec<Vec<u8>>,
}

/// Mirror of f1r3node's `CasperSnapshot`.
#[derive(Debug, Clone, Default)]
pub struct CasperSnapshot {
    pub dag: DagRepresentation,
    pub last_finalized_block: Vec<u8>,
    pub lca: Vec<u8>,
    pub tips: Vec<Vec<u8>>,
    pub parents: Vec<BlockMessage>,
    pub justifications: HashSet<Justification>,
    pub invalid_blocks: HashMap<Vec<u8>, Vec<u8>>,
    pub deploys_in_scope: HashSet<Vec<u8>>,
    pub max_block_num: i64,
    pub max_seq_nums: HashMap<Vec<u8>, u64>,
    pub on_chain_state: OnChainCasperState,
}

/// Errors during snapshot construction.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotError {
    /// A block's payload bytes could not be decoded as `CordialBlockPayload`.
    PayloadDecodeFailed { block_hash: [u8; 32], reason: String },

    /// Translating a tip block into a `BlockMessage` failed.
    TipTranslationFailed(TranslationError),

    /// Block number overflowed i64 when building the height index.
    BlockNumberOverflow { block_hash: [u8; 32], value: u64 },
}

/// Build a [`CasperSnapshot`] from the current blocklace state.
///
/// The `bonds` map defines which validators count and their stakes.
/// `shard_conf` provides configuration-dependent fields (fault tolerance
/// threshold, deploy lifespan) that aren't derivable from the blocklace.
///
/// Consumers typically call this once per block proposal or block
/// processing cycle. Cost is roughly linear in the blocklace size plus
/// the ancestry walk for `deploys_in_scope`.
pub fn build_snapshot(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    shard_conf: CasperShardConf,
    shard_id: &str,
) -> Result<CasperSnapshot, SnapshotError> {
    // 1. Walk all blocks, decoding payloads as we go. We'll need the
    //    block_number per block for the height index and for max_block_num.
    let all_ids: Vec<BlockIdentity> = blocklace.dom().into_iter().cloned().collect();
    let mut payload_for: HashMap<[u8; 32], CordialBlockPayload> = HashMap::new();
    for id in &all_ids {
        let content = blocklace.content(id).expect("id came from dom()");
        let payload = CordialBlockPayload::from_bytes(&content.payload).map_err(|e| {
            SnapshotError::PayloadDecodeFailed {
                block_hash: id.content_hash,
                reason: e,
            }
        })?;
        payload_for.insert(id.content_hash, payload);
    }

    // 2. Build the simplified DAG representation.
    let mut dag = DagRepresentation::default();
    for id in &all_ids {
        dag.dag_set.insert(id.content_hash.to_vec());

        let payload = payload_for.get(&id.content_hash).expect("indexed above");
        let block_number = i64::try_from(payload.state.block_number).map_err(|_| {
            SnapshotError::BlockNumberOverflow {
                block_hash: id.content_hash,
                value: payload.state.block_number,
            }
        })?;
        dag.block_number_map.insert(id.content_hash.to_vec(), block_number);
        dag.height_map
            .entry(block_number)
            .or_default()
            .insert(id.content_hash.to_vec());
    }

    // 3. child_map: invert the predecessor relation.
    for id in &all_ids {
        let content = blocklace.content(id).expect("id came from dom()");
        for pred_id in &content.predecessors {
            dag.child_map
                .entry(pred_id.content_hash.to_vec())
                .or_default()
                .insert(id.content_hash.to_vec());
        }
    }

    // 4. latest_messages_map: collect validator tips (skips equivocators).
    let validator_tips = collect_validator_tips(blocklace, bonds);
    for (node_id, tip_id) in &validator_tips {
        dag.latest_messages_map
            .insert(node_id.0.clone(), tip_id.content_hash.to_vec());
    }

    // 5. last_finalized_block: compute via finality detector.
    if let Some(lfb_id) = find_last_finalized(blocklace, bonds) {
        dag.last_finalized_block_hash = lfb_id.content_hash.to_vec();
        dag.finalized_blocks_set
            .insert(lfb_id.content_hash.to_vec());
        // All ancestors of the LFB are also finalized. Walk them.
        for anc in blocklace.ancestors_inclusive(&lfb_id) {
            dag.finalized_blocks_set
                .insert(anc.identity.content_hash.to_vec());
        }
    }
    // invalid_blocks_set stays empty — validation rejects before insertion.

    // 6. Fork choice: tips (ranked) and LCA.
    let fc = fork_choice(blocklace, bonds);
    let (tips_vec, lca_bytes): (Vec<Vec<u8>>, Vec<u8>) = match &fc {
        Some(fc) => (
            fc.tips.iter().map(|id| id.content_hash.to_vec()).collect(),
            fc.lca.content_hash.to_vec(),
        ),
        None => (vec![], vec![]),
    };

    // 7. Parents: translate each tip into a BlockMessage for f1r3node's
    //    proposer to consume. (f1r3node uses `parents` as the tip blocks
    //    themselves, not hash references, because the proposer needs the
    //    full block data for state merging.)
    let mut parents = Vec::new();
    if let Some(fc) = &fc {
        for tip_id in &fc.tips {
            let tip_block = blocklace
                .get(tip_id)
                .expect("tip came from fork_choice, must exist");
            let msg = block_to_message(&tip_block, shard_id)
                .map_err(SnapshotError::TipTranslationFailed)?;
            parents.push(msg);
        }
    }

    // 8. Justifications: (validator, latest_block_hash) for each validator tip.
    let justifications: HashSet<Justification> = validator_tips
        .iter()
        .map(|(node_id, tip_id)| Justification {
            validator: node_id.0.clone(),
            latest_block_hash: tip_id.content_hash.to_vec(),
        })
        .collect();

    // 9. deploys_in_scope: walk ancestry of current tips within the
    //    deploy lifespan window.
    let lifespan = shard_conf.deploy_lifespan.max(0) as u64;
    let max_block_num_u64 = payload_for.values().map(|p| p.state.block_number).max().unwrap_or(0);
    let tip_set: HashSet<BlockIdentity> = match &fc {
        Some(fc) => fc.tips.iter().cloned().collect(),
        None => HashSet::new(),
    };
    let deploys_in_scope = if tip_set.is_empty() {
        HashSet::new()
    } else {
        compute_deploys_in_scope(blocklace, &tip_set, max_block_num_u64, lifespan)
    };

    // 10. max_block_num and max_seq_nums.
    let max_block_num = i64::try_from(max_block_num_u64).map_err(|_| {
        // Extremely unlikely (would require 2^63 blocks in the lace), but
        // be principled about it.
        SnapshotError::BlockNumberOverflow {
            block_hash: [0u8; 32],
            value: max_block_num_u64,
        }
    })?;

    let mut max_seq_nums: HashMap<Vec<u8>, u64> = HashMap::new();
    for id in &all_ids {
        *max_seq_nums.entry(id.creator.0.clone()).or_insert(0) += 1;
    }

    // 11. on_chain_state: bonds + active validators.
    let equivocators = blocklace.find_equivacators();
    let bonds_map: HashMap<Vec<u8>, i64> = bonds
        .iter()
        .map(|(n, stake)| (n.0.clone(), *stake as i64))
        .collect();
    let active_validators: Vec<Vec<u8>> = bonds
        .keys()
        .filter(|n| !equivocators.contains(n))
        .map(|n| n.0.clone())
        .collect();

    Ok(CasperSnapshot {
        dag,
        last_finalized_block: dag_lfb(&blocklace_lfb_from(blocklace, bonds)),
        lca: lca_bytes,
        tips: tips_vec,
        parents,
        justifications,
        invalid_blocks: HashMap::new(),
        deploys_in_scope,
        max_block_num,
        max_seq_nums,
        on_chain_state: OnChainCasperState {
            shard_conf,
            bonds_map,
            active_validators,
        },
    })
}

// Small helpers to avoid repeating the LFB lookup with different return types.
fn blocklace_lfb_from(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Option<BlockIdentity> {
    find_last_finalized(blocklace, bonds)
}

fn dag_lfb(id: &Option<BlockIdentity>) -> Vec<u8> {
    id.as_ref()
        .map(|i| i.content_hash.to_vec())
        .unwrap_or_default()
}
