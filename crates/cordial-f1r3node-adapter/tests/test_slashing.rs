use std::collections::HashSet;

use cordial_f1r3node_adapter::slashing::{F1r3SlashDeployFormatter, SlashDeployFormatter};
use cordial_miners_core::consensus::EquivocationEvidence;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

fn node(byte: u8) -> NodeId {
    NodeId(vec![byte; 33])
}

fn block(creator: NodeId, tag: u8, signature: Vec<u8>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = tag;
    content_hash[31] = tag.wrapping_add(1);

    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature,
        },
        content: BlockContent {
            payload: vec![tag, tag.wrapping_add(2)],
            predecessors: HashSet::new(),
        },
    }
}

#[test]
fn core_evidence_cannot_be_formatted_without_verified_host_metadata() {
    let validator = node(7);
    let left = block(validator.clone(), 0x01, vec![0x10; 64]);
    let right = block(validator.clone(), 0x02, vec![0x20; 64]);
    let evidence = vec![EquivocationEvidence::new(validator, 3, vec![left, right])];
    let formatter = F1r3SlashDeployFormatter::new(vec![0xaa; 33]);

    let err = formatter.to_slash_system_deploys(&evidence).unwrap_err();
    assert!(err.to_string().contains("native block hash"));
    assert!(err.to_string().contains("target activation epoch"));
}

#[test]
fn evidence_envelope_also_rejects_unverified_host_metadata() {
    let validator = node(9);
    let left = block(validator.clone(), 0x01, vec![0x11, 0x12, 0x13]);
    let right = block(validator.clone(), 0x02, vec![0x21, 0x22, 0x23]);
    let evidence = vec![EquivocationEvidence::new(
        validator,
        4,
        vec![right.clone(), left.clone()],
    )];
    let formatter = F1r3SlashDeployFormatter::new(vec![0xbb; 33]);

    let err = formatter
        .to_slash_evidence_envelopes(&evidence)
        .unwrap_err();
    assert!(err.to_string().contains("native block hash"));
}

#[test]
fn empty_evidence_produces_no_slash_deploys_or_envelopes() {
    let formatter = F1r3SlashDeployFormatter::new(vec![0xbb; 33]);
    assert!(formatter.to_slash_system_deploys(&[]).unwrap().is_empty());
    assert!(
        formatter
            .to_slash_evidence_envelopes(&[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn slash_formatter_rejects_non_conflicting_single_block_evidence() {
    let validator = node(1);
    let only = block(validator.clone(), 0x01, vec![0x55]);
    let evidence = vec![EquivocationEvidence::new(validator, 0, vec![only])];
    let formatter = F1r3SlashDeployFormatter::new(vec![0xcc; 33]);

    let err = formatter.to_slash_system_deploys(&evidence).unwrap_err();

    assert!(err.to_string().contains("at least two conflicting blocks"));
}

#[test]
fn tiered_evidence_also_rejects_unverified_host_metadata_for_both_tiers() {
    let validator = node(7);
    let left = block(validator.clone(), 1, vec![0x11; 64]);
    let right = block(validator.clone(), 2, vec![0x22; 64]);
    let evidence = vec![EquivocationEvidence::new(
        validator,
        4,
        vec![right.clone(), left.clone()],
    )];
    let formatter = F1r3SlashDeployFormatter::new(vec![0xaa; 33]);
    for ratio in [2500, 10000] {
        let err = formatter
            .to_tiered_slash_deploys(&evidence, ratio)
            .unwrap_err();
        assert!(err.to_string().contains("native block hash"));
        assert!(err.to_string().contains("target activation epoch"));
    }
}

#[test]
fn tiered_formatter_rejects_out_of_range_ratios_even_without_evidence() {
    let formatter = F1r3SlashDeployFormatter::new(vec![0xaa; 33]);
    for ratio in [10001, u32::MAX] {
        assert!(formatter.to_tiered_slash_deploys(&[], ratio).is_err());
    }
}

#[test]
fn empty_tiered_evidence_produces_no_envelopes() {
    let formatter = F1r3SlashDeployFormatter::new(vec![0xaa; 33]);
    for ratio in [0, 2500, 10000] {
        assert!(
            formatter
                .to_tiered_slash_deploys(&[], ratio)
                .unwrap()
                .is_empty()
        );
    }
}
