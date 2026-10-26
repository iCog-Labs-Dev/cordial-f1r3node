use std::collections::HashSet;

use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{
        CordialEquivocationEvidence, EvidenceVerificationError, verify_equivocation_evidence,
    },
    crypto::{self, Secp256k1Scheme},
};
use k256::ecdsa::SigningKey;

fn node(seed: u8) -> NodeId {
    NodeId(
        SigningKey::from_slice(&[seed; 32])
            .unwrap()
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn block(seed: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let content = BlockContent {
        payload: vec![tag],
        predecessors,
    };
    let creator = node(seed);
    let content_hash = crypto::block_hash(&creator, &content);
    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature: crypto::sign(&content_hash, &[seed; 32]),
        },
        content,
    }
}

fn raw() -> CordialEquivocationEvidence {
    CordialEquivocationEvidence::new(
        node(1),
        0,
        vec![block(1, 1, HashSet::new()), block(1, 2, HashSet::new())],
    )
}

#[test]
fn valid_proofs_are_canonical_and_independent_of_input_order() {
    let claim = raw();
    let proof = verify_equivocation_evidence(&claim, &Blocklace::new()).unwrap();
    let mut reversed = claim.clone();
    reversed.blocks.reverse();
    let other = verify_equivocation_evidence(&reversed, &Blocklace::new()).unwrap();
    assert_eq!(proof.canonical_bytes(), other.canonical_bytes());
    assert_eq!(proof.validator(), &node(1));
    assert_eq!(proof.round(), 0);
    assert_eq!(proof.blocks().len(), 2);
}

#[test]
fn fabricated_claims_are_rejected() {
    use EvidenceVerificationError as Error;
    let original = raw();
    let mut wrong_creator = original.clone();
    wrong_creator.validator = node(2);
    let mut unsigned = original.clone();
    unsigned.blocks[0].identity.signature.clear();
    let mut wrong_signature = original.clone();
    wrong_signature.blocks[0].identity.signature =
        crypto::sign(&wrong_signature.blocks[0].identity.content_hash, &[2; 32]);
    let mut wrong_content = original.clone();
    wrong_content.blocks[0].content.payload.push(99);
    let mut duplicate = original.clone();
    duplicate.blocks[1] = duplicate.blocks[0].clone();
    let mut wrong_round = original.clone();
    wrong_round.round = 1;
    let mut missing_block = original.clone();
    missing_block.blocks.pop();
    for (claim, expected) in [
        (wrong_creator, Error::WrongCreator),
        (unsigned, Error::InvalidSignature),
        (wrong_signature, Error::InvalidSignature),
        (wrong_content, Error::InvalidHash),
        (duplicate, Error::DuplicateBlock),
        (wrong_round, Error::WrongRound),
        (missing_block, Error::TooFewBlocks),
    ] {
        assert_eq!(
            verify_equivocation_evidence(&claim, &Blocklace::new()),
            Err(expected)
        );
    }
}

#[test]
fn honest_sequential_blocks_are_not_equivocation() {
    let parent = block(1, 1, HashSet::new());
    let child = block(1, 2, HashSet::from([parent.identity.clone()]));
    let mut lace = Blocklace::new();
    lace.insert(parent.clone(), &Secp256k1Scheme).unwrap();
    let claim = CordialEquivocationEvidence::new(node(1), 0, vec![parent, child]);
    assert_eq!(
        verify_equivocation_evidence(&claim, &lace),
        Err(EvidenceVerificationError::ComparableBlocks)
    );
}

#[test]
fn missing_and_partial_history_cannot_prove_incomparability() {
    let root = block(2, 1, HashSet::new());
    let boundary = block(2, 2, HashSet::from([root.identity.clone()]));
    let branch = block(1, 3, HashSet::from([boundary.identity.clone()]));
    let other = block(1, 4, HashSet::new());
    let claim = CordialEquivocationEvidence::new(node(1), 0, vec![branch, other]);
    let mut lace = Blocklace::new();
    assert_eq!(
        verify_equivocation_evidence(&claim, &lace),
        Err(EvidenceVerificationError::MissingHistory)
    );
    lace.insert_with_trusted_boundary(boundary, &Secp256k1Scheme)
        .unwrap();
    assert_eq!(
        verify_equivocation_evidence(&claim, &lace),
        Err(EvidenceVerificationError::MissingHistory)
    );
}

#[test]
fn incomparable_blocks_of_different_depths_use_the_earliest_round() {
    let parent = block(2, 1, HashSet::new());
    let mut lace = Blocklace::new();
    lace.insert(parent.clone(), &Secp256k1Scheme).unwrap();
    let left = block(1, 2, HashSet::new());
    let right = block(1, 3, HashSet::from([parent.identity]));
    let claim = CordialEquivocationEvidence::new(node(1), 0, vec![left, right]);
    assert_eq!(
        verify_equivocation_evidence(&claim, &lace).unwrap().round(),
        0
    );
}

#[test]
fn decoded_proofs_still_require_authentication_and_complete_ancestry() {
    use cordial_miners_core::consensus::decode_equivocation_evidence;
    let parent = block(2, 7, HashSet::new());
    let mut lace = Blocklace::new();
    lace.insert(parent.clone(), &Secp256k1Scheme).unwrap();
    let claim = CordialEquivocationEvidence::new(
        node(1),
        1,
        vec![
            block(1, 8, HashSet::from([parent.identity.clone()])),
            block(1, 9, HashSet::from([parent.identity])),
        ],
    );
    let proof = verify_equivocation_evidence(&claim, &lace).unwrap();
    let bytes = proof.canonical_bytes();
    let mut decoded = decode_equivocation_evidence(&bytes).unwrap();
    assert_eq!(
        verify_equivocation_evidence(&decoded, &lace)
            .unwrap()
            .canonical_bytes(),
        bytes
    );
    assert_eq!(
        verify_equivocation_evidence(&decoded, &Blocklace::new()),
        Err(EvidenceVerificationError::MissingHistory)
    );
    decoded.blocks[0].identity.signature.clear();
    assert_eq!(
        verify_equivocation_evidence(&decoded, &lace),
        Err(EvidenceVerificationError::InvalidSignature)
    );
    for end in 0..bytes.len() {
        assert!(decode_equivocation_evidence(&bytes[..end]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_equivocation_evidence(&trailing).is_err());
    assert!(decode_equivocation_evidence(&[1]).is_err());
}

#[test]
fn stored_signature_variants_do_not_change_proof_validity() {
    use k256::ecdsa::{Signature, signature::hazmat::RandomizedPrehashSigner};
    use k256::elliptic_curve::rand_core::OsRng;
    let claim = raw();
    let expected = verify_equivocation_evidence(&claim, &Blocklace::new()).unwrap();
    let mut variant = claim.blocks[0].clone();
    let signature: Signature = SigningKey::from_slice(&[1; 32])
        .unwrap()
        .sign_prehash_with_rng(&mut OsRng, &variant.identity.content_hash)
        .unwrap();
    variant.identity.signature = signature.to_der().as_bytes().to_vec();
    assert_ne!(
        variant.identity.signature,
        claim.blocks[0].identity.signature
    );
    let mut lace = Blocklace::new();
    lace.insert(variant, &Secp256k1Scheme).unwrap();
    assert_eq!(
        verify_equivocation_evidence(&claim, &lace).unwrap(),
        expected
    );
    // Signed predecessor references also resolve by consensus identity.
    let parent = claim.blocks[0].clone();
    let descendants = CordialEquivocationEvidence::new(
        node(2),
        1,
        vec![
            block(2, 3, HashSet::from([parent.identity.clone()])),
            block(2, 4, HashSet::from([parent.identity.clone()])),
        ],
    );
    let mut original_history = Blocklace::new();
    original_history.insert(parent, &Secp256k1Scheme).unwrap();
    assert_eq!(
        verify_equivocation_evidence(&descendants, &lace).unwrap(),
        verify_equivocation_evidence(&descendants, &original_history).unwrap()
    );
    // The insertion boundary still rejects a second signature variant.
    assert!(
        lace.insert(claim.blocks[0].clone(), &Secp256k1Scheme)
            .is_err()
    );
}
