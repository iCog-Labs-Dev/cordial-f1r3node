//! Runtime abstraction for executing deploys and producing state transitions.
//!
//! This module defines the `RuntimeManager` trait — a minimal interface the
//! blocklace needs from an execution engine. It does NOT embed f1r3node's
//! RSpace tuplespace or Rholang interpreter; real RSpace integration will
//! live in a separate crate (planned for Phase 3) that implements this trait.
//!
//! The mock implementation [`MockRuntime`] is deterministic and suitable for
//! tests: given the same pre-state hash and deploys, it always produces the
//! same post-state hash. Costs are computed from deploy code length as a
//! stand-in for phlogiston accounting.
//!
//! ## What the runtime is responsible for
//!
//! Given a pre-state hash and an ordered list of signed deploys, the runtime:
//! 1. Executes each deploy's code against the state identified by `pre_state_hash`
//! 2. Accumulates execution costs (phlogiston)
//! 3. Produces a new `post_state_hash` reflecting the updated state
//! 4. Classifies each deploy as processed (with cost) or rejected (with reason)
//! 5. Optionally runs system deploys (slash, close block)
//!
//! ## What the runtime is NOT responsible for
//!
//! - Block creation, signing, or propagation
//! - Validator set management (bonds are read from the input, not mutated)
//! - Deploy pool management or selection
//! - Consensus decisions (fork choice, finality)
//!
//! The runtime is a pure function: `(pre_state, deploys) -> (post_state, results)`.

use std::collections::HashMap;

use crate::execution::payload::{
    Bond, ProcessedDeploy, ProcessedSystemDeploy, RejectReason, RejectedDeploy, SignedDeploy,
};
use crate::types::NodeId;

/// Input to runtime execution for a single block.
#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    /// Hash of the state before executing this block's deploys.
    pub pre_state_hash: Vec<u8>,

    /// User deploys to execute, in order.
    pub deploys: Vec<SignedDeploy>,

    /// System deploys to execute (slash equivocators, close block).
    pub system_deploys: Vec<SystemDeployRequest>,

    /// Current validator bonds.
    pub bonds: Vec<Bond>,

    /// Block number being executed.
    pub block_number: u64,
}

/// A system-level operation to execute as part of a block.
#[derive(Debug, Clone)]
pub enum SystemDeployRequest {
    /// Slash an equivocator.
    Slash {
        validator: NodeId,
        invalid_block_hash: Vec<u8>,
    },
    /// Close the block (seal state transitions).
    CloseBlock,
}

impl SystemDeployRequest {
    /// Guard to ensure invalid_block_hash is exactly 32 bytes.
    ///
    /// f1r3node expects 32-byte block hashes (SHA-256/Blake2b).
    /// Call this before creating Slash requests to prevent invalid hashes.
    pub fn validate_invalid_block_hash(hash: &[u8]) -> Result<(), String> {
        if hash.len() != 32 {
            return Err(format!(
                "invalid_block_hash must be exactly 32 bytes, got {} bytes",
                hash.len()
            ));
        }
        Ok(())
    }
}

/// Output of runtime execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Hash of the state after executing this block's deploys.
    pub post_state_hash: Vec<u8>,

    /// Deploys that executed successfully (or failed during execution).
    pub processed_deploys: Vec<ProcessedDeploy>,

    /// Deploys that were rejected before execution.
    pub rejected_deploys: Vec<RejectedDeploy>,

    /// System deploys that were executed.
    pub system_deploys: Vec<ProcessedSystemDeploy>,

    /// Updated bonds after system deploys (slashing may modify them).
    pub new_bonds: Vec<Bond>,
}

/// Error from runtime operations.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeError {
    /// The requested pre-state hash is not known to the runtime.
    UnknownPreState,
    /// Execution crashed for a reason unrelated to any specific deploy.
    InternalError(String),
}

/// Abstract runtime interface.
///
/// f1r3node's `RuntimeManager` has many concerns (RSpace replay, history
/// exporter, checkpoint lifecycle). This trait captures only what the
/// blocklace consensus layer needs: compute a block's state transition.
pub trait RuntimeManager {
    /// Execute a block's deploys against the given pre-state.
    ///
    /// Returns the resulting post-state hash and per-deploy outcomes.
    /// This should be deterministic: the same input produces the same output.
    fn execute_block(&mut self, request: ExecutionRequest)
    -> Result<ExecutionResult, RuntimeError>;

    /// Validate that a block's declared post-state hash matches what the
    /// runtime produces when replaying the block's deploys.
    ///
    /// Returns `Ok(true)` if valid, `Ok(false)` if the hashes disagree.
    /// Replaces f1r3node's `InvalidTransaction` validation step.
    fn validate_post_state(
        &mut self,
        request: ExecutionRequest,
        declared_post_state_hash: &[u8],
    ) -> Result<bool, RuntimeError> {
        let result = self.execute_block(request)?;
        Ok(result.post_state_hash == declared_post_state_hash)
    }
}

/// Deterministic mock runtime for testing.
///
/// Produces predictable post-state hashes from inputs via SHA-256 over
/// `(pre_state_hash, deploy_signatures, bond_summary, block_number)`.
/// Costs are derived from deploy code length (1 phlo per byte).
///
/// This is a stand-in for real RSpace execution. It does NOT actually
/// interpret Rholang; it just models the state-transition contract so
/// consensus code can be exercised end-to-end.
pub struct MockRuntime {
    /// Known pre-state hashes, populated as blocks are executed. In strict
    /// mode the next block's pre-state must match a prior post-state.
    known_states: std::collections::HashSet<Vec<u8>>,

    /// When true, any pre-state hash is accepted. Useful for tests that
    /// execute blocks out of order or don't care about state chaining.
    permissive: bool,

    /// Per-validator slashed stake (decorative — tracks running total).
    slashed: HashMap<NodeId, u64>,
}

impl MockRuntime {
    /// Create a new mock runtime in strict mode. The genesis pre-state
    /// (`vec![]`) is pre-registered so the first block has a valid starting
    /// point; subsequent blocks must use the prior block's post-state as
    /// their pre-state.
    pub fn new() -> Self {
        let mut known_states = std::collections::HashSet::new();
        known_states.insert(vec![]); // genesis
        Self {
            known_states,
            permissive: false,
            slashed: HashMap::new(),
        }
    }

    /// Create a mock runtime that accepts any pre-state hash.
    pub fn permissive() -> Self {
        Self {
            known_states: std::collections::HashSet::new(),
            permissive: true,
            slashed: HashMap::new(),
        }
    }

    fn is_permissive(&self) -> bool {
        self.permissive
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeManager for MockRuntime {
    fn execute_block(
        &mut self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, RuntimeError> {
        // 1. Check pre-state is known (unless permissive)
        if !self.is_permissive() && !self.known_states.contains(&request.pre_state_hash) {
            return Err(RuntimeError::UnknownPreState);
        }

        // 2. Classify deploys
        let mut processed = Vec::new();
        let mut rejected = Vec::new();

        for deploy in &request.deploys {
            // Reject deploys with empty signatures (degenerate case)
            if deploy.signature.is_empty() {
                rejected.push(RejectedDeploy {
                    deploy: deploy.clone(),
                    reason: RejectReason::InvalidSignature,
                });
                continue;
            }

            // "Cost" = 1 phlo per byte of deploy term, capped by phlo_limit
            let natural_cost = deploy.deploy.term.len() as u64;
            let cost = natural_cost.min(deploy.deploy.phlo_limit);
            // Simulate failure if phlo limit is exceeded
            let is_failed = natural_cost > deploy.deploy.phlo_limit;

            processed.push(ProcessedDeploy {
                deploy: deploy.clone(),
                cost,
                is_failed,
            });
        }

        // 3. Execute system deploys and apply bond effects
        let mut new_bonds = request.bonds.clone();
        let mut system_results = Vec::new();

        for sd in &request.system_deploys {
            match sd {
                SystemDeployRequest::Slash {
                    validator,
                    invalid_block_hash: _,
                } => {
                    // Remove the slashed validator's bond
                    let prior_stake = new_bonds
                        .iter()
                        .find(|b| &b.validator == validator)
                        .map(|b| b.stake)
                        .unwrap_or(0);
                    new_bonds.retain(|b| &b.validator != validator);
                    *self.slashed.entry(validator.clone()).or_insert(0) += prior_stake;
                    system_results.push(ProcessedSystemDeploy::Slash {
                        validator: validator.clone(),
                        succeeded: prior_stake > 0,
                    });
                }
                SystemDeployRequest::CloseBlock => {
                    system_results.push(ProcessedSystemDeploy::CloseBlock { succeeded: true });
                }
            }
        }

        // 4. Derive post-state hash deterministically from inputs + outputs
        let post_state_hash = compute_mock_post_state(
            &request.pre_state_hash,
            &processed,
            &system_results,
            &new_bonds,
            request.block_number,
        );

        // Remember this post-state for subsequent blocks
        self.known_states.insert(post_state_hash.clone());

        Ok(ExecutionResult {
            post_state_hash,
            processed_deploys: processed,
            rejected_deploys: rejected,
            system_deploys: system_results,
            new_bonds,
        })
    }
}

/// Deterministic post-state hash computation for the mock runtime.
///
/// Hashes a canonical representation of (pre-state, processed deploys,
/// system deploys, bonds, block number) using SHA-256. The same inputs
/// always produce the same output.
fn compute_mock_post_state(
    pre_state_hash: &[u8],
    processed: &[ProcessedDeploy],
    system: &[ProcessedSystemDeploy],
    bonds: &[Bond],
    block_number: u64,
) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update((pre_state_hash.len() as u64).to_le_bytes());
    hasher.update(pre_state_hash);
    hasher.update(block_number.to_le_bytes());

    hasher.update((processed.len() as u64).to_le_bytes());
    for pd in processed {
        hasher.update((pd.deploy.signature.len() as u64).to_le_bytes());
        hasher.update(&pd.deploy.signature);
        hasher.update(pd.cost.to_le_bytes());
        hasher.update([pd.is_failed as u8]);
    }

    hasher.update((system.len() as u64).to_le_bytes());
    for sd in system {
        match sd {
            ProcessedSystemDeploy::Slash {
                validator,
                succeeded,
            } => {
                hasher.update([0u8]); // tag
                hasher.update((validator.0.len() as u64).to_le_bytes());
                hasher.update(&validator.0);
                hasher.update([*succeeded as u8]);
            }
            ProcessedSystemDeploy::CloseBlock { succeeded } => {
                hasher.update([1u8]);
                hasher.update([*succeeded as u8]);
            }
        }
    }

    // Sort bonds for deterministic ordering regardless of caller order
    let mut sorted_bonds = bonds.to_vec();
    sorted_bonds.sort_by(|a, b| a.validator.0.cmp(&b.validator.0));
    hasher.update((sorted_bonds.len() as u64).to_le_bytes());
    for b in &sorted_bonds {
        hasher.update((b.validator.0.len() as u64).to_le_bytes());
        hasher.update(&b.validator.0);
        hasher.update(b.stake.to_le_bytes());
    }

    hasher.finalize().to_vec()
}
