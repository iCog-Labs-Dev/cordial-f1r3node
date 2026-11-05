//! Deterministic authenticated forks for structural slashing conformance.
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{
        CordialEquivocationEvidence, VerifiedEquivocationEvidence, verify_equivocation_evidence,
    },
    crypto,
};
use cordial_por::{
    EquivocationAuthorizationContext, EquivocationAuthorizationPolicy, EquivocationPenalty,
    PorError,
};
use k256::ecdsa::SigningKey;
use std::{collections::HashSet, sync::OnceLock};

fn keys() -> &'static Vec<(NodeId, [u8; 32])> {
    static KEYS: OnceLock<Vec<(NodeId, [u8; 32])>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys: Vec<_> = (1..=8)
            .map(|seed| {
                let private = [seed; 32];
                let signing = SigningKey::from_slice(&private).unwrap();
                let node = NodeId(
                    signing
                        .verifying_key()
                        .to_encoded_point(true)
                        .as_bytes()
                        .to_vec(),
                );
                (node, private)
            })
            .collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        keys
    })
}
pub fn node(id: u8) -> NodeId {
    keys()[usize::from(id - 1)].0.clone()
}

// Explicit test authorization policy. Production uses finalized Cordial payloads.
struct TestAuthorization;
impl EquivocationAuthorizationPolicy for TestAuthorization {
    fn authorize(
        &self,
        _: &EquivocationAuthorizationContext,
        _: &VerifiedEquivocationEvidence,
    ) -> Result<(), PorError> {
        Ok(())
    }
}

fn proof(id: u8, tag: u8) -> VerifiedEquivocationEvidence {
    let blocks = [0, 1].map(|branch| {
        let content = BlockContent {
            payload: vec![tag, branch],
            predecessors: HashSet::new(),
        };
        let creator = node(id);
        let content_hash = crypto::block_hash(&creator, &content);
        Block {
            identity: BlockIdentity {
                content_hash,
                creator,
                signature: crypto::sign(&content_hash, &keys()[usize::from(id - 1)].1),
            },
            content,
        }
    });
    verify_equivocation_evidence(
        &CordialEquivocationEvidence::new(node(id), 0, blocks.to_vec()),
        &Blocklace::new(),
    )
    .unwrap()
}

pub fn penalty_at(id: u8, tag: u8, round: u64) -> EquivocationPenalty {
    EquivocationPenalty::authorize(
        &proof(id, tag),
        EquivocationAuthorizationContext {
            shard_id: b"root".to_vec(),
            source_finalized_wave: round - 1,
            round,
        },
        &TestAuthorization,
    )
    .unwrap()
}
