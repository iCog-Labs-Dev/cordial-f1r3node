//! Approval logic for Cordial Miners.
//!
//! A block `b` approves a block `b'` if `b'` is in the causal history of `b`
//! and `b` does not observe any equivocation of the creator of `b'` at the
//! round of `b'`.
//!
//! This module implements the binary approval predicate from the paper
//! (Definition 18) and extends it with validator weighting (stake) for 
//! Proof-of-Stake support.

use std::collections::{HashMap, HashSet};

use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};
use crate::consensus::cordiality::observed_block_ids;
use crate::consensus::round::depth;

/// A threshold for consensus predicates, using integer math for determinism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalThreshold {
    pub numerator: u128,
    pub denominator: u128,
}

impl ApprovalThreshold {
    pub const SUPERMAJORITY: Self = Self { numerator: 2, denominator: 3 };
}

/// Returns true if block `b` approves block `b'` per Definition 18.
///
/// Binary approval requires:
/// 1. `b'` is in the observed history of `b`.
/// 2. `b` does not observe any equivocation of `node(b')` at `round(b')`.
pub fn approves_binary(
    blocklace: &Blocklace,
    b: &BlockIdentity,
    b_prime: &BlockIdentity,
) -> bool {
    let Some(candidate) = blocklace.get(b_prime) else {
        return false;
    };
    let creator = &candidate.identity.creator;
    let round = depth(blocklace, b_prime).unwrap_or(0);

    let Some(approver) = blocklace.get(b) else {
        return false;
    };
    
    // 1. Observation check: is b_prime in obs(b)?
    let observed = observed_block_ids(blocklace, &approver);
    if !observed.contains(b_prime) {
        return false;
    }

    // 2. Equivocation check: does b observe multiple blocks by creator at round?
    // We check all blocks in observed history for conflicts.
    let blocks_at_round = observed
        .iter()
        .filter(|id| &id.creator == creator && depth(blocklace, id) == Some(round))
        .count();

    blocks_at_round < 2
}

/// Returns true if the total weight of validators who approve `candidate` 
/// (as seen by `approver`) meets the threshold.
///
/// This implements the weighted approval used for ratification.
pub fn approves_weighted(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    approver_id: &BlockIdentity,
    candidate_id: &BlockIdentity,
    threshold: &ApprovalThreshold,
) -> bool {
    // The approver block itself must approve the candidate.
    if !approves_binary(blocklace, approver_id, candidate_id) {
        return false;
    }

    let Some(approver) = blocklace.get(approver_id) else {
        return false;
    };
    
    let observed = observed_block_ids(blocklace, &approver);
    let mut support_weight: u128 = 0;
    let mut seen_validators = HashSet::new();

    // Iterate through all blocks observed by the approver.
    // Sum weights of unique validators whose blocks approve the candidate.
    for obs_id in &observed {
        let v_id = &obs_id.creator;
        if seen_validators.contains(v_id) {
            continue;
        }

        if approves_binary(blocklace, obs_id, candidate_id) {
            if let Some(stake) = bonds.get(v_id) {
                support_weight += *stake as u128;
                seen_validators.insert(v_id.clone());
            }
        }
    }

    let total_weight: u128 = bonds.values().sum::<u64>() as u128;
    if total_weight == 0 {
        return false;
    }

    // Cross-multiplication for threshold check: support/total > num/den
    support_weight * threshold.denominator > total_weight * threshold.numerator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::types::BlockContent;
    use crate::crypto::NoopVerifier;

    fn mock_id(creator: &str, hash: &str) -> BlockIdentity {
        BlockIdentity {
            creator: NodeId(creator.as_bytes().to_vec()),
            content_hash: hash.as_bytes().to_vec(),
            signature: Vec::new(),
        }
    }

    fn mock_block(creator: &str, hash: &str, preds: Vec<BlockIdentity>) -> Block {
        Block {
            identity: mock_id(creator, hash),
            content: BlockContent {
                payload: Vec::new(),
                predecessors: preds.into_iter().collect(),
            },
        }
    }

    #[test]
    fn test_approves_binary_observation() {
        let mut bl = Blocklace::new();
        let v = NoopVerifier;

        let b1 = mock_block("alice", "h1", vec![]);
        let id1 = b1.identity.clone();
        bl.insert(b1, &v).unwrap();

        let b2 = mock_block("bob", "h2", vec![id1.clone()]);
        let id2 = b2.identity.clone();
        bl.insert(b2, &v).unwrap();

        let b3 = mock_block("charlie", "h3", vec![]);
        let id3 = b3.identity.clone();
        bl.insert(b3, &v).unwrap();

        // bob observes alice
        assert!(approves_binary(&bl, &id2, &id1));
        // bob does not observe charlie
        assert!(!approves_binary(&bl, &id2, &id3));
    }

    #[test]
    fn test_approves_binary_equivocation() {
        let mut bl = Blocklace::new();
        let v = NoopVerifier;

        // Alice equivocates in round 0
        let a1 = mock_block("alice", "h1", vec![]);
        let aid1 = a1.identity.clone();
        bl.insert(a1, &v).unwrap();

        let a2 = mock_block("alice", "h2", vec![]);
        let aid2 = a2.identity.clone();
        bl.insert(a2, &v).unwrap();

        // Bob observes only a1 -> approves
        let b1 = mock_block("bob", "hb1", vec![aid1.clone()]);
        let bid1 = b1.identity.clone();
        bl.insert(b1, &v).unwrap();
        assert!(approves_binary(&bl, &bid1, &aid1));

        // Charlie observes both a1 and a2 -> does NOT approve a1
        let c1 = mock_block("charlie", "hc1", vec![aid1.clone(), aid2.clone()]);
        let cid1 = c1.identity.clone();
        bl.insert(c1, &v).unwrap();
        assert!(!approves_binary(&bl, &cid1, &aid1));
    }

    #[test]
    fn test_approves_weighted_no_double_count() {
        let mut bl = Blocklace::new();
        let v = NoopVerifier;
        let mut bonds = HashMap::new();
        let alice_node = NodeId("alice".as_bytes().to_vec());
        bonds.insert(alice_node.clone(), 100);
        let bob_node = NodeId("bob".as_bytes().to_vec());
        bonds.insert(bob_node.clone(), 0);

        let threshold = ApprovalThreshold { numerator: 1, denominator: 2 }; // > 50%

        let a1 = mock_block("alice", "h1", vec![]);
        let aid1 = a1.identity.clone();
        bl.insert(a1, &v).unwrap();

        // Bob creates two blocks, both observing a1
        let b1 = mock_block("bob", "hb1", vec![aid1.clone()]);
        let bid1 = b1.identity.clone();
        bl.insert(b1, &v).unwrap();

        let b2 = mock_block("bob", "hb2", vec![bid1.clone()]);
        let bid2 = b2.identity.clone();
        bl.insert(b2, &v).unwrap();

        // Charlie observes both b1 and b2
        let c1 = mock_block("charlie", "hc1", vec![bid2.clone()]);
        let cid1 = c1.identity.clone();
        bl.insert(c1, &v).unwrap();

        // Weight should be 100 (from alice) + 0 (from bob) = 100.
        // Even though bob has 2 blocks observing alice, bob's weight is only added once.
        assert!(approves_weighted(&bl, &bonds, &cid1, &aid1, &threshold));
    }
}
