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

// Test if wave bounds are computed correctly
#[test]
fn wave_bounds_computed_correctly() {
    assert_eq!(first_round_of_wave(0, 5), Some(0)); // The first round of wave 0 with wavelength 5 is calculated as wave * wavelength = 0 * 5 = 0.
    assert_eq!(last_round_of_wave(0, 5), Some(4)); // The last round of wave 0 with wavelength 5 is calculated as first round + wavelength - 1 = 0 + 5 - 1 = 4.
    assert_eq!(first_round_of_wave(2, 5), Some(10)); // The first round of wave 2 with wavelength 5 is calculated as wave * wavelength = 2 * 5 = 10.
    assert_eq!(last_round_of_wave(2, 5), Some(14)); // The last round of wave 2 with wavelength 5 is calculated as first round + wavelength - 1 = 10 + 5 - 1 = 14.
    assert_eq!(rounds_of_wave(2, 5), Some(10..=14)); // the rounds of wave 2 with wavelength 5 are calculated as the range from the first round to the last round, which is 10..=14.
    assert_eq!(leader_round_of_wave(2, 5), Some(10)); // The leader round of wave 2 with wavelength 5 is the first round of that wave, which is 10.
}

// Test if round membership is determined correctly
#[test]
fn first_round_and_membership_helpers_match_wave_structure() {
    assert!(is_first_round_of_wave(6, 3)); // Round 6 is the first round of its wave because it belongs to wave 2 (6 / 3 = 2), and the first round of wave 2 is 6 (2 * 3 = 6).
    assert!(!is_first_round_of_wave(7, 3)); // Round 7 is not the first round of its wave because it belongs to wave 2 (7 / 3 = 2), and the first round of wave 2 is 6 (2 * 3 = 6).
    assert!(round_is_in_wave(7, 2, 3)); // Round 7 belongs to wave 2 because wave_of_round(7, 3) returns Some(2).
    assert!(!round_is_in_wave(7, 1, 3)); // Round 7 does not belong to wave 1 because wave_of_round(7, 3) returns Some(2), not Some(1).
}

// Test leader block is taken from first round of wave only
#[test]
fn leader_blocks_are_taken_from_first_round_of_wave_only() {
    let mut blocklace = Blocklace::new();
    let g1 = create_mock_block(1, 1, HashSet::new()); // This block is created by miner 1 and has a hash byte of 1. It has no predecessors.
    let g2 = create_mock_block(2, 2, HashSet::new()); // This block is created by miner 2 and has a hash byte of 2. It has no predecessors.
    insert(&mut blocklace, g1.clone());
    insert(&mut blocklace, g2.clone());

    let r1_leader = create_mock_block(1, 3, HashSet::from([g1.identity.clone()])); // This block is created by miner 1 and has a hash byte of 3. It has no predecessors, so it belongs to round 0.
    let r1_other = create_mock_block(3, 4, HashSet::from([g2.identity.clone()])); // This block is created by miner 2 and has a hash byte of 4. It has no predecessors, so it belongs to round 0.

    insert(&mut blocklace, r1_leader.clone());
    insert(&mut blocklace, r1_other.clone());

    let r2_same_creator = create_mock_block(1, 5, HashSet::from([r1_leader.identity.clone()])); // This block is created by miner 1 and has a hash byte of 5. It has no predecessors, so it belongs to round 0.
    insert(&mut blocklace, r2_same_creator.clone());

    let leaders = leader_blocks_of_wave(&blocklace, 1, 1, |wave| match wave {
        1 => Some(NodeId(vec![1])),
        _ => None,
    });
    assert_eq!(leaders.len(), 1); // Only r2_leader should be the leader block of wave 1, because it is the only block that belongs to the first round of wave 1 and is created by the selected leader (miner 1). r2_same_creator does not belong to wave 1, so it should not be considered a leader block for that wave.
    assert!(leaders.contains(&r1_leader)); // r1_leader should not be a leader block of wave 1, because it belongs to round 0, not round 1.
}

// Tests if no selected leader results in no leader blocks
#[test]
fn no_selected_leader_results_in_no_leader_blocks() {
    let mut blocklace = Blocklace::new();
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, g1.clone());

    let leaders = leader_blocks_of_wave(&blocklace, 0, 3, |_| None);
    assert_eq!(leaders.len(), 0); // No selected leader, so no leader blocks should be returned.
}
