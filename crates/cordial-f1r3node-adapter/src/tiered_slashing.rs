//! Adapter-only tiered slash preparation. Host encoding and execution remain
//! backward-compatible; these types do not enforce runtime punishment or order.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result, bail};
use cordial_miners_core::NodeId;
use cordial_miners_core::execution::{ExecutionRequest, SignedDeploy};
use cordial_por::{PorConfig, compute_slash_penalty};

use crate::slashing::FormattedSlashEvidence;

#[derive(Debug, Clone)]
struct RoundWeights {
    validators: HashMap<NodeId, u64>,
    total: u64,
}

/// Immutable reputation snapshots indexed by the core evidence round.
///
/// Callers supply the Cordial-authorized active reputation projection for each
/// round. The proposer never substitutes its current bond table, a PoR rating
/// round, or a block number for these explicitly bound historical inputs.
#[derive(Debug, Clone)]
pub struct TieredSlashPolicy {
    config: PorConfig,
    rounds: BTreeMap<u64, RoundWeights>,
}

impl TieredSlashPolicy {
    pub fn new(config: PorConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            rounds: BTreeMap::new(),
        })
    }

    pub fn with_round_weights(
        mut self,
        round: u64,
        validators: HashMap<NodeId, u64>,
    ) -> Result<Self> {
        if self.rounds.contains_key(&round) {
            bail!("reputation weights already configured for evidence round {round}");
        }
        // The public PoR helper accepts u64 totals. Fail explicitly instead of
        // truncating a wider total or duplicating its correlation formula here.
        let total = validators.values().try_fold(0u64, |total, weight| {
            total
                .checked_add(*weight)
                .context("total reputation weight exceeds u64")
        })?;
        if total == 0 {
            bail!("evidence round {round} must have positive total reputation weight");
        }
        self.rounds
            .insert(round, RoundWeights { validators, total });
        Ok(self)
    }

    pub fn penalty_ratio_bps(&self, round: u64, offenders: &[NodeId]) -> Result<u32> {
        let snapshot = self
            .rounds
            .get(&round)
            .with_context(|| format!("missing reputation weights for evidence round {round}"))?;
        let offenders = offenders.iter().collect::<BTreeSet<_>>();
        if offenders.is_empty() {
            bail!("a slash penalty requires at least one equivocating validator");
        }
        let equivocating_weight = offenders.into_iter().try_fold(0u64, |total, offender| {
            let weight = snapshot.validators.get(offender).with_context(|| {
                format!("unknown equivocating validator in evidence round {round}")
            })?;
            if *weight == 0 {
                bail!("equivocating validator has zero active weight in evidence round {round}");
            }
            total
                .checked_add(*weight)
                .context("equivocating reputation weight exceeds u64")
        })?;
        let penalty = compute_slash_penalty(equivocating_weight, snapshot.total, &self.config)?;
        let scaled = u128::from(penalty) * 10_000;
        let scale = u128::from(self.config.scale);
        if !scaled.is_multiple_of(scale) {
            bail!("slash penalty cannot be represented exactly in basis points");
        }
        u32::try_from(scaled / scale).context("slash penalty basis points exceed u32")
    }
}

/// Logical adapter preparation order, not an assertion about host execution.
#[derive(Debug, Clone)]
pub enum ExecutionBatchItem {
    SlashSystemDeploy {
        validator: NodeId,
        round: u64,
        evidence: FormattedSlashEvidence,
    },
    UserDeploy(SignedDeploy),
    CloseBlock,
}

/// Read-only prepared batch retaining metadata until the execution boundary.
#[derive(Debug, Clone)]
pub struct PreparedExecutionRequest {
    request: ExecutionRequest,
    batch: Vec<ExecutionBatchItem>,
}

impl PreparedExecutionRequest {
    pub(crate) fn new(request: ExecutionRequest, batch: Vec<ExecutionBatchItem>) -> Self {
        Self { request, batch }
    }

    pub fn batch(&self) -> &[ExecutionBatchItem] {
        &self.batch
    }

    pub fn legacy_request(&self) -> &ExecutionRequest {
        &self.request
    }

    /// Project onto the existing runtime interface. This intentionally drops
    /// adapter-only penalty metadata and does not change host execution order.
    pub fn into_legacy_request(self) -> ExecutionRequest {
        self.request
    }
}
