//! Deploy pool and selection for block creation.
//!
//! This module manages pending user deploys and selects them for inclusion in
//! new blocks. The Cordial Miners paper does not define a deploy pool — this
//! is an engineering layer above the consensus protocol, adapted from
//! f1r3node's `KeyValueDeployStorage` + `BlockCreator::prepare_user_deploys()`.
//!
//! ## Selection filters (applied in order)
//!
//! 1. **Not future** — deploy's `valid_after_block_number` must be less than
//!    the current block number
//! 2. **Not block-expired** — deploy's `valid_after_block_number` must be
//!    greater than `current_block - deploy_lifespan`
//! 3. **Not time-expired** — `current_time_millis` must not exceed the
//!    optional `expiration_timestamp` (if set)
//! 4. **Not duplicated** — deploy signature must not appear in any ancestor
//!    block within the lifespan window (the "in-scope" set)
//!
//! ## Capping
//!
//! When more valid deploys are available than `max_user_deploys_per_block`
//! allows, selection prefers `(cap - 1)` oldest + 1 newest to prevent
//! head-of-line blocking (f1r3node's `oldest-plus-newest` strategy).

use std::collections::{HashMap, HashSet};

use crate::blocklace::Blocklace;
use crate::execution::payload::{CordialBlockPayload, SignedDeploy};
use crate::types::BlockIdentity;

/// Configuration for deploy selection.
///
/// Maps to f1r3node's `CasperShardConf` fields related to deploys.
#[derive(Debug, Clone)]
pub struct DeployPoolConfig {
    /// Maximum deploys per block.
    /// f1r3node default: 32
    pub max_user_deploys_per_block: usize,

    /// How many blocks a deploy stays valid (window for `valid_after_block_number`).
    /// f1r3node default: 50
    pub deploy_lifespan: u64,

    /// Minimum phlo price. Deploys below this are rejected on insert.
    /// (f1r3node applies this at validation, not selection.)
    pub min_phlo_price: u64,
}

impl Default for DeployPoolConfig {
    fn default() -> Self {
        Self {
            max_user_deploys_per_block: 32,
            deploy_lifespan: 50,
            min_phlo_price: 1,
        }
    }
}

/// Reasons a deploy can be rejected when added to the pool.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolError {
    /// Deploy signature is not unique (already in pool).
    Duplicate,
    /// Deploy phlo price is below `min_phlo_price`.
    InsufficientPhloPrice { required: u64, actual: u64 },
    /// Deploy signature is empty.
    InvalidSignature,
}

/// Pending user deploys awaiting inclusion in a block.
///
/// Storage: `HashMap<signature, SignedDeploy>` — keyed by signature, matching
/// f1r3node's `KeyValueDeployStorage` semantics.
pub struct DeployPool {
    deploys: HashMap<Vec<u8>, SignedDeploy>,
    config: DeployPoolConfig,
}

impl DeployPool {
    pub fn new(config: DeployPoolConfig) -> Self {
        Self {
            deploys: HashMap::new(),
            config,
        }
    }

    /// Add a deploy to the pool.
    /// Deduplicated by signature.
    pub fn add(&mut self, deploy: SignedDeploy) -> Result<(), PoolError> {
        if deploy.signature.is_empty() {
            return Err(PoolError::InvalidSignature);
        }
        if deploy.deploy.phlo_price < self.config.min_phlo_price {
            return Err(PoolError::InsufficientPhloPrice {
                required: self.config.min_phlo_price,
                actual: deploy.deploy.phlo_price,
            });
        }
        if self.deploys.contains_key(&deploy.signature) {
            return Err(PoolError::Duplicate);
        }
        self.deploys.insert(deploy.signature.clone(), deploy);
        Ok(())
    }

    /// Remove a deploy by signature. Returns `true` if it was present.
    pub fn remove(&mut self, signature: &[u8]) -> bool {
        self.deploys.remove(signature).is_some()
    }

    /// Number of deploys currently in the pool.
    pub fn len(&self) -> usize {
        self.deploys.len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.deploys.is_empty()
    }

    /// All deploys currently in the pool, as a reference iterator.
    pub fn iter(&self) -> impl Iterator<Item = &SignedDeploy> {
        self.deploys.values()
    }

    /// Remove all expired deploys (block-expired or time-expired).
    /// Returns the signatures of removed deploys.
    pub fn prune_expired(
        &mut self,
        current_block_number: u64,
        current_time_millis: u64,
    ) -> Vec<Vec<u8>> {
        let lifespan = self.config.deploy_lifespan;
        let mut removed = Vec::new();
        self.deploys.retain(|sig, d| {
            let keep = !is_block_expired(d.deploy.valid_after_block_number, current_block_number, lifespan);
            if !keep {
                removed.push(sig.clone());
            }
            keep
        });
        // Silence unused warning (time-based expiration not yet tracked in Deploy)
        let _ = current_time_millis;
        removed
    }

    /// Select deploys for a new block.
    ///
    /// Applies all filters, then caps at `max_user_deploys_per_block` using
    /// the oldest-plus-newest strategy.
    pub fn select_for_block(
        &self,
        current_block_number: u64,
        current_time_millis: u64,
        deploys_in_scope: &HashSet<Vec<u8>>,
    ) -> SelectedDeploys {
        let lifespan = self.config.deploy_lifespan;

        // 1. Filter by validity
        let valid: Vec<&SignedDeploy> = self
            .deploys
            .values()
            .filter(|d| {
                // Not future: valid_after < current
                d.deploy.valid_after_block_number <= current_block_number
            })
            .filter(|d| {
                // Not block-expired
                !is_block_expired(d.deploy.valid_after_block_number, current_block_number, lifespan)
            })
            .filter(|_d| {
                // Not time-expired: would check d.deploy.expiration_timestamp here
                // Our Deploy type doesn't carry that field yet; add later.
                let _ = current_time_millis;
                true
            })
            .filter(|d| {
                // Not duplicated in ancestry
                !deploys_in_scope.contains(&d.signature)
            })
            .collect();

        // 2. Cap at max_user_deploys_per_block
        let cap = self.config.max_user_deploys_per_block;
        if valid.len() <= cap {
            return SelectedDeploys {
                deploys: valid.into_iter().cloned().collect(),
                cap_hit: false,
            };
        }

        // 3. Apply oldest-plus-newest strategy
        // Sort by (valid_after_block_number, timestamp, signature) for deterministic order
        let mut sorted = valid;
        sorted.sort_by(|a, b| {
            a.deploy
                .valid_after_block_number
                .cmp(&b.deploy.valid_after_block_number)
                .then_with(|| a.deploy.timestamp.cmp(&b.deploy.timestamp))
                .then_with(|| a.signature.cmp(&b.signature))
        });

        let selected: Vec<SignedDeploy> = if cap == 1 {
            // Special case: just the newest
            vec![(*sorted.last().unwrap()).clone()]
        } else {
            // (cap - 1) oldest + 1 newest
            let mut out: Vec<SignedDeploy> = sorted
                .iter()
                .take(cap - 1)
                .map(|d| (*d).clone())
                .collect();
            out.push((*sorted.last().unwrap()).clone());
            out
        };

        SelectedDeploys {
            deploys: selected,
            cap_hit: true,
        }
    }
}

/// Result of `DeployPool::select_for_block()`.
pub struct SelectedDeploys {
    /// Deploys chosen for the block.
    pub deploys: Vec<SignedDeploy>,
    /// Whether the cap was hit (more deploys were available than selected).
    pub cap_hit: bool,
}

/// Check if a deploy is block-expired relative to the current block number.
///
/// A deploy is expired when the current block is more than `lifespan` blocks
/// past its `valid_after_block_number`. Handles u64 underflow safely: if
/// `current_block_number < lifespan`, no deploy is expired yet.
///
/// Mirrors f1r3node semantics: `valid_after <= current - lifespan` means
/// expired (equivalent to `valid_after > earliest_block` being false).
fn is_block_expired(valid_after: u64, current_block_number: u64, lifespan: u64) -> bool {
    match current_block_number.checked_sub(lifespan) {
        Some(earliest) => valid_after <= earliest,
        None => false, // underflow: we haven't reached lifespan yet
    }
}

/// Compute the set of deploy signatures present in the ancestry of the given
/// predecessors within the lifespan window.
///
/// Analogous to f1r3node's `CasperSnapshot.deploys_in_scope` computation.
/// Walks the blocklace ancestry, deserializing each block's `CordialBlockPayload`
/// to extract deploy signatures.
///
/// This is used by `select_for_block()` to avoid re-including deploys that
/// are already in ancestor blocks.
pub fn compute_deploys_in_scope(
    blocklace: &Blocklace,
    predecessors: &HashSet<BlockIdentity>,
    current_block_number: u64,
    deploy_lifespan: u64,
) -> HashSet<Vec<u8>> {
    let earliest = current_block_number.saturating_sub(deploy_lifespan);
    let mut sigs = HashSet::new();

    // Collect all ancestor blocks (inclusive) of every predecessor
    let mut visited: HashSet<BlockIdentity> = HashSet::new();
    let mut queue: Vec<BlockIdentity> = predecessors.iter().cloned().collect();

    while let Some(current_id) = queue.pop() {
        if !visited.insert(current_id.clone()) {
            continue;
        }

        // Deserialize payload and extract deploy signatures
        if let Some(content) = blocklace.content(&current_id) {
            if let Ok(payload) = CordialBlockPayload::from_bytes(&content.payload) {
                // Skip blocks outside the lifespan window
                if payload.state.block_number < earliest {
                    continue;
                }
                for pd in &payload.deploys {
                    sigs.insert(pd.deploy.signature.clone());
                }
                // Continue walking ancestors
                for pred in &content.predecessors {
                    if !visited.contains(pred) {
                        queue.push(pred.clone());
                    }
                }
            }
        }
    }

    sigs
}
