//! Formatting Cordial Miners equivocation evidence as f1r3node slash deploys.
//!
//! The core crate retains raw equivocation evidence without host-node types.
//! This adapter crosses that boundary and emits the byte-identical protobuf
//! payload f1r3node stores in block bodies for slash system deploys.

use anyhow::{Result, bail};
use prost::Message;
use prost::bytes::Bytes;

use cordial_miners_core::block::Block;
use cordial_miners_core::consensus::EquivocationEvidence;
use cordial_miners_core::types::{BlockIdentity, NodeId};

use models::casper::{
    ProcessedSystemDeployProto, SlashSystemDeployDataProto, SystemDeployDataProto,
    system_deploy_data_proto::SystemDeploy,
};

/// Converts pure-core equivocation evidence into host-specific slash deploy bytes.
pub trait SlashDeployFormatter<V, P, Id> {
    fn to_slash_system_deploys(
        &self,
        evidence: &[EquivocationEvidence<V, P, Id>],
    ) -> Result<Vec<Vec<u8>>>;

    /// Retain adapter-only penalty metadata alongside the unchanged host bytes.
    fn to_tiered_slash_deploys(
        &self,
        _evidence: &[EquivocationEvidence<V, P, Id>],
        _penalty_ratio_bps: u32,
    ) -> Result<Vec<FormattedSlashEvidence>> {
        bail!("this formatter does not support tiered slash evidence")
    }
}

/// Adapter-local proof package: f1r3node slash bytes plus raw Cordial block bytes.
///
/// RSpace only consumes the `slash_system_deploy` bytes. The serialized
/// `conflicting_block_bytes` are retained for later proof transport, audits,
/// and signature verification without changing the f1r3node protobuf shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedSlashEvidence {
    /// Adapter metadata only; the legacy host protobuf does not carry it.
    pub penalty_ratio_bps: u32,
    pub slash_system_deploy: Vec<u8>,
    pub conflicting_block_bytes: Vec<Vec<u8>>,
}

/// f1r3node formatter for Cordial Miners slash evidence.
///
/// `issuer_public_key` is the public key of the validator proposing the slash
/// deploy, matching CBC Casper's `SlashDeploy(invalidBlockHash, issuerPublicKey, ...)`
/// construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct F1r3SlashDeployFormatter {
    issuer_public_key: Vec<u8>,
}

impl F1r3SlashDeployFormatter {
    pub fn new(issuer_public_key: Vec<u8>) -> Self {
        Self { issuer_public_key }
    }

    pub fn issuer_public_key(&self) -> &[u8] {
        &self.issuer_public_key
    }

    pub fn to_slash_evidence_envelopes(
        &self,
        evidence: &[EquivocationEvidence<NodeId, Block, BlockIdentity>],
    ) -> Result<Vec<FormattedSlashEvidence>> {
        // The legacy path represents the original full-slash intent.
        self.to_tiered_slash_deploys(evidence, 10_000)
    }

    /// Format precomputed penalties without extending the host wire format.
    /// Actual tiered punishment requires a future host integration.
    pub fn to_tiered_slash_deploys(
        &self,
        evidence: &[EquivocationEvidence<NodeId, Block, BlockIdentity>],
        penalty_ratio_bps: u32,
    ) -> Result<Vec<FormattedSlashEvidence>> {
        if penalty_ratio_bps > 10_000 {
            bail!("slash penalty must be between 0 and 10000 basis points");
        }
        evidence
            .iter()
            .map(|record| {
                let slash_system_deploy = self.encode_record(record)?;
                let conflicting_block_bytes = serialize_conflicting_blocks(record)?;
                Ok(FormattedSlashEvidence {
                    penalty_ratio_bps,
                    slash_system_deploy,
                    conflicting_block_bytes,
                })
            })
            .collect()
    }

    fn encode_record(
        &self,
        record: &EquivocationEvidence<NodeId, Block, BlockIdentity>,
    ) -> Result<Vec<u8>> {
        let invalid_block_hash = select_invalid_block_hash(record)?;
        Ok(encode_slash_processed_system_deploy(
            invalid_block_hash,
            &self.issuer_public_key,
        ))
    }
}

impl SlashDeployFormatter<NodeId, Block, BlockIdentity> for F1r3SlashDeployFormatter {
    fn to_slash_system_deploys(
        &self,
        evidence: &[EquivocationEvidence<NodeId, Block, BlockIdentity>],
    ) -> Result<Vec<Vec<u8>>> {
        evidence
            .iter()
            .map(|record| self.encode_record(record))
            .collect()
    }

    fn to_tiered_slash_deploys(
        &self,
        evidence: &[EquivocationEvidence<NodeId, Block, BlockIdentity>],
        penalty_ratio_bps: u32,
    ) -> Result<Vec<FormattedSlashEvidence>> {
        F1r3SlashDeployFormatter::to_tiered_slash_deploys(self, evidence, penalty_ratio_bps)
    }
}

fn encode_slash_processed_system_deploy(
    invalid_block_hash: [u8; 32],
    issuer_public_key: &[u8],
) -> Vec<u8> {
    let proto = ProcessedSystemDeployProto {
        system_deploy: Some(SystemDeployDataProto {
            system_deploy: Some(SystemDeploy::SlashSystemDeploy(
                SlashSystemDeployDataProto {
                    invalid_block_hash: Bytes::copy_from_slice(&invalid_block_hash),
                    issuer_public_key: Bytes::copy_from_slice(issuer_public_key),
                },
            )),
        }),
        deploy_log: vec![],
        error_msg: String::new(),
    };

    proto.encode_to_vec()
}

fn select_invalid_block_hash(
    record: &EquivocationEvidence<NodeId, Block, BlockIdentity>,
) -> Result<[u8; 32]> {
    if record.blocks.len() < 2 {
        bail!("slash evidence requires at least two conflicting blocks");
    }

    if record
        .blocks
        .iter()
        .any(|block| block.identity.creator != record.validator)
    {
        bail!("slash evidence contains a block from a different validator");
    }

    // f1r3node's SlashDeploy names one invalid block hash. Cordial evidence
    // retains the full conflicting set separately, so choose the lowest content
    // hash as the deterministic representative independent of signature bytes.
    let mut identities = record
        .blocks
        .iter()
        .map(|block| &block.identity)
        .collect::<Vec<_>>();
    identities.sort_by_key(|identity| identity.content_hash);

    Ok(identities[0].content_hash)
}

fn serialize_conflicting_blocks(
    record: &EquivocationEvidence<NodeId, Block, BlockIdentity>,
) -> Result<Vec<Vec<u8>>> {
    let mut blocks = record.blocks.iter().collect::<Vec<_>>();
    blocks.sort_by(|a, b| a.identity.cmp(&b.identity));

    blocks
        .into_iter()
        .map(|block| bincode::serialize(block).map_err(Into::into))
        .collect()
}
