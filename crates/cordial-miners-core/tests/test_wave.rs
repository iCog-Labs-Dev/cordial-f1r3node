use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::wave::{
    first_round_of_wave, is_first_round_of_wave, last_round_of_wave, leader_blocks_of_wave,
    leader_round_of_wave, round_is_in_wave, rounds_of_wave, wave_of_round,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

// Mock verifier that accepts all blocks. We only need to test the wave functions, so we don't care about signatures.
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

// Helper function to create a mock block with a given creator, hash byte, and predecessors.

fn create_mock_block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
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

// Helper function to insert a block into the blocklace using the mock verifier.
fn insert(blocklace: &mut Blocklace, block: Block) {
    let verifier = MockVerifier;
    blocklace.insert(block, &verifier).expect("insert failed");
}

// Wave partitioning matches fixed wavelength
#[test]
fn wave_partitioning_matches_fixed_wavelength() {
    assert_eq!(wave_of_round(0, 3), Some(0)); // This test is for the wave_of_round function, which should return 0 for rounds 0, 1, and 2 when the wavelength is 3. why 0 is the expected wave for these rounds? Because the wave of a round is calculated as round / wavelength, so for rounds 0, 1, and 2, we have: 0 / 3 = 0, 1 / 3 = 0, 2 / 3 = 0. Therefore, all these rounds belong to wave 0.
    assert_eq!(wave_of_round(1, 3), Some(0));
    assert_eq!(wave_of_round(2, 3), Some(0));
    assert_eq!(wave_of_round(3, 3), Some(1)); // For round 3, we have 3 / 3 = 1, so it belongs to wave 1.
    assert_eq!(wave_of_round(4, 3), Some(1));
    assert_eq!(wave_of_round(7, 3), Some(2));
    assert_eq!(wave_of_round(8, 0), None); // A wavelength of 0 is invalid, so the function should return None.
}
