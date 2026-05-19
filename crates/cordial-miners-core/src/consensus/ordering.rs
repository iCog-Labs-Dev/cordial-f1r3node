use std::collections::{BTreeSet, HashMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::approval::approves;
use crate::consensus::cordiality::{ratifies, weighted_ratifies};
use crate::consensus::finality::{
    final_leader_for_wave, latest_final_leader, latest_weighted_final_leader,
    weighted_final_leader_for_wave,
};
use crate::consensus::round::depth;
use crate::consensus::wave::wave_of_round;
use crate::types::{BlockIdentity, NodeId};

#[derive(Debug, Clone, Default)]
pub struct OrderingCache {
    generation: usize,
    approved_blocks_by_leader: HashMap<BlockIdentity, HashSet<Block>>,
    sorted_approved_by_leader: HashMap<BlockIdentity, Result<Vec<BlockIdentity>, OrderingError>>,
    previous_final_by_leader: HashMap<PreviousLeaderCacheKey, Option<BlockIdentity>>,
    weighted_previous_final_by_leader: HashMap<WeightedPreviousLeaderCacheKey, Option<BlockIdentity>>,
    tau_output_by_latest_leader: HashMap<TauOutputCacheKey, Result<Vec<BlockIdentity>, OrderingError>>,
    weighted_tau_output_by_latest_leader:
        HashMap<WeightedTauOutputCacheKey, Result<Vec<BlockIdentity>, OrderingError>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PreviousLeaderCacheKey {
    current_leader: BlockIdentity,
    wavelength: u64,
    n: usize,
    f: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WeightedPreviousLeaderCacheKey {
    current_leader: BlockIdentity,
    wavelength: u64,
    bonds_fingerprint: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TauOutputCacheKey {
    latest_leader: BlockIdentity,
    wavelength: u64,
    n: usize,
    f: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WeightedTauOutputCacheKey {
    latest_leader: BlockIdentity,
    wavelength: u64,
    bonds_fingerprint: u64,
}

struct TauState {
    emitted: BTreeSet<BlockIdentity>,
    ordered: Vec<BlockIdentity>,
}

struct TauConfig<F>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
}

struct WeightedTauConfig<'a, F>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    wavelength: u64,
    bonds: &'a HashMap<NodeId, u64>,
    leader_selection: F,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderingError {
    CycleDetected,
}

pub fn approved_blocks_for_leader(blocklace: &Blocklace, leader: &BlockIdentity) -> HashSet<Block> {
    if blocklace.get(leader).is_none() {
        return HashSet::new();
    }

    blocklace
        .dom()
        .into_iter()
        .filter_map(|id| blocklace.get(id))
        .filter(|block| approves(blocklace, leader, &block.identity))
        .collect()
}

/// Return a deterministic topological order of `blocks`.
///
/// The order respects predecessor edges within the supplied block set. When
/// multiple blocks are ready at the same time, ties are broken by the natural
/// ordering of `BlockIdentity`, yielding a stable result across nodes.
///
/// Returns [`OrderingError::CycleDetected`] if the supplied subset contains a
/// cycle, instead of silently returning a partial order.
pub fn xsort(blocks: &HashSet<Block>) -> Result<Vec<BlockIdentity>, OrderingError> {
    let block_ids: HashSet<BlockIdentity> =
        blocks.iter().map(|block| block.identity.clone()).collect();
    let mut dependents: HashMap<BlockIdentity, Vec<BlockIdentity>> = HashMap::new();
    let mut indegree: HashMap<BlockIdentity, usize> = HashMap::new();

    for block in blocks {
        let id = block.identity.clone();
        indegree.entry(id.clone()).or_insert(0);

        for predecessor in &block.content.predecessors {
            if !block_ids.contains(predecessor) {
                continue;
            }

            dependents
                .entry(predecessor.clone())
                .or_default()
                .push(id.clone());
            *indegree.entry(id.clone()).or_insert(0) += 1;
        }
    }

    let mut ready: BTreeSet<BlockIdentity> = indegree
        .iter()
        .filter_map(|(id, degree)| if *degree == 0 { Some(id.clone()) } else { None })
        .collect();
    let mut ordered = Vec::with_capacity(blocks.len());

    while let Some(next) = ready.pop_first() {
        ordered.push(next.clone());

        if let Some(children) = dependents.get(&next) {
            for child in children {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
    }

    if ordered.len() != blocks.len() {
        return Err(OrderingError::CycleDetected);
    }

    Ok(ordered)
}

fn sync_cache_generation(blocklace: &Blocklace, cache: &mut OrderingCache) {
    let generation = blocklace.dom().len();
    if cache.generation != generation {
        cache.generation = generation;
        cache.approved_blocks_by_leader.clear();
        cache.sorted_approved_by_leader.clear();
        cache.previous_final_by_leader.clear();
        cache.weighted_previous_final_by_leader.clear();
        cache.tau_output_by_latest_leader.clear();
        cache.weighted_tau_output_by_latest_leader.clear();
    }
}

fn bonds_fingerprint(bonds: &HashMap<NodeId, u64>) -> u64 {
    let mut entries: Vec<_> = bonds.iter().collect();
    entries.sort_by_key(|(left, _)| *left);

    let mut hasher = DefaultHasher::new();
    for (node, weight) in entries {
        node.hash(&mut hasher);
        weight.hash(&mut hasher);
    }
    hasher.finish()
}

fn approved_blocks_for_leader_cached(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    cache: &mut OrderingCache,
) -> HashSet<Block> {
    sync_cache_generation(blocklace, cache);

    if let Some(blocks) = cache.approved_blocks_by_leader.get(leader) {
        return blocks.clone();
    }

    let blocks = approved_blocks_for_leader(blocklace, leader);
    cache
        .approved_blocks_by_leader
        .insert(leader.clone(), blocks.clone());
    blocks
}

fn sorted_approved_fragment(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    cache: &mut OrderingCache,
) -> Result<Vec<BlockIdentity>, OrderingError> {
    sync_cache_generation(blocklace, cache);

    if let Some(sorted) = cache.sorted_approved_by_leader.get(leader) {
        return sorted.clone();
    }

    let approved = approved_blocks_for_leader_cached(blocklace, leader, cache);
    let sorted = xsort(&approved);
    cache
        .sorted_approved_by_leader
        .insert(leader.clone(), sorted.clone());
    sorted
}

fn previous_final_leader_cached<F>(
    blocklace: &Blocklace,
    current_leader: &BlockIdentity,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
    cache: &mut OrderingCache,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    sync_cache_generation(blocklace, cache);

    let key = PreviousLeaderCacheKey {
        current_leader: current_leader.clone(),
        wavelength,
        n,
        f,
    };

    if let Some(previous) = cache.previous_final_by_leader.get(&key) {
        return previous.clone();
    }

    let previous =
        previous_final_leader(blocklace, current_leader, wavelength, n, f, leader_selection);
    cache.previous_final_by_leader.insert(key, previous.clone());
    previous
}

fn weighted_previous_final_leader_cached<F>(
    blocklace: &Blocklace,
    current_leader: &BlockIdentity,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
    cache: &mut OrderingCache,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    sync_cache_generation(blocklace, cache);

    let key = WeightedPreviousLeaderCacheKey {
        current_leader: current_leader.clone(),
        wavelength,
        bonds_fingerprint: bonds_fingerprint(bonds),
    };

    if let Some(previous) = cache.weighted_previous_final_by_leader.get(&key) {
        return previous.clone();
    }

    let previous =
        weighted_previous_final_leader(blocklace, current_leader, wavelength, bonds, leader_selection);
    cache
        .weighted_previous_final_by_leader
        .insert(key, previous.clone());
    previous
}

/// Return the newest earlier final leader ratified by `current_leader`.
///
/// This is the recursion edge used by `tau`: given a current leader block,
/// walk backward through earlier waves and return the most recent final leader
/// that the current block ratifies. The caller is responsible for passing the
/// paper-native consensus parameters used to determine finality.
pub fn previous_final_leader<F>(
    blocklace: &Blocklace,
    current_leader: &BlockIdentity,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let current_block = blocklace.get(current_leader)?;
    let current_round = depth(blocklace, current_leader)?;
    let current_wave = wave_of_round(current_round, wavelength)?;

    if current_wave == 0 {
        return None;
    }

    for wave in (0..current_wave).rev() {
        let Some(previous_leader) =
            final_leader_for_wave(blocklace, wave, wavelength, n, f, leader_selection)
        else {
            continue;
        };
        let Some(previous_block) = blocklace.get(&previous_leader) else {
            continue;
        };

        if ratifies(blocklace, &current_block, &previous_block, n, f) {
            return Some(previous_leader);
        }
    }

    None
}

/// Return the newest earlier weighted-final leader ratified by `current_leader`.
///
/// This is the stake-weighted recursion edge for a future `weighted_tau`.
/// It mirrors [`previous_final_leader`] but uses weighted finality and
/// weighted ratification over the supplied bonded validator set.
pub fn weighted_previous_final_leader<F>(
    blocklace: &Blocklace,
    current_leader: &BlockIdentity,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let current_block = blocklace.get(current_leader)?;
    let current_round = depth(blocklace, current_leader)?;
    let current_wave = wave_of_round(current_round, wavelength)?;

    if current_wave == 0 {
        return None;
    }

    for wave in (0..current_wave).rev() {
        let Some(previous_leader) =
            weighted_final_leader_for_wave(blocklace, wave, wavelength, bonds, leader_selection)
        else {
            continue;
        };
        let Some(previous_block) = blocklace.get(&previous_leader) else {
            continue;
        };

        if weighted_ratifies(blocklace, &current_block, &previous_block, bonds) {
            return Some(previous_leader);
        }
    }

    None
}

/// Return the paper-native ordered output sequence of the blocklace.
///
/// `tau` anchors on the latest final leader, recursively emits the output
/// induced by earlier final leaders, then appends the current leader's
/// approved blocks in deterministic topological order, excluding any block
/// already emitted by earlier recursion.
pub fn tau<F>(
    blocklace: &Blocklace,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
) -> Result<Vec<BlockIdentity>, OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let Some(latest_leader) = latest_final_leader(blocklace, wavelength, n, f, leader_selection)
    else {
        return Ok(Vec::new());
    };

    let config = TauConfig {
        wavelength,
        n,
        f,
        leader_selection,
    };
    let mut state = TauState {
        emitted: BTreeSet::new(),
        ordered: Vec::new(),
    };
    tau_from_leader(blocklace, &latest_leader, &config, &mut state)?;
    Ok(state.ordered)
}

/// Cached variant of [`tau`].
///
/// Reuses per-leader approved block sets and sorted fragments across repeated
/// calls. Cache entries are invalidated automatically when the blocklace size
/// changes.
pub fn tau_with_cache<F>(
    blocklace: &Blocklace,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
    cache: &mut OrderingCache,
) -> Result<Vec<BlockIdentity>, OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    sync_cache_generation(blocklace, cache);

    let Some(latest_leader) = latest_final_leader(blocklace, wavelength, n, f, leader_selection)
    else {
        return Ok(Vec::new());
    };

    let key = TauOutputCacheKey {
        latest_leader: latest_leader.clone(),
        wavelength,
        n,
        f,
    };
    if let Some(ordered) = cache.tau_output_by_latest_leader.get(&key) {
        return ordered.clone();
    }

    let config = TauConfig {
        wavelength,
        n,
        f,
        leader_selection,
    };
    let mut state = TauState {
        emitted: BTreeSet::new(),
        ordered: Vec::new(),
    };
    let ordered = match tau_from_leader_cached(blocklace, &latest_leader, &config, cache, &mut state)
    {
        Ok(()) => Ok(state.ordered),
        Err(err) => Err(err),
    };
    cache
        .tau_output_by_latest_leader
        .insert(key, ordered.clone());
    ordered
}

/// Return the stake-weighted ordered output sequence of the blocklace.
///
/// `weighted_tau` mirrors [`tau`] but anchors on the latest weighted final
/// leader and walks the weighted-final leader chain via
/// [`weighted_previous_final_leader`]. This is the PoS / f1r3node-oriented
/// parallel to the paper-native unweighted output function.
pub fn weighted_tau<F>(
    blocklace: &Blocklace,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
) -> Result<Vec<BlockIdentity>, OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let Some(latest_leader) =
        latest_weighted_final_leader(blocklace, wavelength, bonds, leader_selection)
    else {
        return Ok(Vec::new());
    };

    let config = WeightedTauConfig {
        wavelength,
        bonds,
        leader_selection,
    };
    let mut state = TauState {
        emitted: BTreeSet::new(),
        ordered: Vec::new(),
    };
    weighted_tau_from_leader(blocklace, &latest_leader, &config, &mut state)?;
    Ok(state.ordered)
}

/// Cached variant of [`weighted_tau`].
///
/// Reuses per-leader approved block sets and sorted fragments across repeated
/// calls. Cache entries are invalidated automatically when the blocklace size
/// changes.
pub fn weighted_tau_with_cache<F>(
    blocklace: &Blocklace,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
    cache: &mut OrderingCache,
) -> Result<Vec<BlockIdentity>, OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    sync_cache_generation(blocklace, cache);

    let Some(latest_leader) =
        latest_weighted_final_leader(blocklace, wavelength, bonds, leader_selection)
    else {
        return Ok(Vec::new());
    };

    let key = WeightedTauOutputCacheKey {
        latest_leader: latest_leader.clone(),
        wavelength,
        bonds_fingerprint: bonds_fingerprint(bonds),
    };
    if let Some(ordered) = cache.weighted_tau_output_by_latest_leader.get(&key) {
        return ordered.clone();
    }

    let config = WeightedTauConfig {
        wavelength,
        bonds,
        leader_selection,
    };
    let mut state = TauState {
        emitted: BTreeSet::new(),
        ordered: Vec::new(),
    };
    let ordered =
        match weighted_tau_from_leader_cached(blocklace, &latest_leader, &config, cache, &mut state)
        {
            Ok(()) => Ok(state.ordered),
            Err(err) => Err(err),
        };
    cache
        .weighted_tau_output_by_latest_leader
        .insert(key, ordered.clone());
    ordered
}

fn tau_from_leader<F>(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    config: &TauConfig<F>,
    state: &mut TauState,
) -> Result<(), OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if let Some(previous) = previous_final_leader(
        blocklace,
        leader,
        config.wavelength,
        config.n,
        config.f,
        config.leader_selection,
    ) {
        tau_from_leader(blocklace, &previous, config, state)?;
    }

    let newly_approved: HashSet<Block> = approved_blocks_for_leader(blocklace, leader)
        .into_iter()
        .filter(|block| !state.emitted.contains(&block.identity))
        .collect();

    for id in xsort(&newly_approved)? {
        if state.emitted.insert(id.clone()) {
            state.ordered.push(id);
        }
    }

    Ok(())
}

fn tau_from_leader_cached<F>(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    config: &TauConfig<F>,
    cache: &mut OrderingCache,
    state: &mut TauState,
) -> Result<(), OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if let Some(previous) = previous_final_leader_cached(
        blocklace,
        leader,
        config.wavelength,
        config.n,
        config.f,
        config.leader_selection,
        cache,
    ) {
        tau_from_leader_cached(blocklace, &previous, config, cache, state)?;
    }

    for id in sorted_approved_fragment(blocklace, leader, cache)? {
        if state.emitted.insert(id.clone()) {
            state.ordered.push(id);
        }
    }

    Ok(())
}

fn weighted_tau_from_leader<'a, F>(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    config: &WeightedTauConfig<'a, F>,
    state: &mut TauState,
) -> Result<(), OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if let Some(previous) = weighted_previous_final_leader(
        blocklace,
        leader,
        config.wavelength,
        config.bonds,
        config.leader_selection,
    ) {
        weighted_tau_from_leader(blocklace, &previous, config, state)?;
    }

    let newly_approved: HashSet<Block> = approved_blocks_for_leader(blocklace, leader)
        .into_iter()
        .filter(|block| !state.emitted.contains(&block.identity))
        .collect();

    for id in xsort(&newly_approved)? {
        if state.emitted.insert(id.clone()) {
            state.ordered.push(id);
        }
    }

    Ok(())
}

fn weighted_tau_from_leader_cached<'a, F>(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    config: &WeightedTauConfig<'a, F>,
    cache: &mut OrderingCache,
    state: &mut TauState,
) -> Result<(), OrderingError>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if let Some(previous) = weighted_previous_final_leader_cached(
        blocklace,
        leader,
        config.wavelength,
        config.bonds,
        config.leader_selection,
        cache,
    ) {
        weighted_tau_from_leader_cached(blocklace, &previous, config, cache, state)?;
    }

    for id in sorted_approved_fragment(blocklace, leader, cache)? {
        if state.emitted.insert(id.clone()) {
            state.ordered.push(id);
        }
    }

    Ok(())
}
