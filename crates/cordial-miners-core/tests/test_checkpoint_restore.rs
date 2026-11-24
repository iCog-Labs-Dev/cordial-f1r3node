use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, CheckpointOrigin, CheckpointRestoreError,
    NodeId,
    consensus::{
        CheckpointGc, checkpoint_after_finality, checkpoint_after_weighted_finality, tau,
        weighted_tau,
    },
    crypto::{CryptoVerifier, block_hash},
};
use std::collections::{HashMap, HashSet};

struct Verify;
impl CryptoVerifier for Verify {
    type Error = &'static str;
    fn verify_block(&self, _: &BlockContent, sig: &[u8], _: &NodeId) -> Result<(), Self::Error> {
        if sig == [1] {
            Ok(())
        } else {
            Err("bad signature")
        }
    }
}
fn leader(_: u64) -> Option<NodeId> {
    Some(NodeId(vec![1]))
}
fn chain() -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut parents = HashSet::new();
    for n in 0..12u8 {
        let creator = NodeId(vec![1]);
        let content = BlockContent {
            payload: vec![n],
            predecessors: parents,
        };
        let identity = BlockIdentity {
            content_hash: block_hash(&creator, &content),
            creator,
            signature: vec![1],
        };
        parents = HashSet::from([identity.clone()]);
        blocks.push(Block { identity, content });
    }
    blocks
}

#[test]
fn restored_state_and_ordering_match_actual_pruning() {
    let blocks = chain();
    let bonds = HashMap::from([(NodeId(vec![1]), 10)]);
    for origin in [
        CheckpointOrigin::Explicit,
        CheckpointOrigin::PaperFinal,
        CheckpointOrigin::WeightedFinal,
    ] {
        let mut original = Blocklace::new();
        for block in &blocks {
            original.insert(block.clone(), &Verify).unwrap();
        }
        let report = match origin {
            CheckpointOrigin::Explicit => original
                .prune_below_checkpoint(&blocks[9].identity)
                .unwrap(),
            CheckpointOrigin::PaperFinal => {
                checkpoint_after_finality(&mut original, 3, 1, 0, leader)
                    .unwrap()
                    .unwrap()
            }
            CheckpointOrigin::WeightedFinal => {
                checkpoint_after_weighted_finality(&mut original, 3, &bonds, leader)
                    .unwrap()
                    .unwrap()
            }
        };
        assert!(!report.removed.is_empty());
        let checkpoint = original.get(&report.checkpoint).unwrap();
        let mut restored = Blocklace::restore_from_checkpoint(
            checkpoint,
            origin,
            report.checkpoint_depth,
            original.checkpoint_order_prefix().to_vec(),
            original.checkpoint_weighted_order_prefix().to_vec(),
            &Verify,
        )
        .unwrap();
        for block in &blocks {
            if original.get(&block.identity).is_some() && block.identity != report.checkpoint {
                restored.insert(block.clone(), &Verify).unwrap();
            }
        }
        assert_eq!(restored.checkpoint(), original.checkpoint());
        assert_eq!(restored.checkpoint_origin(), original.checkpoint_origin());
        assert_eq!(restored.checkpoint_depth(), original.checkpoint_depth());
        assert_eq!(
            restored.checkpoint_order_prefix(),
            original.checkpoint_order_prefix()
        );
        assert_eq!(
            restored.checkpoint_weighted_order_prefix(),
            original.checkpoint_weighted_order_prefix()
        );
        assert_eq!(
            tau(&restored, 3, 1, 0, leader),
            tau(&original, 3, 1, 0, leader)
        );
        assert_eq!(
            weighted_tau(&restored, 3, &bonds, leader),
            weighted_tau(&original, 3, &bonds, leader)
        );
        assert!(restored.is_closed());
        assert!(!restored.has_trusted_boundaries());
    }
}

#[test]
fn bad_signature_and_inconsistent_prefixes_fail_closed() {
    let block = chain().remove(9);
    let prefix = vec![block.identity.consensus_identity()];
    for (origin, paper, weighted) in [
        (
            CheckpointOrigin::WeightedFinal,
            prefix.clone(),
            prefix.clone(),
        ),
        (CheckpointOrigin::PaperFinal, vec![], prefix.clone()),
        (CheckpointOrigin::Explicit, vec![], vec![]),
        (
            CheckpointOrigin::WeightedFinal,
            vec![],
            vec![chain()[0].identity.clone()],
        ),
    ] {
        assert!(matches!(
            Blocklace::restore_from_checkpoint(block.clone(), origin, 9, paper, weighted, &Verify),
            Err(CheckpointRestoreError::InvalidPrefix(_))
        ));
    }
    let mut bad = block;
    bad.identity.signature = vec![2];
    assert!(matches!(
        Blocklace::restore_from_checkpoint(
            bad,
            CheckpointOrigin::WeightedFinal,
            9,
            vec![],
            prefix,
            &Verify
        ),
        Err(CheckpointRestoreError::InvalidSignature(_))
    ));
}
