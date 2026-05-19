use std::collections::{BTreeSet, HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::approval::approves;
use crate::consensus::cordiality::ratifies;
use crate::consensus::finality::final_leader_for_wave;
use crate::consensus::round::depth;
use crate::consensus::wave::wave_of_round;
use crate::types::BlockIdentity;

pub fn approved_blocks_for_leader(
    blocklace: &Blocklace,
    leader: &BlockIdentity,
) -> HashSet<Block> {
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
    let block_ids: HashSet<BlockIdentity> = blocks.iter().map(|block| block.identity.clone()).collect();
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
    F: Fn(u64) -> Option<crate::types::NodeId> + Copy,
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
