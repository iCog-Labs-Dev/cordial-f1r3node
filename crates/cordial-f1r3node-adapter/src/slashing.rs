//! Boundary between Cordial equivocation evidence and f1r3node slash deploys.
//!
//! The core crate retains raw equivocation evidence without host-node types.
//! The current host requires a verified native block hash and activation epoch
//! to authorize a slash. Neither is present in the core evidence, so formatting
//! fails until that host metadata is carried across the adapter boundary.

use anyhow::{Result, bail};

use cordial_miners_core::block::Block;
use cordial_miners_core::consensus::EquivocationEvidence;
use cordial_miners_core::types::{BlockIdentity, NodeId};

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
    /// Adapter metadata only; the host protobuf does not carry this ratio.
    pub penalty_ratio_bps: u32,
    pub slash_system_deploy: Vec<u8>,
    pub conflicting_block_bytes: Vec<Vec<u8>>,
}

/// f1r3node formatter for Cordial Miners slash evidence.
///
/// The current evidence type cannot prove the host block hash or target epoch.
/// Nonempty evidence therefore returns an error rather than an unauthorized
/// slash deploy. `issuer_public_key` is retained for the future host-backed path.
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

    /// Retain precomputed penalties without extending the host wire format.
    /// Nonempty evidence still requires verified host metadata before emission.
    pub fn to_tiered_slash_deploys(
        &self,
        evidence: &[EquivocationEvidence<NodeId, Block, BlockIdentity>],
        penalty_ratio_bps: u32,
    ) -> Result<Vec<FormattedSlashEvidence>> {
        if penalty_ratio_bps > 10_000 {
            bail!("slash penalty must be between 0 and 10000 basis points");
        }
        let deploys = self.to_slash_system_deploys(evidence)?;
        evidence
            .iter()
            .zip(deploys)
            .map(|(record, slash_system_deploy)| {
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
        validate_evidence(record)?;
        bail!(
            "slash evidence lacks a verified f1r3node native block hash and target activation epoch"
        )
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

fn validate_evidence(record: &EquivocationEvidence<NodeId, Block, BlockIdentity>) -> Result<()> {
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

    Ok(())
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
