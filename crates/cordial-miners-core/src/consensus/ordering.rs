use std::collections::{BTreeSet, HashMap, HashSet};

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
pub fn xsort(blocks: &HashSet<Block>) -> Vec<BlockIdentity> {
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

    ordered
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
) -> Vec<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let Some(latest_leader) = latest_final_leader(blocklace, wavelength, n, f, leader_selection)
    else {
        return Vec::new();
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
    tau_from_leader(blocklace, &latest_leader, &config, &mut state);
    state.ordered
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
) -> Vec<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let Some(latest_leader) =
        latest_weighted_final_leader(blocklace, wavelength, bonds, leader_selection)
    else {
        return Vec::new();
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
    weighted_tau_from_leader(blocklace, &latest_leader, &config, &mut state);
    state.ordered
}

fn tau_from_leader<F>(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    config: &TauConfig<F>,
    state: &mut TauState,
) where
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
        tau_from_leader(blocklace, &previous, config, state);
    }

    let newly_approved: HashSet<Block> = approved_blocks_for_leader(blocklace, leader)
        .into_iter()
        .filter(|block| !state.emitted.contains(&block.identity))
        .collect();

    for id in xsort(&newly_approved) {
        if state.emitted.insert(id.clone()) {
            state.ordered.push(id);
        }
    }
}

fn weighted_tau_from_leader<'a, F>(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
    config: &WeightedTauConfig<'a, F>,
    state: &mut TauState,
) where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if let Some(previous) = weighted_previous_final_leader(
        blocklace,
        leader,
        config.wavelength,
        config.bonds,
        config.leader_selection,
    ) {
        weighted_tau_from_leader(blocklace, &previous, config, state);
    }

    let newly_approved: HashSet<Block> = approved_blocks_for_leader(blocklace, leader)
        .into_iter()
        .filter(|block| !state.emitted.contains(&block.identity))
        .collect();

    for id in xsort(&newly_approved) {
        if state.emitted.insert(id.clone()) {
            state.ordered.push(id);
        }
    }
}
