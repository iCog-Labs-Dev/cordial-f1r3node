//! Tests for `shard_conf` — `CasperShardConf` equivalent (Phase 3.6).

use std::time::Duration;

use cordial_miners_core::execution::DeployPoolConfig;

use cordial_f1r3node_adapter::shard_conf::{CasperShardConf, FinalizerConf};

// ── FinalizerConf defaults ──────────────────────────────────────────────

#[test]
fn finalizer_conf_default_matches_f1r3node() {
    let c = FinalizerConf::default();
    assert_eq!(c.work_budget, Duration::from_secs(8));
    assert_eq!(c.step_timeout, Duration::from_secs(1));
    assert_eq!(c.catchup_work_budget, Duration::from_secs(8));
    assert_eq!(c.catchup_step_timeout, Duration::from_secs(1));
}

// ── CasperShardConf defaults ────────────────────────────────────────────

#[test]
fn casper_shard_conf_default_matches_f1r3node_new() {
    // Matches f1r3node's `CasperShardConf::new()` field-by-field.
    let c = CasperShardConf::default();
    assert_eq!(c.fault_tolerance_threshold, 0.0);
    assert_eq!(c.shard_name, "");
    assert_eq!(c.parent_shard_id, "");
    assert_eq!(c.finalization_rate, 0);
    assert_eq!(c.max_number_of_parents, 0);
    assert_eq!(c.max_parent_depth, 0);
    assert_eq!(c.synchrony_constraint_threshold, 0.0);
    assert_eq!(c.height_constraint_threshold, 0);
    assert_eq!(c.deploy_lifespan, 0);
    assert_eq!(c.casper_version, 0);
    assert_eq!(c.config_version, 0);
    assert_eq!(c.bond_minimum, 0);
    assert_eq!(c.bond_maximum, 0);
    assert_eq!(c.epoch_length, 0);
    assert_eq!(c.quarantine_length, 0);
    assert_eq!(c.min_phlo_price, 0);
    assert!(c.disable_late_block_filtering);
    assert!(!c.disable_validator_progress_check);
    assert!(!c.enable_mergeable_channel_gc);
    assert_eq!(c.mergeable_channels_gc_depth_buffer, 10);
    assert_eq!(c.finalizer_conf, FinalizerConf::default());
    assert_eq!(c.synchrony_recovery_stall_window, Duration::from_secs(60));
    assert_eq!(c.synchrony_recovery_cooldown, Duration::from_secs(20));
    assert_eq!(c.synchrony_recovery_max_bypasses, 2);
    assert!(c.synchrony_finalized_baseline_enabled);
    assert_eq!(c.synchrony_finalized_baseline_max_distance, 2048);
    assert_eq!(c.max_user_deploys_per_block, 32);
}

// ── from_cordial(): import DeployPoolConfig fields ──────────────────────

#[test]
fn from_cordial_carries_deploy_pool_fields() {
    let pool = DeployPoolConfig {
        max_user_deploys_per_block: 16,
        deploy_lifespan: 100,
        min_phlo_price: 5,
    };
    let c = CasperShardConf::from_cordial(&pool, "my-shard");

    assert_eq!(c.shard_name, "my-shard");
    assert_eq!(c.deploy_lifespan, 100);
    assert_eq!(c.min_phlo_price, 5);
    assert_eq!(c.max_user_deploys_per_block, 16);

    // A sane default threshold matching the 2/3 supermajority
    assert!((c.fault_tolerance_threshold - 0.333).abs() < 1e-6);
}

#[test]
fn from_cordial_leaves_other_fields_at_defaults() {
    let pool = DeployPoolConfig::default();
    let c = CasperShardConf::from_cordial(&pool, "root");
    let d = CasperShardConf::default();

    // The fields NOT derived from DeployPoolConfig should match defaults.
    assert_eq!(c.parent_shard_id, d.parent_shard_id);
    assert_eq!(c.finalization_rate, d.finalization_rate);
    assert_eq!(c.max_number_of_parents, d.max_number_of_parents);
    assert_eq!(c.max_parent_depth, d.max_parent_depth);
    assert_eq!(c.bond_minimum, d.bond_minimum);
    assert_eq!(c.bond_maximum, d.bond_maximum);
    assert_eq!(c.epoch_length, d.epoch_length);
    assert_eq!(c.quarantine_length, d.quarantine_length);
    assert_eq!(c.disable_late_block_filtering, d.disable_late_block_filtering);
    assert_eq!(c.enable_mergeable_channel_gc, d.enable_mergeable_channel_gc);
    assert_eq!(c.finalizer_conf, d.finalizer_conf);
    assert_eq!(c.synchrony_recovery_max_bypasses, d.synchrony_recovery_max_bypasses);
    assert_eq!(
        c.synchrony_finalized_baseline_max_distance,
        d.synchrony_finalized_baseline_max_distance
    );
}

#[test]
fn from_cordial_clamps_overflowing_pool_values() {
    // Construct a pathological pool with u64/usize values that can't fit
    // into f1r3node's i64/u32 fields. These should saturate rather than
    // wrap or panic.
    let pool = DeployPoolConfig {
        max_user_deploys_per_block: usize::MAX,
        deploy_lifespan: u64::MAX,
        min_phlo_price: u64::MAX,
    };
    let c = CasperShardConf::from_cordial(&pool, "edge");
    assert_eq!(c.max_user_deploys_per_block, u32::MAX);
    assert_eq!(c.deploy_lifespan, i64::MAX);
    assert_eq!(c.min_phlo_price, i64::MAX);
}

// ── to_snapshot_conf(): project full conf down ──────────────────────────

#[test]
fn to_snapshot_conf_copies_relevant_fields() {
    let pool = DeployPoolConfig {
        max_user_deploys_per_block: 8,
        deploy_lifespan: 42,
        min_phlo_price: 7,
    };
    let mut c = CasperShardConf::from_cordial(&pool, "root");
    c.max_number_of_parents = 4;
    c.max_parent_depth = 0; // zero → None in the projection

    let proj = c.to_snapshot_conf();
    assert!((proj.fault_tolerance_threshold - 0.333).abs() < 1e-6);
    assert_eq!(proj.shard_name, "root");
    assert_eq!(proj.max_number_of_parents, 4);
    assert_eq!(proj.max_parent_depth, None);
    assert_eq!(proj.deploy_lifespan, 42);
    assert_eq!(proj.min_phlo_price, 7);
}

#[test]
fn to_snapshot_conf_maps_positive_max_parent_depth_to_some() {
    let mut c = CasperShardConf::default();
    c.max_parent_depth = 16;
    let proj = c.to_snapshot_conf();
    assert_eq!(proj.max_parent_depth, Some(16));
}

// ── Clone / equality sanity ────────────────────────────────────────────

#[test]
fn casper_shard_conf_is_cloneable_and_comparable() {
    let a = CasperShardConf::from_cordial(&DeployPoolConfig::default(), "s1");
    let b = a.clone();
    assert_eq!(a, b);

    let mut c = a.clone();
    c.shard_name = "s2".to_string();
    assert_ne!(a, c);
}
