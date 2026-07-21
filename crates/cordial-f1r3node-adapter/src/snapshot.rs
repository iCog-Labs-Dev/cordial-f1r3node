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
//! | `last_finalized_block` | `latest_finalized_block_id(blocklace, bonds)`        |
//! | `ordered_finalized_blocks` | `weighted_tau(blocklace, bonds)` as block hashes |
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
//!
//! ## Single-validator fork awareness is shared across seams
//!
//! [`latest_finalized_block_id`] (used by `build_snapshot` for
//! `CasperSnapshot::last_finalized_block`) and
//! [`ordered_block_identities_with_cache`] (used by
//! [`ordered_finalized_output`] and [`ordered_finalized_block_hashes_with_cache`])
//! both resolve the single-validator leader/fork question through the same
//! [`single_validator_leader`] helper. This guarantees they can never
//! disagree about whether a single bonded validator has equivocated: if
//! [`single_validator_leader`] reports a fork, both `last_finalized_block`
//! and `OrderedFinalizedOutput::anchor` come back empty/`None` for the same
//! blocklace state. Each caller still independently decides how much extra
//! work to do beyond that shared decision — `latest_finalized_block_id`
//! never computes a full `weighted_tau` ordering since it only needs the
//! anchor, while `ordered_block_identities_with_cache` additionally computes
//! ordered blocks.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    OrderingCache, collect_validator_tips, compute_all_depths, fork_choice, last_round_of_wave,
    latest_weighted_final_leader, wave_of_round, weighted_tau_with_cache, xsort,
};
use cordial_miners_core::execution::{CordialBlockPayload, compute_deploys_in_scope};
use cordial_miners_core::types::{BlockIdentity, NodeId};

use crate::block_translation::{BlockMessage, Justification, TranslationError, block_to_message};
use crate::ordered_output::OrderedFinalizedOutput;

const ES_WAVELENGTH: u64 = 3;

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
    pub ordered_finalized_blocks: Vec<Vec<u8>>,
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
    PayloadDecodeFailed {
        block_hash: [u8; 32],
        reason: String,
    },

    /// Translating a tip block into a `BlockMessage` failed.
    TipTranslationFailed(TranslationError),

    /// Block number overflowed i64 when building the height index.
    BlockNumberOverflow { block_hash: [u8; 32], value: u64 },

    /// Updating a shared ordered-output reader would rewrite a previously
    /// published finalized prefix.
    OrderedOutputPrefixViolation,
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
        dag.block_number_map
            .insert(id.content_hash.to_vec(), block_number);
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

    // 5. last_finalized_block: compute via paper-native leader finality.
    if let Some(lfb_id) = latest_finalized_block_id(blocklace, bonds) {
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
    let max_block_num_u64 = payload_for
        .values()
        .map(|p| p.state.block_number)
        .max()
        .unwrap_or(0);
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
        ordered_finalized_blocks: ordered_finalized_block_hashes(blocklace, bonds),
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
    latest_finalized_block_id(blocklace, bonds)
}

fn dag_lfb(id: &Option<BlockIdentity>) -> Vec<u8> {
    id.as_ref()
        .map(|i| i.content_hash.to_vec())
        .unwrap_or_default()
}

/// Resolve the latest weighted final leader using the current ES defaults:
/// wavelength 3 and deterministic round-robin leader election over the
/// bonded validators in lexicographic `NodeId` order.
///
/// Shares [`single_validator_leader`] with [`ordered_block_identities_with_cache`]
/// for the single-validator case, so this function and
/// [`ordered_finalized_output`]'s `anchor` field can never disagree about
/// whether a lone bonded validator has equivocated.
pub(crate) fn latest_finalized_block_id(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Option<BlockIdentity> {
    let leaders = ordered_validators(bonds);

    if leaders.is_empty() {
        return None;
    }

    if leaders.len() == 1 && blocklace.checkpoint().is_none() {
        match single_validator_leader(blocklace, &leaders[0]) {
            SingleValidatorLeader::Fork => return None,
            SingleValidatorLeader::Found(leader) => return Some(leader),
            SingleValidatorLeader::Incomplete => {
                // No complete wave yet for this validator — fall through to
                // the weighted-leader lookup, which can still detect
                // finality via self-ratification.
            }
        }
    }

    latest_weighted_final_leader(blocklace, ES_WAVELENGTH, bonds, |wave| {
        let idx = usize::try_from(wave).ok()? % leaders.len();
        Some(leaders[idx].clone())
    })
}

pub(crate) fn ordered_finalized_block_hashes(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Vec<Vec<u8>> {
    ordered_finalized_block_hashes_with_cache(blocklace, bonds, &mut OrderingCache::default())
}

/// Unified dispatch: ordered finalized block identities and anchor in one pass.
///
/// Shares the "empty leaders → single-validator → multi-validator
/// weighted_tau" dispatch between the hash and [`OrderedFinalizedOutput`]
/// callers so the branch structure lives in one place.
///
/// ## Single-validator fallthrough
///
/// For a single bonded validator we first try the fast single-validator
/// path ([`single_validator_output`]). That path can come back empty for
/// two very different reasons, which must not be conflated:
///
/// - **Equivocation** (same-round fork): the validator signed two blocks in
///   the same round. This is adversarial. We must never fall through to the
///   `weighted_tau` path in this case, because that path has no fork
///   awareness and could report an anchor/blocks derived from the
///   equivocating validator's frontier anyway — silently reintroducing the
///   exact anchor/blocks inconsistency this dispatch exists to prevent.
/// - **Incomplete wave**: normal and expected — happens every round until
///   enough blocks accumulate. Safe, and even desirable, to fall through to
///   `weighted_tau`, which can detect finality via self-ratification even
///   with less than one full wave of blocks.
///
/// So the fork check is performed explicitly and short-circuits before any
/// fallthrough decision is made; only "no complete wave yet" falls through.
pub(crate) fn ordered_block_identities_with_cache(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    cache: &mut OrderingCache,
) -> (Vec<BlockIdentity>, Option<BlockIdentity>) {
    let leaders = ordered_validators(bonds);

    if leaders.is_empty() {
        return (Vec::new(), None);
    }

    if leaders.len() == 1 && blocklace.checkpoint().is_none() {
        match single_validator_leader(blocklace, &leaders[0]) {
            SingleValidatorLeader::Fork => {
                // Equivocation: this is a terminal answer, never fall
                // through. Trusting weighted_tau here would let an
                // equivocating validator's frontier drive the
                // ordering/anchor.
                return (Vec::new(), None);
            }
            SingleValidatorLeader::Found(leader) => {
                let observed = blocklace
                    .observe(&leader)
                    .into_iter()
                    .filter_map(|id| blocklace.get(&id))
                    .collect();
                let blocks = xsort(&observed).unwrap_or_default();
                return (blocks, Some(leader));
            }
            SingleValidatorLeader::Incomplete => {
                // No complete wave yet for this validator — genuinely fall
                // through to the weighted_tau path below, which can still
                // detect finality via self-ratification.
            }
        }
    }

    let leader_of_wave = |wave: u64| -> Option<NodeId> {
        let idx = usize::try_from(wave).ok()? % leaders.len();
        Some(leaders[idx].clone())
    };

    let anchor = latest_weighted_final_leader(blocklace, ES_WAVELENGTH, bonds, leader_of_wave);

    let blocks = weighted_tau_with_cache(blocklace, ES_WAVELENGTH, bonds, 0, leader_of_wave, cache)
        .unwrap_or_default();

    (blocks, anchor)
}

pub(crate) fn ordered_finalized_block_hashes_with_cache(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    cache: &mut OrderingCache,
) -> Vec<Vec<u8>> {
    let (blocks, _anchor) = ordered_block_identities_with_cache(blocklace, bonds, cache);
    blocks
        .into_iter()
        .map(|id| id.content_hash.to_vec())
        .collect()
}

/// Build an [`OrderedFinalizedOutput`] from the current blocklace state.
///
/// This is the read-only adapter seam that returns the latest finalized
/// ordered fragment from live mirrored state.
#[allow(
    dead_code,
    reason = "retained as the uncached snapshot ordering helper"
)]
pub(crate) fn ordered_finalized_output(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> OrderedFinalizedOutput {
    let (blocks, anchor) =
        ordered_block_identities_with_cache(blocklace, bonds, &mut OrderingCache::default());

    OrderedFinalizedOutput::new(
        blocks,
        anchor,
        ES_WAVELENGTH,
        bonds.len(),
        blocklace.dom().len(),
    )
}

fn ordered_validators(bonds: &HashMap<NodeId, u64>) -> Vec<NodeId> {
    let mut validators: Vec<NodeId> = bonds.keys().cloned().collect();
    validators.sort();
    validators
}

/// Outcome of resolving a single bonded validator's latest finalized leader.
///
/// Shared by [`latest_finalized_block_id`] and
/// [`ordered_block_identities_with_cache`] so both seams make the exact same
/// fork/leader decision from the exact same blocklace state — see the
/// module-level doc for why this matters.
enum SingleValidatorLeader {
    /// The validator signed two blocks in the same round (equivocation).
    /// Terminal — callers must not fall back to a fork-unaware computation.
    Fork,
    /// A complete wave was found; here is its leader.
    Found(BlockIdentity),
    /// No complete wave exists yet for this validator (not adversarial,
    /// just early). Callers may fall through to a fork-unaware fallback
    /// (e.g. `weighted_tau`'s self-ratification) in this case only.
    Incomplete,
}

/// Resolve the single-validator leader/fork outcome for `validator`.
///
/// This is the single source of truth for "has this lone bonded validator
/// equivocated, and if not, what did it finalize" — see
/// [`SingleValidatorLeader`] and the module-level doc.
fn single_validator_leader(blocklace: &Blocklace, validator: &NodeId) -> SingleValidatorLeader {
    let depths = compute_all_depths(blocklace);

    if has_same_round_fork(&depths, validator) {
        return SingleValidatorLeader::Fork;
    }

    match latest_single_validator_finalized_block_id_from_depths(&depths, validator, ES_WAVELENGTH)
    {
        Some(leader) => SingleValidatorLeader::Found(leader),
        None => SingleValidatorLeader::Incomplete,
    }
}

fn has_same_round_fork(depths: &HashMap<BlockIdentity, u64>, validator: &NodeId) -> bool {
    let mut rounds = HashMap::new();

    for (id, round) in depths {
        if &id.creator != validator {
            continue;
        }

        let count = rounds.entry(*round).or_insert(0usize);
        *count += 1;
        if *count > 1 {
            return true;
        }
    }

    false
}

/// Find the latest complete-wave leader for `validator` from pre-computed
/// depths. Returns `None` if no wave is fully observed yet for this
/// validator. Callers needing fork-awareness should go through
/// [`single_validator_leader`] rather than calling this directly.
fn latest_single_validator_finalized_block_id_from_depths(
    depths: &HashMap<BlockIdentity, u64>,
    validator: &NodeId,
    wavelength: u64,
) -> Option<BlockIdentity> {
    if wavelength == 0 {
        return None;
    }

    let max_round = depths.values().copied().max()?;
    let latest_wave = wave_of_round(max_round, wavelength)?;

    for wave in (0..=latest_wave).rev() {
        let last_round = last_round_of_wave(wave, wavelength)?;
        if last_round > max_round {
            continue;
        }

        let first_round = wave.checked_mul(wavelength)?;
        let mut leader: Option<BlockIdentity> = None;
        let mut complete_wave = true;

        for round in first_round..=last_round {
            let mut round_blocks = depths
                .iter()
                .filter(|(id, depth)| **depth == round && id.creator == *validator)
                .map(|(id, _)| id.clone());

            let Some(round_block) = round_blocks.next() else {
                complete_wave = false;
                break;
            };

            if round_blocks.next().is_some() {
                complete_wave = false;
                break;
            }

            if round == first_round {
                leader = Some(round_block);
            }
        }

        if complete_wave {
            return leader;
        }
    }

    None
}
