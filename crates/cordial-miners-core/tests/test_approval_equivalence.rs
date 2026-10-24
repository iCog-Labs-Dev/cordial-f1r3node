//! `approves` is answered from a shared ancestor index. This pins it to the
//! literal Definition 18 computed from `Blocklace::observe`, on random DAGs
//! that include equivocating creators (issue #263).

use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::approves;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn block(creator: u8, tag: u16, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator;
    content_hash[1..3].copy_from_slice(&tag.to_le_bytes());
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

/// Definition 18, written directly against `observe` with no shared state.
fn reference_approves(
    blocklace: &Blocklace,
    approver: &BlockIdentity,
    target: &BlockIdentity,
) -> bool {
    let observed = blocklace.observe(approver);
    if !observed.contains(target) {
        return false;
    }
    let target_closure = blocklace.observe(target);
    for other in blocklace.blocks_by(&target.creator) {
        if other.identity == *target {
            continue;
        }
        let other_closure = blocklace.observe(&other.identity);
        let target_precedes_other = other_closure.contains(target);
        let other_precedes_target = target_closure.contains(&other.identity);
        if !target_precedes_other && !other_precedes_target && observed.contains(&other.identity) {
            return false;
        }
    }
    true
}

fn random_blocklace(rng: &mut Rng, creators: u8, blocks: u16, equivocate: bool) -> Blocklace {
    let mut blocklace = Blocklace::new();
    let mut inserted: Vec<Block> = Vec::new();
    let mut last_by_creator: Vec<Option<BlockIdentity>> = vec![None; creators as usize + 1];

    for tag in 0..blocks {
        let creator = 1 + rng.below(u64::from(creators)) as u8;
        let mut predecessors = HashSet::new();
        // Honest creators always reference their own previous block.
        if let Some(previous) = &last_by_creator[creator as usize] {
            predecessors.insert(previous.clone());
        }
        for _ in 0..rng.below(4) {
            if !inserted.is_empty() {
                let pick = rng.below(inserted.len() as u64) as usize;
                predecessors.insert(inserted[pick].identity.clone());
            }
        }
        // Occasionally make creator 1 equivocate: drop its own-chain link.
        if equivocate && creator == 1 && rng.below(3) == 0 {
            predecessors.retain(|p| p.creator != node(1));
        }
        let candidate = block(creator, tag, predecessors);
        if blocklace.insert(candidate.clone(), &MockVerifier).is_ok() {
            last_by_creator[creator as usize] = Some(candidate.identity.clone());
            inserted.push(candidate);
        }
    }
    blocklace
}

fn assert_equivalent(blocklace: &Blocklace, label: &str) {
    let ids: Vec<BlockIdentity> = blocklace.dom().into_iter().cloned().collect();
    for approver in &ids {
        for target in &ids {
            assert_eq!(
                approves(blocklace, approver, target),
                reference_approves(blocklace, approver, target),
                "{label}: approves diverged from Definition 18",
            );
        }
    }
}

#[test]
fn approves_matches_definition_without_equivocation() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for case in 0..40 {
        let blocklace = random_blocklace(&mut rng, 4, 30, false);
        assert_equivalent(&blocklace, &format!("honest case {case}"));
    }
}

#[test]
fn approves_matches_definition_with_equivocation() {
    let mut rng = Rng(0x0FED_CBA9_8765_4321);
    for case in 0..40 {
        let blocklace = random_blocklace(&mut rng, 4, 30, true);
        assert_equivalent(&blocklace, &format!("equivocation case {case}"));
    }
}

#[test]
fn index_is_rebuilt_after_insert() {
    let mut blocklace = Blocklace::new();
    let genesis = block(1, 1, HashSet::new());
    blocklace.insert(genesis.clone(), &MockVerifier).unwrap();

    let child = block(2, 2, HashSet::from([genesis.identity.clone()]));
    // Before the child exists it cannot approve anything.
    assert!(!approves(&blocklace, &child.identity, &genesis.identity));

    blocklace.insert(child.clone(), &MockVerifier).unwrap();
    assert!(approves(&blocklace, &child.identity, &genesis.identity));
    assert!(approves(&blocklace, &child.identity, &child.identity));
}
