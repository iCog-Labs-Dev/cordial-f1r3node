//! `CasperShardConf` equivalent (Phase 3.6).
//!
//! f1r3node's `CasperShardConf` is a large config struct carrying everything
//! from fault-tolerance thresholds to mergeable-channel GC tuning knobs.
//! This module mirrors it so our adapter's [`super::snapshot::CasperSnapshot`]
//! carries a plausible config through the `Casper` trait.
//!
//! ## Design decisions
//!
//! - **Plain data struct, no serde.** f1r3node uses HOCON via serde for
//!   config parsing. We're not a config loader — consumers build a
//!   `CasperShardConf` in Rust by calling `Default::default()` or
//!   `from_cordial()` and mutating fields directly.
//!
//! - **Durations modelled with `std::time::Duration`.** Matches f1r3node.
//!
//! - **Single source of truth for overlapping fields.** Our
//!   [`blocklace::execution::DeployPoolConfig`] already carries
//!   `max_user_deploys_per_block`, `deploy_lifespan`, and `min_phlo_price`.
//!   `CasperShardConf::from_cordial()` imports those three plus a shard
//!   name and leaves everything else at sensible defaults. This keeps the
//!   blocklace-native and f1r3node configs from drifting.
//!
//! - **Fields we don't use.** Many `CasperShardConf` fields configure
//!   behaviour the blocklace doesn't have (synchrony constraint recovery,
//!   mergeable-channel GC, epoch length). We keep them in the mirror for
//!   wire parity, but our consensus logic never reads them. When the
//!   Casper trait adapter lands (3.1), it will pass them through to f1r3node
//!   callers that do care.
//!
//! ## Relation to [`super::snapshot::CasperShardConf`]
//!
//! Phase 3.3 introduced a minimal `CasperShardConf` inside the `snapshot`
//! module with just the fields `build_snapshot` needs. That type now
//! becomes a slice of this fuller one — see [`CasperShardConf::to_snapshot_conf`].

use std::time::Duration;

use cordial_miners_core::execution::DeployPoolConfig;

/// Finalizer timing / budget configuration. Mirror of f1r3node's
/// `casper_conf::FinalizerConf`.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalizerConf {
    pub work_budget: Duration,
    pub step_timeout: Duration,
    pub catchup_work_budget: Duration,
    pub catchup_step_timeout: Duration,
}

impl Default for FinalizerConf {
    fn default() -> Self {
        // Matches f1r3node defaults in casper_conf.rs:
        //   work_budget = 8s, step_timeout = 1s,
        //   catchup_work_budget = 8s, catchup_step_timeout = 1s
        Self {
            work_budget: Duration::from_secs(8),
            step_timeout: Duration::from_secs(1),
            catchup_work_budget: Duration::from_secs(8),
            catchup_step_timeout: Duration::from_secs(1),
        }
    }
}

/// Full mirror of f1r3node's `CasperShardConf`.
///
/// Every field is documented with its f1r3node default so consumers can
/// produce config that matches a real f1r3node node without guessing.
/// Defaults match `casper::rust::casper::CasperShardConf::new()`.
#[derive(Debug, Clone, PartialEq)]
pub struct CasperShardConf {
    /// Threshold (0.0–1.0) above which a block is considered finalized.
    ///
    /// Cordial Miners' finality is a strict 2/3 supermajority check; this
    /// field exists for wire parity with f1r3node and is typically set to
    /// something close to `0.333` to match the 2/3 threshold.
    pub fault_tolerance_threshold: f32,

    /// Shard identifier string. Used in `BlockMessage.shard_id` during
    /// translation.
    pub shard_name: String,

    /// Parent shard identifier (hierarchical sharding). Empty for root.
    pub parent_shard_id: String,

    /// How often finalization runs (in blocks). Not used by Cordial Miners
    /// finality, which runs on every check.
    pub finalization_rate: i32,

    /// Upper bound on parents per block. Not enforced by the blocklace's
    /// unbounded predecessor set — f1r3node consumers may impose their own
    /// limit when calling `block_to_message`.
    pub max_number_of_parents: i32,

    /// How far back the fork-choice considers parents. Mirror field; our
    /// `fork_choice` doesn't prune by depth.
    pub max_parent_depth: i32,

    /// Synchrony-constraint fault-tolerance threshold. Unused by us.
    pub synchrony_constraint_threshold: f32,

    /// Synchrony-constraint height threshold. Unused by us.
    pub height_constraint_threshold: i64,

    /// Deploy validity window in blocks. Matches
    /// `DeployPoolConfig::deploy_lifespan`.
    pub deploy_lifespan: i64,

    /// Casper protocol version (advisory).
    pub casper_version: i64,

    /// Config schema version (advisory).
    pub config_version: i64,

    /// Minimum stake a validator can bond.
    pub bond_minimum: i64,

    /// Maximum stake a validator can bond.
    pub bond_maximum: i64,

    /// Blocks per epoch (for epoch-based rotation, unused by us).
    pub epoch_length: i32,

    /// Blocks a slashed validator stays quarantined.
    pub quarantine_length: i32,

    /// Minimum phlo price a deploy must pay. Matches
    /// `DeployPoolConfig::min_phlo_price`.
    pub min_phlo_price: i64,

    /// Disable late-block filtering in DAG merging (testing flag).
    pub disable_late_block_filtering: bool,

    /// Disable validator-progress check (for standalone mode).
    pub disable_validator_progress_check: bool,

    /// Enable mergeable-channel GC (multi-parent mode only).
    pub enable_mergeable_channel_gc: bool,

    /// Safety margin for mergeable-channel GC depth.
    pub mergeable_channels_gc_depth_buffer: i32,

    /// Finalizer timing / work budget.
    pub finalizer_conf: FinalizerConf,

    /// How long a stalled node waits before triggering synchrony recovery.
    pub synchrony_recovery_stall_window: Duration,

    /// Cooldown between synchrony recovery bypasses.
    pub synchrony_recovery_cooldown: Duration,

    /// Max number of synchrony recovery bypasses allowed.
    pub synchrony_recovery_max_bypasses: u32,

    /// Whether the finalized-baseline synchrony floor is enabled.
    pub synchrony_finalized_baseline_enabled: bool,

    /// Max distance (blocks) the baseline can trail the finalized tip.
    pub synchrony_finalized_baseline_max_distance: u64,

    /// Maximum user deploys per block. Matches
    /// `DeployPoolConfig::max_user_deploys_per_block`.
    pub max_user_deploys_per_block: u32,
}

impl Default for CasperShardConf {
    fn default() -> Self {
        // Matches CasperShardConf::new() in f1r3node.
        Self {
            fault_tolerance_threshold: 0.0,
            shard_name: String::new(),
            parent_shard_id: String::new(),
            finalization_rate: 0,
            max_number_of_parents: 0,
            max_parent_depth: 0,
            synchrony_constraint_threshold: 0.0,
            height_constraint_threshold: 0,
            deploy_lifespan: 0,
            casper_version: 0,
            config_version: 0,
            bond_minimum: 0,
            bond_maximum: 0,
            epoch_length: 0,
            quarantine_length: 0,
            min_phlo_price: 0,
            disable_late_block_filtering: true,
            disable_validator_progress_check: false,
            enable_mergeable_channel_gc: false,
            mergeable_channels_gc_depth_buffer: 10,
            finalizer_conf: FinalizerConf::default(),
            synchrony_recovery_stall_window: Duration::from_secs(60),
            synchrony_recovery_cooldown: Duration::from_secs(20),
            synchrony_recovery_max_bypasses: 2,
            synchrony_finalized_baseline_enabled: true,
            synchrony_finalized_baseline_max_distance: 2048,
            max_user_deploys_per_block: 32,
        }
    }
}

impl CasperShardConf {
    /// Build a `CasperShardConf` from the Cordial-native
    /// [`DeployPoolConfig`], filling in a shard name and leaving other
    /// f1r3node-specific knobs at their defaults.
    ///
    /// This is the recommended starting point when setting up a Casper
    /// adapter: the deploy-related fields stay aligned between our
    /// `DeployPool` and the snapshot the adapter exposes to f1r3node.
    pub fn from_cordial(pool: &DeployPoolConfig, shard_name: impl Into<String>) -> Self {
        Self {
            shard_name: shard_name.into(),
            deploy_lifespan: i64::try_from(pool.deploy_lifespan).unwrap_or(i64::MAX),
            min_phlo_price: i64::try_from(pool.min_phlo_price).unwrap_or(i64::MAX),
            max_user_deploys_per_block: u32::try_from(pool.max_user_deploys_per_block)
                .unwrap_or(u32::MAX),
            // A conservative fault-tolerance threshold matching the 2/3
            // supermajority Cordial Miners uses for finality. Callers can
            // still override this after construction.
            fault_tolerance_threshold: 0.333,
            ..Self::default()
        }
    }

    /// Project this config down to the snapshot-module's minimal form.
    ///
    /// Used when building a [`super::snapshot::CasperSnapshot`], which only
    /// needs a handful of the fields.
    pub fn to_snapshot_conf(&self) -> crate::snapshot::CasperShardConf {
        crate::snapshot::CasperShardConf {
            fault_tolerance_threshold: self.fault_tolerance_threshold,
            shard_name: self.shard_name.clone(),
            max_number_of_parents: self.max_number_of_parents,
            max_parent_depth: if self.max_parent_depth > 0 {
                Some(self.max_parent_depth)
            } else {
                None
            },
            deploy_lifespan: self.deploy_lifespan,
            min_phlo_price: self.min_phlo_price,
        }
    }
}
