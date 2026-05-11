use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::cordiality::{blocks_that_approve, is_supermajority, ratifies, super_ratifies},
    crypto::CryptoVerifier,
};
use std::collections::HashSet;

#[cfg(test)]
mod tests {
    use super::*;

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

    fn create_test_block(
        creator_id: u8,
        hash_byte: u8,
        predecessors: HashSet<BlockIdentity>,
    ) -> Block {
        let mut content_hash = [0u8; 32];
        content_hash[0] = hash_byte;
        Block {
            identity: BlockIdentity {
                content_hash,
                creator: NodeId(vec![creator_id]),
                signature: vec![],
            },
            content: BlockContent {
                payload: vec![],
                predecessors,
            },
        }
    }

    fn insert(blocklace: &mut Blocklace, block: Block) {
        let verifier = MockVerifier;
        blocklace.insert(block, &verifier).expect("insert failed");
    }

    #[test]
    fn test_ratification_success_with_supermajority() {
        // Create a simple blocklace with 4 miners (n=4, f=1)
        let mut blocklace = Blocklace::new();

        // Create genesis block
        let genesis = create_test_block(1, 1, HashSet::new());
        insert(&mut blocklace, genesis.clone());

        // Create target block at round 1
        let target = create_test_block(1, 4, HashSet::from([genesis.identity.clone()]));
        insert(&mut blocklace, target.clone());

        // Create approving blocks at round 2 (supermajority: > (4+1)/2 = 2.5, so need 3)
        let approver1 = create_test_block(2, 7, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver1.clone());

        let approver2 = create_test_block(3, 10, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver2.clone());

        let approver3 = create_test_block(4, 13, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver3.clone());

        // Create ratifier block at round 3
        let ratifier = create_test_block(
            1,
            16,
            HashSet::from([
                approver1.identity.clone(),
                approver2.identity.clone(),
                approver3.identity.clone(),
            ]),
        );
        insert(&mut blocklace, ratifier.clone());

        // Test ratification - should succeed with supermajority
        let result = ratifies(&blocklace, &ratifier, &target, 4, 1);
        assert!(result);
    }

    #[test]
    fn test_ratification_failure_without_supermajority() {
        // Create a simple blocklace with 4 miners (n=4, f=1)
        let mut blocklace = Blocklace::new();

        // Create genesis block
        let genesis = create_test_block(1, 1, HashSet::new());
        insert(&mut blocklace, genesis.clone());

        // Create target block at round 1
        let target = create_test_block(1, 4, HashSet::from([genesis.identity.clone()]));
        insert(&mut blocklace, target.clone());

        // Create only 2 approving blocks at round 2 (below supermajority threshold)
        let approver1 = create_test_block(2, 7, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver1.clone());

        let approver2 = create_test_block(3, 10, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver2.clone());

        // Create ratifier block at round 3
        let ratifier = create_test_block(
            1,
            16,
            HashSet::from([approver1.identity.clone(), approver2.identity.clone()]),
        );
        insert(&mut blocklace, ratifier.clone());

        // Test ratification - should fail without supermajority
        let result = ratifies(&blocklace, &ratifier, &target, 4, 1);
        assert!(!result);
    }

    #[test]
    fn test_super_ratification_success() {
        // Create a simple blocklace with 4 miners (n=4, f=1)
        let mut blocklace = Blocklace::new();

        // Create genesis block
        let genesis = create_test_block(1, 1, HashSet::new());
        insert(&mut blocklace, genesis.clone());

        // Create target block at round 1
        let target = create_test_block(1, 4, HashSet::from([genesis.identity.clone()]));
        insert(&mut blocklace, target.clone());

        // Create approving blocks at round 2
        let approver1 = create_test_block(2, 7, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver1.clone());

        let approver2 = create_test_block(3, 10, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver2.clone());

        let approver3 = create_test_block(4, 13, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver3.clone());

        // Create ratifying blocks at round 3 (each observes a supermajority of approvers)
        let ratifier1 = create_test_block(
            1,
            16,
            HashSet::from([
                approver1.identity.clone(),
                approver2.identity.clone(),
                approver3.identity.clone(),
            ]),
        );
        insert(&mut blocklace, ratifier1.clone());

        let ratifier2 = create_test_block(
            2,
            19,
            HashSet::from([
                approver1.identity.clone(),
                approver2.identity.clone(),
                approver3.identity.clone(),
            ]),
        );
        insert(&mut blocklace, ratifier2.clone());

        let ratifier3 = create_test_block(
            3,
            22,
            HashSet::from([
                approver1.identity.clone(),
                approver2.identity.clone(),
                approver3.identity.clone(),
            ]),
        );
        insert(&mut blocklace, ratifier3.clone());

        let ratifiers = HashSet::from([ratifier1, ratifier2, ratifier3]);

        // Test super-ratification - should succeed with supermajority
        let result = super_ratifies(&blocklace, &ratifiers, &target, 4, 1);
        assert!(result);
    }

    #[test]
    fn test_super_ratification_failure() {
        // Create a simple blocklace with 4 miners (n=4, f=1)
        let mut blocklace = Blocklace::new();

        // Create genesis block
        let genesis = create_test_block(1, 1, HashSet::new());
        insert(&mut blocklace, genesis.clone());

        // Create target block at round 1
        let target = create_test_block(1, 4, HashSet::from([genesis.identity.clone()]));
        insert(&mut blocklace, target.clone());

        // Create only 2 ratifying blocks at round 2 (below supermajority threshold)
        let ratifier1 = create_test_block(2, 7, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, ratifier1.clone());

        let ratifier2 = create_test_block(3, 10, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, ratifier2.clone());

        let ratifiers = HashSet::from([ratifier1, ratifier2]);

        // Test super-ratification - should fail without supermajority
        let result = super_ratifies(&blocklace, &ratifiers, &target, 4, 1);
        assert!(!result);
    }

    #[test]
    fn test_blocks_that_approve_success() {
        // Create a simple blocklace
        let mut blocklace = Blocklace::new();

        // Create target block
        let target = create_test_block(1, 1, HashSet::new());
        insert(&mut blocklace, target.clone());

        // Create approver block that observes target
        let approver = create_test_block(2, 4, HashSet::from([target.identity.clone()]));
        insert(&mut blocklace, approver.clone());

        // Test blocks_that_approve - should return approver
        let result = blocks_that_approve(&blocklace, &approver, &target);
        assert!(result.contains(&approver));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_blocks_that_approve_with_equivocation() {
        // Create a simple blocklace
        let mut blocklace = Blocklace::new();

        // Create target block
        let target = create_test_block(1, 1, HashSet::new());
        insert(&mut blocklace, target.clone());

        // Create equivocating block by same creator
        let equivocator_block = create_test_block(1, 7, HashSet::new());
        insert(&mut blocklace, equivocator_block.clone());

        // Create approver block that observes both target and equivocating block
        let approver = create_test_block(
            2,
            4,
            HashSet::from([target.identity.clone(), equivocator_block.identity.clone()]),
        );
        insert(&mut blocklace, approver.clone());

        // Test blocks_that_approve - should return empty set due to equivocation
        let result = blocks_that_approve(&blocklace, &approver, &target);
        assert!(result.is_empty());
    }

    #[test]
    fn test_is_supermajority() {
        // Test supermajority calculation: n=4, f=1, threshold = (4+1)/2 = 2.5, so need 3
        let mut blocks = HashSet::new();

        // Add blocks from 3 different creators
        blocks.insert(create_test_block(1, 1, HashSet::new()));
        blocks.insert(create_test_block(2, 4, HashSet::new()));
        blocks.insert(create_test_block(3, 7, HashSet::new()));

        // Should be supermajority
        assert!(is_supermajority(&blocks, 4, 1));

        // Add block from 4th creator (should still be supermajority)
        blocks.insert(create_test_block(4, 10, HashSet::new()));

        // Should still be supermajority
        assert!(is_supermajority(&blocks, 4, 1));

        // Remove two blocks (only 2 distinct creators left - below threshold for n=4, f=1 which is > 2.5)
        blocks.remove(&create_test_block(3, 7, HashSet::new()));
        blocks.remove(&create_test_block(4, 10, HashSet::new()));

        // Should not be supermajority
        assert!(!is_supermajority(&blocks, 4, 1));
    }

    #[test]
    fn test_edge_cases() {
        // Test with n=1, f=0 (single miner)
        let mut blocks = HashSet::new();
        blocks.insert(create_test_block(1, 1, HashSet::new()));

        assert!(is_supermajority(&blocks, 1, 0));

        // Test with n=2, f=0 (two miners)
        let mut blocks = HashSet::new();
        blocks.insert(create_test_block(1, 1, HashSet::new()));
        blocks.insert(create_test_block(2, 4, HashSet::new()));

        assert!(is_supermajority(&blocks, 2, 0));

        // Test with n=2, f=1 (two miners, one faulty)
        let mut blocks = HashSet::new();
        blocks.insert(create_test_block(1, 1, HashSet::new()));
        blocks.insert(create_test_block(2, 4, HashSet::new()));

        assert!(is_supermajority(&blocks, 2, 1));
    }
}
