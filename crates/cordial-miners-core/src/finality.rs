use std::collections::{BTreeSet, HashMap, HashSet};

use crate::blocklace::Blocklace;
use crate::consensus::finality::{FinalityStatus, check_finality};
use crate::types::{BlockIdentity, NodeId};

pub fn tau(blocklace: &Blocklace, bonds: &HashMap<NodeId, u64>) -> Vec<BlockIdentity> {
    let leaders = finalized_leader_chain(blocklace, bonds);
    if leaders.is_empty() {
        return vec![];
    }

    let mut output: Vec<BlockIdentity> = Vec::new();
    let mut already_output: HashSet<BlockIdentity> = HashSet::new();

    for leader in &leaders {
        let approved = approved_causal_history(blocklace, leader);
        let new_blocks: HashSet<BlockIdentity> = approved
            .difference(&already_output)
            .cloned()
            .collect();

        let sorted = xsort(new_blocks, blocklace);
        for id in sorted {
            already_output.insert(id.clone());
            output.push(id);
        }
    }

    output
}

pub fn approves(blocklace: &Blocklace, b: &BlockIdentity, target: &BlockIdentity) -> bool {
    let causal_history = blocklace.ancestors_inclusive(b);

    let observes_target = causal_history
        .iter()
        .any(|block| &block.identity == target);

    if !observes_target {
        return false;
    }

    let target_creator = &target.creator;

    let sees_equivocating_sibling = causal_history.iter().any(|block| {
        &block.identity != target
            && &block.identity.creator == target_creator
            && !blocklace.precedes(&block.identity, target)
            && !blocklace.precedes(target, &block.identity)
    });

    !sees_equivocating_sibling
}

pub fn approved_causal_history(
    blocklace: &Blocklace,
    leader_id: &BlockIdentity,
) -> HashSet<BlockIdentity> {
    blocklace
        .ancestors_inclusive(leader_id)
        .into_iter()
        .filter(|block| approves(blocklace, leader_id, &block.identity))
        .map(|block| block.identity)
        .collect()
}

pub fn xsort(ids: HashSet<BlockIdentity>, blocklace: &Blocklace) -> Vec<BlockIdentity> {
    if ids.is_empty() {
        return vec![];
    }

    let mut in_degree: HashMap<BlockIdentity, usize> = ids
        .iter()
        .map(|id| (id.clone(), 0))
        .collect();

    let mut successor_map: HashMap<BlockIdentity, Vec<BlockIdentity>> = HashMap::new();

    for id in &ids {
        if let Some(content) = blocklace.content(id) {
            for pred_id in &content.predecessors {
                if ids.contains(pred_id) {
                    *in_degree.get_mut(id).unwrap() += 1;
                    successor_map
                        .entry(pred_id.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }
    }

    let mut ready: BTreeSet<BlockIdentity> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();

    let mut result = Vec::with_capacity(ids.len());

    while let Some(current) = ready.pop_first() {
        result.push(current.clone());

        if let Some(successors) = successor_map.get(&current) {
            for succ in successors {
                let deg = in_degree.get_mut(succ).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(succ.clone());
                }
            }
        }
    }

    result
}

pub fn finalized_leader_chain(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Vec<BlockIdentity> {
    let finalized: HashSet<BlockIdentity> = blocklace
        .dom()
        .into_iter()
        .filter(|id| {
            matches!(
                check_finality(blocklace, id, bonds),
                FinalityStatus::Finalized { .. }
            )
        })
        .cloned()
        .collect();

    xsort(finalized, blocklace)
}
