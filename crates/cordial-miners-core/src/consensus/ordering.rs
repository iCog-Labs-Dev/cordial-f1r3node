use std::collections::HashSet;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::approval::approves;
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